// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// events_test.go: `events` (rs/tests/events_test.rs): the source's events
// as items a program reads one by one, and the operators a renderer over
// them needs (push, pop, top, count, quoted, repeat). The proof is a
// renderer of arbitrary nesting, which nothing before events could
// express: a tiny YAML-like block form written by a scan-emit whose state
// is the stack of open containers, one keyword marker each, asserted byte
// for byte; then the affine rule over events, and the plan report.

import (
	"bytes"
	"strings"
	"testing"

	tt "github.com/tabnas/transduce/go"
	tabnasyaml "github.com/tabnas/yaml/go"
)

// yamlLike is a YAML-like block renderer over the events. The state is the
// stack of open containers, one marker each: :object-open and :array-open
// for a container with nothing written yet (its opening line is held until
// its first child or its end, so an empty one writes as {} or [] on the
// key's line), :object for an object waiting for a key, :member for one
// whose key is written and waits for the value, :array for an array
// waiting for an item. A mapping under a key goes on the next lines two
// spaces in; a sequence item is `- `; a mapping in a sequence has its
// first key on the dash's line; a sequence in a sequence starts on the
// next line, two spaces in; a root scalar stands alone. Strings and keys
// go through quoted; a number is told from a string by kind and written by
// its lexeme.
const yamlLike = `def indent [ctx]
  repeat (count ctx) "  "

def replace-top [marker s]
  push marker (pop s)

; A value completed: an object waiting for its member's value waits for
; its next key.
def done [s]
  match (count s)
    case 0 s
    case _
      match (top s)
        case :member (replace-top :object s)
        case _ s

; What comes before a value whose parent is the top of ctx: a space on
; the key's line, or a dash, for a scalar or an empty container; a line
; break for a mapping or a sequence under a key; a dash and the first key
; for a mapping in a sequence; a dash alone for a sequence in a sequence.
def lead [kind ctx]
  match (count ctx)
    case 0 ""
    case _
      match (top ctx)
        case :member
          match kind
            case :mapping (concat "\n" (indent ctx))
            case :sequence "\n"
            case _ " "
        case :array
          match kind
            case :sequence (concat (indent (pop ctx)) "-\n")
            case _ (concat (indent (pop ctx)) "- ")

; A held array is opened as a sequence by its first item.
def opened [s]
  match (count s)
    case 0 s
    case _
      match (top s)
        case :array-open (replace-top :array s)
        case _ s

def opening [s]
  match (count s)
    case 0 ""
    case _
      match (top s)
        case :array-open (lead :sequence (pop s))
        case _ ""

def yaml-scalar [value]
  match (kind value)
    case :null "null"
    case :boolean
      match value
        case true "true"
        case false "false"
    case :number (scalar-text csv-options value)
    case _ (quoted value)

def step [s event]
  match event
    case object-start
      transition (push :object-open (opened s)) [(opening s)]
    case array-start
      transition (push :array-open (opened s)) [(opening s)]
    case (key name)
      match (top s)
        case :object-open
          transition (replace-top :member s) [(lead :mapping (pop s)) (quoted name) ":"]
        case :object
          transition (replace-top :member s) [(indent (pop s)) (quoted name) ":"]
    case (scalar value)
      let [s2 (opened s)]
        transition (done s2) [(opening s) (lead :scalar s2) (yaml-scalar value) "\n"]
    case object-end
      match (top s)
        case :object-open
          transition (done (pop s)) [(lead :scalar (pop s)) "{}\n"]
        case _
          transition (done (pop s)) []
    case array-end
      match (top s)
        case :array-open
          transition (done (pop s)) [(lead :scalar (pop s)) "[]\n"]
        case _
          transition (done (pop s)) []

def finish [s]
  match (count s)
    case 0 []
    case _ (fail "the events ended inside a container")

def export [input]
  join ""
    scan-emit [] step finish (events input)
`

