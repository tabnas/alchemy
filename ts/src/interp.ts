/* Copyright (c) 2026 tabnas, MIT License */

// The evaluator (design brief section 4.4; spec sections 10.1 and 10.3). A
// port of rs/src/interp.rs.
//
// Values are evaluated eagerly; streams and texts are plans that
// evaluation builds and never runs. The `Runtime` holds the program's and
// the standard library's definitions, evaluates a definition once on first
// use (recursion was refused by the resolver, so there is no cycle to
// meet), and applies functions: a closure binds its parameters in the
// environment it closed over and evaluates its body in its own scope, a
// native runs its implementation, a partial supplies its arguments first.
// `export` applies the program's `export` to the host's input plan; what
// comes back is the plan `./lower` turns into sinks.
//
// Two scopes exist: a program's definitions see the program first, then
// the library, then the natives; the library's see the library and the
// natives. Lexical, so a program that defines its own `csv` changes what
// its own code means and nothing the library does.
//
// The two standard compositions run natively when their arguments have the
// standard shapes (`table-from-json BINDING input` and `csv OPTIONS
// events`, from the library's own definitions, not a program's shadowing
// ones); `withNative(false)` turns that off so the differential test can
// run the interpreted text.
//
// Failures: a wrong argument count is `DSL_TYPE_ERROR` (`arity`); a value
// of the wrong kind where the program chose it is `DSL_TYPE_ERROR`
// (`type_mismatch`); a `fail "message"` is `INPUT_INVALID` at the form's
// position. The checker reports what it can before anything runs; these
// are the runtime's own answers for what it could not see.
//
// The depth: every walk here is a generator driven by `run`
// (`./trampoline`), so evaluation nests on an explicit stack and the
// JavaScript stack stays a few frames deep. MAX_EVAL_DEPTH is counted as
// the Rust crate counts it, a level per form and per finite text node
// written, and a function applied to itself meets it as `recursion` in
// Node's default stack, where the Rust crate needs a thread of 64 MiB.

import { AbortFlag, Duplicates, Fail, Limits, Renderers, Routers, utf8Bytes } from './shared'

import { Expr, SourceSpan, Sources, position as positionIn, sourceFile, span as spanOf } from './ast'
import { isFail } from './fail'
import type { CompileOptions } from './program'
import { Resolved, fnForm, fnParams } from './resolve'
import { Stdlib, fileOf, stdlib } from './stdlib'
import { NativeImpl, implOf, truth } from './stdlib/natives'
import { arityAccepts, arityText, native } from './stdlib/registry'
import { G, isWalk, run } from './trampoline'
import {
  Closure,
  Env,
  Func,
  NULL,
  Plan,
  Scope,
  Val,
  bind,
  bool,
  envGet,
  envHas,
  fnVal,
  funcDescribe,
  isLiveVal,
  keyword,
  kindText,
  liveKind,
  num,
  str,
  streamVal,
  textVal,
  toJsonText,
  typeError,
  valEquals,
  vectorVal,
} from './value'
import { brief, csvOptions, isTableBinding } from './lower'

// How deep evaluation may nest: a form inside a form, a function's body
// inside the call that applied it, a definition's value inside the form
// that named it. The resolver refuses a definition that names itself, but
// a function that is handed itself (`def w [f] (f f)`, then `(w w)`)
// recurses through values the resolver cannot see; this bound turns that
// into a `recursion` failure. A chain of definitions each naming the next,
// or of functions each calling the next, nests a level per link and meets
// the same bound, which is therefore also the bound on how deep a value a
// program builds can nest.
export const MAX_EVAL_DEPTH = 1_000

// How many forms building a plan may evaluate: `compile` runs `export`
// over the input's plan, which evaluates everything not under a stream,
// and a small program can ask for an exponential amount of that work
// (`def d [g] (fn [x] (g (g x)))` nested forty deep). Past this bound
// building the plan is `RESOURCE_LIMIT_EXCEEDED` naming `max_plan_steps`.
// The work a stream does per item is bounded by the host's abort flag
// instead (`withAbort`).
export const MAX_PLAN_STEPS = 1_000_000

