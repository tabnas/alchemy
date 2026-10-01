// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

import (
	"fmt"
	"math"
	"strconv"
	"strings"
	"unicode/utf8"

	tr "github.com/tabnas/render/go"
	tt "github.com/tabnas/transduce/go"
)

// natives.go: the natives' implementations (rs/src/stdlib/registry.rs),
// keyed by name beside the registry's table of names, arities, kinds,
// signatures and effects (registry.go), so that table stays the one copy
// of the metadata; TestEveryNativeHasAnImplementation holds the two to the
// same names.
//
// Every operator takes its data last (spec section 9.3). The
// implementations are eager for values and build plans for streams and
// texts (spec section 10.3): `map` over a vector maps now, `map` over a
// stream answers a plan the runtime lowers once.
//
// Failures on data (a descriptor that is not an object, a path segment
// that is not a string) are INPUT_INVALID, the code the native table
// transducer raises for the same shapes, so the interpreted and the
// native standard library fail alike; a value of the wrong kind where the
// program, not the data, chose it is DSL_TYPE_ERROR.

// nativeCall is the implementation of a native: the runtime (to apply
// functions), the arguments, and the span of the form being evaluated,
// for a diagnostic.
type nativeCall func(rt *Runtime, a []Val, at SourceSpan) (Val, *Fail)

// nativeImpls are the natives' implementations, by name. Filled in init,
// since several implementations apply functions, and applying a native
// reads this map.
var nativeImpls map[string]nativeCall

func init() {
	nativeImpls = map[string]nativeCall{
		"get":             nGet,
		"get-path":        nGetPath,
		"as-path":         nAsPath,
		"as-vector":       nAsVector,
		"record":          nRecord,
		"entry":           nEntry,
		"vector":          nVector,
		"push":            nPush,
		"pop":             nPop,
		"top":             nTop,
		"count":           nCount,
		"keys":            nKeys,
		"length":          nLength,
		"compare":         nCompare,
		"number-class":    nNumberClass,
		"kind":            nKind,
		"path":            nPath,
		"root":            nRoot,
		"each-index":      nEachIndex,
		"each-member":     nEachMember,
		"property":        nProperty,
		"index":           nIndex,
		"compose":         nCompose,
		"capture":         nCapture,
		"route":           nRoute,
		"select":          nSelect,
		"events":          nEvents,
		"scan-emit":       nScanEmit,
		"transition":      nTransition,
		"partial":         nPartial,
		"map":             nMap,
		"filter":          nFilter,
		"concat-map":      nConcatMap,
		"join":            nJoin,
		"concat":          nConcat,
		"text":            nText,
		"replace-text":    nReplaceText,
		"scalar-text":     nScalarText,
		"quoted":          nQuoted,
		"string-join":     nStringJoin,
		"repeat":          nRepeat,
		"fail":            nFail,
		"is-ready":        nIsReady,
		"require-columns": nRequireColumns,
		"schema":          nSchema,
		"row":             constructor("row"),
		"ready":           constructor("ready"),
		"selected":        constructor("selected"),
		"table-end":       constructor("table-end"),
		"no-schema":       constructor("no-schema"),
		"missing":         constructor(MissingTag),
		"object-start":    constructor("object-start"),
		"object-end":      constructor("object-end"),
		"array-start":     constructor("array-start"),
		"array-end":       constructor("array-end"),
		"key":             nKey,
		"scalar":          nScalar,
		"json":            nJSON,
		"records":         nRecords,
		"csv-table":       nCsvTable,
	}
}

// ---------------------------------------------------------------------------
// Argument helpers
// ---------------------------------------------------------------------------

func inputInvalid(message string) *Fail { return tt.InputFail(message) }

// isData is whether a value is data (what a document can hold), as
// opposed to a function, a selector, a capture, a stream or a text.
func isData(v Val) bool {
	switch v.(type) {
	case NullVal, BoolVal, NumVal, StrVal, *VectorVal, *RecordVal:
		return true
	}
	return IsMissing(v)
}

func asStr(op, what string, v Val) (string, *Fail) {
	if s, ok := v.(StrVal); ok {
		return string(s), nil
	}
	return "", typeError(fmt.Sprintf("%s: %s must be a string, not %s", op, what, KindOf(v)))
}

func asKeyword(op, what string, v Val) (string, *Fail) {
	if k, ok := v.(KeywordVal); ok {
		return string(k), nil
	}
	return "", typeError(fmt.Sprintf("%s: %s must be a keyword, not %s", op, what, KindOf(v)))
}

func asFn(op, what string, v Val) (Fn, *Fail) {
	if f, ok := v.(Fn); ok {
		return f, nil
	}
	return nil, typeError(fmt.Sprintf("%s: %s must be a function, not %s", op, what, KindOf(v)))
}

