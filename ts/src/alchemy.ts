/* Copyright (c) 2026 tabnas, MIT License */

// tabnas-alchemy for TypeScript: the alchemy grammar plugin and the
// language, ported from the Rust crate in ../rs (the reference
// implementation).
//
// The pipeline reads left to right: `./lex` and `./grammar` (the reader)
// → `./ast` → `./desugar` → `./resolve` → `./check` → `./interp` (builds
// the plan `./value` describes) → `./lower` (the transduce and render
// sinks); `./effects` reads the plan for `explain`; `./program` is the API
// a host embeds (`compile`, `compileSources`, `Program`). The natives are
// listed in `./stdlib/registry` and implemented in `./stdlib/natives`.
//
// `./shared` holds the types alchemy, transduce and render share, and
// `Routers` and `Renderers`, the stages the lowering builds: transduce and
// render import it (`@tabnas/alchemy/shared`) and implement those two, and
// a host passes their implementations to `compile` as `{ routers,
// renderers }`. Alchemy imports neither package. The `alchemy` command is
// `@tabnas/alchemy-cli`.
//
// The checker and the evaluator recurse on an explicit stack
// (`./trampoline`), so MAX_NESTING, MAX_APPLIED and MAX_EVAL_DEPTH are
// reached as failures in Node's default stack, where the Rust crate runs
// them on a thread of 64 MiB.

// VERSION is this package's version. It MUST equal package.json "version",
// which is the crate's (rs/Cargo.toml): the runtimes ship as one version.
export const VERSION = '0.2.3'

export {
  MAX_NESTING,
  TOO_DEEP,
  Sources,
  canonical,
  canonicalForm,
  format,
  fromValue,
  isAtom,
  jsonString,
  position,
  programFromValue,
  sameProgram,
  sameShape,
  sourceFile,
  span,
  spanText,
  symbolOf,
} from './ast'
export type { Expr, SourceFile, SourceSpan } from './ast'

export {
  UNNAMED,
  alchemy,
  failFrom,
  grammarDocument,
  grammarSource,
  make,
  parse,
  parseFile,
  parseValue,
  refs,
} from './grammar'

export { IN, DE, NL, KW, MATCHER, isJsonNumber, isSymbolChar } from './lex'

export { MESSAGES, desugarExpr, desugarProgram } from './desugar'

export { Resolved, SPECIAL_FORMS, defParams, fnForm, fnParams, resolve } from './resolve'
export type { Def, NameKind, Outer } from './resolve'

export * as types from './types'
export type { Type } from './types'

export { MAX_APPLIED, checkProgram, checkStdlibFile, noExport, stdlibSignature } from './check'
export type { Checked } from './check'

export { Program, analyze, analyzeSources, compile, compileSources } from './program'
export type { Analyzed, CompileOptions, Output, Source } from './program'

export { MAX_EVAL_DEPTH, MAX_PLAN_STEPS, Runtime, arityError, partial } from './interp'
export type { Bounds, Measure } from './interp'

export {
  Lowering,
  brief,
  csvOptions,
  isTableBinding,
  jsonOptions,
  rendererNamed,
  stateBounds,
  writeFinite,
} from './lower'
export type { ItemSink, Renderer } from './lower'

export { run, isWalk } from './trampoline'
export type { G } from './trampoline'

// Whether a thrown value is a `Fail`, from whichever copy of the shared unit
// made it: what a host tells a failure from a defect by.
export { isFail } from './fail'

export {
  csvDialect,
  defaultCsvDialect,
  explain,
  explainJson,
  isInferred,
  summarize,
  summaryJson,
  summaryText,
} from './effects'
export type { CsvDialect, EffectSummary, Explainable, RendererProfile, Retention } from './effects'

export * as value from './value'
export type { Func, Plan, Val } from './value'

export { Stdlib, SOURCES, fileOf, load as loadStdlib, nativeKind, outer, source, stdlib } from './stdlib'

export {
  arityAccepts,
  arityExact,
  arityText,
  call as nativeCall,
  installCalls,
  native,
  natives,
} from './stdlib/registry'
export type { Arity, Kind, Native } from './stdlib/registry'

export {
  CAPTURE_LIMITS,
  LENGTH_CHUNK,
  captureBudget,
  getField,
  implOf,
  kindWord,
  numberText,
  quote,
  quotedLen,
  shortestNumber,
  truth,
  unimplemented,
} from './stdlib/natives'
export type { NativeImpl } from './stdlib/natives'

// The shared types (`./shared`, also `@tabnas/alchemy/shared`). Four names
// are this package's own above and keep their meaning here: `jsonString`
// (`./ast`), `isJsonNumber` (`./lex`), `numberText` (`./stdlib/natives`)
// and `isFail` (`./fail`, which also sees a `Fail` from another copy of the
// shared unit); the shared versions are in `@tabnas/alchemy/shared`.
export * from './shared'