// How often, in evaluation steps, the abort flag is read.
const ABORT_EVERY = 64

// What a message that already names a standard-library position holds.
const LIBRARY_AT = ' (at stdlib/'

// A `DSL_TYPE_ERROR` with the `arity` finer code.
export function arityError(what: string, wanted: number | string, got: number): Fail {
  return new Fail('DSL_TYPE_ERROR', `arity: ${what} takes ${wanted} argument(s), got ${got}`)
}

// What a bounded walk over a value found: its size in the transduce
// measure (payload bytes plus an allowance per node) and its nesting.
export type Measure = { bytes: number; depth: number }

// The bounds a `measure` walk holds a value to.
export type Bounds = {
  // Bytes counted for every node on top of its payload.
  nodeBytes: number
  maxBytes: number
  // The `Limits` field `maxBytes` came from, for the failure.
  bytesLimit: string
  // The deepest nesting allowed, under `max_depth`.
  maxDepth: number
  // What is being measured, for the message.
  what: string
}

// One thing a measure walk still has to visit.
type Node =
  | { node: 'val'; val: Val }
  | { node: 'plan'; plan: Plan }
  | { node: 'env'; env: Env }
  // A function held by something other than a value: the function a
  // partial applies, when it is itself a partial, or the one a plan
  // applies.
  | { node: 'fn'; f: Func }

// What a function value holds, one level below `depth`: a closure its
// frames; a partial its arguments and the function it applies. A partial
// of a partial holds the one before it, so a chain of them (a state that
// wraps itself once per item) is walked link by link, each one node and one
// level.
function pushFn(f: Func, depth: number, pending: Array<[Node, number]>): void {
  switch (f.fn) {
    case 'native':
      return
    case 'closure':
      pending.push([{ node: 'env', env: f.closure.env }, depth + 1])
      return
    case 'partial': {
      for (const arg of f.partial.args) pending.push([{ node: 'val', val: arg }, depth + 1])
      const inner = f.partial.f
      switch (inner.fn) {
        case 'native':
          return
        case 'closure':
          pending.push([{ node: 'env', env: inner.closure.env }, depth + 1])
          return
        case 'partial':
          pending.push([{ node: 'fn', f: inner }, depth + 1])
          return
      }
    }
  }
}

// The evaluator for one program.
export class Runtime {
  readonly program: Resolved
  readonly sources: Sources
  readonly lib: Stdlib
  // The stages the lowering builds (`Router`, `TableFromJson`, `ScanEmit`,
  // `Guarded`): transduce's, passed in.
  readonly routers: Routers
  // The renderers and text stages the lowering builds, and the number
  // layout the natives write: render's, passed in.
  readonly renderers: Renderers
  private isNative = true
  private policy: Duplicates = 'reject'
  // The limits values a program builds are measured against (a cell's text
  // under `max_scalar_bytes`, a `scan-emit` state under
  // `max_metadata_bytes` and `max_depth`): the host's at run time, the
  // defaults while `compile` builds the plan.
  private hostLimits: Limits = Limits.default()
  // The host's cancellation, read every ABORT_EVERY steps.
  private abortFlag: AbortFlag = new AbortFlag()
  // The evaluation steps this runtime may take, when bounded
  // (MAX_PLAN_STEPS while building a plan).
  private fuel: number | null = null
  private steps = 0
  private depth = 0
  // Definition values, evaluated on first use, by scope and name.
  private cache = new Map<string, Val>()

  // A runtime over a resolved program and its source texts (read only for
  // positions in diagnostics), with the fast paths on and duplicate members
  // rejected, building its stages from the routers and renderers a host
  // passes (`compile`'s options).
  constructor(program: Resolved, sources: Sources, options: CompileOptions) {
    if (null == options?.routers || null == options?.renderers) {
      throw new TypeError(
        "a program's runtime needs { routers, renderers }: @tabnas/transduce's routers and @tabnas/render's renderers",
      )
    }
    this.program = program
    this.sources = sources
    this.lib = stdlib()
    this.routers = options.routers
    this.renderers = options.renderers
  }

