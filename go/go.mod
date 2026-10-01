module github.com/tabnas/alchemy/go

go 1.24.7

// The engine, the JSON grammar the grammar document and a run's input are
// read with, and the protocols a program is lowered onto: transduce's and
// render's Go ports, not yet released, resolved through a go.work over
// the sibling checkouts until they are.
require (
	github.com/tabnas/json/go v0.5.11
	github.com/tabnas/parser/go v0.12.7
	github.com/tabnas/render/go v0.1.0
	github.com/tabnas/transduce/go v0.1.0
)

// The grammars the tests read documents with, the debug model, and the
// shared fixture runner.
require (
	github.com/tabnas/csv/go v0.5.11
	github.com/tabnas/debug/go v0.3.9
	github.com/tabnas/jsonl/go v0.1.10
	github.com/tabnas/support/go v0.3.5
	github.com/tabnas/yaml/go v0.5.15
)

require github.com/tabnas/jsonic/go v0.7.2 // indirect