func asSelector(op, what string, v Val) (tt.Selector, *Fail) {
	if s, ok := v.(*SelectorVal); ok {
		return s.Selector, nil
	}
	return tt.Selector{}, typeError(fmt.Sprintf("%s: %s must be a selector, not %s", op, what, KindOf(v)))
}

func asStream(op string, v Val) (*Plan, *Fail) {
	if s, ok := v.(StreamVal); ok {
		return s.Plan, nil
	}
	return nil, typeError(fmt.Sprintf("%s: the data must be a stream, not %s", op, KindOf(v)))
}

func asSeq(op string, v Val) (Seq, *Fail) {
	switch x := v.(type) {
	case *VectorVal:
		return Seq{Vector: x.Items}, nil
	case StreamVal:
		return Seq{Stream: x.Plan}, nil
	}
	return Seq{}, typeError(fmt.Sprintf("%s: the data must be a vector or a stream, not %s", op, KindOf(v)))
}

// textlike is a string or a text: what the text algebra accepts as an
// item.
func textlike(op string, v Val) *Fail {
	switch v.(type) {
	case StrVal, TextVal:
		return nil
	}
	return typeError(fmt.Sprintf("%s: expected a string or a text, not %s", op, KindOf(v)))
}

// asItems is the items of a vector the program built (its stack, its
// outputs), or the type error that names the operator: what is not a
// vector here is the program's mistake, not the data's (`as-vector` takes
// data).
func asItems(op string, v Val) ([]Val, *Fail) {
	if x, ok := v.(*VectorVal); ok {
		return x.Items, nil
	}
	return nil, typeError(fmt.Sprintf("%s: the data must be a vector, not %s", op, KindOf(v)))
}

// isWhole is whether a number has no fractional part, as Rust's
// `fract() == 0.0` answers it: never for an infinity or NaN, whose
// fractional part is NaN.
func isWhole(v float64) bool { return v-math.Trunc(v) == 0 }

// asIndex is a non-negative whole number that fits an index.
func asIndex(v Val) (int, bool) {
	n, ok := v.(NumVal)
	if !ok || !(n.Value >= 0) || !isWhole(n.Value) || n.Value > math.MaxUint32 {
		return 0, false
	}
	return int(n.Value), true
}

// option is the field of the options record, or the type error that names
// it.
func option(op string, options Val, key string) (Val, *Fail) {
	if _, ok := options.(*RecordVal); !ok {
		return nil, typeError(fmt.Sprintf("%s: the options must be a record, not %s", op, KindOf(options)))
	}
	v, _ := Field(options, key)
	if IsMissing(v) {
		return nil, typeError(fmt.Sprintf("%s: the options have no :%s", op, key))
	}
	return v, nil
}

// ---------------------------------------------------------------------------
// Numbers
// ---------------------------------------------------------------------------

// shortestNumber is the shortest text that reads back as value, laid out
// as the render package lays it out: positional within the JavaScript
// range, exponent form outside it. The two must agree byte for byte, which
// the differential test checks on every number without a lexeme, so this
// is render's own formatter.
func shortestNumber(value float64) string { return tr.FormatValue(value) }

// numberText is the text of a number as a renderer writes it, with the
// renderer's checks: a lexeme that is not a JSON number is INVALID_NUMBER,
// a non-finite value TARGET_VALUE_UNREPRESENTABLE.
func numberText(value float64, lexeme string, hasLexeme bool) (string, *Fail) {
	if hasLexeme && !tr.IsJSONNumber(lexeme) {
		return "", NewFail(CodeInvalidNumber, strconv.Quote(lexeme)+" is not a JSON number")
	}
	if math.IsNaN(value) || math.IsInf(value, 0) {
		message := floatText(value) + " has no representation as a number"
		if hasLexeme {
			message = fmt.Sprintf("%s is %s as a number, which has no representation", strconv.Quote(lexeme), floatText(value))
		}
		return "", NewFail(CodeTargetValueUnrepresentable, message)
	}
	if hasLexeme {
		return lexeme, nil
	}
	return shortestNumber(value), nil
}

// ---------------------------------------------------------------------------
// The implementations
// ---------------------------------------------------------------------------

func nGet(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	var key string
	switch k := a[0].(type) {
	case KeywordVal:
		key = string(k)
	case StrVal:
		key = string(k)
	default:
		return nil, typeError("get: the key must be a keyword or a string, not " + KindOf(a[0]))
	}
	return getField(key, a[1])
}