  // The limits the values a program builds are measured against.
  withLimits(limits: Limits): this {
    this.hostLimits = { ...limits }
    return this
  }

  // The host's cancellation: once it is set, the next evaluation step that
  // reads it fails with `ABORTED`, however long the item's computation
  // would have run.
  withAbort(abort: AbortFlag): this {
    this.abortFlag = abort
    return this
  }

  // Bound the evaluation steps this runtime may take (`null`: no bound).
  withFuel(fuel: number | null): this {
    this.fuel = fuel
    return this
  }

  // Run the standard compositions natively (the default) or through the
  // library's own definitions.
  withNative(native: boolean): this {
    this.isNative = native
    return this
  }

  // How a materialized capture treats a repeated member name; the default
  // rejects it (spec section 18.2, the strict mapping profile).
  withDuplicates(duplicates: Duplicates): this {
    this.policy = duplicates
    return this
  }

  get native(): boolean {
    return this.isNative
  }

  get duplicates(): Duplicates {
    return this.policy
  }

  get limits(): Limits {
    return this.hostLimits
  }

  // The abort flag the program's stages read between steps, for a stage of
  // the host's or of transduce's that polls it too.
  get abort(): AbortFlag {
    return this.abortFlag
  }

  // How many levels deep evaluation is now: zero between items.
  get level(): number {
    return this.depth
  }

  // One evaluation step: the fuel, and every ABORT_EVERY steps the abort
  // flag. Every form evaluated takes one, and so does every node a finite
  // text or a measured value walks.
  tick(): void {
    const n = ++this.steps
    if (null !== this.fuel && n > this.fuel) {
      const max = this.fuel
      throw Fail.limit(
        'max_plan_steps',
        max,
        `building the plan took more than ${max} evaluation steps; a program that computes this much before it reads its input is refused`,
      )
    }
    if (0 === n % ABORT_EVERY && this.abortFlag.isAborted()) throw Fail.aborted()
  }

  // Enter one level of evaluation at `at`, or the `recursion` failure past
  // MAX_EVAL_DEPTH. The evaluator takes a level per form, and so does
  // writing a finite text, whose `concat-map` applies its function as it
  // writes: a function that answers a text applying itself recurses there,
  // not in the evaluator. Every `enter` that returns is matched by a
  // `leave`, in a `finally`.
  enter(at?: SourceSpan): void {
    const depth = ++this.depth
    if (depth > MAX_EVAL_DEPTH) {
      this.depth--
      const fail = new Fail(
        'STREAMABILITY_UNKNOWN',
        `recursion: evaluation nested past ${MAX_EVAL_DEPTH} levels: a function applied to itself, or definitions or calls chained that deep; strict mode refuses recursion without a bound`,
      )
      throw undefined === at ? fail : this.failAt(fail, at)
    }
  }

  // The level `enter` took, given back.
  leave(): void {
    this.depth--
  }

  // The 1-based row and column of a span of the program's own text, in its
  // own file's text when the program has several sources; undefined for a
  // span of the standard library's, whose rows are not the program's.
  position(sp: SourceSpan): [number, number] | undefined {
    if (undefined !== fileOf(sp)) return undefined
    return this.sources.position(sp)
  }

  // `fail` with the position of `at`, when it has none yet. A form of the
  // program gives its row and column, and its file when the program was
  // compiled from several sources. A form of the standard library gives
  // none, since a row there would name a line of the user's file that says
  // something else; the message ends with the library file, row and column
  // instead, `(at stdlib/table.alc:68:5)`, once, and a form of the program
  // around the library's call, when the failure passes one on its way out,
  // gives the row and column.
  failAt(fail: Fail, at: SourceSpan): Fail {
    if (null != fail.row) return fail
    const lib = fileOf(at)
    if (undefined !== lib) {
      const [file, src] = lib
      if (!fail.message.includes(LIBRARY_AT)) {
        const [row, col] = positionIn(at, src)
        fail.message = `${fail.message} (at ${file}:${row}:${col})`
      }
      return fail
    }
    return this.sources.failAt(fail, at)
  }

