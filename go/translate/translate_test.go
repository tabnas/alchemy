// Copyright (c) 2026 tabnas, MIT License

package translate

// translate_test.go: the composition of a translation from the formats'
// parts (rs/src/translate.rs's tests and rs/tests/translate_test.rs): a
// descriptor read into a part, the main each route composes, word for word
// as the Rust crate composes it, and every route compiling, with the
// standard library's adapters, to a program whose output is a text. The
// parts here are stand-ins (a render that is json, an embed that hands its
// input on); running real parts needs transduce's routers and render's
// renderers, and is alchemy-cli's. Compiling asks nothing of the stages,
// so these tests pass none (alchemy.CompileSources takes nil for both).

import (
	"reflect"
	"strings"
	"testing"

	alchemy "github.com/tabnas/alchemy/go"
	"github.com/tabnas/alchemy/go/shared"
)

// The inferred table's binding and the CSV options, spelled out, so the
// mains below are held to the Rust crate's text byte for byte.
const (
	inferredText   = `(record (entry :columns :infer) (entry :rows (path each-index)))`
	csvOptionsText = `(record (entry :delimiter ",") (entry :newline "\r\n") (entry :header true) (entry :null-text "") (entry :missing "") (entry :non-finite :literal) (entry :no-columns :empty))`
)

func part(id, translate string, render *PartText) *Part {
	return PartFromDescriptor(Descriptor{
		Package:  "tabnas-" + id,
		Manifest: `{"languageId": "` + id + `", "translate": ` + translate + `}`,
		Render:   render,
	})
}

// own is a render of the format's own: json under another name.
func own(id string) *PartText {
	return NewPartText(id+"-render", "def "+id+"-render [input] (json input)")
}

func tree(t *testing.T, id, root string) *Part {
	t.Helper()
	p := part(id, `{"reads": "tree", "writes": "tree", "root": "`+root+`", "render": "alchemy/render.alc"}`, own(id))
	if p == nil {
		t.Fatalf("%s: no part", id)
	}
	return p
}

// full is a part as the integration tests make one: its render alchemy's
// csv or json when the manifest names one, else its own.
func full(t *testing.T, id, translate string, lift, embed *PartText) *Part {
	t.Helper()
	render := own(id)
	switch {
	case strings.Contains(translate, `"render": "csv"`):
		render = CarriedPartText("csv")
	case strings.Contains(translate, `"render": "json"`):
		render = CarriedPartText("json")
	}
	p := PartFromDescriptor(Descriptor{
		Package:  "tabnas-" + id,
		Manifest: `{"languageId": "` + id + `", "translate": ` + translate + `}`,
		Lift:     lift,
		Embed:    embed,
		Render:   render,
	})
	if p == nil {
		t.Fatalf("%s: no part", id)
	}
	return p
}

func mustCompose(t *testing.T, source, target *Part, options Options) *Composition {
	t.Helper()
	c, f := Compose(source, target, options, "main")
	if f != nil {
		t.Fatalf("%s: %v", target.ID, f)
	}
	return c
}

