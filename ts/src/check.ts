/* Copyright (c) 2026 tabnas, MIT License */

// The checker: conservative inference over the core forms. A port of
// rs/src/check.rs; its module docs are the full account.
//
// Every definition of a program is checked, `export` first with its
// `input` bound to `JsonEvents`, the others with their parameters unknown.
// A native's arguments are checked against its signature, a
// standard-library definition's against the signature the library declares
// for it (`stdlibSignature`), a program's own definition's against what
// its body inferred. What cannot be inferred is `Unknown` and passes; what
// is known to be wrong is reported, DSL_TYPE_ERROR with a finer code as the
// first word of the message: `arity`, `type_mismatch`,
// `protocol_mismatch`, `no_export`, `bad_output`.
//
// Ownership: a binding of `Stream`, `JsonEvents` or `Text` type is affine.
// Used twice in its scope it is STREAM_REUSED (`reused`), where the two
// arms of an `if` or the cases of a `match` are alternatives; used inside a
// `fn` body from the scope around it, it is STREAM_REUSED (`captured`).
// Strict mode: the function given to `map`, `filter` or `concat-map` over a
// stream, and the step and finish of `scan-emit`, must resolve statically,
// else STREAMABILITY_UNKNOWN (`dynamic`). A result of `export` that cannot
// be typed is STREAMABILITY_UNKNOWN (`unknown_output`).
//
// The walks recurse per level of a form, which MAX_NESTING bounds, and the
// checker follows a stream into the bodies it is passed to at most
// MAX_APPLIED definitions deep; definitions are typed in dependency order,
// so a chain of definitions does not nest the walk.

import { Code, Fail } from '@tabnas/transduce'

import { Expr, SourceSpan, Sources, canonicalForm, symbolOf } from './ast'
import { Output } from './output'
import { Def, Resolved, fnForm } from './resolve'
import { Native, arityAccepts, arityExact, arityText, native } from './stdlib/registry'
import {
  Type,
  Unknown,
  Never,
  Null,
  Bool,
  Num,
  Str,
  Keyword,
  Value,
  Record,
  Selector,
  CaptureSpec,
  TableEvent,
  JsonEvents,
  Text,
  accepts,
  events,
  func,
  funcOf,
  isAffine,
  isData,
  isProtocol,
  isStreamOrSource,
  isTextlike,
  itemOf,
  join,
  streamOf,
  tableEvents,
  tagged,
  typeEq,
  typeText,
  typesEq,
  vectorOf,
} from './types'

// What the checker learned about a program.
export type Checked = {
  // The type of `export`'s result.
  export: Type
  // The protocol the host renders, decided by that type.
  output: Output
  // Every other definition's type.
  defs: Map<string, Type>
}

// The signature the standard library declares for one of its definitions;
// the library's text is checked against it and programs call it through
// it.
export function stdlibSignature(name: string): Type | undefined {
  switch (name) {
    case 'public-column':
      return func([Record], Record)
    case 'table-inferred-column':
      return func([Str], Record)
    case 'table-row':
      return func([Unknown, Value], tagged('row'))
    case 'table-first-row':
      return func([Record, Unknown, Value], tagged('transition'))
    case 'table-step':
      return func([Record, Unknown, Unknown], tagged('transition'))
    case 'table-finish':
      return func([Unknown], vectorOf(TableEvent))
    case 'table-finish-for':
      return func([Record, Unknown], vectorOf(TableEvent))
    case 'table-captures':
      return func([Record], vectorOf(CaptureSpec))
    case 'table-from-json':
      return func([Record, JsonEvents], tableEvents())
    case 'csv-options':
      return Record
    case 'csv-field':
      return func([Record, Value], Text)
    case 'csv-row':
      return func([Record, vectorOf(Value)], Text)
    case 'csv':
      return func([Record, tableEvents()], Text)
    default:
      return undefined
  }
}

// The type of a native as a value: a constant's value, or the function.
function nativeType(n: Native): Type {
  if ('constant' === n.kind) {
    if ('root' === n.name || 'each-index' === n.name || 'each-member' === n.name) return Selector
    if ('missing' === n.name) return Value
    return tagged(n.name)
  }
  const k = arityExact(n.arity)
  return undefined === k ? Unknown : funcOf(k)
}

// The checker's walks recurse per level of a form, and through the bodies
// a stream is passed to: MAX_NESTING levels, MAX_APPLIED bodies deep, is
// tens of thousands of nested calls, past what Node's default stack holds.
// So the walks are generators, and a call is a request: `yield
// this.infer(...)` hands the callee's generator to `run`, which drives
// every pending call on an explicit stack of its own and resumes the
// caller with the result (or throws the callee's failure into it, so a
// `finally` that restores the environment still runs). The JavaScript
// stack stays a few frames deep however deep the walk goes.
type G<T> = Generator<unknown, T, any>