  // `failAt` for whatever was thrown: a `Fail` is positioned, whichever
  // copy of the shared unit made it (`isFail`); anything else (a defect,
  // not a program's failure) passes unchanged.
  private positioned(err: unknown, at: SourceSpan): unknown {
    return isFail(err) ? this.failAt(err, at) : err
  }

  // The definition `name` denotes in `scope`, with the scope it was found
  // in.
  private findDef(scope: Scope, name: string) {
    if ('program' === scope) {
      const def = this.program.get(name)
      if (undefined !== def) return { scope: 'program' as Scope, def }
    }
    const def = this.lib.get(name)
    return undefined === def ? undefined : { scope: 'stdlib' as Scope, def }
  }

  // The value of a definition, evaluated once.
  *defValue(scope: Scope, name: string): G<Val | undefined> {
    const found = this.findDef(scope, name)
    if (undefined === found) return undefined
    const key = found.scope + '\u0000' + found.def.name
    const cached = this.cache.get(key)
    if (undefined !== cached) return cached
    const def = found.def
    let value: Val
    const params = fnParams(def.value)
    if (undefined !== params && 'list' === def.value.kind) {
      value = fnVal({
        fn: 'closure',
        closure: {
          name: def.name,
          params,
          body: def.value.items[2],
          env: null,
          scope: found.scope,
          span: def.value.span,
        },
      })
    } else {
      value = yield this.eval(def.value, null, found.scope)
    }
    this.cache.set(key, value)
    return value
  }

  // The value a free symbol denotes in `scope`: a definition, a native
  // constant's value, or a native function.
  private *global(scope: Scope, name: string, sp: SourceSpan): G<Val> {
    const v: Val | undefined = yield this.defValue(scope, name)
    if (undefined !== v) return v
    const n = native(name)
    if (undefined !== n) {
      if ('constant' === n.kind) return implOf(n)(this, [], sp) as Val
      return fnVal({ fn: 'native', native: n })
    }
    throw this.failAt(new Fail('DSL_TYPE_ERROR', `unknown_name: ${name} is not defined`), sp)
  }

  // Evaluate one form.
  *eval(expr: Expr, env: Env, scope: Scope): G<Val> {
    this.tick()
    this.enter(expr.span)
    try {
      switch (expr.kind) {
        case 'symbol': {
          const local = envGet(env, expr.name)
          if (undefined !== local) return local
          return yield this.global(scope, expr.name, expr.span)
        }
        case 'keyword':
          return keyword(expr.name)
        case 'str':
          return str(expr.value)
        case 'num': {
          const value = Number(expr.lexeme)
          if (Number.isNaN(value)) throw this.failAt(typeError(`${expr.lexeme} is not a number`), expr.span)
          return num(value, expr.lexeme)
        }
        case 'bool':
          return bool(expr.value)
        case 'null':
          return NULL
        case 'vector': {
          const out: Val[] = []
          for (const item of expr.items) {
            const v: Val = yield this.eval(item, env, scope)
            if (isLiveVal(v)) {
              throw this.failAt(
                typeError(`a vector cannot hold ${liveKind(v)}; a stream is used once, where it is`),
                expr.span,
              )
            }
            out.push(v)
          }
          return vectorVal(out)
        }
        case 'list':
          return yield this.list(expr.items, expr.span, env, scope)
      }
    } finally {
      this.leave()
    }
  }