// The renderer writes a nested document, with an empty object and an
// empty array, strings that need escapes, and a scalar root, byte for byte
// as the block form has them.
func TestEventsRenderANestedDocumentAsAYamlLikeBlock(t *testing.T) {
	// The documents spell numbers by lexemes (1.5e3, -0) and repeat a
	// member, which only the incremental source keeps.
	requireIncremental(t)
	program := mustCompile(t, yamlLike, "render.alc")
	if program.Output() != OutputText {
		t.Errorf("%s", program.Output())
	}
	// Every event is needed, so nothing is pruned.
	if rowSelectorText(program) != "<none>" {
		t.Errorf("%s", rowSelectorText(program))
	}
	for _, c := range []struct{ doc, want string }{
		{`{"a":"x","b":["y",null,{"c":"q\"x\n","d":{}}],"e":[],"f":{"g":[[]]},"n":1.5e3,"z":-0}`,
			"\"a\": \"x\"\n" +
				"\"b\":\n" +
				"  - \"y\"\n" +
				"  - null\n" +
				"  - \"c\": \"q\\\"x\\n\"\n" +
				"    \"d\": {}\n" +
				"\"e\": []\n" +
				"\"f\":\n" +
				"  \"g\":\n" +
				"    - []\n" +
				"\"n\": 1.5e3\n" +
				"\"z\": -0\n"},
		// A number is told from a string by kind and written by its
		// lexeme; a string that spells a number stays quoted.
		{"42", "42\n"},
		{`["42",42]`, "- \"42\"\n- 42\n"},
		// A scalar root stands alone; its escapes are JSON's, with DEL and
		// the C1 controls escaped too, and everything else as itself.
		{`"tab\there \u0001 \u007f \u0085 caf\u00e9 \\ \""`, "\"tab\\there \\u0001 \\u007f \\u0085 café \\\\ \\\"\"\n"},
		{"true", "true\n"},
		{"null", "null\n"},
		// Empty containers at the root, and a sequence in a sequence.
		{"{}", "{}\n"},
		{"[]", "[]\n"},
		{`[["a"],[],{"k":[false]}]`, "-\n  - \"a\"\n- []\n- \"k\":\n    - false\n"},
		// A key the source repeats is delivered as it arrives, twice:
		// nothing is mapped by key between events.
		{`{"a":"1","a":"2"}`, "\"a\": \"1\"\n\"a\": \"2\"\n"},
	} {
		if got := render(t, program, c.doc); got != c.want {
			t.Errorf("%s:\n got %q\nwant %q", c.doc, got, c.want)
		}
	}
	// The stack is what the stage retains: a document nested deeper than
	// max_depth is refused by the source before the state could grow to
	// it, and the failure names the limit.
	deep := strings.Repeat("[", 40) + "1" + strings.Repeat("]", 40)
	shallow := tt.DefaultLimits()
	shallow.MaxDepth = 16
	_, f := hostRun(t, program, deep, RenderDefault, shallow)
	if f == nil || f.Code != CodeResourceLimitExceeded || f.Limit.Name != "max_depth" {
		t.Errorf("%v", f)
	}
}

// keyForms writes each key on its own line, in the explicit form past
// YAML's 1024-character limit on an implicit key (length and compare), and
// each number in YAML's spellings, the non-finite ones included
// (number-class), as a YAML render does.
const keyForms = `def key-line [name]
  match (compare (length (quoted name)) 1024)
    case :greater ["? " (quoted name) "\n"]
    case _ [(quoted name) ":\n"]

def number-line [n]
  match (number-class n)
    case :finite [(scalar-text csv-options n) "\n"]
    case :infinity [".inf\n"]
    case :negative-infinity ["-.inf\n"]
    case :nan [".nan\n"]

def step [s event]
  match event
    case (key name) (transition s (key-line name))
    case (scalar v)
      match (kind v)
        case :number (transition s (number-line v))
        case _ (transition s [])
    case _ (transition s [])

def finish [s] []

def export [input]
  join ""
    scan-emit [] step finish (events input)
`

// Past 1024 characters, quotes included, a key takes the explicit form;
// at 1024 it is still implicit.
func TestAKeyPastTheImplicitLimitTakesTheExplicitForm(t *testing.T) {
	program := mustCompile(t, keyForms, "keys.alc")
	long := strings.Repeat("k", 1023)
	if got := render(t, program, `{"`+long[:1022]+`":1}`); got != "\""+long[:1022]+"\":\n1\n" {
		t.Errorf("%q", got)
	}
	if got := render(t, program, `{"`+long+`":1}`); got != "? \""+long+"\"\n1\n" {
		t.Errorf("%q", got)
	}
}

// YAML's non-finite numbers arrive as numbers with no lexeme, and a
// program writes them in YAML's spellings, where scalar-text would refuse
// them as JSON and CSV must; a quoted '.inf' is a string.
func TestTheNonFiniteNumbersAreWrittenInYamlsSpellings(t *testing.T) {
	program := mustCompile(t, keyForms, "keys.alc")
	var buffer bytes.Buffer
	sink, f := program.Sink(&buffer, RenderDefault, tt.DefaultLimits(), tt.NewMetrics())
	if f != nil {
		t.Fatal(f)
	}
	if _, f := tt.NewParserSource(tabnasyaml.MakeJsonic(), "- .inf\n- -.inf\n- .nan\n- 1.5\n- '.inf'\n").Run(sink); f != nil {
		t.Fatal(f)
	}
	if buffer.String() != ".inf\n-.inf\n.nan\n1.5\n" {
		t.Errorf("%q", buffer.String())
	}
}

