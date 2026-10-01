/* Copyright (c) 2026 tabnas, MIT License */

// tabnas-alchemy for TypeScript: the alchemy grammar plugin and the front
// end of the language, ported from the Rust crate in ../rs (the
// reference implementation).
//
// The pipeline reads left to right: `./lex` and `./grammar` (the reader)
// → `./ast` → `./desugar` → `./resolve` → `./check`, with `./effects`
// reading a built plan for `explain`. `./program` is the API around the
// front end (`analyze`, `analyzeSources`).
//
// Pass 2 of the port adds the evaluator (`./interp`, building the plan
// `./value` describes), the lowering to transduce and render sinks
// (`./lower`), `compile` and `Program` in `./program`, and the
// natives' implementations (`installCalls` in `./stdlib/registry`).

// VERSION is this package's version. It MUST equal package.json "version",
// which is the crate's (rs/Cargo.toml): the runtimes ship as one version.
export const VERSION = '0.1.1'

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

export { analyze, analyzeSources } from './program'
export type { Analyzed, Output, Source } from './program'

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