function run<T>(root: G<T>): T {
  const stack: G<any>[] = [root]
  let input: any = undefined
  let failed = false
  let failure: unknown = undefined
  for (;;) {
    const top = stack[stack.length - 1]
    let step: IteratorResult<unknown, any>
    try {
      step = failed ? top.throw(failure) : top.next(input)
      failed = false
    } catch (err) {
      stack.pop()
      if (0 === stack.length) throw err
      failed = true
      failure = err
      continue
    }
    if (step.done) {
      stack.pop()
      if (0 === stack.length) return step.value
      input = step.value
    } else {
      stack.push(step.value as G<any>)
      input = undefined
    }
  }
}

// A local binding.
type Local = { name: string; ty: Type }
type Env = Local[]

function lookup(env: Env, name: string): Type | undefined {
  for (let i = env.length - 1; i >= 0; i--) {
    if (env[i].name === name) return env[i].ty
  }
  return undefined
}

// How many definitions deep the checker follows a stream into the bodies
// it is passed to (`applied`). Past it a definition is typed by what its
// body inferred with its parameters unknown, and the runtime's guard is
// what stops a second use.
export const MAX_APPLIED = 32

// A head, for messages.
function describe(head: Expr): string {
  return 'symbol' === head.kind ? head.name : canonicalForm(head)
}

function longer(a: SourceSpan[], b: SourceSpan[]): SourceSpan[] {
  return b.length > a.length ? b : a
}

// The checker for one scope's definitions: a program's, or one standard
// library file's.
class Checker {
  memo = new Map<string, Type>()
  // A definition's type with its parameters bound to the argument types of
  // a call that passes it a stream, by name and argument types.
  appliedTypes: Array<[string, Type[], Type]> = []
  // How many `applied` bodies are being typed, one inside the next.
  applying = 0

  constructor(
    readonly sources: Sources,
    readonly defs: Map<string, Def>,
    // The library file's declared signatures stand for its definitions; a
    // program's are inferred.
    readonly declared: boolean,
  ) {}

  fail(code: Code, finer: string, message: string, sp: SourceSpan): Fail {
    return this.sources.failAt(new Fail(code, `${finer}: ${message}`), sp)
  }

  typeError(finer: string, message: string, sp: SourceSpan): Fail {
    return this.fail('DSL_TYPE_ERROR', finer, message, sp)
  }

  // `type_mismatch` or `protocol_mismatch`, by what was wanted and what
  // came.
  mismatch(what: string, expected: Type, actual: Type, sp: SourceSpan): Fail {
    const finer = isProtocol(expected) && isProtocol(actual) ? 'protocol_mismatch' : 'type_mismatch'
    return this.typeError(finer, `${what} must be ${typeText(expected)}, not ${typeText(actual)}`, sp)
  }

  expect(what: string, expected: Type, actual: Type, sp: SourceSpan): void {
    if (!accepts(expected, actual)) throw this.mismatch(what, expected, actual, sp)
  }

  // The type of a name bound outside the locals: a definition of this
  // scope, a library definition, or a native.
  *globalType(name: string): G<Type | undefined> {
    if (this.defs.has(name)) return (yield this.defType(name))
    const sig = stdlibSignature(name)
    if (undefined !== sig) return sig
    const n = native(name)
    return undefined === n ? undefined : nativeType(n)
  }

  // The type of one of this scope's definitions, inferred once.
  *defType(name: string): G<Type> {
    const memo = this.memo.get(name)
    if (undefined !== memo) return memo
    if (this.declared) {
      const sig = stdlibSignature(name)
      if (undefined !== sig) {
        this.memo.set(name, sig)
        return sig
      }
    }
    const def = this.defs.get(name) as Def
    let ty: Type
    const value = def.value
    const form = 'list' === value.kind ? fnForm(value.items) : undefined
    if ('list' === value.kind && undefined !== form) {
      ty = (yield this.fnType(value.items, new Array(form[0].length).fill(Unknown), []))
    } else {
      ty = (yield this.infer(value, []))
    }
    this.memo.set(name, ty)
    return ty
  }

  // Check a definition whose signature is declared: the body against the
  // declaration.
  *checkDeclared(def: Def, declared: Type): G<void> {
    const value = def.value
    const form = 'list' === value.kind ? fnForm(value.items) : undefined
    if ('list' === value.kind && undefined !== form && 'Fn' === declared.t) {
      const names = form[0]
      if (names.length !== declared.params.length) {
        throw this.typeError(
          'arity',
          `${def.name} is declared with ${declared.params.length} parameter(s) but takes ${names.length}`,
          def.span,
        )
      }
      const got = (yield this.fnType(value.items, declared.params, []))
      if ('Fn' !== got.t) throw new Error('fnType answers a Fn')
      this.expect(`the result of ${def.name}`, declared.result, got.result, def.span)
      return
    }
    const got = (yield this.infer(value, []))
    this.expect(`the value of ${def.name}`, declared, got, def.span)
  }

