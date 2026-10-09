// Copyright (c) 2026 tabnas, MIT License

// Package translate composes a translation between formats from the
// formats' parts (rs/src/translate.rs; admin ADR-27; the design is
// tabnas/transduce's docs/translation.md).
//
// A format's package hands over its translation parts through a structural
// descriptor (translate() in each runtime): its manifest,
// tabnas.plugin.json, whose translate object names the shapes the format
// reads as and writes from, the root its render needs, the schema its
// events carry when they are not a plain tree, and the loss its render
// declares; and the alchemy text of its lift, its embed and its render.
// This package reads a descriptor into a Part and composes the program that
// translates a document of one format into another:
//
//  1. the source's events, through its lift when the target writes from
//     records and the source reads as records first;
//  2. for a target that writes from a tree: an embed into the target's
//     schema when the target has one and the source's events are not of it
//     (a schema-only target, one with no embed, refuses before any output),
//     else the root adapter when the target's render needs an object or an
//     array (`wrap-object`, `wrap-array`: the standard library's, which pass
//     a root of the right kind through);
//  3. for a target that writes from records, a tree's rows through the
//     inferred table, its root an array (`wrap-array`), or a program's table
//     as it is;
//  4. the target's render: a part's own, or alchemy's `json` or `csv`.
//
// The composed program is a one-line export linked with the parts' sources
// (Composition.Compile). A host runs it as it runs any program, and keeps a
// tree's contract in front of a render that writes from one (Front).
// Nothing here knows a format by name: every decision is the descriptors'.
//
// It is the Rust crate's translate module, a package of its own here
// because it builds on the language's API as a host does, and because its
// names (Native, RenderJSON) would otherwise meet the language's.
package translate

import (
	"encoding/json"
	"fmt"
	"strings"

	alchemy "github.com/tabnas/alchemy/go"
	"github.com/tabnas/alchemy/go/shared"
)

// Shape is a shape a format reads as or writes from: the manifest's reads
// and writes.
type Shape uint8

// The shapes.
const (
	// ShapeTree is a document's events.
	ShapeTree Shape = iota
	// ShapeRecords is a table's rows.
	ShapeRecords
)

// shapeNamed is the shape a manifest names.
func shapeNamed(name any) (Shape, bool) {
	switch name {
	case "tree":
		return ShapeTree, true
	case "records":
		return ShapeRecords, true
	}
	return ShapeTree, false
}

// String is the shape's name in a manifest.
func (s Shape) String() string {
	if s == ShapeRecords {
		return "records"
	}
	return "tree"
}

// Root is the root a format's render needs: the manifest's root. The zero
// value is RootAny, as a manifest that names none.
type Root uint8

// The roots.
const (
	// RootAny is any value.
	RootAny Root = iota
	// RootObject is a table or a section map at the root (TOML, INI).
	RootObject
	// RootArray is a sequence at the root (JSON Lines; and every render
	// that writes from records, whose rows are the elements of the root
	// array).
	RootArray
)

// rootNamed is the root a manifest names.
func rootNamed(name any) (Root, bool) {
	switch name {
	case "object":
		return RootObject, true
	case "array":
		return RootArray, true
	case "any":
		return RootAny, true
	}
	return RootAny, false
}

// String is the root's name in a manifest.
func (r Root) String() string {
	switch r {
	case RootObject:
		return "object"
	case RootArray:
		return "array"
	}
	return "any"
}

// PartText is one part as a format's package hands it over: the entry
// point a host calls, and its alchemy source. HasSource says whether there
// is one (a render alchemy carries has none); an empty Source with
// HasSource set is an empty text, as an empty lexeme is on shared.Event.
type PartText struct {
	Entry     string
	Source    string
	HasSource bool
}

// NewPartText is a part with its alchemy source.
func NewPartText(entry, source string) *PartText {
	return &PartText{Entry: entry, Source: source, HasSource: true}
}

// CarriedPartText is a render alchemy carries, named by its entry point
// (json, csv), with no source.
func CarriedPartText(entry string) *PartText {
	return &PartText{Entry: entry}
}

// Descriptor is a format package's structural descriptor: what its
// translate() answers, and the name its parts are named by in diagnostics
// (the package's, tabnas-yaml). A part the package does not hand over is
// nil.
type Descriptor struct {
	Package  string
	Manifest string
	Lift     *PartText
	Embed    *PartText
	Render   *PartText
}