  private *list(items: Expr[], sp: SourceSpan, env: Env, scope: Scope): G<Val> {
    const head = items[0]
    if (undefined === head) throw this.failAt(typeError('an empty list is not a call'), sp)
    const special = 'symbol' === head.kind && !envHas(env, head.name) ? head.name : undefined
    switch (special) {
      case 'fn': {
        const form = fnForm(items)
        if (undefined === form) {
          throw this.failAt(new Fail('DSL_TYPE_ERROR', 'bad_fn: fn takes [params] of symbols and one body'), sp)
        }
        return fnVal({
          fn: 'closure',
          closure: { params: form[0], body: form[1], env, scope, span: sp },
        })
      }
      case 'let': {
        const badLet = () =>
          this.failAt(new Fail('DSL_TYPE_ERROR', 'bad_let: let takes one binding [name value] and one body'), sp)
        const binding = items[1]
        const body = items[2]
        if (3 !== items.length || undefined === binding || 'vector' !== binding.kind || undefined === body) {
          throw badLet()
        }
        const name = binding.items[0]
        const value = binding.items[1]
        if (2 !== binding.items.length || undefined === name || 'symbol' !== name.kind || undefined === value) {
          throw badLet()
        }
        const bound: Val = yield this.eval(value, env, scope)
        return yield this.eval(body, bind(env, name.name, bound), scope)
      }
      case 'if': {
        if (4 !== items.length) {
          throw this.failAt(
            new Fail('DSL_TYPE_ERROR', 'bad_if: if takes a condition and exactly two branches'),
            sp,
          )
        }
        const condition: Val = yield this.eval(items[1], env, scope)
        let chosen: boolean
        try {
          chosen = truth('if', condition)
        } catch (err) {
          throw this.positioned(err, items[1].span)
        }
        return yield this.eval(chosen ? items[2] : items[3], env, scope)
      }
      case 'match': {
        const subject = items[1]
        if (undefined === subject) {
          throw this.failAt(
            new Fail('DSL_TYPE_ERROR', 'bad_match: match takes a value and (case pattern body) clauses'),
            sp,
          )
        }
        const value: Val = yield this.eval(subject, env, scope)
        for (const clause of items.slice(2)) {
          if ('list' !== clause.kind) continue
          const parts = clause.items
          if (3 !== parts.length || 'symbol' !== parts[0].kind || 'case' !== parts[0].name) continue
          const bound: Env | undefined = yield this.matches(parts[1], value, env, scope)
          if (undefined !== bound) return yield this.eval(parts[2], bound, scope)
        }
        throw this.failAt(new Fail('DSL_TYPE_ERROR', `no_match: no case matches ${brief(value)}`), sp)
      }
      case 'def':
        throw this.failAt(new Fail('DSL_TYPE_ERROR', 'misplaced_def: def is only allowed at the top level'), sp)
      default: {
        const f: Val = yield this.eval(head, env, scope)
        if ('fn' !== f.v) {
          throw this.failAt(typeError(`${kindText(f)} is not a function and cannot be called`), head.span)
        }
        const args: Val[] = []
        for (let i = 1; i < items.length; i++) args.push(yield this.eval(items[i], env, scope))
        return yield this.apply(f.f, args, sp)
      }
    }
  }

  // Apply a function. `at` is the span of the call, for diagnostics.
  *apply(f: Func, args: Val[], at: SourceSpan): G<Val> {
    // A partial supplies its arguments first, link by link.
    let here = f
    let all = args
    while ('partial' === here.fn) {
      all = [...here.partial.args, ...all]
      here = here.partial.f
    }
    if ('closure' === here.fn) {
      const c = here.closure
      if (c.params.length !== all.length) {
        throw this.failAt(arityError(funcDescribe(here), c.params.length, all.length), at)
      }
      if (this.isNative && 'stdlib' === c.scope) {
        const fast = this.fastPath(c, all, at)
        if (undefined !== fast) return fast
      }
      let env = c.env
      for (let i = 0; i < c.params.length; i++) env = bind(env, c.params[i], all[i])
      return yield this.eval(c.body, env, c.scope)
    }
    const n = here.native
    if (!arityAccepts(n.arity, all.length)) {
      throw this.failAt(arityError(n.name, arityText(n.arity), all.length), at)
    }
    const impl: NativeImpl = implOf(n)
    try {
      const out = impl(this, all, at)
      return isWalk(out) ? yield out : out
    } catch (err) {
      throw this.positioned(err, at)
    }
  }

