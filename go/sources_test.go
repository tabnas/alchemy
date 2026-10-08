// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// sources_test.go: CompileSources (rs/tests/sources_test.rs): several
// sources linked into one program, as a host links a format's parts
// (libraries of definitions prefixed by the format's name, with no export)
// with the program that calls them. The program runs as the same text in
// one file runs; a failure in any source carries that source's file, row
// and column, in the field and in the failure's display, at every stage
// that positions one: the reader, the desugarer, the resolver and the
// checker here; the linked program's run, and a failure the run meets in a
// part, are alchemy-cli's (go/e2e/sources_test.go), since a run needs
// transduce's routers and render's renderers.

import (
	"encoding/json"
	"fmt"
	"reflect"
	"strings"
	"testing"
	"unicode/utf8"
)

// part is a render part: definitions prefixed by the format's name, no
// export.
const part = "; A part of the lines format: each item quoted, one to a line.\ndef lines-line [item]\n  concat (quoted item) \"\\n\"\n\ndef lines-render [items]\n  concat-map lines-line items\n"

const partFile = "lines/render.alc"

// mainSrc is the program that calls the part.
const mainSrc = "def export [input]\n  lines-render (select (path each-index) input)\n"

const mainFile = "main.alc"

func sourcesOf(list ...[2]string) []Source {
	out := make([]Source, len(list))
	for i, s := range list {
		out[i] = Source{File: s[0], Text: s[1]}
	}
	return out
}

// linked is the program with the part second, as the design's diagnostics
// test has it.
func linked(partText string) (*Program, *Fail) {
	return CompileSources(sourcesOf([2]string{mainFile, mainSrc}, [2]string{partFile, partText}), routers, renderers)
}

// at is the 1-based row and column of needle on the 1-based row of text,
// as a failure carries them.
func at(t testing.TB, text string, row int, needle string) (uint64, uint64) {
	t.Helper()
	line := strings.Split(text, "\n")[row-1]
	i := strings.Index(line, needle)
	if i < 0 {
		t.Fatalf("%q is not on row %d", needle, row)
	}
	return uint64(row), uint64(utf8.RuneCountInString(line[:i]) + 1)
}

// assertAt: the failure names file and the position (row, column), in the
// fields, in its display, and in its JSON.
func assertAt(t testing.TB, f *Fail, file string, row, column uint64) {
	t.Helper()
	if f.File != file || f.Row != row || f.Column != column {
		t.Errorf("%v: want %s:%d:%d", f, file, row, column)
	}
	if shown := fmt.Sprintf("(%s:%d:%d)", file, row, column); !strings.HasSuffix(f.Error(), shown) {
		t.Errorf("%v does not end %s", f, shown)
	}
	data, _ := f.MarshalJSON()
	var j map[string]any
	if err := json.Unmarshal(data, &j); err != nil || j["file"] != file {
		t.Errorf("%s", data)
	}
}

