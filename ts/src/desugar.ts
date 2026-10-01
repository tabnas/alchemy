/* Copyright (c) 2026 tabnas, MIT License */

// Desugaring: the conveniences rewritten into the core forms. A port of
// rs/src/desugar.rs.
//
// - `(def name [params] body)` becomes `(def name (fn [params] body))`;
//   `(def name value)` is already core.
// - `(pipe init step ...)` becomes nested data-last application: a bare
//   symbol step `f` is `(f acc)`, a non-empty list step `(f a b)` is
//   `(f a b acc)`, and an empty list, a literal, a keyword or a vector is
//   an error, because there is nothing to apply.
// - `let`, `if` and `match` are checked for shape and left as they are.
//
// Anything else passes through unchanged, `(pipe)` included.
//
// Spans survive: a generated node carries the span of the form it came
// from. Failures are DSL_PARSE_ERRORs whose message begins with the code
// the grammar document declares (`empty_step`, `bad_def`, `bad_let`,
// `bad_if`, `bad_match`, `too_deep`), with the row and column of the
// offending form; the source is passed in for that.
//
// A rewrite can nest deeper than what it read (`def` wraps a body in a
// `fn`, `pipe` nests the threaded value one level per step), so the depth
// of every result is carried alongside it and checked against MAX_NESTING
// as each container is built. The walk recurses per level of what the
// reader built, which MAX_NESTING bounds; a long pipe is a loop.

import { Fail } from '@tabnas/transduce'

import { Expr, SourceSpan, MAX_NESTING, TOO_DEEP, position, symbolOf } from './ast'

// The messages, by code, as the grammar document also declares them; a
// test holds the two in step.
export const MESSAGES: ReadonlyArray<readonly [string, string]> = [
  ['empty_step', 'a pipe step must be a symbol or a non-empty list'],
  ['bad_def', 'def takes a name and a value, or a name, [params] and a body'],
  ['bad_let', 'let takes one binding [name value] and one body'],
  ['bad_if', 'if takes a condition and exactly two branches'],
  ['bad_match', 'match takes a value and (case pattern body) clauses'],
  ['too_deep', TOO_DEEP],
]

function message(code: string): string {
  return MESSAGES.find(([known]) => known === code)?.[1] ?? 'invalid form'
}

function fail(code: string, sp: SourceSpan, src: string): Fail {
  const [row, col] = position(sp, src)
  return new Fail('DSL_PARSE_ERROR', `${code}: ${message(code)}`).at(row, col)
}

// Desugar a whole program, form by form. `src` is the program's source,
// read only to give a failure its row and column.
export function desugarProgram(forms: Expr[], src: string): Expr[] {
  return forms.map((form) => desugarExpr(form, src))
}

// Desugar one form, innermost first, so a rewrite sees core items.
export function desugarExpr(form: Expr, src: string): Expr {
  return deep(form, src).expr
}

// A desugared form with its nesting: 0 for an atom, one more than its
// deepest item for a list or vector.
type Deep = { expr: Expr; depth: number }

function deep(form: Expr, src: string): Deep {
  switch (form.kind) {
    case 'list':
      return rewrite(each(form.items, src), form.span, src)
    case 'vector':
      return container(each(form.items, src), form.span, src, (items, sp) => ({
        kind: 'vector',
        items,
        span: sp,
      }))
    default:
      return { expr: form, depth: 0 }
  }
}

function each(items: Expr[], src: string): Deep[] {
  return items.map((item) => deep(item, src))
}

// `d` when it is within the bound, `too_deep` at `sp` otherwise.
function bounded(d: Deep, sp: SourceSpan, src: string): Deep {
  if (d.depth > MAX_NESTING) throw fail('too_deep', sp, src)
  return d
}

// A list or vector over `items`, one level deeper than its deepest item.
function container(
  items: Deep[],
  sp: SourceSpan,
  src: string,
  make: (items: Expr[], sp: SourceSpan) => Expr,
): Deep {
  let depth = 0
  for (const item of items) depth = Math.max(depth, item.depth)
  return bounded({ expr: make(items.map((item) => item.expr), sp), depth: 1 + depth }, sp, src)
}

function list(items: Deep[], sp: SourceSpan, src: string): Deep {
  return container(items, sp, src, (items, sp) => ({ kind: 'list', items, span: sp }))
}

function rewrite(items: Deep[], sp: SourceSpan, src: string): Deep {
  switch (symbolOf(items[0]?.expr)) {
    case 'def':
      return def(items, sp, src)
    case 'pipe':
      return pipe(items, sp, src)
    case 'let':
      return shape(items, sp, src, 'bad_let', isLet)
    case 'if':
      return shape(items, sp, src, 'bad_if', (items) => 4 === items.length)
    case 'match':
      return shape(items, sp, src, 'bad_match', isMatch)
    default:
      return list(items, sp, src)
  }
}

// `(def name value)` stays; `(def name [params] body)` wraps the body in a
// `fn` carrying the `def`'s span.
function def(items: Deep[], sp: SourceSpan, src: string): Deep {
  const named = undefined !== symbolOf(items[1]?.expr)
  if (3 === items.length && named) return list(items, sp, src)
  if (4 === items.length && named && 'vector' === items[2].expr.kind) {
    const params = items[2]
    const body = items[3]
    const fn: Deep = {
      depth: 1 + Math.max(params.depth, body.depth),
      expr: { kind: 'list', items: [{ kind: 'symbol', name: 'fn', span: sp }, params.expr, body.expr], span: sp },
    }
    return list([items[0], items[1], fn], sp, src)
  }
  throw fail('bad_def', sp, src)
}

// `(pipe init step ...)` threaded data-last; `(pipe)` passes through.
function pipe(items: Deep[], sp: SourceSpan, src: string): Deep {
  if (items.length < 2) return list(items, sp, src)
  let acc = items[1]
  for (let i = 2; i < items.length; i++) {
    const step = items[i]
    const at = step.expr
    if ('symbol' === at.kind) {
      acc = {
        depth: acc.depth + 1,
        expr: { kind: 'list', items: [{ kind: 'symbol', name: at.name, span: at.span }, acc.expr], span: at.span },
      }
    } else if ('list' === at.kind && 0 < at.items.length) {
      // The step's own items are one level below it; the threaded value
      // joins them.
      acc = {
        depth: Math.max(step.depth, acc.depth + 1),
        expr: { kind: 'list', items: [...at.items, acc.expr], span: at.span },
      }
    } else {
      throw fail('empty_step', at.span, src)
    }
    // Checked per step: a long pipe fails at the step that passes the
    // bound, not after the whole chain is built.
    acc = bounded(acc, sp, src)
  }
  return acc
}

function shape(items: Deep[], sp: SourceSpan, src: string, code: string, ok: (items: Deep[]) => boolean): Deep {
  if (ok(items)) return list(items, sp, src)
  throw fail(code, sp, src)
}

// `(let [name value] body)`.
function isLet(items: Deep[]): boolean {
  if (3 !== items.length) return false
  const binding = items[1].expr
  return 'vector' === binding.kind && 2 === binding.items.length && undefined !== symbolOf(binding.items[0])
}

// `(match value (case pattern body) ...)`.
function isMatch(items: Deep[]): boolean {
  return (
    items.length >= 2 &&
    items.slice(2).every((clause) => {
      const c = clause.expr
      return 'list' === c.kind && 3 === c.items.length && 'case' === symbolOf(c.items[0])
    })
  )
}
