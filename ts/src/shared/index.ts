/* Copyright (c) 2026 tabnas, MIT License */

// The shared types: what alchemy, transduce and render agree on, owned by
// alchemy and imported by the other two as `@tabnas/alchemy/shared`, an
// entry that loads this directory and nothing else. Nothing here imports
// anything outside it.
//
// No class here has `private` or `protected` members or `#` fields.
// TypeScript compares a class with any of those nominally, so two installed
// copies of this package (one in an application, one under a dependency
// that peers on it, or a linked checkout beside a registry copy, as the
// fleet's CI builds them) would refuse each other's `AbortFlag`, `Sink` or
// `Routers`. Without them the types compare by shape and the copies agree.
//
// - the source protocol `JsonEvent` (`JsonEvents/1`) and the push boundary
//   `Sink`, with `Flow` and the recorders;
// - the table protocol `TableEvent` (`TableRows/1`): `Cell`, `Schema`, the
//   bindings and the standard column mapping;
// - `Selector`, `Path` and `Segment`; `Datum`, the retained value, and its
//   builder; reading the engine's values;
// - `Fail` and the stable failure `Code`s; `Limits`, `Metrics` and
//   `AbortFlag`; the JSON text every runtime shares;
// - captures as data (`CaptureSpec`, `Selected`, `RouteSink`) and a
//   `scan-emit` step's `Transition`;
// - the text boundary (`TextOut`, `Writer`) and the renderers' options;
// - `Routers` and `Renderers`, what alchemy's runtime constructs and calls:
//   transduce implements the first (`routers`), render the second
//   (`renderers`), and a host passes both to `compile`.
//
// The protocol modules were transduce's (`event`, `sink`, `table`, `error`,
// `limits`, `selector`, `datum`, `json`, `value`) and keep its names;
// transduce and render re-export what they exported before, so their
// public APIs are unchanged.

export {
  Datum,
  DatumBuilder,
  byteSize,
  fromJSON,
  fromTabnas,
  getPath,
  takePath,
  toJSON,
  toText,
  walkDatum,
} from './datum'
export type { Duplicates } from './datum'
export { Code, Fail, engineCode, engineDetail, enginePosition, isFail } from './error'
export type { Limit } from './error'
export { Ev, eventEquals, eventText, isEnd, isScalar, isStart } from './event'
export type { JsonEvent, JsonEventType } from './event'
export { isJsonNumber, jsonNumber, jsonString, numberText } from './json'
export { AbortFlag, LIMIT_NAMES, Limits, Metrics, NODE_BYTES, utf8Bytes } from './limits'
export { Path, Selector, keyText, pathText } from './selector'
export type { Segment, Step } from './selector'
export { CountSink, EventRecorder, FnSink, TreeContract, replay } from './sink'
export type { Flow, Sink } from './sink'
export { Cell, Schema, Table, boundColumn, columnFromMeta } from './table'
export type {
  BoundColumn,
  ColumnMapper,
  MissingPolicy,
  PublicColumn,
  TableBinding,
  TableEvent,
  TableSink,
} from './table'
export { engineKeys, engineScalar, isEngineContainer, isEngineMap } from './value'
export type { EngineScalar } from './value'
export { CaptureSpec } from './route'
export type { Budget, CaptureId, CaptureMode, RouteSink, Selected } from './route'
export { Transition } from './scan'
export type { TextOut, Writer } from './text'
export { CsvOptions, JsonOptions, MissingText, Newline } from './options'
export type { MissingRecord, Quoting } from './options'
export type { Routers, Scan } from './routers'
export type { CoalescingOut, JoinOut, Renderers } from './renderers'
