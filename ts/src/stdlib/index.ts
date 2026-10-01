/* Copyright (c) 2026 tabnas, MIT License */

// The standard library: the natives (`./registry`) and the definitions
// written in alchemy itself. A port of rs/src/stdlib/mod.rs.
//
// The package embeds the canonical `stdlib/*.alc` files as one generated
// module, `./sources` (an npm package holds nothing above `ts/`);
// `npm run embed` writes it, and test/shared-sources.test.ts holds it to
// the files. `table.alc` is the metadata-first table transducer and
// `csv.alc` the always-quoted CSV renderer. Both are parsed, desugared and
// resolved once, on first use, and their names are usable from any
// program; a program's own `def` of the same name shadows the library's
// for that program, never for the library itself, whose definitions
// resolve in their own scope.

import { Fail } from '@tabnas/transduce'

import { SourceSpan, Sources, symbolOf } from '../ast'
import { checkStdlibFile } from '../check'
import { desugarProgram } from '../desugar'
import { parseFile } from '../grammar'
import { Def, NameKind, Resolved, resolve } from '../resolve'
import { native } from './registry'
import { SOURCES } from './sources'

export { SOURCES }

// The library, loaded: each file's resolved definitions, and all of them
// by name.
export class Stdlib {
  constructor(
    // One entry per source file, in SOURCES order.
    readonly files: Resolved[],
    // Every definition, by name, in file order.
    readonly defs: Map<string, Def>,
  ) {}

  get(name: string): Def | undefined {
    return this.defs.get(name)
  }

  // The names the library defines, in file order.
  names(): string[] {
    return [...this.defs.keys()]
  }
}

// The kind of a native, as the resolver classifies names.
export function nativeKind(name: string): NameKind | undefined {
  return native(name)?.kind
}

// Parse, desugar, resolve and check the embedded sources. Each file
// resolves against the natives and the other files' definitions, so a
// definition may use one from another file but no file may redefine
// another's; each definition is checked against the signature
// `stdlibSignature` declares for it.
export function load(): Stdlib {
  // Every definition's name first, so a reference across files resolves
  // whatever the file order.
  const parsed: Array<[string, string, ReturnType<typeof desugarProgram>]> = []
  const names = new Set<string>()
  for (const [file, src] of SOURCES) {
    const forms = desugarProgram(parseFile(src, file), src)
    for (const form of forms) {
      if ('list' === form.kind) {
        const name = symbolOf(form.items[1])
        if (undefined !== name) names.add(name)
      }
    }
    parsed.push([file, src, forms])
  }
  const files: Resolved[] = []
  const defs = new Map<string, Def>()
  for (const [file, src, forms] of parsed) {
    const outer = (name: string): NameKind | undefined => (names.has(name) ? 'value' : nativeKind(name))
    const resolved = resolve(forms, Sources.one(file, src), outer)
    checkStdlibFile(resolved, src)
    for (const [name, def] of resolved.defs) {
      if (defs.has(name)) {
        throw new Fail('DSL_TYPE_ERROR', `duplicate_def: ${name} is defined in two standard library files`)
      }
      defs.set(name, def)
    }
    files.push(resolved)
  }
  return new Stdlib(files, defs)
}

let LIB: Stdlib | undefined

// The library, loaded once. The embedded text is part of this package, so
// a text that does not load is a defect of the build, not of any program.
export function stdlib(): Stdlib {
  if (undefined === LIB) {
    try {
      LIB = load()
    } catch (fail) {
      throw new Error(`the embedded standard library does not load: ${fail}`)
    }
  }
  return LIB
}

// The source text of an embedded file, by the name its spans carry.
export function source(file: string): string | undefined {
  return SOURCES.find(([name]) => name === file)?.[1]
}

// The embedded file a span belongs to, as its name and its text, by
// identity: every span of one file shares the file object the reader
// made, and a program's spans never share it, so a program that happens
// to be named `stdlib/table.alc` is not mistaken for the library file.
export function fileOf(sp: SourceSpan): readonly [string, string] | undefined {
  const lib = stdlib()
  for (let i = 0; i < lib.files.length; i++) {
    const first = lib.files[i].defs.values().next().value
    if (undefined !== first && first.span.file === sp.file) return SOURCES[i]
  }
  return undefined
}

// What a program sees outside itself: the library's definitions and the
// natives.
export function outer(name: string): NameKind | undefined {
  if (undefined !== stdlib().get(name)) return 'value'
  return nativeKind(name)
}