// getField is `get key data`: a record's field, `missing` for an absent
// one or from `missing`; INPUT_INVALID from other data, `type_mismatch`
// from what is not data. The native table binds a column record by the
// same rule, so a column function that answers something other than a
// record fails alike both ways.
func getField(key string, data Val) (Val, *Fail) {
	switch {
	case isRecord(data):
		v, _ := Field(data, key)
		return v, nil
	case IsMissing(data):
		return Missing(), nil
	case isData(data):
		return nil, inputInvalid(fmt.Sprintf("get: %s has no member %s; an object was expected", KindOf(data), strconv.Quote(key)))
	}
	return nil, typeError("get: a record was expected, not " + KindOf(data))
}

func isRecord(v Val) bool {
	_, ok := v.(*RecordVal)
	return ok
}

func nGetPath(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	selector, f := asSelector("get-path", "the path", a[0])
	if f != nil {
		return nil, f
	}
	segments, ok := selectorSegments(selector)
	if !ok {
		return nil, typeError("get-path: the path must name one location, not " + selector.String())
	}
	if !isData(a[1]) {
		return nil, typeError("get-path: the data must be a value, not " + KindOf(a[1]))
	}
	return GetPath(a[1], segments), nil
}

func nAsPath(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	items, ok := a[0].(*VectorVal)
	if !ok {
		return nil, inputInvalid("as-path: a path must be an array of segments, not " + KindOf(a[0]))
	}
	selector := tt.Root()
	for _, item := range items.Items {
		if s, ok := item.(StrVal); ok {
			selector = selector.Property(string(s))
			continue
		}
		if i, ok := asIndex(item); ok {
			selector = selector.Index(i)
			continue
		}
		return nil, inputInvalid("as-path: a path segment must be a string or a non-negative integer, not " + KindOf(item))
	}
	return &SelectorVal{Selector: selector}, nil
}

func nAsVector(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	switch {
	case isVector(a[0]):
		return a[0], nil
	case isData(a[0]):
		return nil, inputInvalid("as-vector: an array was expected, not " + KindOf(a[0]))
	}
	return nil, typeError("as-vector: a vector was expected, not " + KindOf(a[0]))
}

func isVector(v Val) bool {
	_, ok := v.(*VectorVal)
	return ok
}

func nRecord(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	r := &RecordVal{keys: make([]string, 0, len(a)), vals: make([]Val, 0, len(a))}
	for _, item := range a {
		t, ok := item.(*TaggedVal)
		if !ok || t.Tag != "entry" || len(t.Fields) != 2 {
			return nil, typeError("record: every argument must be an entry, not " + KindOf(item))
		}
		key, f := asKeyword("record", "an entry's key", t.Fields[0])
		if f != nil {
			return nil, f
		}
		if r.Set(key, t.Fields[1]) {
			return nil, NewFail(CodeDSLTypeError, "duplicate_key: record has two entries for :"+key)
		}
	}
	return r, nil
}

func nEntry(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	if _, f := asKeyword("entry", "the key", a[0]); f != nil {
		return nil, f
	}
	return NewTagged("entry", append([]Val(nil), a...)...), nil
}

func nVector(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	for _, v := range a {
		if IsLive(v) {
			return nil, typeError(fmt.Sprintf("vector: a vector cannot hold %s; a stream is used once, where it is", liveKind(v)))
		}
	}
	return Vector(append([]Val(nil), a...)...), nil
}

// nPush is `push item vector`: a new vector, the item last. Bounded by the
// vector's length, as the three below are: a stack of markers is as long
// as the document's nesting, never its length.
func nPush(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	if IsLive(a[0]) {
		return nil, typeError(fmt.Sprintf("push: a vector cannot hold %s; a stream is used once, where it is", liveKind(a[0])))
	}
	items, f := asItems("push", a[1])
	if f != nil {
		return nil, f
	}
	out := make([]Val, 0, len(items)+1)
	out = append(out, items...)
	return Vector(append(out, a[0])...), nil
}

func nPop(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	items, f := asItems("pop", a[0])
	if f != nil {
		return nil, f
	}
	if len(items) == 0 {
		return nil, typeError("pop: the vector is empty; there is no last item to remove")
	}
	return Vector(append([]Val(nil), items[:len(items)-1]...)...), nil
}

func nTop(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	items, f := asItems("top", a[0])
	if f != nil {
		return nil, f
	}
	if len(items) == 0 {
		return nil, typeError("top: the vector is empty; there is no last item")
	}
	return items[len(items)-1], nil
}

func nCount(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	items, f := asItems("count", a[0])
	if f != nil {
		return nil, f
	}
	return Num(float64(len(items))), nil
}