// Alc is an alchemy part: the file it is named by in diagnostics, its
// entry point, and its text.
type Alc struct {
	File  string
	Entry string
	Text  string
}

// RenderKind is how a format is written: an alchemy render of its own, or
// one alchemy carries.
type RenderKind uint8

// The renders.
const (
	// RenderAlc is the format's own alchemy render, Render.Alc.
	RenderAlc RenderKind = iota
	// RenderJSON is alchemy's json: the render package's JSON renderer.
	RenderJSON
	// RenderCSV is alchemy's csv, under the export's policies (CSVOptions).
	RenderCSV
)

// Render is how a format is written: its kind, and for RenderAlc the
// render (nil otherwise).
type Render struct {
	Kind RenderKind
	Alc  *Alc
}

// Part is a format's translation parts, as its manifest names them and its
// package hands them over.
type Part struct {
	// ID is the manifest's languageId.
	ID string
	// Reads is what the format reads as, in order of preference; never
	// empty.
	Reads []Shape
	// Writes is what the render writes from.
	Writes Shape
	// Root is the root the render needs.
	Root Root
	// Schema is the tree the format's events carry, when not a plain one
	// ("" for a plain tree).
	Schema string
	// Lift is from the events to the first read shape, where they do not
	// carry it (nil for none).
	Lift *Alc
	// Embed is from a plain tree into the schema; its file also holds the
	// reverse (nil for none).
	Embed  *Alc
	Render Render
	// Loss is what a written document does not keep, a sentence each.
	Loss []string
}

// member is a member of a JSON object, as the manifest holds it, and
// whether it is there: never for what is not an object.
func member(v any, key string) (any, bool) {
	object, ok := v.(map[string]any)
	if !ok {
		return nil, false
	}
	value, ok := object[key]
	return value, ok
}

// PartFromDescriptor is the part a descriptor describes, or nil when its
// manifest names no translate object (a format read and not written) or
// one this package cannot take: a shape or a root it does not know, a part
// the package does not hand over, a render that is neither an alchemy file
// nor a render alchemy carries.
func PartFromDescriptor(d Descriptor) *Part {
	var manifest any
	if err := json.Unmarshal([]byte(d.Manifest), &manifest); err != nil {
		return nil
	}
	idValue, _ := member(manifest, "languageId")
	id, ok := idValue.(string)
	if !ok {
		return nil
	}
	t, _ := member(manifest, "translate")
	var reads []Shape
	named, _ := member(t, "reads")
	switch x := named.(type) {
	case string:
		one, ok := shapeNamed(x)
		if !ok {
			return nil
		}
		reads = []Shape{one}
	case []any:
		for _, v := range x {
			shape, ok := shapeNamed(v)
			if !ok {
				return nil
			}
			reads = append(reads, shape)
		}
	default:
		return nil
	}
	if len(reads) == 0 {
		return nil
	}
	writesName, _ := member(t, "writes")
	writes, ok := shapeNamed(writesName)
	if !ok {
		return nil
	}
	root := RootAny
	if rootName, present := member(t, "root"); present {
		if root, ok = rootNamed(rootName); !ok {
			return nil
		}
	}
	schema := ""
	if schemaName, present := member(t, "schema"); present {
		s, ok := schemaName.(string)
		if !ok || s == "" {
			return nil
		}
		schema = s
	}
	file := func(path string) string { return d.Package + "/" + path }
	// alc is the alchemy part the manifest names under key, as the package
	// hands it over: nil when neither names one, the part when both do and
	// it is an alchemy file with an entry and a text, and false (no part)
	// for anything else.
	alc := func(key string, part *PartText) (*Alc, bool) {
		pathValue, present := member(t, key)
		if !present && part == nil {
			return nil, true
		}
		path, ok := pathValue.(string)
		if !present || !ok || !strings.HasSuffix(path, ".alc") || part == nil || part.Entry == "" || !part.HasSource {
			return nil, false
		}
		return &Alc{File: file(path), Entry: part.Entry, Text: part.Source}, true
	}
	lift, ok := alc("lift", d.Lift)
	if !ok {
		return nil
	}
	embed, ok := alc("embed", d.Embed)
	if !ok {
		return nil
	}
	if embed != nil && schema == "" {
		return nil
	}
	part := d.Render
	if part == nil || part.Entry == "" {
		return nil
	}
	renderValue, _ := member(t, "render")
	renderName, ok := renderValue.(string)
	if !ok {
		return nil
	}
	var render Render
	switch {
	case renderName == "json" && part.Entry == "json" && !part.HasSource:
		render = Render{Kind: RenderJSON}
	case renderName == "csv" && part.Entry == "csv" && !part.HasSource:
		render = Render{Kind: RenderCSV}
	case strings.HasSuffix(renderName, ".alc") && part.HasSource:
		render = Render{Kind: RenderAlc, Alc: &Alc{File: file(renderName), Entry: part.Entry, Text: part.Source}}
	default:
		return nil
	}
	loss := []string{}
	if lines, ok := member(t, "loss"); ok {
		if list, ok := lines.([]any); ok {
			for _, l := range list {
				if s, ok := l.(string); ok {
					loss = append(loss, s)
				}
			}
		}
	}
	return &Part{
		ID:     id,
		Reads:  reads,
		Writes: writes,
		Root:   root,
		Schema: schema,
		Lift:   lift,
		Embed:  embed,
		Render: render,
		Loss:   loss,
	}
}

