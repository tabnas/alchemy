module github.com/tabnas/alchemy/go

go 1.24.7

// The engine, the JSON grammar the grammar document and a run's input are
// read with, and the implementations the tests run programs on:
// transduce's Routers and render's Renderers (the package itself depends on
// neither; a host hands them in), which both carry from v0.2.0.
require (
	github.com/tabnas/json/go v0.5.13
	github.com/tabnas/parser/go v0.12.9
	github.com/tabnas/render/go v0.2.0
	github.com/tabnas/transduce/go v0.2.0
)

// The grammars the tests read documents with, the debug model, and the
// shared fixture runner.
require (
	github.com/tabnas/csv/go v0.6.2
	github.com/tabnas/debug/go v0.3.10
	github.com/tabnas/ini/go v0.5.14
	github.com/tabnas/json5/go v0.5.11
	github.com/tabnas/jsonc/go v0.5.10
	github.com/tabnas/jsonic/go v0.7.4
	github.com/tabnas/jsonl/go v0.1.12
	github.com/tabnas/markdown/go v0.7.8
	github.com/tabnas/support/go v0.3.6
	github.com/tabnas/toml/go v0.5.12
	github.com/tabnas/xml/go v0.7.12
	github.com/tabnas/yaml/go v0.5.19
	github.com/tabnas/zon/go v0.5.12
)

require github.com/tabnas/hoover/go v0.3.11 // indirect