// nKeys is `keys record`: the record's keys as strings, in its order,
// which for a captured object is the document's. A bounded operation over
// one record, not a fold: the library's inferred table binding reads its
// columns from the first row with it.
func nKeys(rt *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	r, ok := a[0].(*RecordVal)
	if !ok {
		return nil, typeError("keys: the record must be a record, not " + KindOf(a[0]))
	}
	// A step per key, so the host's abort flag stops a wide record's copy
	// as it stops any other long evaluation.
	out := make([]Val, 0, len(r.keys))
	for _, k := range r.keys {
		if f := rt.Tick(); f != nil {
			return nil, f
		}
		out = append(out, StrVal(k))
	}
	return Vector(out...), nil
}

// lengthChunk is how many bytes of a string `length` counts per
// evaluation step.
const lengthChunk = 64 * 1024

// nLength is `length string`: how many characters the string holds, as a
// column counts them (Unicode scalar values). A long string is counted a
// chunk at a time, an evaluation step each, so the host's abort flag stops
// the count as it stops any long evaluation.
func nLength(rt *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	s, f := asStr("length", "the string", a[0])
	if f != nil {
		return nil, f
	}
	count := 0
	for start := 0; start < len(s); start += lengthChunk {
		if f := rt.Tick(); f != nil {
			return nil, f
		}
		end := start + lengthChunk
		if end > len(s) {
			end = len(s)
		}
		// A character begins at every byte that is not a UTF-8
		// continuation byte (10xxxxxx), wherever the chunk is cut.
		for i := start; i < end; i++ {
			if s[i]&0xC0 != 0x80 {
				count++
			}
		}
	}
	return Num(float64(count)), nil
}

// nCompare is `compare a b`: how two numbers are ordered, `:less`,
// `:equal` or `:greater`, and `:unordered` when either is NaN; -0 and 0
// are equal.
func nCompare(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	number := func(v Val, which string) (float64, *Fail) {
		if n, ok := v.(NumVal); ok {
			return n.Value, nil
		}
		return 0, typeError(fmt.Sprintf("compare: the %s must be a number, not %s", which, KindOf(v)))
	}
	x, f := number(a[0], "first")
	if f != nil {
		return nil, f
	}
	y, f := number(a[1], "second")
	if f != nil {
		return nil, f
	}
	switch {
	case x < y:
		return KeywordVal("less"), nil
	case x > y:
		return KeywordVal("greater"), nil
	case x == y:
		return KeywordVal("equal"), nil
	}
	return KeywordVal("unordered"), nil
}

// nNumberClass is `number-class number`: `:finite`, `:infinity`,
// `:negative-infinity` or `:nan`, so that a program writing a format with
// spellings for the numbers JSON has none for (YAML's `.inf`, `-.inf` and
// `.nan`) can choose them; `scalar-text` refuses those numbers, as JSON
// and CSV must.
func nNumberClass(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	n, ok := a[0].(NumVal)
	if !ok {
		return nil, typeError("number-class: the number must be a number, not " + KindOf(a[0]))
	}
	switch {
	case math.IsNaN(n.Value):
		return KeywordVal("nan"), nil
	case math.IsInf(n.Value, 1):
		return KeywordVal("infinity"), nil
	case math.IsInf(n.Value, -1):
		return KeywordVal("negative-infinity"), nil
	}
	return KeywordVal("finite"), nil
}

// nKind is the kind of a value as a keyword, so that a program can tell a
// string from a number, which no match pattern does: the words KindOf
// uses in messages, without their article.
func nKind(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	// The checker refuses a stream or a text where `kind` is named; this
	// is where `kind` passed as a function meets one (`map kind [(text
	// "x")]`), and a finite text is refused as a live one is.
	word, ok := kindWord(a[0])
	if !ok {
		return nil, typeError(fmt.Sprintf("kind: %s cannot be asked; a stream or a text is used where it is, not inspected", KindOf(a[0])))
	}
	return KeywordVal(word), nil
}

// kindWord is the word `kind` answers for a retained value; none for a
// stream or a text, live or finite, which `kind` refuses.
func kindWord(v Val) (string, bool) {
	switch v.(type) {
	case NullVal:
		return "null", true
	case BoolVal:
		return "boolean", true
	case NumVal:
		return "number", true
	case StrVal:
		return "string", true
	case KeywordVal:
		return "keyword", true
	case *VectorVal:
		return "vector", true
	case *RecordVal:
		return "record", true
	case Fn:
		return "function", true
	case *SelectorVal:
		return "selector", true
	case *CaptureVal:
		return "capture", true
	case *TaggedVal:
		if IsMissing(v) {
			return "missing", true
		}
		return "tagged", true
	}
	return "", false
}

