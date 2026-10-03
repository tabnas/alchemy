/* Copyright (c) 2026 tabnas, MIT License */

// The `alchemy` command (rs/src/bin/alchemy.rs), run by `bin/alchemy`.
//
//   alchemy canon FILE       print the program in canonical form
//   alchemy format FILE      print the program in layout form
//   alchemy check FILE       parse, desugar, resolve, check and build the plan; print nothing and exit 0
//   alchemy explain FILE     print the plan report
//   alchemy run [--render csv|json] [--no-native] [--max-output-bytes N] PROGRAM INPUT
//                            run the program over the JSON document INPUT
//
// `FILE`, `PROGRAM` and `INPUT` may be `-` for standard input (one of them
// per run). Nothing but the answer goes to standard output. A failure is
// the `Fail` as one JSON object on standard error (`code`, `message`, and
// `path`, `limit`, `row`, `col` when they apply, and `output`: `"partial"`
// when bytes had reached standard output). The exit status follows the
// code: 2 for a program that does not read, resolve or check
// (`DSL_PARSE_ERROR`, `DSL_TYPE_ERROR`, `STREAM_REUSED`,
// `STREAMABILITY_UNKNOWN`), for a usage error and for an unreadable file; 1
// for an input or protocol failure (from `check` too, when building the
// plan reaches a `fail`); 5 for `RESOURCE_LIMIT_EXCEEDED`; 3 for
// `OUTPUT_FAILED`; 6 for `ABORTED`. A standard error that cannot be
// written loses the report, not the status.
//
// The Rust command runs on a thread of 64 MiB so the checker's and the
// evaluator's bounds are reached before the stack's end; here they run on
// an explicit stack (`./trampoline`), so Node's default stack is enough.
// `--max-output-bytes` sets the writer's limit; the others are transduce's
// defaults.
//
// `run` parses `INPUT` with the tabnas JSON grammar through transduce's
// `ParserSource`, incrementally, pruning the parsed tree under the
// program's row selector when the plan knows one; other input formats are
// aless's business. The output goes through a coalescing writer to
// standard output and is flushed once, at the end; a failure found after
// bytes were written says so.

import { readFileSync, readSync, writeSync } from 'node:fs'

import { make as makeJson } from '@tabnas/json'
import { FdWriter } from '@tabnas/render'
import { Fail, Limits, Metrics, ParserSource, Prune, SourceMode } from '@tabnas/transduce'

import { canonical, format } from './ast'
import { isFail } from './fail'
import { parseFile } from './grammar'
import { Renderer, rendererNamed } from './lower'
import { Program, compile } from './program'

const USAGE =
  'usage: alchemy canon|format|check|explain FILE\n' +
  '       alchemy run [--render csv|json] [--no-native] [--max-output-bytes N] PROGRAM INPUT\n' +
  '       (a FILE may be - for standard input)'

// A failure and the status it exits with: the code's, except that a usage
// error and an unreadable file exit 2 although their code is
// `INPUT_INVALID`, since they are the command line's fault, not the
// document's.
class Exit {
  constructor(
    readonly fail: Fail,
    readonly status: number,
  ) {}

  static of(fail: Fail): Exit {
    switch (fail.code) {
      case 'DSL_PARSE_ERROR':
      case 'DSL_TYPE_ERROR':
      case 'STREAM_REUSED':
      case 'STREAMABILITY_UNKNOWN':
        return new Exit(fail, 2)
      case 'RESOURCE_LIMIT_EXCEEDED':
        return new Exit(fail, 5)
      case 'OUTPUT_FAILED':
        return new Exit(fail, 3)
      case 'ABORTED':
        return new Exit(fail, 6)
      default:
        return new Exit(fail, 1)
    }
  }
}

function usage(): Exit {
  return new Exit(Fail.input(USAGE), 2)
}

// Write all of `text` to the descriptor, offering a non-blocking pipe the
// rest again when it answers `EAGAIN`.
function writeAll(fd: number, text: string): void {
  let bytes = Buffer.from(text, 'utf8')
  while (0 < bytes.length) {
    let n: number
    try {
      n = writeSync(fd, bytes)
    } catch (err: any) {
      if ('EAGAIN' === err?.code) continue
      throw err
    }
    bytes = bytes.subarray(n)
  }
}

// The failure as one JSON object on standard error. A standard error that
// cannot be written (a closed pipe) loses the report, not the status.
function report(fail: Fail): void {
  try {
    writeAll(2, JSON.stringify(fail.toJSON()) + '\n')
  } catch (_err) {
    // The status still says what happened.
  }
}

// All of standard input, waiting out a non-blocking descriptor's `EAGAIN`.
function readStdin(): Buffer {
  const chunks: Buffer[] = []
  const chunk = Buffer.alloc(64 * 1024)
  for (;;) {
    let n: number
    try {
      n = readSync(0, chunk, 0, chunk.length, null)
    } catch (err: any) {
      if ('EAGAIN' === err?.code) continue
      // A closed or absent standard input reads as empty, as Rust's does.
      if ('EOF' === err?.code) break
      throw err
    }
    if (0 === n) break
    chunks.push(Buffer.from(chunk.subarray(0, n)))
  }
  return Buffer.concat(chunks)
}