func TestADescriptorReadsIntoAPart(t *testing.T) {
	toml := tree(t, "toml", "object")
	if toml.Root != RootObject || !reflect.DeepEqual(toml.Reads, []Shape{ShapeTree}) {
		t.Errorf("%+v", toml)
	}
	if toml.Render.Kind != RenderAlc || *toml.Render.Alc != (Alc{File: "tabnas-toml/alchemy/render.alc", Entry: "toml-render", Text: "def toml-render [input] (json input)"}) {
		t.Errorf("%+v", toml.Render)
	}
	json := part("json", `{"reads": "tree", "writes": "tree", "render": "json"}`, CarriedPartText("json"))
	if json == nil || json.Render != (Render{Kind: RenderJSON}) || json.Root != RootAny {
		t.Errorf("%+v", json)
	}
	// A root, a shape or a render this package cannot take is no part; an
	// embed with no schema is none either.
	for _, translate := range []string{
		`{"reads": "tree", "writes": "tree", "root": "table", "render": "alchemy/render.alc"}`,
		`{"reads": "blob", "writes": "tree", "render": "alchemy/render.alc"}`,
		`{"reads": "tree", "writes": "tree", "render": "x.txt"}`,
		`{"reads": "tree"}`,
	} {
		if p := part("x", translate, own("x")); p != nil {
			t.Errorf("%s: %+v", translate, p)
		}
	}
	embedded := PartFromDescriptor(Descriptor{
		Package:  "tabnas-xml",
		Manifest: `{"languageId": "xml", "translate": {"reads": "tree", "writes": "tree", "root": "any", "embed": "alchemy/embed.alc", "render": "alchemy/render.alc"}}`,
		Embed:    NewPartText("xml-embed", "def xml-embed [input] input"),
		Render:   own("xml"),
	})
	if embedded != nil {
		t.Error("an embed names a schema")
	}
	// A manifest that is no JSON, or whose part the package does not hand
	// over, is no part either; the loss keeps its sentences.
	if p := PartFromDescriptor(Descriptor{Package: "tabnas-x", Manifest: "{", Render: own("x")}); p != nil {
		t.Errorf("%+v", p)
	}
	if p := part("x", `{"reads": "tree", "writes": "tree", "lift": "alchemy/lift.alc", "render": "alchemy/render.alc"}`, own("x")); p != nil {
		t.Errorf("%+v", p)
	}
	lossy := part("x", `{"reads": ["tree"], "writes": "tree", "render": "alchemy/render.alc", "loss": ["One.", 2, "Two."]}`, own("x"))
	if lossy == nil || !reflect.DeepEqual(lossy.Loss, []string{"One.", "Two."}) {
		t.Errorf("%+v", lossy)
	}
}

// A format whose documents are read whole says why in whole, a sentence;
// one that says nothing has none, and a whole that is not a sentence is a
// manifest this package cannot take.
func TestWholeIsTheSentenceAManifestGives(t *testing.T) {
	whole := func(value string) *Part {
		return part("toml", `{"reads": "tree", "writes": "tree", "render": "alchemy/render.alc", "whole": `+value+`}`, own("toml"))
	}
	const sentence = "A table may be defined after the tables that follow it."
	if p := whole(`"` + sentence + `"`); p == nil || p.Whole != sentence {
		t.Errorf("%+v", p)
	}
	if p := tree(t, "json", "any"); p.Whole != "" {
		t.Errorf("%+v", p)
	}
	for _, value := range []string{`""`, `true`} {
		if p := whole(value); p != nil {
			t.Errorf("%s: %+v", value, p)
		}
	}
}

func TestTheRouteWrapsARootAndEmbedsIntoASchema(t *testing.T) {
	o := DefaultOptions()
	toml := tree(t, "toml", "object")
	c := mustCompose(t, nil, toml, o)
	if c.Main != `def export [input] (toml-render (wrap-object "items" input))` {
		t.Errorf("%s", c.Main)
	}
	if !reflect.DeepEqual(c.Adapters, []Adapter{{Kind: AdapterWrapObject, Key: "items"}}) || c.Front != FrontTree {
		t.Errorf("%+v %v", c.Adapters, c.Front)
	}
	if c.Sources[0].File != "tabnas-toml/alchemy/render.alc" {
		t.Errorf("%+v", c.Sources)
	}
	jsonl := tree(t, "jsonl", "array")
	if c := mustCompose(t, nil, jsonl, o); c.Main != `def export [input] (jsonl-render (wrap-array input))` {
		t.Errorf("%s", c.Main)
	}
	yaml := tree(t, "yaml", "any")
	if c := mustCompose(t, nil, yaml, o); c.Main != `def export [input] (yaml-render input)` || len(c.Adapters) != 0 {
		t.Errorf("%s %+v", c.Main, c.Adapters)
	}
	// A schema the source's events are not of: through the embed.
	xml := *tree(t, "xml", "any")
	xml.Schema = "xml-element"
	xml.Embed = &Alc{File: "tabnas-xml/alchemy/embed.alc", Entry: "xml-embed", Text: "def xml-embed [input] input"}
	if c := mustCompose(t, nil, &xml, o); c.Main != `def export [input] (xml-render (xml-embed input))` || len(c.Sources) != 2 {
		t.Errorf("%s %+v", c.Main, c.Sources)
	}
	// A source of that schema: as it is.
	if c := mustCompose(t, &xml, &xml, o); c.Main != `def export [input] (xml-render input)` {
		t.Errorf("%s", c.Main)
	}
	// A schema-only target refuses another tree before any output.
	css := *tree(t, "css", "any")
	css.Schema = "css"
	_, f := Compose(nil, &css, o, "main")
	if f == nil || f.Code != alchemy.CodeTargetValueUnrepresentable || !strings.HasPrefix(f.Message, "schema_only: css writes a css tree") {
		t.Errorf("%v", f)
	}
}