  // `apply`, driven to its answer now: for the stages that apply a
  // program's function per item, outside any evaluation.
  applyNow(f: Func, args: Val[], at: SourceSpan): Val {
    return run(this.apply(f, args, at))
  }

  // The native plan for a standard composition, when the arguments have the
  // standard shapes; undefined runs the library's text.
  private fastPath(closure: Closure, args: Val[], at: SourceSpan): Val | undefined {
    switch (closure.name) {
      case 'table-from-json': {
        if (2 !== args.length) return undefined
        const [binding, input] = args
        if ('stream' !== input.v || !isTableBinding(binding)) return undefined
        return streamVal({ p: 'table-from-json', binding, source: input.plan, at })
      }
      case 'csv': {
        if (2 !== args.length) return undefined
        const [options, events] = args
        if ('stream' !== events.v || undefined === csvOptions(options)) return undefined
        return textVal({ p: 'csv', options, source: events.plan })
      }
      default:
        return undefined
    }
  }

  // Whether a symbol in a pattern compares rather than binds: the same rule
  // the resolver applies (a local binds; a definition or a native constant
  // compares; anything else binds).
  private *patternConstant(name: string, env: Env, scope: Scope, sp: SourceSpan): G<Val | undefined> {
    if (envHas(env, name)) return undefined
    const v: Val | undefined = yield this.defValue(scope, name)
    if (undefined !== v) return v
    const n = native(name)
    if (undefined !== n && 'constant' === n.kind) return implOf(n)(this, [], sp) as Val
    return undefined
  }

  // Match `value` against `pattern`: the environment extended with the
  // pattern's bindings, or undefined.
  *matches(pattern: Expr, value: Val, env: Env, scope: Scope): G<Env | undefined> {
    switch (pattern.kind) {
      case 'symbol': {
        if ('_' === pattern.name) return env
        const constant: Val | undefined = yield this.patternConstant(pattern.name, env, scope, pattern.span)
        if (undefined === constant) return bind(env, pattern.name, value)
        return valEquals(constant, value) ? env : undefined
      }
      case 'keyword':
      case 'str':
      case 'num':
      case 'bool':
      case 'null': {
        const literal: Val = yield this.eval(pattern, env, scope)
        return valEquals(literal, value) ? env : undefined
      }
      case 'vector': {
        if ('vector' !== value.v || value.items.length !== pattern.items.length) return undefined
        let bound: Env = env
        for (let i = 0; i < pattern.items.length; i++) {
          const next: Env | undefined = yield this.matches(pattern.items[i], value.items[i], bound, scope)
          if (undefined === next) return undefined
          bound = next
        }
        return bound
      }
      case 'list': {
        const head = pattern.items[0]
        if (undefined === head || 'symbol' !== head.kind) {
          throw this.failAt(
            new Fail('DSL_TYPE_ERROR', 'bad_pattern: a list pattern is (constructor pattern...)'),
            pattern.span,
          )
        }
        if ('tagged' !== value.v || value.tag !== head.name || value.fields.length !== pattern.items.length - 1) {
          return undefined
        }
        let bound: Env = env
        for (let i = 1; i < pattern.items.length; i++) {
          const next: Env | undefined = yield this.matches(pattern.items[i], value.fields[i - 1], bound, scope)
          if (undefined === next) return undefined
          bound = next
        }
        return bound
      }
    }
  }

  // The program's result: `export` applied to the host's input plan. A
  // stream or a text plan comes back; nothing has been consumed.
  *export(): G<Val> {
    const exp: Val | undefined = yield this.defValue('program', 'export')
    if (undefined === exp) {
      throw new Fail('DSL_TYPE_ERROR', 'no_export: the program has no `def export [input]`')
    }
    if ('fn' !== exp.v) {
      throw new Fail('DSL_TYPE_ERROR', `type_mismatch: export must be a fn [input], not ${kindText(exp)}`)
    }
    const def = this.program.get('export')
    const at = undefined !== def ? def.span : spanOf(sourceFile(this.program.file), 0, 0)
    return yield this.apply(exp.f, [streamVal({ p: 'input' })], at)
  }