  // The type of a `fn` form with its parameters bound to `params`; the body
  // is checked, and an affine parameter's uses counted.
  *fnType(items: Expr[], params: Type[], env: Env): G<Type> {
    const form = fnForm(items)
    if (undefined === form) throw new Error('a fn form')
    const [names, body] = form
    const depth = env.length
    try {
      for (let i = 0; i < names.length && i < params.length; i++) {
        if (isAffine(params[i])) (yield this.affine(names[i], body, env))
        env.push({ name: names[i], ty: params[i] })
      }
      const result = (yield this.infer(body, env))
      return func(params.slice(), result)
    } finally {
      env.length = depth
    }
  }

  // An affine binding `name` scoped over `body`: at most one use, and none
  // inside a nested `fn`.
  *affine(name: string, body: Expr, env: Env): G<void> {
    const uses = (yield this.uses(body, name, env))
    if (uses.length > 1) {
      throw this.fail(
        'STREAM_REUSED',
        'reused',
        `${name} is a stream and is used ${uses.length} times; a stream is consumed once`,
        uses[1],
      )
    }
  }

  // Whether a symbol in a pattern binds (rather than compares): the
  // resolver's rule.
  patternBinds(name: string, env: Env): boolean {
    if ('_' === name) return false
    if (undefined !== lookup(env, name)) return true
    if (this.defs.has(name)) return false
    return 'constant' !== native(name)?.kind
  }

  patternBindsName(pattern: Expr, name: string, env: Env): boolean {
    switch (pattern.kind) {
      case 'symbol':
        return pattern.name === name && this.patternBinds(pattern.name, env)
      case 'vector':
        return pattern.items.some((p) => this.patternBindsName(p, name, env))
      case 'list':
        return pattern.items.slice(1).some((p) => this.patternBindsName(p, name, env))
      default:
        return false
    }
  }

  // The uses of the local `name` in `expr`: the spans, where the arms of an
  // `if` and the cases of a `match` count as alternatives (the longer arm),
  // and a use inside a nested `fn` is `captured`.
  *uses(expr: Expr, name: string, env: Env): G<SourceSpan[]> {
    switch (expr.kind) {
      case 'symbol':
        return expr.name === name ? [expr.span] : []
      case 'vector': {
        const all: SourceSpan[] = []
        for (const item of expr.items) all.push(...(yield this.uses(item, name, env)))
        return all
      }
      case 'list': {
        const items = expr.items
        const head = symbolOf(items[0])
        const form = 'fn' === head ? fnForm(items) : undefined
        if ('fn' === head && undefined !== form) {
          const [params, body] = form
          if (params.includes(name)) return []
          const inner = (yield this.uses(body, name, env))
          if (0 < inner.length) {
            throw this.fail(
              'STREAM_REUSED',
              'captured',
              `${name} is a stream and is captured by a fn; a function may run more than once, and a stream is consumed once`,
              inner[0],
            )
          }
          return []
        }
        if ('let' === head && 3 === items.length) {
          const binding = items[1]
          if ('vector' !== binding.kind) return []
          const all: SourceSpan[] = []
          const value = binding.items[1]
          if (undefined !== value) all.push(...(yield this.uses(value, name, env)))
          if (symbolOf(binding.items[0]) !== name) all.push(...(yield this.uses(items[2], name, env)))
          return all
        }
        if ('if' === head && 4 === items.length) {
          const all = (yield this.uses(items[1], name, env))
          const then = (yield this.uses(items[2], name, env))
          const otherwise = (yield this.uses(items[3], name, env))
          all.push(...longer(then, otherwise))
          return all
        }
        if ('match' === head && items.length >= 2) {
          const all = (yield this.uses(items[1], name, env))
          let cases: SourceSpan[] = []
          for (const clause of items.slice(2)) {
            if ('list' !== clause.kind) continue
            const parts = clause.items
            if (3 !== parts.length || this.patternBindsName(parts[1], name, env)) continue
            cases = longer(cases, (yield this.uses(parts[2], name, env)))
          }
          all.push(...cases)
          return all
        }
        const all: SourceSpan[] = []
        for (const item of items) all.push(...(yield this.uses(item, name, env)))
        return all
      }
      default:
        return []
    }
  }

  // Whether `expr` names a function the planner can see: a `fn`, a
  // definition, a native, or a `partial` of one.
  isStaticFn(expr: Expr, env: Env): boolean {
    if ('symbol' === expr.kind) {
      const name = expr.name
      const n = native(name)
      return (
        undefined === lookup(env, name) &&
        (this.defs.has(name) ||
          undefined !== stdlibSignature(name) ||
          (undefined !== n && 'constant' !== n.kind))
      )
    }
    if ('list' === expr.kind) {
      const head = symbolOf(expr.items[0])
      if ('fn' === head) return undefined !== fnForm(expr.items)
      if ('partial' === head && undefined === lookup(env, 'partial')) {
        const f = expr.items[1]
        return undefined !== f && this.isStaticFn(f, env)
      }
    }
    return false
  }

