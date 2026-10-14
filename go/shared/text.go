// Copyright (c) 2026 tabnas, MIT License

package shared

// TextOut is a consumer of text fragments.
//
// Fragments arrive in order and are concatenated; where the boundaries
// fall carries no meaning. Flush pushes everything held so far to the
// final destination, and a renderer calls it exactly once, at the end of
// the protocol it renders, so that a document that failed half way is not
// flushed as if it were whole.
//
// HasCommitted reports whether any text has reached the final
// destination, so that a failure found now leaves partial output behind.
// A renderer asks it when it fails and reports CommittedOutput from the
// answer. An output that cannot tell answers true, the conservative
// answer (Rust's default method; Go interfaces have none, so every
// implementation says so itself).
type TextOut interface {
	WriteStr(s string) *Fail
	Flush() *Fail
	HasCommitted() bool
}

// RecordEnder is a TextOut that takes the end of a record: the text
// written so far ends with a whole record. A text that is a sequence of
// records marks the end of each one: a CSV row, a value directly inside a
// JSON document's root container, an item concat-map writes, or wherever
// a step of scan-emit answers :record-end among its outputs. render's
// WriteOut holds a record's text until its end is marked, so a run that
// fails part way leaves whole records behind and none half written. A
// combinator passes the mark on, after any text it holds that comes
// before it. It is an interface of its own, which RecordEnd asks for, so
// that a TextOut written before it still is one (Rust's default method
// and TypeScript's optional one do the same).
type RecordEnder interface {
	RecordEnd() *Fail
}

// RecordEnd marks the end of a record on out, when out takes it; an
// output that does not ignores the mark, which is right for an output
// that is the destination itself or holds the whole text.
func RecordEnd(out TextOut) *Fail {
	if r, ok := out.(RecordEnder); ok {
		return r.RecordEnd()
	}
	return nil
}