func TestRecordsTargetsTakeATreesRowsAndALiftedFormatsRecords(t *testing.T) {
	o := DefaultOptions()
	csv := part("csv", `{"reads": "tree", "writes": "records", "root": "array", "render": "csv"}`, CarriedPartText("csv"))
	if csv == nil {
		t.Fatal("csv: no part")
	}
	c := mustCompose(t, nil, csv, o)
	if c.Main != "def export [input] (csv "+csvOptionsText+" (table-from-json "+inferredText+" (wrap-array input)))" {
		t.Errorf("%s", c.Main)
	}
	if !reflect.DeepEqual(c.Adapters, []Adapter{{Kind: AdapterWrapArray}, {Kind: AdapterInferredTable}}) || c.Front != FrontNone {
		t.Errorf("%+v %v", c.Adapters, c.Front)
	}
	if CSVOptions != csvOptionsText || inferred != inferredText {
		t.Errorf("%s\n%s", CSVOptions, inferred)
	}
	md := PartFromDescriptor(Descriptor{
		Package:  "tabnas-markdown",
		Manifest: `{"languageId": "markdown", "translate": {"reads": ["records", "tree"], "writes": "records", "root": "array", "lift": "alchemy/lift.alc", "render": "alchemy/render.alc"}}`,
		Lift:     NewPartText("markdown-lift", "def markdown-lift [input] input"),
		Render:   own("markdown"),
	})
	if md == nil {
		t.Fatal("markdown: no part")
	}
	c = mustCompose(t, md, csv, o)
	if c.Main != "def export [input] (csv "+csvOptionsText+" (markdown-lift input))" || len(c.Adapters) != 0 {
		t.Errorf("%s %+v", c.Main, c.Adapters)
	}
	// A lifted format into a tree's render: its tree as it is.
	yaml := tree(t, "yaml", "any")
	if c := mustCompose(t, md, yaml, o); c.Main != `def export [input] (yaml-render input)` {
		t.Errorf("%s", c.Main)
	}
	// JSON over the source's events as they are: the host's renderer.
	json := part("json", `{"reads": "tree", "writes": "tree", "render": "json"}`, CarriedPartText("json"))
	c = mustCompose(t, nil, json, o)
	if c.Native != NativeJSON {
		t.Errorf("%v", c.Native)
	}
	// A number that is not finite is written as null, which JSON has a
	// spelling for.
	if c.Main != `def export [input] (json (record (entry :non-finite :null)) input)` {
		t.Errorf("%s", c.Main)
	}
}

func TestAProgramsOutputStandsInTheSourcesPlace(t *testing.T) {
	o := DefaultOptions()
	toml := tree(t, "toml", "object")
	c, f := ComposeProgram(alchemy.OutputTableRows, toml, o, "main")
	if f != nil || c.Main != `def export [input] (toml-render (wrap-object "items" (records (program-export input))))` || c.Front != FrontNone {
		t.Errorf("%+v %v", c, f)
	}
	c, f = ComposeProgram(alchemy.OutputJsonEvents, toml, o, "main")
	if f != nil || c.Main != `def export [input] (toml-render (wrap-object "items" (program-export input)))` {
		t.Errorf("%+v %v", c, f)
	}
	if _, f := ComposeProgram(alchemy.OutputText, toml, o, "main"); f == nil || !strings.HasPrefix(f.Message, "render_of_text") {
		t.Errorf("%v", f)
	}
	key := Options{Key: `a "b"`}
	c = mustCompose(t, nil, toml, key)
	if c.Main != `def export [input] (toml-render (wrap-object "a \"b\"" input))` {
		t.Errorf("%s", c.Main)
	}
	found := false
	for _, l := range c.Loss {
		found = found || strings.Contains(l, `"a \"b\""`)
	}
	if !found {
		t.Errorf("%v", c.Loss)
	}
}