  requireStatic(what: string, expr: Expr, env: Env): void {
    if (!this.isStaticFn(expr, env)) {
      throw this.fail(
        'STREAMABILITY_UNKNOWN',
        'dynamic',
        `${what} must be a fn, a definition, a native or a partial of one, so the plan can be analyzed; strict mode refuses a function obtained at run time`,
        expr.span,
      )
    }
  }

  // The types a pattern binds, and a check that a constructor pattern has
  // the constructor's arity.
  pattern(pattern: Expr, matched: Type, env: Env): void {
    switch (pattern.kind) {
      case 'symbol':
        if (this.patternBinds(pattern.name, env)) env.push({ name: pattern.name, ty: matched })
        return
      case 'vector': {
        const item = itemOf(matched) ?? Unknown
        for (const p of pattern.items) this.pattern(p, item, env)
        return
      }
      case 'list': {
        const items = pattern.items
        const head = symbolOf(items[0]) ?? ''
        let fields: Type[]
        switch (head) {
          case 'selected':
            fields = [Keyword, Value]
            break
          case 'schema':
            fields = [vectorOf(Record)]
            break
          case 'row':
            fields = [vectorOf(Value)]
            break
          case 'ready':
            fields = [vectorOf(Record)]
            break
          case 'transition':
            fields = [Unknown, vectorOf(Unknown)]
            break
          case 'entry':
            fields = [Keyword, Unknown]
            break
          case 'key':
            fields = [Str]
            break
          case 'scalar':
            fields = [Value]
            break
          default:
            fields = new Array(Math.max(0, items.length - 1)).fill(Unknown)
        }
        const n = native(head)
        if (undefined !== n && !arityAccepts(n.arity, items.length - 1)) {
          throw this.typeError(
            'arity',
            `the pattern (${head} ...) takes ${arityText(n.arity)} field(s), got ${items.length - 1}`,
            pattern.span,
          )
        }
        const sub = items.slice(1)
        for (let i = 0; i < sub.length && i < fields.length; i++) this.pattern(sub[i], fields[i], env)
        return
      }
      default:
        return
    }
  }

  // Infer the type of one form.
  *infer(expr: Expr, env: Env): G<Type> {
    switch (expr.kind) {
      case 'symbol': {
        const local = lookup(env, expr.name)
        if (undefined !== local) return local
        const global = (yield this.globalType(expr.name))
        if (undefined === global) throw this.typeError('unknown_name', `${expr.name} is not defined`, expr.span)
        return global
      }
      case 'keyword':
        return Keyword
      case 'str':
        return Str
      case 'num':
        return Num
      case 'bool':
        return Bool
      case 'null':
        return Null
      case 'vector': {
        let item = Never
        for (const i of expr.items) {
          const t = (yield this.infer(i, env))
          if (isStreamOrSource(t)) {
            throw this.typeError(
              'type_mismatch',
              `a vector cannot hold a ${typeText(t)}; a stream is used once, where it is`,
              i.span,
            )
          }
          item = join(item, t)
        }
        return vectorOf('Never' === item.t ? Unknown : item)
      }
      case 'list':
        return (yield this.list(expr.items, expr.span, env))
    }
  }

  *list(items: Expr[], sp: SourceSpan, env: Env): G<Type> {
    const head = items[0]
    if (undefined === head) throw this.typeError('type_mismatch', 'an empty list is not a call', sp)
    const name = symbolOf(head)
    const special = undefined !== name && undefined === lookup(env, name) && !this.defs.has(name) ? name : undefined
    if ('fn' === special) {
      const form = fnForm(items)
      if (undefined !== form) {
        return (yield this.fnType(items, new Array(form[0].length).fill(Unknown), env))
      }
    }
    if ('let' === special && 3 === items.length) {
      const binding = items[1]
      const badLet = () => this.typeError('bad_let', 'let takes one binding [name value] and one body', sp)
      if ('vector' !== binding.kind) throw badLet()
      const bname = symbolOf(binding.items[0])
      const value = binding.items[1]
      if (undefined === bname || undefined === value) throw badLet()
      const ty = (yield this.infer(value, env))
      if (isAffine(ty)) (yield this.affine(bname, items[2], env))
      env.push({ name: bname, ty })
      try {
        return (yield this.infer(items[2], env))
      } finally {
        env.pop()
      }
    }
    if ('if' === special && 4 === items.length) {
      const condition = (yield this.infer(items[1], env))
      this.expect('the condition of if', Bool, condition, items[1].span)
      const then = (yield this.infer(items[2], env))
      const otherwise = (yield this.infer(items[3], env))
      return join(then, otherwise)
    }
    if ('match' === special && items.length >= 2) {
      const matched = (yield this.infer(items[1], env))
      let result = Never
      for (const clause of items.slice(2)) {
        if ('list' !== clause.kind) continue
        const parts = clause.items
        if (3 !== parts.length) continue
        const depth = env.length
        try {
          this.pattern(parts[1], matched, env)
          const body = (yield this.infer(parts[2], env))
          result = join(result, body)
        } finally {
          env.length = depth
        }
      }
      return 'Never' === result.t && 2 === items.length ? Unknown : result
    }
    return (yield this.call(items, sp, env))
  }

