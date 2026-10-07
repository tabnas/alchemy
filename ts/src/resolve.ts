/* Copyright (c) 2026 tabnas, MIT License */

// Scopes and linking. A port of rs/src/resolve.rs.
//
// A program is a sequence of `def`s, in any order: every definition sees
// every other, and a name that is bound nowhere (not a local, not a
// definition, not a standard-library name) is DSL_TYPE_ERROR with the
// finer code `unknown_name`, before anything runs. Locals come from `fn`
// parameters, `let` bindings and the bindings a `match` pattern makes;
// each shadows what is outside it.
//
// Recursion is refused here: a definition that reaches itself through the
// references its body makes, directly or through other definitions, is
// STREAMABILITY_UNKNOWN with the finer code `recursion`.
//
// Patterns: `_` matches anything; a symbol naming a standard-library
// constant or a definition of the program compares equal to it; any other
// symbol binds; a keyword, string, number, boolean or `null` compares; a
// vector matches a vector of the same length, item by item;
// `(constructor p ...)` matches the tagged value that constructor makes.
//
// The walk recurses per level of a form, which MAX_NESTING bounds; the
// graph searches over definitions are iterative.

import { Code, Fail } from './shared'

import { Expr, SourceSpan, Sources, symbolOf } from './ast'
import { shapeError } from './desugar'

// What kind of thing a name outside the program denotes.
export type NameKind =
  // A value the symbol denotes without a call (`table-end`).
  | 'constant'
  // A function whose value is a tagged value: also a pattern head.
  | 'constructor'
  // A function.
  | 'function'
  // A definition's value of some other kind (a record).
  | 'value'

// The kind of a name bound outside the program, or undefined.
export type Outer = (name: string) => NameKind | undefined

// The heads of the core forms and the conveniences the desugarer
// rewrites: none may be defined, and each is read by shape.
export const SPECIAL_FORMS: readonly string[] = ['def', 'fn', 'let', 'if', 'match', 'case', 'pipe']

// One top-level definition.
export type Def = {
  readonly name: string
  readonly value: Expr
  // The span of the whole `def` form.
  readonly span: SourceSpan
}

// The parameters, when a definition's value is a `fn`.
export function defParams(def: Def): string[] | undefined {
  return fnParams(def.value)
}

// The parameter names of a `(fn [params] body)` form.
export function fnParams(expr: Expr): string[] | undefined {
  return 'list' === expr.kind ? fnForm(expr.items)?.[0] : undefined
}

// The parameters and the body of the items of a `(fn [params] body)`
// list; undefined for any other shape.
export function fnForm(items: Expr[]): [string[], Expr] | undefined {
  if (3 !== items.length || 'fn' !== symbolOf(items[0])) return undefined
  const params = items[1]
  if ('vector' !== params.kind) return undefined
  const names: string[] = []
  for (const p of params.items) {
    const name = symbolOf(p)
    if (undefined === name) return undefined
    names.push(name)
  }
  return [names, items[2]]
}

function fail(code: Code, finer: string, message: string, sp: SourceSpan, sources: Sources): Fail {
  return sources.failAt(new Fail(code, `${finer}: ${message}`), sp)
}

function typeFail(finer: string, message: string, sp: SourceSpan, sources: Sources): Fail {
  return fail('DSL_TYPE_ERROR', finer, message, sp, sources)
}

// A form whose shape the desugarer owns (`bad_def`, `bad_let`, `bad_if`,
// `bad_match`), met here: the desugarer's own failure, a DSL_PARSE_ERROR
// with its text, at `sp`. The desugarer checks the forms a program writes;
// a `pipe` step's form grows by the threaded value after that check
// (`pipe v let` is `(let v)`, `pipe v (if c a b)` is `(if c a b v)`), so
// the shapes are held again here, by the same rules and with the same
// codes.
function shapeFail(finer: string, sp: SourceSpan, sources: Sources): Fail {
  return sources.failAt(shapeError(finer), sp)
}

// A resolved program: its definitions and the references between them.
export class Resolved {
  // The file the program is named by: its first source's.
  readonly file: string
  readonly defs: Map<string, Def>
  // For each definition, the definitions of this program its value refers
  // to, in first-reference order.
  readonly references: Map<string, string[]>

  constructor(file: string, defs: Map<string, Def>, references: Map<string, string[]>) {
    this.file = file
    this.defs = defs
    this.references = references
  }

  get(name: string): Def | undefined {
    return this.defs.get(name)
  }