// A program writing to a schema-only target makes that schema's tree: such
// a target, which refuses another format's tree, takes a program's events
// into its render. Into a target with an embed a program's output is a
// plain tree, embedded like any source's, its table through records; the
// shape adapters apply as before.
func TestAProgramMakesASchemaOnlyTargetsTree(t *testing.T) {
	o := DefaultOptions()
	css := *tree(t, "css", "any")
	css.Schema = "css"
	c, f := ComposeProgram(alchemy.OutputJsonEvents, &css, o, "main")
	if f != nil || c.Main != `def export [input] (css-render (program-export input))` || len(c.Adapters) != 0 {
		t.Errorf("%+v %v", c, f)
	}
	c, f = ComposeProgram(alchemy.OutputTableRows, &css, o, "main")
	if f != nil || !reflect.DeepEqual(c.Adapters, []Adapter{{Kind: AdapterRecords}}) {
		t.Errorf("%+v %v", c, f)
	}
	// A root of the wrong kind is wrapped for a schema-only target too.
	object := *tree(t, "x", "object")
	object.Schema = "x-tree"
	c, f = ComposeProgram(alchemy.OutputJsonEvents, &object, o, "main")
	if f != nil || !reflect.DeepEqual(c.Adapters, []Adapter{{Kind: AdapterWrapObject, Key: "items"}}) {
		t.Errorf("%+v %v", c, f)
	}
	xml := *tree(t, "xml", "any")
	xml.Schema = "xml-element"
	xml.Embed = &Alc{File: "tabnas-xml/alchemy/embed.alc", Entry: "xml-embed", Text: "def xml-embed [input] input"}
	c, f = ComposeProgram(alchemy.OutputJsonEvents, &xml, o, "main")
	if f != nil || c.Main != `def export [input] (xml-render (xml-embed (program-export input)))` ||
		!reflect.DeepEqual(c.Adapters, []Adapter{{Kind: AdapterEmbed}}) || len(c.Sources) != 2 {
		t.Errorf("%+v %v", c, f)
	}
	c, f = ComposeProgram(alchemy.OutputTableRows, &xml, o, "main")
	if f != nil || c.Main != `def export [input] (xml-render (xml-embed (records (program-export input))))` ||
		!reflect.DeepEqual(c.Adapters, []Adapter{{Kind: AdapterRecords}, {Kind: AdapterEmbed}}) {
		t.Errorf("%+v %v", c, f)
	}
}

// Every route the package composes compiles with the standard library's
// adapters, and its output is a text: the root adapters, the inferred table
// over a wrapped root, an embed, a lift, a program's events and its table,
// into a schema-only target as into any other.
func TestEveryRouteCompilesToAText(t *testing.T) {
	o := DefaultOptions()
	csv := full(t, "csv", `{"reads": "tree", "writes": "records", "root": "array", "render": "csv"}`, nil, nil)
	md := full(t, "markdown",
		`{"reads": ["records", "tree"], "writes": "records", "root": "array", "lift": "alchemy/lift.alc", "render": "alchemy/render.alc"}`,
		NewPartText("markdown-lift", "def markdown-lift [input] (table-from-json "+inferredText+" input)"), nil)
	xml := full(t, "xml",
		`{"reads": "tree", "writes": "tree", "root": "any", "schema": "xml-element", "embed": "alchemy/embed.alc", "render": "alchemy/render.alc"}`,
		nil, NewPartText("xml-embed", "def xml-embed [input] (wrap-array input)"))
	targets := []*Part{tree(t, "toml", "object"), tree(t, "jsonl", "array"), tree(t, "yaml", "any"), csv, xml}
	sources := []*Part{nil, csv, md, xml}
	// programs composes a program's events and its table into target.
	programs := func(target *Part) {
		for _, output := range []alchemy.Output{alchemy.OutputJsonEvents, alchemy.OutputTableRows} {
			c, f := ComposeProgram(output, target, o, "main")
			if f != nil {
				t.Fatalf("%v into %s: %v", output, target.ID, f)
			}
			user := alchemy.Source{File: "p.alc", Text: "def export [input] input"}
			if output == alchemy.OutputTableRows {
				user.Text = "def export [input] (table-from-json " + inferredText + " input)"
			}
			program, f := c.Compile(&user, nil, nil)
			if f != nil {
				t.Errorf("%v into %s: %v\n%s", output, target.ID, f, c.Main)
				continue
			}
			if program.Output() != alchemy.OutputText || c.Front != FrontNone {
				t.Errorf("%s: %v %v", c.Main, program.Output(), c.Front)
			}
		}
	}
	for _, target := range targets {
		for _, source := range sources {
			name := "tree"
			if source != nil {
				name = source.ID
			}
			c := mustCompose(t, source, target, o)
			program, f := c.Compile(nil, nil, nil)
			if f != nil {
				t.Errorf("%s into %s: %v\n%s", name, target.ID, f, c.Main)
				continue
			}
			if program.Output() != alchemy.OutputText {
				t.Errorf("%s: %v", c.Main, program.Output())
			}
		}
		programs(target)
	}
	// A schema-only target refuses each source above, and takes a program's
	// output, which makes its tree.
	css := full(t, "css", `{"reads": "tree", "writes": "tree", "root": "any", "schema": "css", "render": "alchemy/render.alc"}`, nil, nil)
	for _, source := range sources {
		if _, f := Compose(source, css, o, "main"); f == nil {
			t.Errorf("%+v into css: no refusal", source)
		}
	}
	programs(css)
}

