// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

import (
	"fmt"
)

// frontend.go: the front end of compiling a program, one source or
// several linked into one namespace (the first half of
// rs/src/program.rs's compile_sources): read, desugar, link, resolve and
// check. What it answers, an Analysis, is what building the plan starts
// from: the interpreter evaluates export over the resolved definitions,
// positioning its failures through the same Sources, and the checker's
// Output says what the plan's result is rendered as.

// Source is one of the sources a program is compiled from: the file it is
// named by in diagnostics, its text, and the name its own export is linked
// under when another source's export is the program's (ExportAs, "" for
// none).
type Source struct {
	File     string
	Text     string
	ExportAs string
}

// Analysis is a program through the front end: its resolved definitions,
// the sources its spans are positioned in, and what the checker learned.
type Analysis struct {
	Resolved *Resolved
	Sources  *Sources
	Checked  *Checked
}

// Analyze runs the front end over src, named file in diagnostics.
func Analyze(src, file string) (*Analysis, *Fail) {
	return AnalyzeSources([]Source{{File: file, Text: src}})
}

// AnalyzeSources runs the front end over a program linked from several
// sources: a definition in any of them is in scope in all, export is
// defined in one, and a name defined twice, in one file or across two, is
// `duplicate_def`. Each source is named by its file, and the names are
// distinct (`duplicate_file` otherwise). A failure in a program of several
// sources carries the file its position is in.
//
// A source whose export is not the program's is linked with ExportAs: its
// export is defined under that name instead, so that the program's export
// can call it.
func AnalyzeSources(sources []Source) (*Analysis, *Fail) {
	names := make([]string, 0, len(sources))
	texts := make([]string, 0, len(sources))
	for _, s := range sources {
		for _, n := range names {
			if n == s.File {
				return nil, NewFail(CodeDSLTypeError,
					fmt.Sprintf("duplicate_file: %s is given twice; each source has its own name", s.File))
			}
		}
		names = append(names, s.File)
		texts = append(texts, s.Text)
	}
	if len(sources) == 0 {
		return nil, noExport()
	}
	linked := severalSources(names, texts)
	var forms []*Expr
	for _, s := range sources {
		// The reader and the desugarer see one file at a time, so the
		// file is added here; the stages after them position through
		// linked.
		inFile := func(f *Fail) *Fail {
			if linked.NamesFiles() {
				f.InFile(s.File)
			}
			return f
		}
		parsed, f := ParseFile(s.Text, s.File)
		if f != nil {
			return nil, inFile(f)
		}
		own, f := Desugar(parsed, s.Text)
		if f != nil {
			return nil, inFile(f)
		}
		if s.ExportAs != "" {
			name := s.ExportAs
			defined := false
			for _, form := range own {
				if defines(form, "export") {
					defined = true
					break
				}
			}
			if !defined {
				return nil, inFile(NewFail(CodeDSLTypeError,
					fmt.Sprintf("no_export: %s defines no export to link as %s", s.File, name)))
			}
			// Renaming every export is a consistent renaming only while
			// nothing in the source is already called name.
			for _, form := range own {
				if taken, ok := mentions(form, name); ok {
					row, col := linked.Position(taken)
					return nil, inFile(NewFail(CodeDSLTypeError,
						fmt.Sprintf("duplicate_def: %s already names %s, so its export cannot be linked under it; link it under another name", s.File, name)).
						At(uint64(row), uint64(col)))
				}
			}
			for _, form := range own {
				renameSymbol(form, "export", name)
			}
		}
		forms = append(forms, own...)
	}
	resolved, f := Resolve(forms, linked, Outer)
	if f != nil {
		return nil, f
	}
	checked, f := CheckProgram(resolved, linked)
	if f != nil {
		return nil, f
	}
	return &Analysis{Resolved: resolved, Sources: linked, Checked: checked}, nil
}

// defines is whether form is a top-level def of name, as the desugarer
// leaves one: `(def name value)`.
func defines(form *Expr, name string) bool {
	return form.Kind == ExprList && len(form.Items) == 3 &&
		form.Items[0].IsSymbol("def") && form.Items[1].IsSymbol(name)
}

// mentions is the span of the first symbol name in form, when it mentions
// one.
func mentions(form *Expr, name string) (SourceSpan, bool) {
	switch form.Kind {
	case ExprSymbol:
		if form.Text == name {
			return form.Span, true
		}
	case ExprList, ExprVector:
		for _, item := range form.Items {
			if span, ok := mentions(item, name); ok {
				return span, true
			}
		}
	}
	return SourceSpan{}, false
}

// renameSymbol makes every symbol from in form to: the name a def binds
// and every mention of it alike.
func renameSymbol(form *Expr, from, to string) {
	switch form.Kind {
	case ExprSymbol:
		if form.Text == from {
			form.Text = to
		}
	case ExprList, ExprVector:
		for _, item := range form.Items {
			renameSymbol(item, from, to)
		}
	}
}
