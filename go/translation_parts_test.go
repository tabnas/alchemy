// Copyright (c) 2026 tabnas, MIT License

// Fleet conformance for the package-local structural translation interface.
// The adapters below deliberately read every package's fields directly: that
// is the compile-time contract without introducing a shared package type.

package tabnasalchemy

import (
	"encoding/json"
	"fmt"
	"sort"
	"strings"
	"testing"

	tabnascsv "github.com/tabnas/csv/go"
	tabnasini "github.com/tabnas/ini/go"
	tabnasjson "github.com/tabnas/json/go"
	tabnasjson5 "github.com/tabnas/json5/go"
	tabnasjsonc "github.com/tabnas/jsonc/go"
	tabnasjsonic "github.com/tabnas/jsonic/go"
	tabnasjsonl "github.com/tabnas/jsonl/go"
	tabnasmarkdown "github.com/tabnas/markdown/go"
	tabnas "github.com/tabnas/parser/go"
	tabnastoml "github.com/tabnas/toml/go"
	tt "github.com/tabnas/transduce/go"
	tabnasxml "github.com/tabnas/xml/go"
	tabnasyaml "github.com/tabnas/yaml/go"
	tabnaszon "github.com/tabnas/zon/go"
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

type translationDescriptor struct {
	LanguageID string `json:"languageId"`
	Translate  struct {
		Reads  any     `json:"reads"`
		Writes string  `json:"writes"`
		Lift   *string `json:"lift"`
		Render string  `json:"render"`
	} `json:"translate"`
}

func localPart(entry, source string) *structuralPart {
	return &structuralPart{entry: entry, source: source}
}

func packagePart(present bool, fields func() (string, string)) *structuralPart {
	if !present {
		return nil
	}
	entry, source := fields()
	return localPart(entry, source)
}

// grammarParts adapts every package-local declaration by direct field access.
// A renamed or missing field therefore fails this test at compile time.
func grammarParts() map[string]structuralParts {
	csv := tabnascsv.Translate()
	ini := tabnasini.Translate()
	jsonGrammar := tabnasjson.Translate()
	json5 := tabnasjson5.Translate()
	jsonc := tabnasjsonc.Translate()
	jsonic := tabnasjsonic.Translate()
	jsonl := tabnasjsonl.Translate()
	markdown := tabnasmarkdown.Translate()
	toml := tabnastoml.Translate()
	xml := tabnasxml.Translate()
	yaml := tabnasyaml.Translate()
	zon := tabnaszon.Translate()
	return map[string]structuralParts{
		"csv": {
			csv.Manifest,
			packagePart(csv.Lift != nil, func() (string, string) { return csv.Lift.Entry, csv.Lift.Source }),
			packagePart(csv.Render != nil, func() (string, string) { return csv.Render.Entry, csv.Render.Source }),
		},
		"ini": {
			ini.Manifest,
			packagePart(ini.Lift != nil, func() (string, string) { return ini.Lift.Entry, ini.Lift.Source }),
			packagePart(ini.Render != nil, func() (string, string) { return ini.Render.Entry, ini.Render.Source }),
		},
		"json": {
			jsonGrammar.Manifest,
			packagePart(jsonGrammar.Lift != nil, func() (string, string) { return jsonGrammar.Lift.Entry, jsonGrammar.Lift.Source }),
			packagePart(jsonGrammar.Render != nil, func() (string, string) { return jsonGrammar.Render.Entry, jsonGrammar.Render.Source }),
		},
		"json5": {
			json5.Manifest,
			packagePart(json5.Lift != nil, func() (string, string) { return json5.Lift.Entry, json5.Lift.Source }),
			packagePart(json5.Render != nil, func() (string, string) { return json5.Render.Entry, json5.Render.Source }),
		},
		"jsonc": {
			jsonc.Manifest,
			packagePart(jsonc.Lift != nil, func() (string, string) { return jsonc.Lift.Entry, jsonc.Lift.Source }),
			packagePart(jsonc.Render != nil, func() (string, string) { return jsonc.Render.Entry, jsonc.Render.Source }),
		},
		"jsonic": {
			jsonic.Manifest,
			packagePart(jsonic.Lift != nil, func() (string, string) { return jsonic.Lift.Entry, jsonic.Lift.Source }),
			packagePart(jsonic.Render != nil, func() (string, string) { return jsonic.Render.Entry, jsonic.Render.Source }),
		},
		"jsonl": {
			jsonl.Manifest,
			packagePart(jsonl.Lift != nil, func() (string, string) { return jsonl.Lift.Entry, jsonl.Lift.Source }),
			packagePart(jsonl.Render != nil, func() (string, string) { return jsonl.Render.Entry, jsonl.Render.Source }),
		},
		"markdown": {
			markdown.Manifest,
			packagePart(markdown.Lift != nil, func() (string, string) { return markdown.Lift.Entry, markdown.Lift.Source }),
			packagePart(markdown.Render != nil, func() (string, string) { return markdown.Render.Entry, markdown.Render.Source }),
		},
		"toml": {
			toml.Manifest,
			packagePart(toml.Lift != nil, func() (string, string) { return toml.Lift.Entry, toml.Lift.Source }),
			packagePart(toml.Render != nil, func() (string, string) { return toml.Render.Entry, toml.Render.Source }),
		},
		"xml": {
			xml.Manifest,
			packagePart(xml.Lift != nil, func() (string, string) { return xml.Lift.Entry, xml.Lift.Source }),
			packagePart(xml.Render != nil, func() (string, string) { return xml.Render.Entry, xml.Render.Source }),
		},
		"yaml": {
			yaml.Manifest,
			packagePart(yaml.Lift != nil, func() (string, string) { return yaml.Lift.Entry, yaml.Lift.Source }),
			packagePart(yaml.Render != nil, func() (string, string) { return yaml.Render.Entry, yaml.Render.Source }),
		},
		"zon": {
			zon.Manifest,
			packagePart(zon.Lift != nil, func() (string, string) { return zon.Lift.Entry, zon.Lift.Source }),
			packagePart(zon.Render != nil, func() (string, string) { return zon.Render.Entry, zon.Render.Source }),
		},
	}
}

func declaredShapes(t *testing.T, format string, value any) []string {
	t.Helper()
	switch value := value.(type) {
	case string:
		return []string{value}
	case []any:
		out := make([]string, len(value))
		for i, item := range value {
			shape, ok := item.(string)
			if !ok {
				t.Fatalf("%s read shape %d is %#v", format, i, item)
			}
			out[i] = shape
		}
		return out
	default:
		t.Fatalf("%s reads is %#v", format, value)
		return nil
	}
}

func assertStructuralPart(t *testing.T, format, kind string, declared *string, part *structuralPart) {
	t.Helper()
	if declared == nil {
		if part != nil {
			t.Fatalf("%s exposes an undeclared %s", format, kind)
		}
		return
	}
	if part == nil || part.entry == "" {
		t.Fatalf("%s does not expose its declared %s with an explicit entry", format, kind)
	}
	if strings.HasSuffix(*declared, ".alc") {
		if part.source == "" {
			t.Fatalf("%s %s does not embed %s", format, kind, *declared)
		}
	} else if part.entry != *declared || part.source != "" {
		t.Fatalf("%s builtin %s is entry %q with source length %d", format, kind, part.entry, len(part.source))
	}
}

func grammarParser(t *testing.T, format string) *tabnas.Tabnas {
	t.Helper()
	switch format {
	case "csv":
		parser, err := tabnascsv.Make()
		if err != nil {
			t.Fatal(err)
		}
		return parser
	case "ini":
		return tabnasini.MakeJsonic()
	case "json":
		return tabnasjson.Make()
	case "json5":
		parser := tabnasjsonic.Make()
		if err := parser.UseDefaults(tabnasjson5.Json5, tabnasjson5.Defaults()); err != nil {
			t.Fatal(err)
		}
		return parser
	case "jsonc":
		parser := tabnasjsonic.Make()
		if err := parser.Use(tabnasjsonc.Jsonc); err != nil {
			t.Fatal(err)
		}
		return parser
	case "jsonic":
		return tabnasjsonic.Make()
	case "jsonl":
		return tabnasjsonl.Make()
	case "markdown":
		return tabnasmarkdown.Make()
	case "toml":
		return tabnastoml.MakeJsonic()
	case "xml":
		parser := tabnasjsonic.Make()
		if err := parser.Use(tabnasxml.Xml); err != nil {
			t.Fatal(err)
		}
		return parser
	case "yaml":
		return tabnasyaml.MakeJsonic()
	case "zon":
		return tabnaszon.MakeJsonic()
	default:
		t.Fatalf("no parser for %s", format)
		return nil
	}
}

func conformanceMain(parts structuralParts, descriptor translationDescriptor) string {
	if parts.lift != nil {
		return fmt.Sprintf("def export [input]\n  %s (%s (events input))\n", parts.render.entry, parts.lift.entry)
	}
	if descriptor.Translate.Writes == "records" {
		call := fmt.Sprintf("%s (table-from-json conformance-binding input)", parts.render.entry)
		if parts.render.entry == "csv" {
			call = "csv csv-options (table-from-json conformance-binding input)"
		}
		return "def conformance-binding\n  record\n    entry :columns :infer\n" +
			"    entry :rows (path each-index)\n\n" +
			fmt.Sprintf("def export [input]\n  %s\n", call)
	}
	return fmt.Sprintf("def export [input]\n  %s (events input)\n", parts.render.entry)
}

// orderedTranslationValue gives schemas whose render is streaming the member
// order their package contract defines. Go maps carry no order; the parser
// value is otherwise unchanged, and OrderedMap is the engine's ordered object
// representation used by the event source.
func orderedTranslationValue(format string, value any) any {
	switch value := value.(type) {
	case []any:
		out := make([]any, len(value))
		for i, item := range value {
			out[i] = orderedTranslationValue(format, item)
		}
		return out
	case *tabnas.OrderedMap:
		out := tabnas.NewOrderedMap()
		for _, key := range value.Keys {
			out.Set(key, orderedTranslationValue(format, value.Vals[key]))
		}
		return out
	case map[string]any:
		keys := make([]string, 0, len(value))
		for key := range value {
			keys = append(keys, key)
		}
		sort.Strings(keys)
		preferred := []string{}
		switch format {
		case "markdown":
			preferred = []string{"type", "align", "depth", "value", "url", "title", "children"}
		case "xml":
			preferred = []string{"name", "localName", "attributes", "children"}
		}
		out := tabnas.NewOrderedMap()
		placed := map[string]bool{}
		for _, key := range preferred {
			if item, ok := value[key]; ok {
				out.Set(key, orderedTranslationValue(format, item))
				placed[key] = true
			}
		}
		for _, key := range keys {
			if !placed[key] {
				out.Set(key, orderedTranslationValue(format, value[key]))
			}
		}
		return out
	default:
		return value
	}
}

func conformanceEvents(t *testing.T, format, text string) []tt.Event {
	t.Helper()
	value, err := grammarParser(t, format).Parse(text)
	if err != nil {
		t.Fatalf("%s does not parse %q: %v", format, text, err)
	}
	var recorder tt.Recorder
	if _, fail := (tt.ValueSource{Value: orderedTranslationValue(format, value)}).Run(&recorder); fail != nil {
		t.Fatal(fail)
	}
	return recorder.Events
}

func TestStructuralTranslationParts(t *testing.T) {
	shapes := map[string]bool{"records": true, "tree": true}
	samples := map[string]string{
		"csv":      "a,b\n1,x\n2,y\n",
		"ini":      "a=1\nb=x\n",
		"json":     "{\"a\":1,\"b\":[true,null]}\n",
		"json5":    "{a:1,b:['x',true]}\n",
		"jsonc":    "{\"a\":1,/* c */\"b\":true}\n",
		"jsonic":   "a:1,b:true",
		"jsonl":    "{\"a\":1}\n{\"a\":2}\n",
		"markdown": "| a | b |\n| - | - |\n| 1 | x |\n",
		"toml":     "a = 1\nb = \"x\"\n",
		"xml":      "<a x=\"1\">text</a>",
		"yaml":     "a: 1\nb:\n  - x\n  - y\n",
		"zon":      ".{ .a = 1, .b = .{ true, false } }",
	}
	for format, parts := range grammarParts() {
		t.Run(format, func(t *testing.T) {
			var descriptor translationDescriptor
			if err := json.Unmarshal([]byte(parts.manifest), &descriptor); err != nil {
				t.Fatal(err)
			}
			if descriptor.LanguageID != format {
				t.Fatalf("languageId is %q", descriptor.LanguageID)
			}
			reads := declaredShapes(t, format, descriptor.Translate.Reads)
			if len(reads) == 0 {
				t.Fatal("no read shape")
			}
			for _, shape := range reads {
				if !shapes[shape] {
					t.Fatalf("unknown read shape %q", shape)
				}
			}
			if !shapes[descriptor.Translate.Writes] {
				t.Fatalf("unknown write shape %q", descriptor.Translate.Writes)
			}
			assertStructuralPart(t, format, "lift", descriptor.Translate.Lift, parts.lift)
			render := descriptor.Translate.Render
			assertStructuralPart(t, format, "render", &render, parts.render)

			sources := []Source{{File: format + "/conformance.alc", Text: conformanceMain(parts, descriptor)}}
			for _, item := range []struct {
				path *string
				part *structuralPart
			}{
				{descriptor.Translate.Lift, parts.lift},
				{&render, parts.render},
			} {
				if item.part == nil || item.part.source == "" {
					continue
				}
				sources = append(sources, Source{File: *item.path, Text: item.part.source})
			}
			program, fail := CompileSources(sources)
			if fail != nil {
				t.Fatal(fail)
			}
			for kind, part := range map[string]*structuralPart{"lift": parts.lift, "render": parts.render} {
				if part != nil && part.source != "" && program.Resolved().Get(part.entry) == nil {
					t.Fatalf("%s source does not define %s", kind, part.entry)
				}
			}

			first := conformanceEvents(t, format, samples[format])
			rendered, runFail := replayEvents(program, first, RenderDefault, tt.DefaultLimits(), tt.NewMetrics())
			if runFail != nil {
				t.Fatalf("translation parts do not render: %v", runFail)
			}
			second := conformanceEvents(t, format, rendered)
			rerendered, runFail := replayEvents(program, second, RenderDefault, tt.DefaultLimits(), tt.NewMetrics())
			if runFail != nil {
				t.Fatalf("the second render fails: %v", runFail)
			}
			if rerendered != rendered {
				t.Fatalf("render is not round-trip stable:\nfirst:  %q\nsecond: %q", rendered, rerendered)
			}
		})
	}
}
