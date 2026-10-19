// Copyright (c) 2026 tabnas, MIT License

package shared

import "fmt"

// tree.go: the spec of the library's `transduce-tree`, for routers that
// run it natively (rs/src/shared/tree.rs).
//
// `transduce-tree spec input` (stdlib/transduce.alc) makes a tree's JSON
// events of a grammar's rule passes as spec says: a record keyed by rule
// name, each entry a record of what that rule's passes do. The library's
// text is the specification. A host's routers may run it natively
// (TreeRouters), and the lowering asks them to only when the program's
// spec has the standard shape TreeSpec holds: every entry a record whose
// :container is :object or :array, whose :key and :member are each a
// string or a token's index, whose :scalar is a token's index or :value,
// and whose :text is a boolean, each of the five left out at will. A spec
// of any other shape runs the library's text, which says what it does
// with it.
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
//   - It then announces its :member, read as :key is.
//   - A close pass of a rule whose :text is true first yields the text the
//     passes between its open pass and it consumed: the :src of each of
//     their tokens, in order, the tokens of any :text rule closed in
//     between included. Every pass writes its tokens' :src to the text of
//     the innermost :text rule open, if one is: an open pass before its
//     own rule's text opens, a close pass after its own rule's text closes
//     and joins the one around it.
//   - A close pass of a container rule then ends its container, unless the
//     pass's alternate replaces the rule with one the spec names a
//     container: then the container goes on, and the replacing rule's open
//     pass begins none. A close pass of any other rule yields its :scalar
//     :value, the pass's own value, when it has one.
//   - A pass of a rule the spec does not name yields nothing but writes its
//     tokens as above, and the end of the passes inside a container is
//     PROTOCOL_ORDER_ERROR; a :text rule's text still open there is
//     dropped.
//
// The library's failures are the native's, with their codes and texts: a
// key or a member whose token's value is not a string, and a scalar whose
// token's value is a container, are DSL_TYPE_ERROR; a close with no
// container open is the library's `pop` of an empty stack, and a :text
// rule's close with no text open its `top` of an empty one, each
// DSL_TYPE_ERROR too. A text is held to TreeSpec.MaxTextBytes as it grows,
// as the library's `string-join` holds it: a pass's tokens together, then
// the text they join, then a closed text joined to the one around it, each
// past the bound refused with TreeTextRefusal; a pass that consumed no
// tokens joins nothing. (The library's scan also holds its whole state to
// max_metadata_bytes, which a native has no state to match: past that, the
// two refuse with the same code and different limits.)
//
// A router built before :member and :text existed reads neither, so the
// lowering hands a spec that uses either only to routers that say they do
// (TreeTextRouters).

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
// Member is the name the open pass announces after what it begins or
// yields, and Text whether the close pass first yields the text the
// passes in between consumed.
type TreeRule struct {
	Container TreeContainer
	Key       TreeKey
	Scalar    TreeScalar
	Member    TreeKey
	Text      bool
}

// TreeSpec is a `transduce-tree` spec of the standard shape: the rules it
// names, and MaxTextBytes, the most bytes a :text rule's text may hold as
// it grows: the runtime's max_scalar_bytes, which the library's
// `string-join` holds it to. The lowering sets it when it hands the spec
// to a native; zero leaves the texts unbounded.
type TreeSpec struct {
	Rules        map[string]TreeRule
	MaxTextBytes int
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

// UsesMemberOrText says whether any rule names a :member or is a :text
// rule: what a router built before those fields cannot read.
func (s TreeSpec) UsesMemberOrText() bool {
	for _, r := range s.Rules {
		if r.Member.Set || r.Text {
			return true
		}
	}
	return false
}

// TreeTextRefusal is the library's refusal of a text past
// TreeSpec.MaxTextBytes: its `string-join`'s, n the bytes the join would
// hold.
func TreeTextRefusal(n, max int) *Fail {
	return LimitFail("max_scalar_bytes", uint64(max),
		fmt.Sprintf("string-join: the joined string is %d bytes, more than %d", n, max))
}

// WithoutMemberAndText is the spec as the library's `transduce-step`
// reads it: without the :member and :text of its entries, which only
// `transduce-tree`'s own step, `transduce-pass`, reads.
func (s TreeSpec) WithoutMemberAndText() TreeSpec {
	rules := make(map[string]TreeRule, len(s.Rules))
	for name, r := range s.Rules {
		r.Member = TreeKey{}
		r.Text = false
		rules[name] = r
	}
	return TreeSpec{Rules: rules, MaxTextBytes: s.MaxTextBytes}
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

// TreeTextRouters is the optional interface of TreeRouters whose native
// reads a spec's :member and :text, and holds a :text rule's text to
// TreeSpec.MaxTextBytes as it grows. Routers written before those fields
// do not implement it, and the lowering then runs the library's text for a
// spec that uses either, so the native and the text never disagree.
type TreeTextRouters interface {
	TransduceTreeReadsMemberAndText() bool
}
