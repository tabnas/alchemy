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
// record whose `:container` is `:object` or `:array`, whose `:key` is a
// string or a token's index, and whose `:scalar` is a token's index or
// `:value`, each of the three left out at will. A spec of any other shape
// runs the library's text, which says what it does with it.
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
// - A close pass of a container rule ends its container, unless the pass's
//   alternate replaces the rule with one the spec names a container: then
//   the container goes on, and the replacing rule's open pass begins none.
//   A close pass of any other rule yields its `:scalar :value`, the pass's
//   own value, when it has one.
// - A pass of a rule the spec does not name yields nothing, and the end of
//   the passes inside a container is `PROTOCOL_ORDER_ERROR`.
//
// The library's failures are the native's, with their codes and texts: a
// key whose token's value is not a string, and a scalar whose token's
// value is a container, are `DSL_TYPE_ERROR`; a close with no container
// open is the library's `pop` of an empty stack, `DSL_TYPE_ERROR` too.

// The container a rule's passes begin and end.
export type TreeContainer = 'object' | 'array'

// The member name a rule's open pass announces: the `:val` of the pass's
// token at an index, or a name.
export type TreeKey = { readonly token: number } | { readonly name: string }

// The scalar a rule yields: the `:val` of the open pass's token at an
// index, or the close pass's own value.
export type TreeScalar = { readonly token: number } | 'value'

// What one rule's passes do: an entry of the spec. `scalar` is read only
// when `container` is absent, as the library reads it.
export interface TreeRule {
  readonly container?: TreeContainer
  readonly key?: TreeKey
  readonly scalar?: TreeScalar
}

// A `transduce-tree` spec of the standard shape: the rules it names.
export interface TreeSpec {
  readonly rules: ReadonlyMap<string, TreeRule>
}

// Whether the rule named `name` is a container rule: what decides, at a
// close whose alternate replaces its rule with that one, that the
// container goes on.
export function isTreeContainer(spec: TreeSpec, name: string): boolean {
  return undefined !== spec.rules.get(name)?.container
}