// PartOfOutput is the part a program's output stands for, in a source's
// place: JSON events are a plain tree, a table is records, and a text is
// no source (`render_of_text`).
func PartOfOutput(output alchemy.Output) (*Part, *alchemy.Fail) {
	var reads Shape
	switch output {
	case alchemy.OutputJsonEvents:
		reads = ShapeTree
	case alchemy.OutputTableRows:
		reads = ShapeRecords
	default:
		return nil, alchemy.NewFail(alchemy.CodeDSLTypeError,
			"render_of_text: the program renders its own text, which no render takes")
	}
	return &Part{
		ID:     "program",
		Reads:  []Shape{reads},
		Writes: reads,
		Root:   RootAny,
		Render: Render{Kind: RenderJSON},
		Loss:   []string{},
	}, nil
}

// AdapterKind names what a composition runs between the source's events
// and the render.
type AdapterKind uint8

// The adapters.
const (
	// AdapterWrapObject is a root that is not an object as the one member
	// of one, Adapter.Key.
	AdapterWrapObject AdapterKind = iota
	// AdapterWrapArray is a root that is not an array as the one element of
	// one.
	AdapterWrapArray
	// AdapterEmbed is a plain tree into the target's schema, by its embed.
	AdapterEmbed
	// AdapterInferredTable is a tree's rows as records: the inferred table.
	AdapterInferredTable
	// AdapterRecords is records as a tree: one object per row, keyed by the
	// labels.
	AdapterRecords
)

// Adapter is what a composition runs between the source's events and the
// render: its kind, and for AdapterWrapObject the member a root is written
// under.
type Adapter struct {
	Kind AdapterKind
	Key  string
}

// String is the adapter as a host names it.
func (a Adapter) String() string {
	switch a.Kind {
	case AdapterWrapObject:
		return "wrap-object"
	case AdapterWrapArray:
		return "wrap-array"
	case AdapterEmbed:
		return "embed"
	case AdapterInferredTable:
		return "the inferred table"
	}
	return "records"
}

// Loss is what the adapter changes, the host's sentences, printed when it
// runs; an embed's are its format's own loss declaration.
func (a Adapter) Loss() []string {
	switch a.Kind {
	case AdapterWrapObject:
		return []string{fmt.Sprintf("A document whose root is not an object is written as the one member %s of an "+
			"object, since the format's document is one.", quoted(a.Key))}
	case AdapterWrapArray:
		return []string{"A document whose root is not an array is written as the one element of an " +
			"array, since the format's document is a sequence."}
	case AdapterEmbed:
		return nil
	case AdapterInferredTable:
		return []string{
			"The rows are the elements of the root array: an object row's members are its " +
				"cells, an array row's cells are its positions, and a scalar row is one cell " +
				"named value.",
			"The columns are the first row's: a member or a cell a later row adds is not " +
				"written, one it lacks is written empty, a member repeated in a row keeps its " +
				"last value, and a row of another kind than the first has a cell only where the " +
				"first row's columns find one.",
		}
	}
	return []string{"Each row is written as an object keyed by the column labels: a cell the row " +
		"lacks is an absent member, and of two columns with one label the last gives " +
		"the member."}
}