// The inferred table keeps a repeated member's last value, as an export
// does; a route without it keeps the default, which refuses one.
func TestARouteThroughTheInferredTableTakesTheLastOfARepeatedMember(t *testing.T) {
	o := DefaultOptions()
	csv := full(t, "csv", `{"reads": "tree", "writes": "records", "root": "array", "render": "csv"}`, nil, nil)
	for _, c := range []struct {
		target *Part
		want   shared.Duplicates
	}{{csv, shared.LastWins}, {tree(t, "yaml", "any"), shared.Reject}} {
		program, f := mustCompose(t, nil, c.target, o).Compile(nil, nil, nil)
		if f != nil || program.Duplicates() != c.want {
			t.Errorf("%s: %v %v", c.target.ID, program, f)
		}
	}
}

// A part whose render does not compile is a failure naming its file.
func TestARenderThatDoesNotCompileNamesItsFile(t *testing.T) {
	broken := PartFromDescriptor(Descriptor{
		Package:  "tabnas-x",
		Manifest: `{"languageId": "x", "translate": {"reads": "tree", "writes": "tree", "render": "alchemy/render.alc"}}`,
		Render:   NewPartText("x-render", "def x-render [input]\n  (nope input)"),
	})
	if broken == nil {
		t.Fatal("x: no part")
	}
	_, f := mustCompose(t, nil, broken, DefaultOptions()).Compile(nil, nil, nil)
	if f == nil || !strings.HasPrefix(f.Message, "unknown_name") || f.File != "tabnas-x/alchemy/render.alc" || f.Row != 2 || f.Column != 4 {
		t.Errorf("%v", f)
	}
}

// The adapters' sentences and names, as a host prints them.
func TestTheAdaptersNameThemselvesAndTheirLoss(t *testing.T) {
	names := map[AdapterKind]string{
		AdapterWrapObject: "wrap-object", AdapterWrapArray: "wrap-array", AdapterEmbed: "embed",
		AdapterInferredTable: "the inferred table", AdapterRecords: "records",
	}
	for kind, name := range names {
		if got := (Adapter{Kind: kind}).String(); got != name {
			t.Errorf("%d: %s", kind, got)
		}
	}
	if loss := (Adapter{Kind: AdapterEmbed}).Loss(); len(loss) != 0 {
		t.Errorf("%v", loss)
	}
	if loss := (Adapter{Kind: AdapterInferredTable}).Loss(); len(loss) != 2 {
		t.Errorf("%v", loss)
	}
	if ShapeRecords.String() != "records" || RootObject.String() != "object" || RootAny.String() != "any" {
		t.Error(ShapeRecords, RootObject, RootAny)
	}
}
