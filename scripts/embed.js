#!/usr/bin/env node

// The embed lives in ts/embed-grammar.js (`npm run embed` in ts/), as in
// every plugin repository; this shim keeps `make embed` (`node
// scripts/embed.js`) working until the Makefile runs it there directly.
// Arguments (`--check`) pass through.

require('../ts/embed-grammar.js')
