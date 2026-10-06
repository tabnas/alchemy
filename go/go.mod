module github.com/tabnas/alchemy/go

go 1.24.7

// The engine, the JSON grammar the grammar document and a run's input are
// read with, and the implementations the tests run programs on:
// transduce's Routers and render's Renderers (the package itself depends on
// neither; a host hands them in), which both carry from v0.2.0.
require (
	github.com/tabnas/json/go v0.5.14
	github.com/tabnas/parser/go v0.12.10
	github.com/tabnas/render/go v0.2.0
	github.com/tabnas/transduce/go v0.2.0
)

// The grammars the tests read documents with, the debug model, and the
// shared fixture runner.
require (
	github.com/tabnas/csv/go v0.6.3
	github.com/tabnas/debug/go v0.3.11
	github.com/tabnas/ini/go v0.5.15
	github.com/tabnas/json5/go v0.5.12
	github.com/tabnas/jsonc/go v0.5.11
	github.com/tabnas/jsonic/go v0.7.5
	github.com/tabnas/jsonl/go v0.1.13
	github.com/tabnas/markdown/go v0.7.9
	github.com/tabnas/support/go v0.3.7
	github.com/tabnas/toml/go v0.5.13
	github.com/tabnas/xml/go v0.7.13
	github.com/tabnas/yaml/go v0.5.20
	github.com/tabnas/zon/go v0.5.13
)

require github.com/tabnas/hoover/go v0.3.12 // indirect
