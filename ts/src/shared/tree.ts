/* Copyright (c) 2026 tabnas, MIT License */

// The spec of the library's `transduce-tree`, for routers that run it
// natively (rs/src/shared/tree.rs).
//
// `transduce-tree spec input` (`stdlib/transduce.alc`) makes a tree's JSON
// events of a grammar's rule passes as `spec` says: a record keyed by rule
// name, each entry a record of what that rule's passes do. The library's
// text is the specification. A host's routers may run it natively
// (`Routers.transduceTree`), and the lowering asks them to only when the
// program's spec has the standard shape `TreeSpec` holds: every entry a
// record whose `:container` is `:object` or `:array`, whose `:key` and
// `:member` are each a string or a token's index, whose `:scalar` is a
// token's index or `:value`, and whose `:text` is a boolean, each of the
// five left out at will. A spec of any other shape runs the library's
// text, which says what it does with it.
//
// What a native must do is what the text does, pass for pass:
//
// - An open pass of a rule the spec names announces its `:key` first: the
//   string, or the `:val` of the pass's token at that index, nothing when
//   the pass consumed no such token.
// - It then begins its `:container`, unless the rule it replaced carried
//   that container on (below), or yields its `:scalar` from the token at
//   that index, when the pass consumed one; `:value` yields nothing on an
//   open pass, and `:scalar` nothing for a container rule.
// - It then announces its `:member`, read as `:key` is.
// - A close pass of a rule whose `:text` is `true` first yields the text
//   the passes between its open pass and it consumed: the `:src` of each
//   of their tokens, in order, the tokens of any `:text` rule closed in
//   between included. Every pass writes its tokens' `:src` to the text of
//   the innermost `:text` rule open, if one is: an open pass before its
//   own rule's text opens, a close pass after its own rule's text closes
//   and joins the one around it.
// - A close pass of a container rule then ends its container, unless the
//   pass's alternate replaces the rule with one the spec names a
//   container: then the container goes on, and the replacing rule's open
//   pass begins none. A close pass of any other rule yields its
//   `:scalar :value`, the pass's own value, when it has one.
// - A pass of a rule the spec does not name yields nothing but writes its
//   tokens as above, and the end of the passes inside a container is
//   `PROTOCOL_ORDER_ERROR`; a `:text` rule's text still open there is
//   dropped.
//
// The library's failures are the native's, with their codes and texts: a
// key or a member whose token's value is not a string, and a scalar whose
// token's value is a container, are `DSL_TYPE_ERROR`; a close with no
// container open is the library's `pop` of an empty stack, and a `:text`
// rule's close with no text open its `top` of an empty one, each
// `DSL_TYPE_ERROR` too. A text is held to `TreeSpec.maxTextBytes` as it
// grows, as the library's `string-join` holds it: a pass's tokens together,
// then the text they join, then a closed text joined to the one around it,
// each past the bound refused with `treeTextRefusal`; a pass that consumed
// no tokens joins nothing. (The library's scan also holds its whole state
// to `max_metadata_bytes`, which a native has no state to match: past that,
// the two refuse with the same code and different limits.)
//
// A router built before `:member` and `:text` existed reads neither, so
// the lowering hands a spec that uses either only to routers that say they
// do (`Routers.transduceTreeReadsMemberAndText`).

import { Fail } from './error'

// The container a rule's passes begin and end.
export type TreeContainer = 'object' | 'array'

// The member name a rule's open pass announces: the `:val` of the pass's
// token at an index, or a name.
export type TreeKey = { readonly token: number } | { readonly name: string }

// The scalar a rule yields: the `:val` of the open pass's token at an
// index, or the close pass's own value.
export type TreeScalar = { readonly token: number } | 'value'

// What one rule's passes do: an entry of the spec. `scalar` is read only
// when `container` is absent, as the library reads it; `member` is the
// name the open pass announces after what it begins or yields, and `text`
// whether the close pass first yields the text the passes in between
// consumed.
export interface TreeRule {
  readonly container?: TreeContainer
  readonly key?: TreeKey
  readonly scalar?: TreeScalar
  readonly member?: TreeKey
  readonly text?: boolean
}

// A `transduce-tree` spec of the standard shape: the rules it names, and
// the most bytes a `:text` rule's text may hold as it grows, the runtime's
// `max_scalar_bytes`, which the library's `string-join` holds it to. The
// lowering sets the bound when it hands the spec to a native; absent, the
// texts are unbounded.
export interface TreeSpec {
  readonly rules: ReadonlyMap<string, TreeRule>
  readonly maxTextBytes?: number
}

// Whether the rule named `name` is a container rule: what decides, at a
// close whose alternate replaces its rule with that one, that the
// container goes on.
export function isTreeContainer(spec: TreeSpec, name: string): boolean {
  return undefined !== spec.rules.get(name)?.container
}

// Whether any rule names a `:member` or is a `:text` rule: what a router
// built before those fields cannot read.
export function usesMemberOrText(spec: TreeSpec): boolean {
  for (const rule of spec.rules.values()) if (undefined !== rule.member || true === rule.text) return true
  return false
}

// The library's refusal of a text past `TreeSpec.maxTextBytes`: its
// `string-join`'s, `len` the bytes the join would hold.
export function treeTextRefusal(len: number, max: number): Fail {
  return Fail.limit('max_scalar_bytes', max, `string-join: the joined string is ${len} bytes, more than ${max}`)
}

// The spec as the library's `transduce-step` reads it: without the
// `:member` and `:text` of its entries, which only `transduce-tree`'s own
// step, `transduce-pass`, reads.
export function withoutMemberAndText(spec: TreeSpec): TreeSpec {
  const rules = new Map<string, TreeRule>()
  for (const [name, rule] of spec.rules) {
    const { member: _member, text: _text, ...rest } = rule
    rules.set(name, rest)
  }
  return { ...spec, rules }
}