// Front is what a host holds the events to in front of the composed
// program.
type Front uint8

// The fronts.
const (
	// FrontTree is the source's events, which a render that writes from a
	// tree takes: each key once per object, one root value (transduce's
	// TreeContract).
	FrontTree Front = iota
	// FrontNone is nothing: the program's own events, or a table's rows.
	FrontNone
)

// Native is how the composition may be run by a host's own renderer
// instead, when the route is the identity into alchemy's json. The
// composed json writes a number that is not finite as null (JSON has no
// spelling for one, and the target declares the loss), so a host that runs
// its own JSON renderer instead writes such a number as null too.
type Native uint8

// The ways.
const (
	// NativeNone: the composition runs as composed.
	NativeNone Native = iota
	// NativeJSON is the source's events as they are, into the JSON
	// renderer.
	NativeJSON
)

// Options are the composition's options: what the host chooses.
type Options struct {
	// Key is the member a root that is not an object is written under, for
	// a target that needs an object.
	Key string
}

// DefaultOptions are the options a host has not chosen: a root written
// under items.
func DefaultOptions() Options {
	return Options{Key: "items"}
}

// CSVOptions are the CSV options a composed csv runs under: the library's,
// with the export's policy for an absent member, an empty field; a number
// that is not finite written as its word, Infinity, -Infinity or NaN,
// since every CSV cell is text; and a table of no columns (an empty
// document, or rows of no members) written as the empty document.
const CSVOptions = `(record (entry :delimiter ",") (entry :newline "\r\n") ` +
	`(entry :header true) (entry :null-text "") (entry :missing "") ` +
	`(entry :non-finite :literal) (entry :no-columns :empty))`

// JSONOptions is what a composed json runs under: a number that is not
// finite, which JSON has no spelling for, is written as null.
const JSONOptions = `(record (entry :non-finite :null))`

// inferred is the binding of the inferred table: the rows are the root
// array's elements, the columns the first row's.
const inferred = `(record (entry :columns :infer) (entry :rows (path each-index)))`

// ProgramExport is the name a program is linked under when its output
// feeds a render.
const ProgramExport = "program-export"

// Composition is a translation ready to compile: the parts' sources, the
// one-line main, what runs between, and what the host keeps in front.
type Composition struct {
	// Sources are each part's source, in link order: the render, the
	// embed, the lift.
	Sources []alchemy.Source
	// MainFile is the main's file name in diagnostics, and Main its text.
	MainFile string
	Main     string
	Adapters []Adapter
	Front    Front
	Native   Native
	// Loss is the render's loss declaration, then each adapter's.
	Loss []string
}

// Compile links the parts with the main, and program's source when the
// composition is over a program's output (nil otherwise), into one program
// under the policies the adapters need: the inferred table keeps a
// repeated member's last value, as an export does. routers and renderers
// are the stages the program runs on, as for alchemy.CompileSources.
func (c *Composition) Compile(program *alchemy.Source, routers shared.Routers, renderers shared.Renderers) (*alchemy.Program, *alchemy.Fail) {
	sources := make([]alchemy.Source, 0, len(c.Sources)+2)
	for _, s := range c.Sources {
		sources = append(sources, alchemy.Source{File: s.File, Text: s.Text})
	}
	if program != nil {
		p := *program
		p.ExportAs = ProgramExport
		sources = append(sources, p)
	}
	sources = append(sources, alchemy.Source{File: c.MainFile, Text: c.Main})
	compiled, f := alchemy.CompileSources(sources, routers, renderers)
	if f != nil {
		return nil, f
	}
	for _, a := range c.Adapters {
		if a.Kind == AdapterInferredTable {
			return compiled.WithDuplicates(shared.LastWins)
		}
	}
	return compiled, nil
}

// Compose composes the translation of a document read as source describes
// (a plain tree when nil: a format with no parts, or a value a host
// selected below the root) into target. mainFile names the main in
// diagnostics.
func Compose(source, target *Part, options Options, mainFile string) (*Composition, *alchemy.Fail) {
	return composeOver(source, "input", target, options, mainFile, FrontTree)
}

// ComposeProgram composes a program's output into target, in the source's
// place: JSON events are a tree and a table is records (PartOfOutput). The
// program is linked under ProgramExport by Composition.Compile.
func ComposeProgram(output alchemy.Output, target *Part, options Options, mainFile string) (*Composition, *alchemy.Fail) {
	source, f := PartOfOutput(output)
	if f != nil {
		return nil, f
	}
	return composeOver(source, "("+ProgramExport+" input)", target, options, mainFile, FrontNone)
}