const UTF8 = new TextDecoder('utf-8', { fatal: true })

// A file's text, or standard input's for `-`; a file that cannot be read,
// or that is not UTF-8, is the command line's failure (status 2).
function read(file: string): string {
  let bytes: Buffer
  try {
    bytes = '-' === file ? readStdin() : readFileSync(file)
  } catch (err: any) {
    throw new Exit(Fail.input(`cannot read ${file}: ${err?.message ?? err}`), 2)
  }
  try {
    return UTF8.decode(bytes)
  } catch (_err) {
    throw new Exit(Fail.input(`cannot read ${file}: stream did not contain valid UTF-8`), 2)
  }
}

type RunOptions = {
  render?: Renderer
  native: boolean
  maxOutputBytes: number | null
  program: string
  input: string
}

const U64_MAX = 2n ** 64n - 1n

function parseRunOptions(args: ReadonlyArray<string>): RunOptions {
  let render: Renderer | undefined
  let native = true
  let maxOutputBytes: number | null = null
  const files: string[] = []
  let i = 0
  while (i < args.length) {
    const arg = args[i]
    if ('--render' === arg) {
      const name = args[i + 1]
      if (undefined === name) throw usage()
      render = rendererNamed(name)
      if (undefined === render) {
        throw new Exit(Fail.input(`--render takes csv or json, not ${JSON.stringify(name)}`), 2)
      }
      i += 2
    } else if ('--no-native' === arg) {
      native = false
      i += 1
    } else if ('--max-output-bytes' === arg) {
      const n = args[i + 1]
      if (undefined === n) throw usage()
      // As Rust reads a `u64`: digits, a leading `+` allowed, no larger
      // than 2^64 - 1.
      if (!/^\+?[0-9]+$/.test(n) || BigInt(n) > U64_MAX) {
        throw new Exit(Fail.input(`--max-output-bytes takes a number of bytes, not ${JSON.stringify(n)}`), 2)
      }
      maxOutputBytes = Number(BigInt(n))
      i += 2
    } else if (arg.startsWith('--')) {
      throw usage()
    } else {
      files.push(arg)
      i += 1
    }
  }
  if (2 !== files.length) throw usage()
  const [program, input] = files
  if ('-' === program && '-' === input) {
    throw new Exit(Fail.input('only one of PROGRAM and INPUT may be - (standard input)'), 2)
  }
  return { render, native, maxOutputBytes, program, input }
}

// Run the program over one JSON document, writing to standard output.
function execute(program: Program, input: string, render: Renderer | undefined, limits: Limits): void {
  const metrics = new Metrics()
  const sink = program.sink(new FdWriter(1), render, limits, metrics)
  const selector = program.rowSelector()
  const prune = undefined === selector ? Prune.never() : Prune.under(selector)
  try {
    new ParserSource(makeJson(), input)
      .grammar('json')
      .mode(SourceMode.incremental(prune))
      .limits(limits)
      .metrics(metrics)
      .run(sink)
  } catch (err) {
    // A failure the source found (bad JSON after the rows) knows nothing of
    // the output; the writer's count says whether bytes had reached
    // standard output.
    if (isFail(err) && metrics.output_bytes > 0 && !err.committedOutput) err.committed()
    throw err
  }
}

// The text to print for the command line. `run` writes its answer itself
// and answers nothing here.
function command(args: ReadonlyArray<string>): string {
  // The command is judged before any file is read, so an unknown one is
  // the usage error whatever the files hold, and never waits on standard
  // input to say so.
  const [name, ...rest] = args
  switch (name) {
    case 'canon':
    case 'format':
    case 'check':
    case 'explain': {
      if (1 !== rest.length) throw usage()
      const file = rest[0]
      const src = read(file)
      switch (name) {
        case 'canon': {
          const text = canonical(parseFile(src, file))
          return '' === text ? text : text + '\n'
        }
        case 'format':
          return format(parseFile(src, file))
        case 'check':
          compile(src, file)
          return ''
        default:
          return compile(src, file).explain()
      }
    }
    case 'run': {
      const options = parseRunOptions(rest)
      const src = read(options.program)
      let program = compile(src, options.program)
      if (!options.native) program = program.withNative(false)
      const input = read(options.input)
      const limits: Limits = { ...Limits.default(), max_output_bytes: options.maxOutputBytes }
      execute(program, input, options.render, limits)
      return ''
    }
    default:
      throw usage()
  }
}

// The status of a defect of this package (anything thrown that is not a
// `Fail`, from whichever copy of transduce: `isFail`): Rust's for a panic,
// so a script never reads one as an input failure.
const DEFECT = 101

// Run one command line and answer its exit status.
export function main(args: ReadonlyArray<string>): number {
  let text: string
  try {
    text = command(args)
  } catch (err) {
    const exit = err instanceof Exit ? err : isFail(err) ? Exit.of(err) : undefined
    if (undefined === exit) {
      try {
        writeAll(2, `alchemy: internal error: ${err instanceof Error ? (err.stack ?? err.message) : String(err)}\n`)
      } catch (_err) {
        // The status still says what happened.
      }
      return DEFECT
    }
    report(exit.fail)
    return exit.status
  }
  try {
    writeAll(1, text)
  } catch (_err) {
    return 3
  }
  return 0
}
