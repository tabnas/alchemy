// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

import (
	"embed"
	"fmt"
	"sync"
)

// stdlib.go: the standard library's own definitions, written in alchemy
// (rs/src/stdlib/mod.rs).
//
// The module embeds its copies under go/stdlib/*.alc, because a published
// Go module holds nothing above its own root; `go run ./internal/embed`
// writes them from the canonical stdlib/*.alc, and
// shared_sources_test.go holds each copy byte for byte to its file.
//
// table.alc is the metadata-first table transducer and csv.alc the
// always-quoted CSV renderer. Both are parsed, desugared, resolved and
// checked once, on first use, and their names are usable from any
// program; a program's own def of the same name shadows the library's for
// that program, never for the library itself.

//go:embed stdlib/*.alc
var stdlibFS embed.FS

// StdlibFiles are the embedded sources, by the file name their spans
// carry, in load order (the order the Rust crate's SOURCES lists them).
// TestTheEmbeddedStdlibIsEveryFile holds the list to the directory.
var StdlibFiles = []string{"stdlib/table.alc", "stdlib/csv.alc"}

// StdlibSource is the source text of an embedded file, by the name its
// spans carry.
func StdlibSource(file string) (string, bool) {
	for _, name := range StdlibFiles {
		if name == file {
			data, err := stdlibFS.ReadFile(file)
			if err != nil {
				return "", false
			}
			return string(data), true
		}
	}
	return "", false
}

// Stdlib is the library, loaded: each file's resolved definitions, and
// all of them by name.
type Stdlib struct {
	// Files are one entry per source file, in StdlibFiles order.
	Files []*Resolved
	// Defs are every definition, by name; Order is their file order.
	Defs  map[string]*Def
	Order []string
}

// Get is the library definition name, or nil.
func (s *Stdlib) Get(name string) *Def { return s.Defs[name] }

// Names are the names the library defines, in file order.
func (s *Stdlib) Names() []string { return append([]string(nil), s.Order...) }

// nativeKind is the kind of a native, as the resolver classifies names.
func nativeKind(name string) NameKind {
	n := LookupNative(name)
	if n == nil {
		return NameNone
	}
	switch n.Kind {
	case KindConstant:
		return NameConstant
	case KindConstructor:
		return NameConstructor
	}
	return NameFunction
}

// LoadStdlib parses, desugars, resolves and checks the embedded sources.
// Each file resolves against the natives and the other files'
// definitions; each definition is checked against the signature
// StdlibSignature declares for it.
func LoadStdlib() (*Stdlib, *Fail) {
	type parsed struct {
		file, src string
		forms     []*Expr
	}
	// Every definition's name first, so a reference across files resolves
	// whatever the file order.
	var all []parsed
	names := map[string]bool{}
	for _, file := range StdlibFiles {
		src, ok := StdlibSource(file)
		if !ok {
			return nil, NewFail(CodeDSLParseError, file+" is not embedded")
		}
		forms, f := ParseFile(src, file)
		if f != nil {
			return nil, f
		}
		if forms, f = Desugar(forms, src); f != nil {
			return nil, f
		}
		for _, form := range forms {
			if form.Kind == ExprList && len(form.Items) > 1 {
				if name, ok := form.Items[1].Symbol(); ok {
					names[name] = true
				}
			}
		}
		all = append(all, parsed{file, src, forms})
	}
	lib := &Stdlib{Defs: map[string]*Def{}}
	outer := func(name string) NameKind {
		if names[name] {
			return NameValue
		}
		return nativeKind(name)
	}
	for _, p := range all {
		resolved, f := Resolve(p.forms, OneSource(p.file, p.src), outer)
		if f != nil {
			return nil, f
		}
		if f := checkStdlibFile(resolved, p.src); f != nil {
			return nil, f
		}
		for _, name := range resolved.Order {
			if _, dup := lib.Defs[name]; dup {
				return nil, NewFail(CodeDSLTypeError,
					fmt.Sprintf("duplicate_def: %s is defined in two standard library files", name))
			}
			lib.Defs[name] = resolved.Defs[name]
			lib.Order = append(lib.Order, name)
		}
		lib.Files = append(lib.Files, resolved)
	}
	return lib, nil
}

var (
	stdlibOnce   sync.Once
	stdlibLoaded *Stdlib
)

// StdlibLoaded is the library, loaded once. The embedded text is part of
// this module, so a text that does not load is a defect of the build, not
// of any program.
func StdlibLoaded() *Stdlib {
	stdlibOnce.Do(func() {
		lib, f := LoadStdlib()
		if f != nil {
			panic("the embedded standard library does not load: " + f.Error())
		}
		stdlibLoaded = lib
	})
	return stdlibLoaded
}

// stdlibFileOf is the embedded file a span belongs to, as its name and its
// text, by identity: every span of one file shares the File the reader
// made, and a program's spans never share it, so a program that happens
// to be named stdlib/table.alc is not mistaken for the library file.
func stdlibFileOf(span SourceSpan) (string, string, bool) {
	lib := StdlibLoaded()
	for i, resolved := range lib.Files {
		if len(resolved.Order) == 0 {
			continue
		}
		if resolved.Defs[resolved.Order[0]].Span.File == span.File {
			src, _ := StdlibSource(StdlibFiles[i])
			return StdlibFiles[i], src, true
		}
	}
	return "", "", false
}

// Outer is what a program sees outside itself: the library's definitions
// and the natives.
func Outer(name string) NameKind {
	if StdlibLoaded().Get(name) != nil {
		return NameValue
	}
	return nativeKind(name)
}