func composeOver(source *Part, input string, target *Part, options Options, mainFile string, front Front) (*Composition, *alchemy.Fail) {
	reads := []Shape{ShapeTree}
	var lift *Alc
	sourceSchema := ""
	if source != nil {
		reads, lift, sourceSchema = source.Reads, source.Lift, source.Schema
	}
	var sources []alchemy.Source
	var adapters []Adapter
	expr := input
	// treeEvents is whether the source's events reach the render whole, no
	// adapter having taken them apart; only a tree's render reads it (the
	// front, below), so a records render leaves it as it is.
	treeEvents := true
	switch target.Writes {
	case ShapeRecords:
		if reads[0] == ShapeRecords {
			// A format read as records first: through its lift, or as its
			// events are (a program's table).
			if lift != nil {
				expr = "(" + lift.Entry + " " + expr + ")"
				sources = append(sources, alchemy.Source{File: lift.File, Text: lift.Text})
			}
		} else {
			// A tree's rows: the elements of the root array.
			if target.Root == RootArray {
				expr = "(wrap-array " + expr + ")"
				adapters = append(adapters, Adapter{Kind: AdapterWrapArray})
			}
			expr = "(table-from-json " + inferred + " " + expr + ")"
			adapters = append(adapters, Adapter{Kind: AdapterInferredTable})
		}
	case ShapeTree:
		if !hasShape(reads, ShapeTree) {
			// Records only (a program's table): one object per row.
			expr = "(records " + expr + ")"
			adapters = append(adapters, Adapter{Kind: AdapterRecords})
			treeEvents = false
		}
		switch {
		case target.Schema != "" && sourceSchema != target.Schema:
			if target.Embed == nil {
				return nil, alchemy.NewFail(alchemy.CodeTargetValueUnrepresentable, fmt.Sprintf(
					"schema_only: %s writes a %s tree, the tree its own documents "+
						"read as, and this document is not one; a program that makes one "+
						"can be composed with the render", target.ID, target.Schema))
			}
			expr = "(" + target.Embed.Entry + " " + expr + ")"
			sources = append(sources, alchemy.Source{File: target.Embed.File, Text: target.Embed.Text})
			adapters = append(adapters, Adapter{Kind: AdapterEmbed})
		case target.Root == RootObject:
			expr = "(wrap-object " + quoted(options.Key) + " " + expr + ")"
			adapters = append(adapters, Adapter{Kind: AdapterWrapObject, Key: options.Key})
		case target.Root == RootArray:
			expr = "(wrap-array " + expr + ")"
			adapters = append(adapters, Adapter{Kind: AdapterWrapArray})
		}
	}
	native := NativeNone
	if target.Render.Kind == RenderJSON && expr == input {
		native = NativeJSON
	}
	var render string
	switch target.Render.Kind {
	case RenderJSON:
		render = "json " + JSONOptions
	case RenderCSV:
		render = "csv " + CSVOptions
	default:
		alc := target.Render.Alc
		sources = append([]alchemy.Source{{File: alc.File, Text: alc.Text}}, sources...)
		render = alc.Entry
	}
	main := "def export [input] (" + render + " " + expr + ")"
	loss := append([]string{}, target.Loss...)
	for _, a := range adapters {
		loss = append(loss, a.Loss()...)
	}
	// The source's events reach a tree's render as a tree only when no
	// adapter took them apart first; a records render, and a program's
	// events, have nothing in front.
	kept := FrontNone
	if front == FrontTree && treeEvents && target.Writes == ShapeTree {
		kept = FrontTree
	}
	return &Composition{
		Sources:  sources,
		MainFile: mainFile,
		Main:     main,
		Adapters: adapters,
		Front:    kept,
		Native:   native,
		Loss:     loss,
	}, nil
}

// hasShape is whether shapes holds shape.
func hasShape(shapes []Shape, shape Shape) bool {
	for _, s := range shapes {
		if s == shape {
			return true
		}
	}
	return false
}

// quoted is a string as an alchemy string literal (JSON's escapes).
func quoted(s string) string {
	var b strings.Builder
	shared.WriteJSONString(s, &b)
	return b.String()
}