// events consumes the input, so a program that reads it twice, or captures
// it in a function, is refused as any affine stream is.
func TestEventsOverTheInputIsAffine(t *testing.T) {
	twice := "def export [input]\n  concat\n    join \"\" (map (fn [e] \"a\") (events input))\n    join \"\" (map (fn [e] \"b\") (events input))\n"
	if _, f := Compile(twice, "twice.alc", routers, renderers); f == nil || f.Code != CodeStreamReused || !strings.HasPrefix(f.Message, "reused: ") || f.Row != 4 || f.Column != 39 {
		t.Errorf("%v", f)
	}
	withJSON := "def export [input] (concat (json input) (join \"\" (map (fn [e] \"\") (events input))))"
	if _, f := Compile(withJSON, "json.alc", routers, renderers); f == nil || f.Code != CodeStreamReused || !strings.HasPrefix(f.Message, "reused: ") {
		t.Errorf("%v", f)
	}
	captured := "def export [input]\n  concat-map (fn [x] (join \"\" (map (fn [e] \"\") (events input)))) [1]\n"
	if _, f := Compile(captured, "captured.alc", routers, renderers); f == nil || f.Code != CodeStreamReused || !strings.HasPrefix(f.Message, "captured: ") {
		t.Errorf("%v", f)
	}
	// The stream events yields is a stream of items: events, which json
	// takes back, but not table events for csv and not the output; a
	// stream of values is not events.
	for _, c := range []struct{ src, finer string }{
		{"def export [input] (json (select (path each-index) input))", "protocol_mismatch"},
		{"def export [input] (csv csv-options (events input))", "protocol_mismatch"},
		{"def export [input] (events (select (path each-index) input))", "protocol_mismatch"},
		{"def export [input] (events input)", "bad_output"},
	} {
		if _, f := Compile(c.src, "bad.alc", routers, renderers); f == nil || f.Code != CodeDSLTypeError || !strings.HasPrefix(f.Message, c.finer+": ") {
			t.Errorf("%s: %v", c.src, f)
		}
	}
}

// The report names the stage: every event delivered as an item, nothing
// retained by it, and the scan-emit after it conditional on its state.
func TestExplainReportsAnEventsPipeline(t *testing.T) {
	program := mustCompile(t, yamlLike, "render.alc")
	text := program.Explain()
	for _, part := range []string{
		"Protocol:              JsonEvents/1 → Stream<Event> → Stream<Value> → Text\n",
		"Selection:             none; every event is delivered as an item\n",
		"Duplicate members:     preserved; the events are copied as they arrive, not mapped by key\n",
		"Retained state:        what the step returns, no deeper than max_depth, capped at max_metadata_bytes\n",
		"Output order:          source order\n",
	} {
		if !strings.Contains(text, part) {
			t.Errorf("%s\ndoes not hold %q", text, part)
		}
	}
	if !strings.HasPrefix(text, "export: events → scan-emit → join\n") {
		t.Errorf("%s", text)
	}
	j := program.ExplainJSON()
	for _, c := range []struct {
		path []string
		want string
	}{
		{[]string{"chain"}, `["events","scan-emit","join"]`},
		{[]string{"protocol"}, `["JsonEvents/1","Stream<Event>","Stream<Value>","Text"]`},
		{[]string{"confidence"}, `"conditional"`},
		{[]string{"readiness"}, `"event"`},
		{[]string{"renderer", "name"}, `"text"`},
		{[]string{"output"}, `"Text"`},
	} {
		if got := jsonMember(t, j, c.path...); got != c.want {
			t.Errorf("%v: %s", c.path, got)
		}
	}
	retention, _ := j.Get("retention")
	if r, ok := retention.([]any); !ok || len(r) != 1 || jsonMember(t, r[0].(*JSONObject), "scope") != `"state"` {
		t.Errorf("%s", EncodeJSON(retention))
	}
	// A map over the events alone retains nothing at all.
	keys := mustCompile(t, "def export [input]\n  join \"\"\n    map\n      fn [event]\n        match event\n          case (key name) (concat (quoted name) \"\\n\")\n          case _ \"\"\n      events input\n", "keys.alc")
	kj := keys.ExplainJSON()
	if jsonMember(t, kj, "confidence") != `"proven"` || jsonMember(t, kj, "retention") != "[]" {
		t.Errorf("%s", EncodeJSON(kj))
	}
	// The document repeats a member, which only the incremental source
	// keeps.
	requireIncremental(t)
	if got := render(t, keys, `{"a":1,"b":{"c":[2,{"d":3}]},"a":4}`); got != "\"a\"\n\"b\"\n\"c\"\n\"d\"\n\"a\"\n" {
		t.Errorf("%q", got)
	}
}