  // Evaluate a standalone expression in the program's scope, for tests and
  // tools.
  evalProgramExpr(expr: Expr): Val {
    return run(this.eval(expr, null, 'program'))
  }

  // Walk `v` without recursing, counting its size and nesting, and fail as
  // soon as either passes its bound: `RESOURCE_LIMIT_EXCEEDED` naming the
  // byte limit, or `max_depth`. The walk visits a shared part once per
  // reference, so a value built by sharing (a vector of itself twice, forty
  // times over) counts as large as it would be written, and the walk ends
  // at the bound rather than visiting it all. Every node takes an
  // evaluation step, so the abort flag stops it too.
  measure(v: Val, bounds: Bounds): Measure {
    const found: Measure = { bytes: 0, depth: 0 }
    const pending: Array<[Node, number]> = [[{ node: 'val', val: v }, 1]]
    while (0 < pending.length) {
      const [node, depth] = pending.pop() as [Node, number]
      this.tick()
      if (depth > bounds.maxDepth) {
        throw Fail.limit(
          'max_depth',
          bounds.maxDepth,
          `${bounds.what} nests more than ${bounds.maxDepth} levels deep`,
        )
      }
      found.depth = Math.max(found.depth, depth)
      let payload = 0
      switch (node.node) {
        case 'val': {
          const x = node.val
          switch (x.v) {
            case 'null':
            case 'bool':
              break
            case 'num':
              payload = undefined === x.lexeme ? 8 : utf8Bytes(x.lexeme)
              break
            case 'str':
              payload = utf8Bytes(x.value)
              break
            case 'keyword':
              payload = utf8Bytes(x.name)
              break
            case 'vector':
              for (const item of x.items) pending.push([{ node: 'val', val: item }, depth + 1])
              break
            case 'record':
              for (const item of x.fields.values()) pending.push([{ node: 'val', val: item }, depth + 1])
              for (const k of x.fields.keys()) payload += utf8Bytes(k)
              break
            case 'tagged':
              for (const item of x.fields) pending.push([{ node: 'val', val: item }, depth + 1])
              payload = utf8Bytes(x.tag)
              break
            case 'selector':
              payload = utf8Bytes(x.selector.toString())
              break
            case 'capture':
              payload = utf8Bytes(x.spec.tag) + utf8Bytes(x.spec.selector.toString())
              break
            case 'fn':
              pushFn(x.f, depth, pending)
              break
            case 'stream':
            case 'text':
              pending.push([{ node: 'plan', plan: x.plan }, depth + 1])
              break
          }
          break
        }
        case 'plan': {
          const plan = node.plan
          switch (plan.p) {
            case 'lit':
              payload = utf8Bytes(plan.text)
              break
            case 'concat':
              for (const item of plan.items) pending.push([{ node: 'val', val: item }, depth + 1])
              break
            case 'join':
              if ('vector' === plan.items.seq) {
                for (const item of plan.items.items) pending.push([{ node: 'val', val: item }, depth + 1])
              }
              payload = utf8Bytes(plan.sep)
              break
            // A finite concat-map keeps the function it applies until it is
            // written: a partial over the state before holds that state, so
            // the function is walked like any value the plan holds.
            case 'concat-map':
              pending.push([{ node: 'fn', f: plan.f }, depth + 1])
              if ('vector' === plan.items.seq) {
                for (const item of plan.items.items) pending.push([{ node: 'val', val: item }, depth + 1])
              }
              break
            case 'replace':
              pending.push([{ node: 'val', val: plan.source }, depth + 1])
              payload = utf8Bytes(plan.from) + utf8Bytes(plan.to)
              break
            // A live plan's source is the one pass over the input, not a
            // retained value, and is not walked; what the plan itself holds
            // (its functions, its options, a scan's initial state) is.
            case 'map':
            case 'filter':
              pending.push([{ node: 'fn', f: plan.f }, depth + 1])
              break
            case 'scan-emit':
              pending.push([{ node: 'val', val: plan.init }, depth + 1])
              pending.push([{ node: 'fn', f: plan.step }, depth + 1])
              pending.push([{ node: 'fn', f: plan.finish }, depth + 1])
              break
            case 'table-from-json':
              pending.push([{ node: 'val', val: plan.binding }, depth + 1])
              break
            case 'csv-table':
            case 'csv':
              pending.push([{ node: 'val', val: plan.options }, depth + 1])
              break
            case 'route':
              for (const c of plan.specs) payload += utf8Bytes(c.tag) + utf8Bytes(c.selector.toString())
              break
            case 'select':
              payload = utf8Bytes(plan.selector.toString())
              break
            case 'input':
            case 'events':
            case 'records':
            case 'json':
              break
          }
          break
        }
        // A frame's value, and the frames outside it one level further: a
        // chain of frames is held one inside another.
        case 'env':
          if (null !== node.env) {
            pending.push([{ node: 'val', val: node.env.value }, depth + 1])
            pending.push([{ node: 'env', env: node.env.next }, depth + 1])
          }
          break
        case 'fn':
          pushFn(node.f, depth, pending)
          break
      }
      found.bytes += bounds.nodeBytes + payload
      if (found.bytes > bounds.maxBytes) {
        throw Fail.limit(bounds.bytesLimit, bounds.maxBytes, `${bounds.what} holds more than ${bounds.maxBytes} bytes`)
      }
    }
    return found
  }