func nPath(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	selector := tt.Root()
	for _, item := range a {
		switch x := item.(type) {
		case StrVal:
			selector = selector.Property(string(x))
			continue
		case *SelectorVal:
			selector = selector.Compose(x.Selector)
			continue
		}
		if i, ok := asIndex(item); ok {
			selector = selector.Index(i)
			continue
		}
		return nil, typeError("path: a segment must be a string, a non-negative integer or a selector, not " + KindOf(item))
	}
	return &SelectorVal{Selector: selector}, nil
}

func nRoot(_ *Runtime, _ []Val, _ SourceSpan) (Val, *Fail) {
	return &SelectorVal{Selector: tt.Root()}, nil
}

func nEachIndex(_ *Runtime, _ []Val, _ SourceSpan) (Val, *Fail) {
	return &SelectorVal{Selector: tt.Root().EachIndex()}, nil
}

func nEachMember(_ *Runtime, _ []Val, _ SourceSpan) (Val, *Fail) {
	return &SelectorVal{Selector: tt.Root().EachMember()}, nil
}

func nProperty(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	name, f := asStr("property", "the name", a[0])
	if f != nil {
		return nil, f
	}
	return &SelectorVal{Selector: tt.Root().Property(name)}, nil
}

func nIndex(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	i, ok := asIndex(a[0])
	if !ok {
		return nil, typeError("index: the position must be a non-negative integer, not " + DebugString(a[0]))
	}
	return &SelectorVal{Selector: tt.Root().Index(i)}, nil
}

func nCompose(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	first, f := asSelector("compose", "the first selector", a[0])
	if f != nil {
		return nil, f
	}
	second, f := asSelector("compose", "the second selector", a[1])
	if f != nil {
		return nil, f
	}
	return &SelectorVal{Selector: first.Compose(second)}, nil
}

// CaptureLimits are the Limits fields a capture may be bounded by, by the
// keyword that names each; the byte count is the host's, read when the
// plan is lowered (captureBudget).
var CaptureLimits = []string{"max_capture_bytes", "max_metadata_bytes", "max_record_bytes"}

// captureBudget is the bytes the host's limits give the capture limit
// name.
func captureBudget(limits tt.Limits, name string) (int, bool) {
	switch name {
	case "max_capture_bytes":
		return limits.MaxCaptureBytes, true
	case "max_metadata_bytes":
		return limits.MaxMetadataBytes, true
	case "max_record_bytes":
		return limits.MaxRecordBytes, true
	}
	return 0, false
}

func nCapture(rt *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	tag, f := asKeyword("capture", "the tag", a[0])
	if f != nil {
		return nil, f
	}
	selector, f := asSelector("capture", "the selector", a[1])
	if f != nil {
		return nil, f
	}
	spec := tt.MaterializeSpec(tag, selector)
	if len(a) > 2 {
		limit, f := asKeyword("capture", "the limit", a[2])
		if f != nil {
			return nil, f
		}
		name := ""
		for _, n := range CaptureLimits {
			if n == limit {
				name = n
			}
		}
		if name == "" {
			return nil, typeError(fmt.Sprintf("capture: the limit must be one of :%s, not :%s", strings.Join(CaptureLimits, ", :"), limit))
		}
		bytes, ok := captureBudget(rt.Limits(), name)
		if !ok {
			bytes = math.MaxInt
		}
		spec = spec.WithBudget(bytes, name)
	}
	return &CaptureVal{Spec: spec}, nil
}

func nRoute(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	items, ok := a[0].(*VectorVal)
	if !ok {
		return nil, typeError("route: the captures must be a vector, not " + KindOf(a[0]))
	}
	specs := make([]tt.CaptureSpec, 0, len(items.Items))
	for _, item := range items.Items {
		c, ok := item.(*CaptureVal)
		if !ok {
			return nil, typeError("route: every capture must be a capture, not " + KindOf(item))
		}
		specs = append(specs, c.Spec)
	}
	source, f := asStream("route", a[1])
	if f != nil {
		return nil, f
	}
	return StreamVal{Plan: &Plan{Kind: PlanRoute, Specs: specs, Source: source}}, nil
}

func nSelect(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	selector, f := asSelector("select", "the selector", a[0])
	if f != nil {
		return nil, f
	}
	source, f := asStream("select", a[1])
	if f != nil {
		return nil, f
	}
	return StreamVal{Plan: &Plan{Kind: PlanSelect, Selector: selector, Source: source}}, nil
}

func nScanEmit(_ *Runtime, a []Val, at SourceSpan) (Val, *Fail) {
	step, f := asFn("scan-emit", "the step", a[1])
	if f != nil {
		return nil, f
	}
	finish, f := asFn("scan-emit", "the finish", a[2])
	if f != nil {
		return nil, f
	}
	source, f := asStream("scan-emit", a[3])
	if f != nil {
		return nil, f
	}
	return StreamVal{Plan: &Plan{Kind: PlanScanEmit, Init: a[0], Step: step, Finish: finish, Source: source, At: at}}, nil
}

