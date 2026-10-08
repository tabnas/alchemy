// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// stages_test.go: the stages these tests hand Compile and the runtimes
// they evaluate on. Parsing, desugaring, resolving, checking, explaining
// and building a plan ask nothing of the routers and renderers a host
// passes in, so every test here passes inert ones, whose every method
// panics: a test that reached one would be running a program, and those
// tests are alchemy-cli's (go/e2e), the composition root that builds this
// package's programs on transduce's Routers and render's Renderers. This
// package depends on neither.

import (
	"io"

	"github.com/tabnas/alchemy/go/shared"
)

// noStages is what an inert stage says when it is called.
const noStages = "compile-only test: no stages"

// inertRouters is a shared.Routers whose every method panics.
type inertRouters struct{}

func (inertRouters) Router([]shared.CaptureSpec, shared.Limits, shared.Duplicates, *shared.Metrics, shared.RouteSink) (shared.Sink, *Fail) {
	panic(noStages)
}

func (inertRouters) TableFromJSON(shared.TableBinding, shared.Limits, shared.Duplicates, *shared.Metrics, shared.TableSink) (shared.Sink, *Fail) {
	panic(noStages)
}

func (inertRouters) ScanEmit(any, func(any, any) (shared.Transition[any, any], *Fail), func(any) ([]any, *Fail), func(any) (shared.Flow, *Fail)) shared.ScanEmitter {
	panic(noStages)
}

func (inertRouters) Guarded(shared.Sink, shared.Limits, *shared.AbortFlag, *shared.Metrics) shared.Sink {
	panic(noStages)
}

// inertRenderers is a shared.Renderers whose every method panics.
type inertRenderers struct{}

func (inertRenderers) JSON(shared.TextOut, shared.JSONOptions) shared.Sink { panic(noStages) }

func (inertRenderers) CSV(shared.TextOut, shared.CSVOptions) (shared.TableSink, *Fail) {
	panic(noStages)
}

func (inertRenderers) RecordsToJSON(shared.Sink) shared.TableSink { panic(noStages) }

func (inertRenderers) Join(shared.TextOut, string) shared.JoinOut { panic(noStages) }

func (inertRenderers) ReplaceText(shared.TextOut, string, string) shared.TextOut { panic(noStages) }

func (inertRenderers) WriteOut(io.Writer, shared.Limits, *shared.Metrics) shared.TextOut {
	panic(noStages)
}

func (inertRenderers) WriteValue(float64) string { panic(noStages) }

// routers and renderers are the stages the tests compile with: inert.
var (
	routers   shared.Routers   = inertRouters{}
	renderers shared.Renderers = inertRenderers{}
)