// Every stage that positions a failure positions one in the second source
// by that source's own rows and columns, and names its file.
func TestAFailureInTheSecondSourceNamesTheSecondFile(t *testing.T) {
	// The resolver: a name nothing defines.
	typo := strings.Replace(part, "(quoted item)", "(quoted itme)", 1)
	_, f := linked(typo)
	if f == nil || f.Code != CodeDSLTypeError || !strings.HasPrefix(f.Message, "unknown_name: ") {
		t.Fatalf("%v", f)
	}
	row, col := at(t, typo, 3, "itme")
	assertAt(t, f, partFile, row, col)

	// The checker: an argument of the wrong type.
	number := strings.Replace(part, "(quoted item)", "(quoted 1)", 1)
	_, f = linked(number)
	if f == nil || f.Code != CodeDSLTypeError || !strings.HasPrefix(f.Message, "type_mismatch: ") {
		t.Fatalf("%v", f)
	}
	row, col = at(t, number, 3, "1)")
	assertAt(t, f, partFile, row, col)

	// The checker, following a stream into the part: a render handed the
	// wrong shape fails at the render's file and line, not at the call.
	csvPart := "def lines-render [items]\n  csv csv-options items\n"
	events := "def export [input]\n  lines-render (events input)\n"
	_, f = CompileSources(sourcesOf([2]string{mainFile, events}, [2]string{partFile, csvPart}), routers, renderers)
	if f == nil || f.Code != CodeDSLTypeError || !strings.HasPrefix(f.Message, "protocol_mismatch: ") || f.File != partFile || f.Row != 2 {
		t.Errorf("%v", f)
	}

	// The reader: a list left open.
	open := strings.Replace(part, "(quoted item)", "(quoted item", 1)
	_, f = linked(open)
	if f == nil || f.Code != CodeDSLParseError || f.File != partFile || f.Row == 0 || !strings.Contains(f.Error(), "("+partFile+":") {
		t.Errorf("%v", f)
	}

	// The desugarer: a pipe step that is no call.
	pipe := strings.Replace(part, "concat-map lines-line items", "pipe items ()", 1)
	_, f = linked(pipe)
	if f == nil || f.Code != CodeDSLParseError || !strings.HasPrefix(f.Message, "empty_step: ") || f.File != partFile || f.Row != 6 {
		t.Errorf("%v", f)
	}
}

// The linking's own refusals: a name defined in two sources, the one file
// name given twice, and no export in any source.
func TestTheLinkingRefusesACollisionAndAProgramWithNoExport(t *testing.T) {
	// A definition in both: the second is refused, and the message says
	// where the first is.
	twice := part + "\ndef export [input] (json input)\n"
	_, f := linked(twice)
	if f == nil || f.Code != CodeDSLTypeError || !strings.HasPrefix(f.Message, "duplicate_def: export is defined twice; first at main.alc:1:1") {
		t.Fatalf("%v", f)
	}
	row, col := at(t, twice, 8, "def export")
	assertAt(t, f, partFile, row, col)

	// Each source has its own name.
	_, f = CompileSources(sourcesOf([2]string{mainFile, mainSrc}, [2]string{mainFile, part}), routers, renderers)
	if f == nil || f.Code != CodeDSLTypeError || !strings.HasPrefix(f.Message, "duplicate_file: main.alc ") || f.File != "" || f.Row != 0 {
		t.Errorf("%v", f)
	}

	// A part alone is no program.
	if _, f := CompileSources(sourcesOf([2]string{partFile, part}), routers, renderers); f == nil || !strings.HasPrefix(f.Message, "no_export: ") {
		t.Errorf("%v", f)
	}
	if _, f := CompileSources(nil, routers, renderers); f == nil || !strings.HasPrefix(f.Message, "no_export: ") {
		t.Errorf("%v", f)
	}
}

// One source is Compile: its failures carry a row and a column and no
// file, and display as they always have.
func TestOneSourceNamesNoFile(t *testing.T) {
	text := "def export [input]\n  csv csv-options (events input)\n"
	_, f := CompileSources(sourcesOf([2]string{"one.alc", text}), routers, renderers)
	_, same := Compile(text, "one.alc", routers, renderers)
	if f == nil || same == nil || !reflect.DeepEqual(*f, *same) {
		t.Fatalf("%v %v", f, same)
	}
	if f.File != "" || f.Row == 0 {
		t.Errorf("%v", f)
	}
	data, _ := f.MarshalJSON()
	var j map[string]any
	if err := json.Unmarshal(data, &j); err != nil {
		t.Fatal(err)
	}
	if _, has := j["file"]; has {
		t.Errorf("%s", data)
	}
	if shown := fmt.Sprintf("(%d:%d)", f.Row, f.Column); !strings.HasSuffix(f.Error(), shown) {
		t.Errorf("%v", f)
	}
}