func nTransition(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	if !isVector(a[1]) {
		return nil, typeError("transition: the outputs must be a vector, not " + KindOf(a[1]))
	}
	return NewTagged("transition", append([]Val(nil), a...)...), nil
}

func nPartial(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	fn, f := asFn("partial", "the function", a[0])
	if f != nil {
		return nil, f
	}
	return &Partial{F: fn, Args: append([]Val(nil), a[1:]...)}, nil
}

func nMap(rt *Runtime, a []Val, at SourceSpan) (Val, *Fail) {
	fn, f := asFn("map", "the function", a[0])
	if f != nil {
		return nil, f
	}
	seq, f := asSeq("map", a[1])
	if f != nil {
		return nil, f
	}
	if seq.IsStream() {
		return StreamVal{Plan: &Plan{Kind: PlanMap, F: fn, Source: seq.Stream, At: at}}, nil
	}
	out := make([]Val, 0, len(seq.Vector))
	for _, item := range seq.Vector {
		v, f := rt.Apply(fn, []Val{item}, at)
		if f != nil {
			return nil, f
		}
		out = append(out, v)
	}
	return Vector(out...), nil
}

// truth is the boolean an `if`, a `filter` or a `case` decides by;
// anything else is a type error, since nothing is implicitly true or
// false.
func truth(op string, v Val) (bool, *Fail) {
	if b, ok := v.(BoolVal); ok {
		return bool(b), nil
	}
	return false, typeError(fmt.Sprintf("%s: a boolean was expected, not %s", op, KindOf(v)))
}

func nFilter(rt *Runtime, a []Val, at SourceSpan) (Val, *Fail) {
	fn, f := asFn("filter", "the predicate", a[0])
	if f != nil {
		return nil, f
	}
	seq, f := asSeq("filter", a[1])
	if f != nil {
		return nil, f
	}
	if seq.IsStream() {
		return StreamVal{Plan: &Plan{Kind: PlanFilter, F: fn, Source: seq.Stream, At: at}}, nil
	}
	out := []Val{}
	for _, item := range seq.Vector {
		keep, f := rt.Apply(fn, []Val{item}, at)
		if f != nil {
			return nil, f
		}
		yes, f := truth("filter", keep)
		if f != nil {
			return nil, f
		}
		if yes {
			out = append(out, item)
		}
	}
	return Vector(out...), nil
}

func nConcatMap(_ *Runtime, a []Val, at SourceSpan) (Val, *Fail) {
	fn, f := asFn("concat-map", "the function", a[0])
	if f != nil {
		return nil, f
	}
	seq, f := asSeq("concat-map", a[1])
	if f != nil {
		return nil, f
	}
	return TextVal{Plan: &Plan{Kind: PlanConcatMap, F: fn, Seq: seq, At: at}}, nil
}

func nJoin(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	sep, f := asStr("join", "the separator", a[0])
	if f != nil {
		return nil, f
	}
	seq, f := asSeq("join", a[1])
	if f != nil {
		return nil, f
	}
	for _, item := range seq.Vector {
		if f := textlike("join", item); f != nil {
			return nil, f
		}
	}
	return TextVal{Plan: &Plan{Kind: PlanJoin, Sep: sep, Seq: seq}}, nil
}

func nConcat(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	live := 0
	for _, item := range a {
		if f := textlike("concat", item); f != nil {
			return nil, f
		}
		if IsLive(item) {
			live++
		}
	}
	if live > 1 {
		return nil, NewFail(CodeStreamReused, "reused: concat was given two live texts; the input is consumed once")
	}
	return TextVal{Plan: newConcat(append([]Val(nil), a...))}, nil
}

func nText(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	s, f := asStr("text", "the argument", a[0])
	if f != nil {
		return nil, f
	}
	return TextVal{Plan: litPlan(s)}, nil
}

func nReplaceText(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	from, f := asStr("replace-text", "the literal to find", a[0])
	if f != nil {
		return nil, f
	}
	to, f := asStr("replace-text", "the replacement", a[1])
	if f != nil {
		return nil, f
	}
	if f := textlike("replace-text", a[2]); f != nil {
		return nil, f
	}
	return TextVal{Plan: &Plan{Kind: PlanReplace, From: from, To: to, Text: a[2]}}, nil
}