  // A call: the head's type decides how the arguments are checked.
  *call(items: Expr[], sp: SourceSpan, env: Env): G<Type> {
    const head = items[0]
    const args = items.slice(1)
    // A native named directly (and not shadowed) is checked by its own
    // signature, which knows more than a function type can say.
    const name = symbolOf(head)
    if (
      undefined !== name &&
      undefined === lookup(env, name) &&
      !this.defs.has(name) &&
      undefined === stdlibSignature(name)
    ) {
      const n = native(name)
      if (undefined !== n) return (yield this.nativeCall(n, args, sp, env))
    }
    let f = (yield this.infer(head, env))
    const argTypes: Type[] = []
    for (const arg of args) argTypes.push((yield this.infer(arg, env)))
    if (argTypes.some(isAffine)) {
      f = (yield this.applied(head, argTypes, env)) ?? f
    }
    if ('Fn' === f.t) {
      if (f.params.length !== args.length) {
        throw this.typeError(
          'arity',
          `${describe(head)} takes ${f.params.length} argument(s), got ${args.length}`,
          sp,
        )
      }
      for (let i = 0; i < args.length; i++) {
        this.expect(`an argument of ${describe(head)}`, f.params[i], argTypes[i], args[i].span)
      }
      return f.result
    }
    if ('Unknown' === f.t || 'Never' === f.t) return Unknown
    throw this.typeError(
      'type_mismatch',
      `${describe(head)} is a ${typeText(f)} and cannot be called`,
      head.span,
    )
  }

  // The type of a definition or a `fn` literal applied to arguments of
  // which one is a stream, a text or the source, typed again with its
  // parameters bound to the arguments' types: the parameter is then affine
  // in the body, as `export`'s `input` is in its own. Undefined for any
  // other head, a count that does not match (the call reports it), or past
  // the bound.
  *applied(head: Expr, args: Type[], env: Env): G<Type | undefined> {
    if (this.applying >= MAX_APPLIED) return undefined
    if ('symbol' === head.kind && undefined === lookup(env, head.name) && this.defs.has(head.name)) {
      const def = this.defs.get(head.name) as Def
      if ('list' !== def.value.kind) return undefined
      const items = def.value.items
      const form = fnForm(items)
      if (undefined === form || form[0].length !== args.length) return undefined
      const found = this.appliedTypes.find(([n, a]) => n === head.name && typesEq(a, args))
      if (undefined !== found) return found[2]
      this.applying++
      let typed: Type
      try {
        typed = (yield this.fnType(items, args, []))
      } finally {
        this.applying--
      }
      this.appliedTypes.push([def.name, args.slice(), typed])
      return typed
    }
    if ('list' === head.kind && undefined === lookup(env, 'fn')) {
      const form = fnForm(head.items)
      if (undefined !== form && form[0].length === args.length) {
        this.applying++
        try {
          return (yield this.fnType(head.items, args, env))
        } finally {
          this.applying--
        }
      }
    }
    return undefined
  }

  // A function argument of a higher-order native: its type, with a `fn`
  // literal's parameters bound to the item types it will see.
  *fnArg(arg: Expr, params: Type[], env: Env): G<Type> {
    if ('list' === arg.kind) {
      const form = fnForm(arg.items)
      if (undefined !== form && form[0].length === params.length) {
        return (yield this.fnType(arg.items, params, env))
      }
    }
    return (yield this.infer(arg, env))
  }

  // The result type of a function type applied, when known.
  static resultOf(f: Type): Type {
    return 'Fn' === f.t ? f.result : Unknown
  }

  expectFn(what: string, arity: number, f: Type, sp: SourceSpan): void {
    if ('Fn' === f.t) {
      if (f.params.length !== arity) {
        throw this.typeError(
          'arity',
          `${what} takes a function of ${arity} argument(s), not ${f.params.length}`,
          sp,
        )
      }
      return
    }
    if ('Unknown' === f.t || 'Never' === f.t) return
    throw this.mismatch(what, funcOf(arity), f, sp)
  }

  // A function argument's parameters against the types a higher-order
  // native gives it, as a direct call checks its arguments. `sp` is the
  // data the items come from.
  expectParams(what: string, f: Type, given: Type[], sp: SourceSpan): void {
    if ('Fn' === f.t) {
      for (let i = 0; i < f.params.length && i < given.length; i++) {
        this.expect(`an item given to ${what}`, f.params[i], given[i], sp)
      }
    }
  }

  // A sequence argument: a vector or a stream of items; the items' type
  // and whether it is a stream.
  expectSeq(what: string, t: Type, sp: SourceSpan): [Type, boolean] {
    switch (t.t) {
      case 'Vector':
        return [t.item, false]
      case 'Stream':
        return [t.item, true]
      case 'Unknown':
      case 'Never':
        return [Unknown, false]
      case 'JsonEvents':
        throw this.typeError(
          'protocol_mismatch',
          `${what} must be a vector or a stream of items, not JsonEvents; select or route what the stream should yield, or read its events`,
          sp,
        )
      default:
        throw this.mismatch(what, vectorOf(Unknown), t, sp)
    }
  }