  // The definitions `name` reaches, transitively, in first-reference
  // order; `name` itself is not among them (recursion was refused).
  reachable(name: string): string[] {
    const out: string[] = []
    const seen = new Set<string>()
    const pending: string[] = [...(this.references.get(name) ?? [])].reverse()
    while (0 < pending.length) {
      const next = pending.pop() as string
      if (seen.has(next)) continue
      seen.add(next)
      const refs = this.references.get(next)
      if (refs) pending.push(...[...refs].reverse())
      out.push(next)
    }
    return out
  }

  // Every definition, each after all the definitions it refers to: the
  // order in which typing or evaluating them one by one never has to
  // reach through a definition not yet done. An iterative depth-first
  // search; recursion was refused, so the graph has no cycle.
  dependencyOrder(): string[] {
    const order: string[] = []
    const done = new Set<string>()
    const open = new Set<string>()
    for (const start of this.defs.keys()) {
      if (done.has(start)) continue
      const path: Array<[string, number]> = [[start, 0]]
      open.add(start)
      while (0 < path.length) {
        const top = path[path.length - 1]
        const refs = this.references.get(top[0]) ?? []
        if (top[1] < refs.length) {
          const child = refs[top[1]++]
          if (!done.has(child) && !open.has(child)) {
            open.add(child)
            path.push([child, 0])
          }
        } else {
          open.delete(top[0])
          done.add(top[0])
          order.push(top[0])
          path.pop()
        }
      }
    }
    return order
  }

  // `recursion` at the first definition on a cycle, naming the cycle.
  refuseRecursion(sources: Sources): void {
    const marks = new Map<string, 'open' | 'done'>()
    for (const start of this.defs.keys()) {
      if (marks.has(start)) continue
      const path: Array<[string, number]> = [[start, 0]]
      marks.set(start, 'open')
      while (0 < path.length) {
        const top = path[path.length - 1]
        const refs = this.references.get(top[0]) ?? []
        if (top[1] >= refs.length) {
          marks.set(top[0], 'done')
          path.pop()
          continue
        }
        const child = refs[top[1]++]
        const mark = marks.get(child)
        if ('done' === mark) continue
        if ('open' === mark) {
          const from = Math.max(
            0,
            path.findIndex(([n]) => n === child),
          )
          const cycle = path.slice(from).map(([n]) => n)
          const def = this.defs.get(child) as Def
          const through = 1 === cycle.length ? 'itself' : cycle.slice(1).join(', then ')
          throw fail(
            'STREAMABILITY_UNKNOWN',
            'recursion',
            `${child} reaches itself through ${through}; strict mode refuses recursion`,
            def.span,
            sources,
          )
        }
        marks.set(child, 'open')
        path.push([child, 0])
      }
    }
  }
}

// Link a desugared program, whose forms may come from several sources:
// `sources` are their texts, which position a failure in the file its form
// was read from. `outer` answers the kind of a name bound outside the
// program (the standard library and the natives), or undefined.
export function resolve(forms: Expr[], sources: Sources, outer: Outer): Resolved {
  const defs = new Map<string, Def>()
  for (const form of forms) {
    const sp = form.span
    if ('list' !== form.kind || 3 !== form.items.length || 'def' !== symbolOf(form.items[0])) {
      throw typeFail('not_def', 'a top-level form must be a def', sp, sources)
    }
    const value = form.items[2]
    const nameForm = form.items[1]
    // The desugarer's `bad_def` always comes first; checked again as the
    // desugarer checks it.
    if ('symbol' !== nameForm.kind) throw shapeFail('bad_def', nameForm.span, sources)
    const name = nameForm.name
    if (SPECIAL_FORMS.includes(name)) {
      throw typeFail('reserved', `${name} is a special form and cannot be defined`, sp, sources)
    }
    const first = defs.get(name)
    if (undefined !== first) {
      // Across several sources the first definition may be in another
      // file, so the message says where it is.
      let message = `${name} is defined twice`
      if (sources.namesFiles()) {
        const [row, col] = sources.position(first.span)
        message = `${name} is defined twice; first at ${first.span.file.name}:${row}:${col}`
      }
      throw typeFail('duplicate_def', message, sp, sources)
    }
    defs.set(name, { name, value, span: sp })
  }

  const references = new Map<string, string[]>()
  for (const def of defs.values()) {
    const walker = new Walker(sources, outer, defs)
    walker.expr(def.value)
    references.set(def.name, walker.refs)
  }

  const resolved = new Resolved(sources.first(), defs, references)
  resolved.refuseRecursion(sources)
  return resolved
}