// nScalarText is the text of a scalar cell under the options' policies:
// what the CSV renderer writes for the same cell, so that the interpreted
// and the native `csv` agree byte for byte.
func nScalarText(rt *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	options, cell := a[0], a[1]
	switch x := cell.(type) {
	case NullVal:
		v, f := option("scalar-text", options, "null-text")
		if f != nil {
			return nil, f
		}
		s, f := asStr("scalar-text", ":null-text", v)
		if f != nil {
			return nil, f
		}
		return StrVal(s), nil
	case BoolVal:
		return StrVal(strconv.FormatBool(bool(x))), nil
	case NumVal:
		s, f := numberText(x.Value, x.Lexeme, x.HasLexeme)
		if f != nil {
			return nil, f
		}
		return StrVal(s), nil
	case StrVal:
		return x, nil
	case *VectorVal, *RecordVal:
		s, f := rt.jsonText(cell)
		if f != nil {
			return nil, f
		}
		return StrVal(s), nil
	}
	if IsMissing(cell) {
		v, f := option("scalar-text", options, "missing")
		if f != nil {
			return nil, f
		}
		switch m := v.(type) {
		case KeywordVal:
			if m == "error" {
				return nil, NewFail(CodeMissingValue, "a cell has no value and the options map missing to :error")
			}
		case StrVal:
			return m, nil
		}
		return nil, typeError("scalar-text: :missing must be :error or a string, not " + KindOf(v))
	}
	return nil, typeError("scalar-text: a scalar was expected, not " + KindOf(cell))
}

// quote is the double-quoted form of s: the JSON string form (RFC 8259's
// escapes for the quote, the backslash and U+0000 to U+001F, the short
// ones where they exist, \u00xx otherwise, in the render package's
// lowercase) with U+007F to U+009F escaped the same way, since YAML's
// double-quoted style reads JSON's escapes but its printable set excludes
// the C1 controls. Every other character is written as itself.
func quote(s string) string {
	var b strings.Builder
	b.Grow(quotedLen(s))
	b.WriteByte('"')
	for _, c := range s {
		switch {
		case c == '"':
			b.WriteString(`\"`)
		case c == '\\':
			b.WriteString(`\\`)
		case c == '\n':
			b.WriteString(`\n`)
		case c == '\t':
			b.WriteString(`\t`)
		case c == '\r':
			b.WriteString(`\r`)
		case c == '\b':
			b.WriteString(`\b`)
		case c == '\f':
			b.WriteString(`\f`)
		case c < 0x20 || (c >= 0x7f && c <= 0x9f):
			fmt.Fprintf(&b, `\u%04x`, c)
		default:
			b.WriteRune(c)
		}
	}
	b.WriteByte('"')
	return b.String()
}

// quotedLen is the length of quote's result, counted before it is built:
// two quotes, and each character as itself, as a two-byte escape, or as
// the six bytes of \u00XX.
func quotedLen(s string) int {
	n := 2
	for _, c := range s {
		switch {
		case c == '"' || c == '\\' || c == '\n' || c == '\t' || c == '\r' || c == '\b' || c == '\f':
			n += 2
		case c < 0x20 || (c >= 0x7f && c <= 0x9f):
			n += 6
		default:
			n += utf8.RuneLen(c)
		}
	}
	return n
}

// nQuoted is `quoted string`: the double-quoted form. The result is one
// scalar of the output, up to six bytes per byte of the string, so it is
// held to max_scalar_bytes, refused before it is built.
func nQuoted(rt *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	s, f := asStr("quoted", "the string", a[0])
	if f != nil {
		return nil, f
	}
	max := rt.Limits().MaxScalarBytes
	if n := quotedLen(s); n > max {
		return nil, tt.LimitFail("max_scalar_bytes", uint64(max),
			fmt.Sprintf("quoted: the quoted form is %d bytes, more than %d", n, max))
	}
	return StrVal(quote(s)), nil
}

// nStringJoin is `string-join separator strings`: the strings of a vector
// joined into one string, the separator between them, so that a program
// can build one scalar from several: a table cell from the runs of a
// Markdown cell, a failure message that names a key. The result is one
// scalar, so it is held to max_scalar_bytes, refused before it is built.
func nStringJoin(rt *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	separator, f := asStr("string-join", "the separator", a[0])
	if f != nil {
		return nil, f
	}
	items, f := asItems("string-join", a[1])
	if f != nil {
		return nil, f
	}
	n := 0
	for i, item := range items {
		s, ok := item.(StrVal)
		if !ok {
			return nil, typeError("string-join: every item must be a string, not " + KindOf(item))
		}
		n += len(s)
		if i > 0 {
			n += len(separator)
		}
	}
	max := rt.Limits().MaxScalarBytes
	if n > max {
		return nil, tt.LimitFail("max_scalar_bytes", uint64(max),
			fmt.Sprintf("string-join: the joined string is %d bytes, more than %d", n, max))
	}
	var b strings.Builder
	b.Grow(n)
	for i, item := range items {
		if i > 0 {
			b.WriteString(separator)
		}
		b.WriteString(string(item.(StrVal)))
	}
	return StrVal(b.String()), nil
}