  *nativeCall(n: Native, args: Expr[], sp: SourceSpan, env: Env): G<Type> {
    if (!arityAccepts(n.arity, args.length)) {
      throw this.typeError(
        'arity',
        `${n.name} takes ${arityText(n.arity)} argument(s), got ${args.length}`,
        sp,
      )
    }
    const name = n.name
    // The natives whose function argument is typed by the items it sees.
    if ('map' === name || 'filter' === name || 'concat-map' === name) {
      const data = (yield this.infer(args[1], env))
      const [item, streaming] = this.expectSeq(`the data of ${name}`, data, args[1].span)
      if (streaming) this.requireStatic(`the function of ${name} over a stream`, args[0], env)
      const f = (yield this.fnArg(args[0], [item], env))
      this.expectFn(`the function of ${name}`, 1, f, args[0].span)
      this.expectParams(`the function of ${name}`, f, [item], args[1].span)
      const result = Checker.resultOf(f)
      if ('map' === name) return streaming ? streamOf(result) : vectorOf(result)
      if ('filter' === name) {
        this.expect(`the result of the predicate of ${name}`, Bool, result, args[0].span)
        return data
      }
      if (!isTextlike(result)) {
        throw this.mismatch('the result of the function of concat-map', Text, result, args[0].span)
      }
      return Text
    }
    if ('scan-emit' === name) {
      const init = (yield this.infer(args[0], env))
      if (isAffine(init)) {
        throw this.typeError('type_mismatch', `the state of scan-emit cannot be a ${typeText(init)}`, args[0].span)
      }
      const source = (yield this.infer(args[3], env))
      const [item, streaming] = this.expectSeq('the stream of scan-emit', source, args[3].span)
      if (!streaming && !typeEq(source, Unknown) && !typeEq(source, Never)) {
        throw this.mismatch('the stream of scan-emit', streamOf(Unknown), source, args[3].span)
      }
      this.requireStatic('the step of scan-emit', args[1], env)
      this.requireStatic('the finish of scan-emit', args[2], env)
      const step = (yield this.fnArg(args[1], [Unknown, item], env))
      this.expectFn('the step of scan-emit', 2, step, args[1].span)
      this.expectParams('the step of scan-emit', step, [Unknown, item], args[3].span)
      this.expect('the result of the step of scan-emit', tagged('transition'), Checker.resultOf(step), args[1].span)
      const finish = (yield this.fnArg(args[2], [Unknown], env))
      this.expectFn('the finish of scan-emit', 1, finish, args[2].span)
      this.expect('the result of the finish of scan-emit', vectorOf(Unknown), Checker.resultOf(finish), args[2].span)
      return streamOf(Unknown)
    }
    const types: Type[] = []
    for (const arg of args) types.push((yield this.infer(arg, env)))
    const at = (i: number) => args[i].span
    const t = (i: number) => types[i]
    // Helpers for the common parameter kinds.
    const data = (i: number, what: string) => {
      if (!isData(t(i))) throw this.mismatch(what, Value, t(i), at(i))
    }
    const noStream = (i: number, what: string) => {
      if (isAffine(t(i))) {
        throw this.typeError(
          'type_mismatch',
          `${what} cannot hold a ${typeText(t(i))}; a stream or a text is used once, where it is`,
          at(i),
        )
      }
    }
    const textlike = (i: number, what: string) => {
      if (!isTextlike(t(i))) throw this.mismatch(what, Text, t(i), at(i))
    }
    const among = (ty: Type, ...names: string[]) => names.includes(ty.t)
    switch (name) {
      case 'get': {
        if (!among(t(0), 'Keyword', 'String', 'Unknown', 'Never')) {
          throw this.mismatch('the key of get', Keyword, t(0), at(0))
        }
        const d = t(1)
        if (!among(d, 'Record', 'Value', 'Unknown', 'Never') && !('Tagged' === d.t && 'missing' === d.tag)) {
          throw this.mismatch('the data of get', Record, d, at(1))
        }
        return Unknown
      }
      case 'get-path':
        this.expect('the path of get-path', Selector, t(0), at(0))
        data(1, 'the data of get-path')
        return Value
      case 'as-path':
        data(0, 'the data of as-path')
        return Selector
      case 'as-vector':
        data(0, 'the data of as-vector')
        return vectorOf(Value)
      case 'record':
        for (let i = 0; i < args.length; i++) this.expect('an argument of record', tagged('entry'), t(i), at(i))
        return Record
      case 'entry':
        this.expect('the key of entry', Keyword, t(0), at(0))
        noStream(1, 'a record')
        return tagged('entry')
      case 'vector': {
        let item = Never
        for (let i = 0; i < args.length; i++) {
          if (isStreamOrSource(t(i))) noStream(i, 'a vector')
          item = join(item, t(i))
        }
        return vectorOf('Never' === item.t ? Unknown : item)
      }
      case 'push': {
        if (isStreamOrSource(t(0))) noStream(0, 'a vector')
        this.expect('the vector of push', vectorOf(Unknown), t(1), at(1))
        const v = t(1)
        return vectorOf('Vector' === v.t ? join(v.item, t(0)) : Unknown)
      }
      case 'pop':
        this.expect('the vector of pop', vectorOf(Unknown), t(0), at(0))
        return 'Vector' === t(0).t ? t(0) : vectorOf(Unknown)
      case 'top':
        this.expect('the vector of top', vectorOf(Unknown), t(0), at(0))
        return itemOf(t(0)) ?? Unknown
      case 'count':
        this.expect('the vector of count', vectorOf(Unknown), t(0), at(0))
        return Num
      case 'keys':
        this.expect('the record of keys', Record, t(0), at(0))
        return vectorOf(Str)
      case 'length':
        this.expect('the string of length', Str, t(0), at(0))
        return Num
      case 'compare':
        this.expect('the first number of compare', Num, t(0), at(0))
        this.expect('the second number of compare', Num, t(1), at(1))
        return Keyword
      case 'number-class':
        this.expect('the number of number-class', Num, t(0), at(0))
        return Keyword
      case 'kind':
        if (isAffine(t(0))) throw this.mismatch('the value of kind', Value, t(0), at(0))
        return Keyword
      case 'path':
        for (let i = 0; i < args.length; i++) {
          if (!among(t(i), 'String', 'Number', 'Selector', 'Unknown', 'Never')) {
            throw this.mismatch('a segment of path', Str, t(i), at(i))
          }
        }
        return Selector
      case 'property':
        this.expect('the name of property', Str, t(0), at(0))
        return Selector
      case 'index':
        this.expect('the position of index', Num, t(0), at(0))
        return Selector
      case 'compose':
        this.expect('the first selector of compose', Selector, t(0), at(0))
        this.expect('the second selector of compose', Selector, t(1), at(1))
        return Selector
      case 'capture':
        this.expect('the tag of capture', Keyword, t(0), at(0))
        this.expect('the selector of capture', Selector, t(1), at(1))
        if (3 === args.length) this.expect('the limit of capture', Keyword, t(2), at(2))
        return CaptureSpec
      case 'route':
        this.expect('the captures of route', vectorOf(CaptureSpec), t(0), at(0))
        this.expect('the input of route', JsonEvents, t(1), at(1))
        return streamOf(tagged('selected'))
      case 'select':
        this.expect('the selector of select', Selector, t(0), at(0))
        this.expect('the input of select', JsonEvents, t(1), at(1))
        return streamOf(Value)
      case 'events':
        this.expect('the input of events', JsonEvents, t(0), at(0))
        return events()
      case 'transition':
        noStream(0, 'a state')
        this.expect('the outputs of transition', vectorOf(Unknown), t(1), at(1))
        return tagged('transition')
      case 'partial': {
        const f = t(0)
        if ('Fn' === f.t) {
          if (f.params.length < args.length - 1) {
            throw this.typeError(
              'arity',
              `partial supplies ${args.length - 1} argument(s) to a function of ${f.params.length}`,
              sp,
            )
          }
          for (let i = 1; i < args.length; i++) {
            noStream(i, 'a partial application')
            this.expect('an argument of partial', f.params[i - 1], t(i), at(i))
          }
          return func(f.params.slice(args.length - 1), f.result)
        }
        if ('Unknown' === f.t || 'Never' === f.t) return Unknown
        throw this.mismatch('the function of partial', funcOf(1), f, at(0))
      }
      case 'join': {
        this.expect('the separator of join', Str, t(0), at(0))
        const [item] = this.expectSeq('the items of join', t(1), at(1))
        if (!isTextlike(item)) throw this.mismatch('an item of join', Text, item, at(1))
        return Text
      }
      case 'concat':
        for (let i = 0; i < args.length; i++) textlike(i, 'an item of concat')
        return Text
      case 'text':
        this.expect('the argument of text', Str, t(0), at(0))
        return Text
      case 'replace-text':
        this.expect('the literal of replace-text', Str, t(0), at(0))
        this.expect('the replacement of replace-text', Str, t(1), at(1))
        textlike(2, 'the text of replace-text')
        return Text
      case 'scalar-text':
        this.expect('the options of scalar-text', Record, t(0), at(0))
        data(1, 'the cell of scalar-text')
        return Str
      case 'quoted':
        this.expect('the string of quoted', Str, t(0), at(0))
        return Str
      case 'repeat':
        this.expect('the count of repeat', Num, t(0), at(0))
        this.expect('the string of repeat', Str, t(1), at(1))
        return Str
      case 'string-join':
        this.expect('the separator of string-join', Str, t(0), at(0))
        this.expect('the strings of string-join', vectorOf(Unknown), t(1), at(1))
        return Str
      case 'fail':
        this.expect('the message of fail', Str, t(0), at(0))
        return Never
      case 'is-ready':
        return Bool
      case 'require-columns':
        return vectorOf(Record)
      case 'schema':
        this.expect('the columns of schema', vectorOf(Unknown), t(0), at(0))
        return tagged('schema')
      case 'row':
        this.expect('the cells of row', vectorOf(Unknown), t(0), at(0))
        return tagged('row')
      case 'ready':
        noStream(0, 'a state')
        return tagged('ready')
      case 'selected':
        this.expect('the tag of selected', Keyword, t(0), at(0))
        return tagged('selected')
      case 'key':
        this.expect('the name of key', Str, t(0), at(0))
        return tagged('key')
      case 'scalar':
        if (!among(t(0), 'Null', 'Bool', 'Number', 'String', 'Value', 'Unknown', 'Never')) {
          throw this.typeError(
            'type_mismatch',
            `the value of scalar must be null, a boolean, a number or a string, not ${typeText(t(0))}`,
            at(0),
          )
        }
        return tagged('scalar')
      case 'json':
        this.expect('the events of json', JsonEvents, t(0), at(0))
        return Text
      case 'csv-table':
        this.expect('the options of csv-table', Record, t(0), at(0))
        this.expect('the table events of csv-table', tableEvents(), t(1), at(1))
        return tableEvents()
      case 'records':
        this.expect('the table events of records', tableEvents(), t(0), at(0))
        return JsonEvents
      // The constants are not calls; a call of one is an arity error above.
      // Anything else is a native this table does not know.
      default:
        throw this.typeError('type_mismatch', `${name} is not callable here`, sp)
    }
  }
}