class Walker {
  locals: string[] = []
  refs: string[] = []
  seen = new Set<string>()

  constructor(
    readonly sources: Sources,
    readonly outer: Outer,
    readonly defs: Map<string, Def>,
  ) {}

  isLocal(name: string): boolean {
    return this.locals.includes(name)
  }

  refer(name: string) {
    if (!this.seen.has(name)) {
      this.seen.add(name)
      this.refs.push(name)
    }
  }

  symbol(name: string, sp: SourceSpan) {
    if (this.isLocal(name)) return
    if (this.defs.has(name)) {
      this.refer(name)
      return
    }
    if (undefined !== this.outer(name)) return
    throw typeFail('unknown_name', `${name} is not defined`, sp, this.sources)
  }

  expr(expr: Expr): void {
    switch (expr.kind) {
      case 'symbol':
        return this.symbol(expr.name, expr.span)
      case 'vector':
        for (const item of expr.items) this.expr(item)
        return
      case 'list':
        return this.list(expr.items, expr.span)
      default:
        return
    }
  }

  list(items: Expr[], sp: SourceSpan): void {
    switch (symbolOf(items[0])) {
      case 'fn': {
        const form = fnForm(items)
        if (undefined === form) {
          throw typeFail('bad_fn', 'fn takes [params] of symbols and one body', sp, this.sources)
        }
        const depth = this.locals.length
        this.locals.push(...form[0])
        try {
          this.expr(form[1])
        } finally {
          this.locals.length = depth
        }
        return
      }
      case 'let': {
        // `(let [name value] body)`, as the desugarer has it.
        const binding = items[1]
        if (3 !== items.length || 'vector' !== binding.kind || 2 !== binding.items.length) {
          throw shapeFail('bad_let', sp, this.sources)
        }
        const name = symbolOf(binding.items[0])
        const value = binding.items[1]
        const body = items[2]
        if (undefined === name) throw shapeFail('bad_let', sp, this.sources)
        this.expr(value)
        this.locals.push(name)
        try {
          this.expr(body)
        } finally {
          this.locals.pop()
        }
        return
      }
      case 'if':
        // A condition and exactly two branches, as the desugarer has it.
        if (4 !== items.length) throw shapeFail('bad_if', sp, this.sources)
        for (const item of items.slice(1)) this.expr(item)
        return
      case 'match': {
        // A value and `(case pattern body)` clauses, as the desugarer has
        // it; a clause that is not one is named.
        const value = items[1]
        if (undefined === value) throw shapeFail('bad_match', sp, this.sources)
        this.expr(value)
        for (const clause of items.slice(2)) {
          if ('list' !== clause.kind) throw shapeFail('bad_match', clause.span, this.sources)
          const parts = clause.items
          if (3 !== parts.length || 'case' !== symbolOf(parts[0])) {
            throw shapeFail('bad_match', clause.span, this.sources)
          }
          const depth = this.locals.length
          const bound: string[] = []
          this.pattern(parts[1], bound)
          this.locals.push(...bound)
          try {
            this.expr(parts[2])
          } finally {
            this.locals.length = depth
          }
        }
        return
      }
      case 'def':
        throw typeFail('misplaced_def', 'def is only allowed at the top level', sp, this.sources)
      default:
        for (const item of items) this.expr(item)
    }
  }

  // Whether a symbol in a pattern compares rather than binds.
  isConstantPattern(name: string): boolean {
    if (this.isLocal(name)) return false
    if (this.defs.has(name)) {
      this.refer(name)
      return true
    }
    return 'constant' === this.outer(name)
  }

  pattern(pattern: Expr, bound: string[]): void {
    switch (pattern.kind) {
      case 'symbol':
        if ('_' !== pattern.name && !this.isConstantPattern(pattern.name)) bound.push(pattern.name)
        return
      case 'vector':
        for (const item of pattern.items) this.pattern(item, bound)
        return
      case 'list': {
        const head = symbolOf(pattern.items[0])
        if (undefined !== head && 'constructor' === this.outer(head)) {
          for (const item of pattern.items.slice(1)) this.pattern(item, bound)
          return
        }
        if (undefined !== head) {
          throw typeFail(
            'bad_pattern',
            `${head} is not a constructor; a list pattern is (constructor pattern...)`,
            pattern.span,
            this.sources,
          )
        }
        throw typeFail('bad_pattern', 'a list pattern is (constructor pattern...)', pattern.span, this.sources)
      }
      default:
        return
    }
  }
}
