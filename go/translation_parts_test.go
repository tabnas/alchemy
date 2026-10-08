// Copyright (c) 2026 tabnas, MIT License

// The structural translation interface: a program composed with a format's
// render part, compiled from several sources. The fleet conformance that
// reads every grammar package's declared parts and runs them round trip
// is alchemy-cli's (go/e2e/translation_parts_test.go): it runs programs on
// transduce's routers and render's renderers, over documents the grammars
// read.

package tabnasalchemy

import (
	"fmt"
	"testing"
)

type structuralPart struct {
	entry  string
	source string
}

type structuralParts struct {
	manifest string
	lift     *structuralPart
	render   *structuralPart
}

func localPart(entry, source string) *structuralPart {
	return &structuralPart{entry: entry, source: source}
}

func conformanceMain(parts structuralParts, reads, writes string, lifted bool) (string, error) {
	// The producer is structural fact: only the preferred shape may come
	// from a lift; every other shape is the grammar's raw tree. The adapter
	// is selected from the descriptor, so a false reads value type-fails.
	source := "events input"
	if lifted {
		source = fmt.Sprintf("%s (events input)", parts.lift.entry)
	}
	definitions := ""
	adapted := ""
	switch {
	case reads == writes:
		adapted = source
	case reads == "tree" && writes == "records":
		definitions = "def conformance-binding\n  record\n    entry :columns :infer\n" +
			"    entry :rows (path each-index)\n\n"
		adapted = fmt.Sprintf("table-from-json conformance-binding (%s)", source)
	case reads == "records" && writes == "tree":
		adapted = fmt.Sprintf("records (%s)", source)
	case writes == "text":
		definitions = "def conformance-text [input]\n" +
			"  join \"\"\n" +
			"    map\n" +
			"      fn [event]\n" +
			"        match event\n" +
			"          case (scalar value) (scalar-text csv-options value)\n" +
			"          case _ \"\"\n" +
			"      input\n\n"
		adapted = fmt.Sprintf("conformance-text (%s)", source)
	default:
		return "", fmt.Errorf("no %s-to-%s conformance adapter", reads, writes)
	}
	call := fmt.Sprintf("%s (%s)", parts.render.entry, adapted)
	if parts.render.entry == "csv" {
		call = fmt.Sprintf("csv csv-options (%s)", adapted)
	}
	return definitions + fmt.Sprintf("def export [input]\n  %s\n", call), nil
}

func TestTextShapeComposition(t *testing.T) {
	parts := structuralParts{
		render: localPart("textual-render", "def textual-render [input]\n  input\n"),
	}
	main, err := conformanceMain(parts, "tree", "text", false)
	if err != nil {
		t.Fatal(err)
	}
	program, fail := CompileSources([]Source{
		{File: "conformance.alc", Text: main},
		{File: "alchemy/render.alc", Text: parts.render.source},
	}, routers, renderers)
	if fail != nil {
		t.Fatal(fail)
	}
	if program.Resolved().Get(parts.render.entry) == nil {
		t.Fatalf("render source does not define %s", parts.render.entry)
	}
}