// A stream of events a program built reaches any taker of JSON events, the
// reverse of events: the events and back through json is the document
// again, lexemes kept; a scan-emit over them rewrites it on the way;
// table-from-json reads the rows a step made; an item that is not an event
// is refused as the stream runs, naming it, where the checker lets a
// stream of unknown items through; and a stream of values is refused
// before anything runs.
func TestEventsAProgramBuildsFeedAnyTakerOfJSONEvents(t *testing.T) {
	// The documents spell numbers by lexemes (1.50, -0).
	requireIncremental(t)
	back := mustCompile(t, "def export [input] (json (events input))", "back.alc")
	if back.Output() != OutputText {
		t.Errorf("%s", back.Output())
	}
	nested := `{"a":"x","b":["y",null,{"c":1.50,"d":{}}],"e":[],"n":-0,"t":true}`
	if got := render(t, back, nested); got != nested+"\n" {
		t.Errorf("%q", got)
	}
	rename := mustCompile(t, "def step [s e]\n  match e\n    case (key \"a\") (transition s [(key \"b\")])\n    case _ (transition s [e])\n\ndef export [input]\n  json (scan-emit [] step (fn [s] []) (events input))", "rename.alc")
	if got := render(t, rename, `{"a":1,"x":{"a":[2]}}`); got != "{\"b\":1,\"x\":{\"b\":[2]}}\n" {
		t.Errorf("%q", got)
	}
	// The rows a step made: each scalar of the root array wrapped as an
	// object of one member, read by table-from-json, written by records.
	wrap := mustCompile(t, "def step [s e]\n  match s\n    case 0 (transition 1 [e])\n    case _\n      match e\n        case (scalar v) (transition 1 [object-start (key \"value\") e object-end])\n        case _ (transition 1 [e])\n\ndef rows (record (entry :columns :infer) (entry :rows (path each-index)))\n\ndef export [input]\n  json (records (table-from-json rows (scan-emit 0 step (fn [s] []) (events input))))", "wrap.alc")
	if got := render(t, wrap, `[1,"two",true]`); got != "[{\"value\":1},{\"value\":\"two\"},{\"value\":true}]\n" {
		t.Errorf("%q", got)
	}
	// An item that is not an event is refused where it arrives.
	bad := mustCompile(t, "def export [input] (json (scan-emit [] (fn [s e] (transition s [1])) (fn [s] []) (events input)))", "bad.alc")
	out, f := hostRun(t, bad, "[1]", RenderDefault, tt.DefaultLimits())
	if f == nil || f.Code != CodeProtocolOrderError || !strings.HasPrefix(f.Message, "an event was expected, not") || out != "" {
		t.Errorf("%v %q", f, out)
	}
	// A stream of values is not a stream of events: the checker says so.
	if _, f := Compile("def export [input] (json (select (path each-index) input))", "values.alc", routers, renderers); f == nil || !strings.HasPrefix(f.Message, "protocol_mismatch") {
		t.Errorf("%v", f)
	}
	// The source's limits hold on the events a program made: a document
	// built deeper than max_depth from a flat input, or a key longer than
	// max_key_bytes, is refused where the limit is passed, as the source's
	// own events would be.
	deepen := mustCompile(t, "def step [s e]\n  match e\n    case (scalar true) (transition s [array-start])\n    case (scalar false) (transition s [array-end])\n    case _ (transition s [])\n\ndef export [input]\n  json (scan-emit [] step (fn [s] []) (events input))", "deepen.alc")
	flat := func(n int) string {
		items := strings.Repeat("true,", n) + strings.TrimSuffix(strings.Repeat("false,", n), ",")
		return "[" + items + "]"
	}
	limits := tt.DefaultLimits()
	limits.MaxDepth = 8
	if out, f := hostRun(t, deepen, flat(8), RenderDefault, limits); f != nil || out != strings.Repeat("[", 8)+strings.Repeat("]", 8)+"\n" {
		t.Errorf("%q %v", out, f)
	}
	if _, f := hostRun(t, deepen, flat(9), RenderDefault, limits); f == nil || f.Code != CodeResourceLimitExceeded || f.Limit.Name != "max_depth" {
		t.Errorf("%v", f)
	}
	widen := mustCompile(t, "def export [input] (json (scan-emit [] (fn [s e] (transition s [object-start (key (repeat 40 \"k\")) e object-end])) (fn [s] []) (events input)))", "widen.alc")
	keyLimits := tt.DefaultLimits()
	keyLimits.MaxKeyBytes = 16
	if _, f := hostRun(t, widen, "1", RenderDefault, keyLimits); f == nil || f.Code != CodeResourceLimitExceeded || f.Limit.Name != "max_key_bytes" {
		t.Errorf("%v", f)
	}
}