// nRepeat is `repeat count string`: the string count times over. The
// result is one scalar of the output (a line's indentation), so it is held
// to max_scalar_bytes, refused before it is built.
func nRepeat(rt *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	// The count is judged before it is narrowed, so that any whole
	// non-negative count past the limit is the limit's refusal, and only a
	// negative or a fractional one is a type error.
	n, ok := a[0].(NumVal)
	if !ok {
		return nil, typeError("repeat: the count must be a number, not " + KindOf(a[0]))
	}
	if !(n.Value >= 0) || !isWhole(n.Value) {
		spelled := floatText(n.Value)
		if n.HasLexeme {
			spelled = n.Lexeme
		}
		return nil, typeError("repeat: the count must be a whole number of at least 0, not " + spelled)
	}
	s, f := asStr("repeat", "the string", a[1])
	if f != nil {
		return nil, f
	}
	max := rt.Limits().MaxScalarBytes
	if n.Value*float64(len(s)) > float64(max) {
		return nil, tt.LimitFail("max_scalar_bytes", uint64(max),
			fmt.Sprintf("repeat: %s times %d bytes is more than %d", floatText(n.Value), len(s), max))
	}
	// Within the limit, the count fits an int, or the string is empty.
	if s == "" {
		return StrVal(""), nil
	}
	return StrVal(strings.Repeat(s, int(n.Value))), nil
}

func nFail(rt *Runtime, a []Val, at SourceSpan) (Val, *Fail) {
	message, f := asStr("fail", "the message", a[0])
	if f != nil {
		return nil, f
	}
	return nil, rt.FailAt(NewFail(CodeInputInvalid, message), at)
}

func isReadyValue(v Val) bool {
	t, ok := v.(*TaggedVal)
	return ok && t.Tag == "ready" && len(t.Fields) == 1
}

func nIsReady(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	return BoolVal(isReadyValue(a[0])), nil
}

func nRequireColumns(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	if isReadyValue(a[0]) {
		return a[0].(*TaggedVal).Fields[0], nil
	}
	return nil, NewFail(CodeInputOrderViolation,
		"a row began before the column metadata had completed; rows must follow their metadata")
}

// constructor is a native whose value is a tagged value with its name as
// the tag.
func constructor(name string) nativeCall {
	return func(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
		return NewTagged(name, append([]Val(nil), a...)...), nil
	}
}

// nSchema is `schema columns`: the table's one schema. A table has at most
// max_columns columns, so a schema past it is refused where it is built,
// whatever consumes it: the native table binds no more, and the library's
// twin of it, whose events may reach no renderer that would check them, is
// held to the same count.
func nSchema(rt *Runtime, a []Val, at SourceSpan) (Val, *Fail) {
	if columns, ok := a[0].(*VectorVal); ok {
		max := rt.Limits().MaxColumns
		if len(columns.Items) > max {
			return nil, tt.LimitFail("max_columns", uint64(max),
				fmt.Sprintf("the schema declares %d columns, more than %d", len(columns.Items), max))
		}
	}
	return constructor("schema")(rt, a, at)
}

func nKey(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	if _, f := asStr("key", "the name", a[0]); f != nil {
		return nil, f
	}
	return NewTagged("key", append([]Val(nil), a...)...), nil
}

func nScalar(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	switch a[0].(type) {
	case NullVal, BoolVal, NumVal, StrVal:
		return NewTagged("scalar", append([]Val(nil), a...)...), nil
	}
	return nil, typeError("scalar: the value must be null, a boolean, a number or a string, not " + KindOf(a[0]))
}

func nEvents(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	source, f := asStream("events", a[0])
	if f != nil {
		return nil, f
	}
	return StreamVal{Plan: &Plan{Kind: PlanEvents, Source: source}}, nil
}

func nCsvTable(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	source, f := asStream("csv-table", a[1])
	if f != nil {
		return nil, f
	}
	return StreamVal{Plan: &Plan{Kind: PlanCsvTable, Options: a[0], Source: source}}, nil
}

func nJSON(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	source, f := asStream("json", a[0])
	if f != nil {
		return nil, f
	}
	return TextVal{Plan: &Plan{Kind: PlanJSON, Source: source}}, nil
}

func nRecords(_ *Runtime, a []Val, _ SourceSpan) (Val, *Fail) {
	source, f := asStream("records", a[0])
	if f != nil {
		return nil, f
	}
	return StreamVal{Plan: &Plan{Kind: PlanRecords, Source: source}}, nil
}