// The output protocol an `export` result type decides, or why it does not.
function outputOf(ty: Type): Output | [Code, string, string] {
  switch (ty.t) {
    case 'Text':
    case 'String':
      return 'Text'
    case 'JsonEvents':
      return 'JsonEvents/1'
    case 'Stream':
      if (accepts(TableEvent, ty.item)) return 'TableRows/1'
      return [
        'DSL_TYPE_ERROR',
        'bad_output',
        `export answers a Stream<${typeText(ty.item)}>; render it as a text (join, concat-map), or make table events of it`,
      ]
    case 'Unknown':
      return [
        'STREAMABILITY_UNKNOWN',
        'unknown_output',
        'the result of export cannot be typed; it must be a text, table events or JSON events',
      ]
    default:
      return [
        'DSL_TYPE_ERROR',
        'bad_output',
        `export answers a ${typeText(ty)}; it must answer a text, table events or JSON events`,
      ]
  }
}

// The failure of a program with no `export`.
export function noExport(): Fail {
  return new Fail('DSL_TYPE_ERROR', 'no_export: the program has no `def export [input]`')
}

// Check a resolved program: `export` with its input, then every other
// definition.
export function checkProgram(resolved: Resolved, sources: Sources): Checked {
  const checker = new Checker(sources, resolved.defs, false)
  const exp = resolved.get('export')
  if (undefined === exp) throw noExport()
  const value = exp.value
  const form = 'list' === value.kind ? fnForm(value.items) : undefined
  if ('list' !== value.kind || undefined === form) {
    throw checker.typeError('type_mismatch', 'export must be a fn [input]', exp.span)
  }
  if (1 !== form[0].length) {
    throw checker.typeError('arity', `export takes one parameter, the input, not ${form[0].length}`, exp.span)
  }
  // The definitions `export` reaches, typed first and each after the ones
  // it names, so typing `export` finds them memoized and never recurses
  // along a chain of definitions.
  const order = resolved.dependencyOrder()
  const reached = new Set(resolved.reachable('export'))
  for (const name of order) {
    if (reached.has(name)) run(checker.defType(name))
  }
  const typed = run(checker.fnType(value.items, [JsonEvents], []))
  if ('Fn' !== typed.t) throw new Error('fnType answers a Fn')
  const exportType = typed.result
  const output = outputOf(exportType)
  if (Array.isArray(output)) {
    const [code, finer, message] = output
    throw checker.fail(code, finer, message, exp.span)
  }
  checker.memo.set('export', func([JsonEvents], exportType))
  for (const name of order) {
    if ('export' !== name) run(checker.defType(name))
  }
  const defs = new Map<string, Type>()
  for (const name of resolved.defs.keys()) {
    if ('export' !== name) defs.set(name, run(checker.defType(name)))
  }
  return { export: exportType, output: output as Output, defs }
}

// Check one standard library file: every definition against its declared
// signature; a definition without one is a defect of the library.
export function checkStdlibFile(resolved: Resolved, src: string): void {
  const sources = Sources.one(resolved.file, src)
  const checker = new Checker(sources, resolved.defs, true)
  for (const def of resolved.defs.values()) {
    const declared = stdlibSignature(def.name)
    if (undefined === declared) {
      throw checker.typeError(
        'undeclared',
        `the standard library defines ${def.name} without a declared signature`,
        def.span,
      )
    }
    run(checker.checkDeclared(def, declared))
  }
}
