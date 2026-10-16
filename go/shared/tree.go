// Copyright (c) 2026 tabnas, MIT License

package shared

// tree.go: the spec of the library's `transduce-tree`, for routers that
// run it natively (rs/src/shared/tree.rs).
//
// `transduce-tree spec input` (stdlib/transduce.alc) makes a tree's JSON
// events of a grammar's rule passes as spec says: a record keyed by rule
// name, each entry a record of what that rule's passes do. The library's
// text is the specification. A host's routers may run it natively
// (TreeRouters), and the lowering asks them to only when the program's
// spec has the standard shape TreeSpec holds: every entry a record whose
// :container is :object or :array, whose :key is a string or a token's
// index, and whose :scalar is a token's index or :value, each of the three
// left out at will. A spec of any other shape runs the library's text,
// which says what it does with it.
//
// What a native must do is what the text does, pass for pass:
//
//   - An open pass of a rule the spec names announces its :key first: the
//     string, or the :val of the pass's token at that index, nothing when
//     the pass consumed no such token.
//   - It then begins its :container, unless the rule it replaced carried
//     that container on (below), or yields its :scalar from the token at
//     that index, when the pass consumed one; :value yields nothing on an
//     open pass, and :scalar nothing for a container rule.
//   - A close pass of a container rule ends its container, unless the
//     pass's alternate replaces the rule with one the spec names a
//     container: then the container goes on, and the replacing rule's open
//     pass begins none. A close pass of any other rule yields its :scalar
//     :value, the pass's own value, when it has one.
//   - A pass of a rule the spec does not name yields nothing, and the end of
//     the passes inside a container is PROTOCOL_ORDER_ERROR.
//
// The library's failures are the native's, with their codes and texts: a
// key whose token's value is not a string, and a scalar whose token's
// value is a container, are DSL_TYPE_ERROR; a close with no container open
// is the library's `pop` of an empty stack, DSL_TYPE_ERROR too.

// TreeContainer is the container a rule's passes begin and end.
type TreeContainer uint8

// The containers; TreeNoContainer is a rule that is not one.
const (
	TreeNoContainer TreeContainer = iota
	TreeObject
	TreeArray
)

// TreeKey is the member name a rule's open pass announces: the :val of
// the pass's token at Token, or Name. Set says whether the rule has one.
type TreeKey struct {
	Set     bool
	IsToken bool
	Token   int
	Name    string
}

// TreeScalar is the scalar a rule yields: the :val of the open pass's
// token at Token, or the close pass's own value. Set says whether the rule
// has one.
type TreeScalar struct {
	Set   bool
	Value bool
	Token int
}

// TreeRule is what one rule's passes do: an entry of the spec. Scalar is
// read only when Container is TreeNoContainer, as the library reads it.
type TreeRule struct {
	Container TreeContainer
	Key       TreeKey
	Scalar    TreeScalar
}

// TreeSpec is a `transduce-tree` spec of the standard shape: the rules it
// names.
type TreeSpec struct {
	Rules map[string]TreeRule
}

// Rule is the entry of the rule named name, and whether the spec names
// it.
func (s TreeSpec) Rule(name string) (TreeRule, bool) {
	r, ok := s.Rules[name]
	return r, ok
}

// IsContainer says whether the rule named name is a container rule: what
// decides, at a close whose alternate replaces its rule with that one,
// that the container goes on.
func (s TreeSpec) IsContainer(name string) bool {
	r, ok := s.Rules[name]
	return ok && r.Container != TreeNoContainer
}

// TreeRouters is the optional interface of Routers that run the library's
// `transduce-tree` natively (transduce's NewTransduceTree): the parser's
// rule passes in and a tree's JSON events out to downstream, as TreeSpec
// says a native must make them. The lowering asks for it where a program
// applies `transduce-tree` to the host's parser events with a spec of the
// standard shape, the runtime's native compositions on, and hands it the
// sink the library's `as-events` would write to, the source's limits
// already guarding it. Routers that do not implement it, or answer nil,
// have the library's text run instead.
type TreeRouters interface {
	TransduceTree(spec TreeSpec, downstream Sink) ParserSink
}
