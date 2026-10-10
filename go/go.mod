module github.com/tabnas/alchemy/go

go 1.24.7

// The engine, and the JSON grammar the grammar document is read with. The
// package depends on no stage: a host hands Compile its Routers and
// Renderers (transduce's and render's), and the tests that run programs on
// them are alchemy-cli's (github.com/tabnas/alchemy-cli/go/e2e).
require (
	github.com/tabnas/json/go v0.5.17
	github.com/tabnas/parser/go v0.12.11
)

// The debug model and the shared fixture runner, for the tests.
require (
	github.com/tabnas/debug/go v0.3.13
	github.com/tabnas/support/go v0.3.9
)