  // The compact JSON text of a vector or a record, as a cell writes it
  // (number lexemes kept), bounded by `max_scalar_bytes`: the text is one
  // scalar of the output, and the native table holds a container cell's
  // text to the same bound. The failure comes exactly when the text is
  // longer than the bound; a lower bound of its length is counted first,
  // without writing it, so a value built by sharing (a vector of a vector of
  // itself, forty levels deep) fails at the limit rather than being written
  // out.
  jsonText(v: Val): string {
    const max = this.hostLimits.max_scalar_bytes
    const tooLong = () => Fail.limit('max_scalar_bytes', max, `a cell's JSON text holds more than ${max} bytes`)
    if (this.jsonTextFloor(v, max) > max) throw tooLong()
    const text = toJsonText(v)
    if (utf8Bytes(text) > max) throw tooLong()
    return text
  }

  // At most the length of `v`'s compact JSON text, counted without writing
  // it or recursing, and without going on once it passes `max`: a
  // container's brackets, a member's quoted key and colon, a string's
  // quotes around its bytes (escapes only lengthen it), a number's lexeme
  // (or one digit), four for `null` and the booleans. Every value counts at
  // least one, so the walk ends within `max` values, and every one takes an
  // evaluation step, so the abort flag stops it too.
  private jsonTextFloor(v: Val, max: number): number {
    let total = 0
    const pending: Val[] = [v]
    while (0 < pending.length) {
      const x = pending.pop() as Val
      this.tick()
      let here = 0
      switch (x.v) {
        case 'null':
        case 'bool':
          here = 4
          break
        case 'num':
          here = undefined === x.lexeme ? 1 : Math.max(1, utf8Bytes(x.lexeme))
          break
        case 'str':
          here = utf8Bytes(x.value) + 2
          break
        case 'vector':
          for (const item of x.items) pending.push(item)
          here = 2
          break
        case 'record':
          for (const item of x.fields.values()) pending.push(item)
          here = 2
          for (const k of x.fields.keys()) here += utf8Bytes(k) + 3
          break
        // Not JSON: the conversion refuses it.
        default:
          here = 0
      }
      total += here
      if (total > max) break
    }
    return total
  }
}

// Make a partial application value, for callers outside the evaluator.
export function partial(f: Func, args: Val[]): Val {
  return fnVal({ fn: 'partial', partial: { f, args } })
}
