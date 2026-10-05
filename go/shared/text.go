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
