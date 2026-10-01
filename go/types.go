// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

import "strings"

// types.go: the checker's types (rs/src/types.rs).
//
// The types tell three things apart that the syntax does not: a reusable
// value, a single-use resource, and a protocol. Value is any data a
// document can hold; Vector<T> a finite retained collection; Stream<T> an
// ordered single-use sequence, of which TableEvents (Stream<TableEvent>)
// and Stream<Event> are two; JsonEvents the single-use source; Text
// single-use incremental text; String a finite retained string. Unknown
// is what inference could not decide, accepted everywhere; Never is the
// type of `fail`.

// TypeKind is the kind of a Type.
type TypeKind uint8

// The kinds of type.
const (
	TUnknown TypeKind = iota
	TNever
	TNull
	TBool
	TNumber
	TString
	TKeyword
	TValue
	TVector
	TRecord
	TSelector
	TCaptureSpec
	TTagged
	TTableEvent
	TEvent
	TFn
	TStream
	TJsonEvents
	TText
)

// Type is a type. Item is a vector's or a stream's item type; Tag a tagged
// value's constructor name; Params and Result a function's.
type Type struct {
	Kind   TypeKind
	Item   *Type
	Tag    string
	Params []Type
	Result *Type
}

// The simple types.
var (
	Unknown     = Type{Kind: TUnknown}
	Never       = Type{Kind: TNever}
	NullT       = Type{Kind: TNull}
	BoolT       = Type{Kind: TBool}
	NumberT     = Type{Kind: TNumber}
	StringT     = Type{Kind: TString}
	KeywordT    = Type{Kind: TKeyword}
	ValueT      = Type{Kind: TValue}
	RecordT     = Type{Kind: TRecord}
	SelectorT   = Type{Kind: TSelector}
	CaptureSpec = Type{Kind: TCaptureSpec}
	TableEventT = Type{Kind: TTableEvent}
	EventT      = Type{Kind: TEvent}
	JsonEvents  = Type{Kind: TJsonEvents}
	TextT       = Type{Kind: TText}
)

// VectorOf is Vector<item>.
func VectorOf(item Type) Type { return Type{Kind: TVector, Item: &item} }

// StreamOf is Stream<item>.
func StreamOf(item Type) Type { return Type{Kind: TStream, Item: &item} }

// TableEvents is Stream<TableEvent>: the TableRows/1 protocol.
func TableEvents() Type { return StreamOf(TableEventT) }

// Events is Stream<Event>: the source's events as items.
func Events() Type { return StreamOf(EventT) }

// Tagged is a constructor's value, by the constructor's name.
func Tagged(name string) Type { return Type{Kind: TTagged, Tag: name} }

// Func is a function of params answering result.
func Func(params []Type, result Type) Type {
	return Type{Kind: TFn, Params: params, Result: &result}
}

// FuncOf is a function of n parameters about which nothing more is known.
func FuncOf(n int) Type {
	params := make([]Type, n)
	for i := range params {
		params[i] = Unknown
	}
	return Func(params, Unknown)
}

// IsAffine is whether a binding of this type is used at most once.
func (t Type) IsAffine() bool {
	return t.Kind == TStream || t.Kind == TJsonEvents || t.Kind == TText
}

// IsProtocol is whether this is one of the protocols, so a mismatch is
// between protocols rather than between kinds of value.
func (t Type) IsProtocol() bool { return t.IsAffine() }

// IsData is whether this type is data: what a document can hold.
func (t Type) IsData() bool {
	switch t.Kind {
	case TNull, TBool, TNumber, TString, TRecord, TValue, TUnknown, TNever:
		return true
	case TVector:
		return t.Item.IsData()
	case TTagged:
		return t.Tag == "missing"
	}
	return false
}

// IsStreamOrSource is whether this is a stream or the source: what a
// vector can never hold.
func (t Type) IsStreamOrSource() bool {
	return t.Kind == TStream || t.Kind == TJsonEvents
}

// IsStream is whether this type is a stream.
func (t Type) IsStream() bool { return t.Kind == TStream }

// IsTextlike is whether a text combinator takes a value of this type as an
// item.
func (t Type) IsTextlike() bool {
	switch t.Kind {
	case TString, TText, TValue, TUnknown, TNever:
		return true
	}
	return false
}

