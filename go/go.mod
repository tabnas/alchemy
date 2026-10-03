module github.com/tabnas/alchemy/go

go 1.24.7

// The engine, the JSON grammar the grammar document and a run's input are
// read with, and the protocols a program is lowered onto: transduce's and
// render's Go ports, not yet released, resolved through a go.work over
// the sibling checkouts until they are.
require (
	github.com/tabnas/json/go v0.5.12
	github.com/tabnas/parser/go v0.12.9
	github.com/tabnas/render/go v0.1.1
	github.com/tabnas/transduce/go v0.1.1
)

// The grammars the tests read documents with, the debug model, and the
// shared fixture runner.
require (
	github.com/tabnas/csv/go v0.6.1
	github.com/tabnas/debug/go v0.3.9
	github.com/tabnas/ini/go v0.5.13
	github.com/tabnas/json5/go v0.5.10
	github.com/tabnas/jsonc/go v0.5.9
	github.com/tabnas/jsonic/go v0.7.3
	github.com/tabnas/jsonl/go v0.1.11
	github.com/tabnas/markdown/go v0.7.7
	github.com/tabnas/support/go v0.3.5
	github.com/tabnas/toml/go v0.5.10
	github.com/tabnas/xml/go v0.7.11
	github.com/tabnas/yaml/go v0.5.17
	github.com/tabnas/zon/go v0.5.11
)

require github.com/tabnas/hoover/go v0.3.10 // indirect
