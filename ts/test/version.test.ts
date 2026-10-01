/* Copyright (c) 2026 tabnas, MIT License */

// The exported VERSION must equal package.json "version", which is the
// crate's (rs/Cargo.toml): the runtimes ship as one version.

import { describe, it } from 'node:test'
import assert from 'node:assert'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { VERSION } from '../dist/alchemy'

const pkg = JSON.parse(readFileSync(join(__dirname, '..', 'package.json'), 'utf8'))

describe('version', () => {
  it('VERSION matches package.json', () => {
    assert.equal(
      VERSION,
      pkg.version,
      `VERSION drift: ${pkg.name} exports ${VERSION} but package.json is ${pkg.version}`,
    )
  })

  it('VERSION is the crate version', () => {
    const cargo = readFileSync(join(__dirname, '..', '..', 'rs', 'Cargo.toml'), 'utf8')
    const crate = /^version = "([^"]+)"/m.exec(cargo)?.[1]
    assert.equal(VERSION, crate)
    assert.equal(VERSION, '0.1.1')
  })
})