// Equal is structural equality.
func (t Type) Equal(o Type) bool {
	if t.Kind != o.Kind {
		return false
	}
	switch t.Kind {
	case TVector, TStream:
		return t.Item.Equal(*o.Item)
	case TTagged:
		return t.Tag == o.Tag
	case TFn:
		if len(t.Params) != len(o.Params) {
			return false
		}
		for i := range t.Params {
			if !t.Params[i].Equal(o.Params[i]) {
				return false
			}
		}
		return t.Result.Equal(*o.Result)
	}
	return true
}

// Accepts is whether a value of type actual may be given where t is
// expected. The checker is conservative: Value is any data, so it passes
// where a particular kind of data is wanted, as Unknown does; a type that
// can never be it does not.
func (t Type) Accepts(actual Type) bool {
	switch {
	case t.Kind == TUnknown, actual.Kind == TUnknown, actual.Kind == TNever:
		return true
	case t.Kind == TValue:
		return actual.IsData()
	case actual.Kind == TValue:
		return t.IsData()
	case t.Kind == TVector && actual.Kind == TVector, t.Kind == TStream && actual.Kind == TStream:
		return t.Item.Accepts(*actual.Item)
	case t.Kind == TTableEvent && actual.Kind == TTagged:
		switch actual.Tag {
		case "schema", "row", "table-end":
			return true
		}
		return false
	case t.Kind == TEvent && actual.Kind == TTagged:
		switch actual.Tag {
		case "object-start", "object-end", "array-start", "array-end", "key", "scalar":
			return true
		}
		return false
	case t.Kind == TFn && actual.Kind == TFn:
		return len(t.Params) == len(actual.Params) && t.Result.Accepts(*actual.Result)
	case t.Kind == TJsonEvents && actual.Kind == TStream:
		// A stream of events reaches any taker of JSON events.
		return EventT.Accepts(*actual.Item)
	}
	return t.Equal(actual)
}

// JoinTypes is the type of a value that is one of two: the same type when
// they agree, a text when one is a text and the other a string, else
// unknown.
func JoinTypes(a, b Type) Type {
	switch {
	case a.Kind == TNever:
		return b
	case b.Kind == TNever:
		return a
	case a.Equal(b):
		return a
	case a.Kind == TText && b.Kind == TString, a.Kind == TString && b.Kind == TText:
		return TextT
	case a.Kind == TVector && b.Kind == TVector:
		return VectorOf(JoinTypes(*a.Item, *b.Item))
	case a.IsData() && b.IsData():
		return ValueT
	}
	return Unknown
}

// ItemType is the type of the items of a Vector or Stream.
func (t Type) ItemType() (Type, bool) {
	if t.Kind == TVector || t.Kind == TStream {
		return *t.Item, true
	}
	return Type{}, false
}

// String is the type as the messages print it.
func (t Type) String() string {
	switch t.Kind {
	case TUnknown:
		return "Unknown"
	case TNever:
		return "Never"
	case TNull:
		return "Null"
	case TBool:
		return "Bool"
	case TNumber:
		return "Number"
	case TString:
		return "String"
	case TKeyword:
		return "Keyword"
	case TValue:
		return "Value"
	case TVector:
		return "Vector<" + t.Item.String() + ">"
	case TRecord:
		return "Record"
	case TSelector:
		return "Selector"
	case TCaptureSpec:
		return "CaptureSpec"
	case TTagged:
		return t.Tag
	case TTableEvent:
		return "TableEvent"
	case TEvent:
		return "Event"
	case TFn:
		var b strings.Builder
		b.WriteString("Fn(")
		for i, p := range t.Params {
			if i > 0 {
				b.WriteByte(' ')
			}
			b.WriteString(p.String())
		}
		b.WriteString(" -> ")
		b.WriteString(t.Result.String())
		b.WriteByte(')')
		return b.String()
	case TStream:
		if t.Item.Kind == TTableEvent {
			return "TableEvents"
		}
		return "Stream<" + t.Item.String() + ">"
	case TJsonEvents:
		return "JsonEvents"
	case TText:
		return "Text"
	}
	return "?"
}
