// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

import (
	"io"

	"github.com/tabnas/alchemy/go/shared"
)

// program.go: the API a host embeds (rs/src/program.rs): Compile a program
// (or CompileSources, several files linked into one), ask it what it
// produces, and take a Sink to push the source's events into.
//
// A program runs on stages it does not implement: the host hands Compile
// transduce's Routers and render's Renderers (shared.Routers and
// shared.Renderers, the interfaces this module declares and those two
// implement), and every runtime the program makes builds its stages
// through them.
//
// The host pushes transduce Events into the sink and one End; the sink
// writes the output as it goes and flushes it at End. A failure comes back
// from the event call that found it, as a transduce *Fail whose
// CommittedOutput says whether bytes had already reached the writer.

// Program is a compiled program: parsed, desugared, resolved and checked,
// with its plan built. A Program is never changed after it is built: the
// With methods answer a new one, so one may be shared, and each Sink it
// makes runs on its own runtime.
type Program struct {
	resolved *Resolved
	sources  *Sources
	// output is what the checker decided from export's type.
	output     Output
	native     bool
	duplicates shared.Duplicates
	// abort is the host's cancellation, handed to every sink's runtime.
	abort *shared.AbortFlag
	// routers and renderers are the stages the program runs on, the
	// host's, handed to every runtime it makes.
	routers   shared.Routers
	renderers shared.Renderers
	// result is export applied to the input plan under the flags above.
	result Val
}

// Compile parses, desugars, resolves and checks src, named file in
// diagnostics, and builds its plan: export applied to the input's plan,
// which evaluates everything a stream does not defer, so a `fail` or a
// `no_match` on that path is reported here, with its own code. Building
// the plan is bounded by MaxPlanSteps and MaxEvalDepth.
//
// routers and renderers are the stages the program runs on: transduce's
// Routers and render's Renderers. The plan is built with them (a number's
// text is the renderer's), and every sink the program makes runs on them.
func Compile(src, file string, routers shared.Routers, renderers shared.Renderers) (*Program, *Fail) {
	return CompileSources([]Source{{File: file, Text: src}}, routers, renderers)
}

// CompileSources compiles a program from several sources linked into one
// namespace: a definition in any of them is in scope in all, export is
// defined in one, and a name defined twice, in one file or across two, is
// `duplicate_def`. Each source is named by its file, and the names are
// distinct (`duplicate_file` otherwise). A failure in a program of several
// sources carries the file its position is in, Fail.File, displayed
// `(file:row:col)`, beside the row and column; a program of one source is
// Compile, which positions without a file. This is how a host links a
// format's parts (its lift and render: libraries of definitions with no
// export) with the program that calls them.
//
// A source whose export is not the program's is linked with
// Source.ExportAs: its export is defined under that name instead, so that
// the program's export can call it. This is how a host composes a whole
// program's output into a format's render (`def export [input]
// (yaml-render (program-export input))`, with the user's program linked
// as `program-export`), in one plan under one set of limits.
//
// routers and renderers are the stages the program runs on, as for Compile.
func CompileSources(sources []Source, routers shared.Routers, renderers shared.Renderers) (*Program, *Fail) {
	a, f := AnalyzeSources(sources)
	if f != nil {
		return nil, f
	}
	return buildProgram(a.Resolved, a.Sources, a.Checked.Output, true, shared.Reject, shared.NewAbortFlag(), routers, renderers)
}

func buildProgram(resolved *Resolved, sources *Sources, output Output, native bool, duplicates shared.Duplicates, abort *shared.AbortFlag, routers shared.Routers, renderers shared.Renderers) (*Program, *Fail) {
	result, f := NewRuntime(resolved, sources).
		WithNative(native).
		WithDuplicates(duplicates).
		WithFuel(MaxPlanSteps).
		WithAbort(abort).
		WithRouters(routers).
		WithRenderers(renderers).
		Export()
	if f != nil {
		return nil, f
	}
	return &Program{
		resolved:   resolved,
		sources:    sources,
		output:     output,
		native:     native,
		duplicates: duplicates,
		abort:      abort,
		routers:    routers,
		renderers:  renderers,
		result:     result,
	}, nil
}

// WithNative is the same program with the standard compositions run
// through the library's own definitions (false) or natively (true, the
// default). The differential test runs both.
func (p *Program) WithNative(native bool) (*Program, *Fail) {
	return buildProgram(p.resolved, p.sources, p.output, native, p.duplicates, p.abort, p.routers, p.renderers)
}

// WithDuplicates is the same program with another policy for repeated
// member names in captured values; the default rejects them.
func (p *Program) WithDuplicates(duplicates shared.Duplicates) (*Program, *Fail) {
	return buildProgram(p.resolved, p.sources, p.output, p.native, duplicates, p.abort, p.routers, p.renderers)
}

// WithAbort is the same program with the host's cancellation: every sink
// it makes reads the flag as the program's functions run, so a timeout
// stops a long computation on one item with ABORTED rather than waiting
// for the item to finish. The source takes the same flag
// (ParserSource.Abort) to stop between events.
func (p *Program) WithAbort(abort *shared.AbortFlag) *Program {
	if abort == nil {
		abort = shared.NewAbortFlag()
	}
	q := *p
	q.abort = abort
	return &q
}

// Native reports whether the standard compositions run natively.
func (p *Program) Native() bool { return p.native }

// Duplicates is the policy for a member name repeated in a captured scope.
func (p *Program) Duplicates() shared.Duplicates { return p.duplicates }

// File is the file name the program was compiled under: the first, of
// several.
func (p *Program) File() string { return p.resolved.File }

// Resolved is the program's resolved definitions.
func (p *Program) Resolved() *Resolved { return p.resolved }

// Result is the plan export answered: a text or a stream.
func (p *Program) Result() Val { return p.result }

// Output is what the program produces, and so what the host renders:
// decided by the checker from export's type.
func (p *Program) Output() Output { return p.output }

// PlanOutput is the protocol the built plan produces; the checker's Output
// agrees with it, and a test holds the two together.
func (p *Program) PlanOutput() Output {
	if s, ok := p.result.(StreamVal); ok {
		if s.Plan.Protocol() == ProtocolJSONEvents {
			return OutputJsonEvents
		}
		return OutputTableRows
	}
	return OutputText
}

func (p *Program) plan() *Plan {
	switch x := p.result.(type) {
	case StreamVal:
		return x.Plan
	case TextVal:
		return x.Plan
	}
	return nil
}

// RowSelector is the selector under which the source is read one row at a
// time, when the plan makes one known: the table binding's :rows, a
// select's selector, or the one multi-location capture of a route. A host
// that streams a verified grammar prunes the parse under it.
func (p *Program) RowSelector() (shared.Selector, bool) {
	plan := p.plan()
	if plan == nil {
		return shared.Selector{}, false
	}
	root := rootStage(plan)
	switch root.Kind {
	case PlanTableFromJSON:
		if rows, ok := Field(root.Binding, "rows"); ok {
			if s, ok := rows.(*SelectorVal); ok {
				return s.Selector, true
			}
		}
	case PlanSelect:
		return root.Selector, true
	case PlanRoute:
		var multi []shared.Selector
		for _, spec := range root.Specs {
			if spec.Selector.IsMulti() {
				multi = append(multi, spec.Selector)
			}
		}
		switch {
		case len(multi) == 1:
			return multi[0], true
		case len(multi) == 0 && len(root.Specs) == 1:
			return root.Specs[0].Selector, true
		}
	}
	return shared.Selector{}, false
}

// rootStage is the stage that reads the input directly: the plan whose
// source is the input, or the input itself.
func rootStage(plan *Plan) *Plan {
	here := plan
	for {
		var next *Plan
		switch here.Kind {
		case PlanInput:
			return here
		case PlanRoute, PlanSelect, PlanEvents, PlanScanEmit, PlanMap, PlanFilter,
			PlanTableFromJSON, PlanRecords, PlanCsvTable, PlanCsv, PlanJSON:
			if here.Source.Kind == PlanInput {
				return here
			}
			next = here.Source
		case PlanConcatMap, PlanJoin:
			if !here.Seq.IsStream() {
				return here
			}
			next = here.Seq.Stream
		case PlanConcat:
			if here.Live < 0 {
				return here
			}
			switch x := here.Items[here.Live].(type) {
			case TextVal:
				next = x.Plan
			case StreamVal:
				next = x.Plan
			default:
				return here
			}
		case PlanReplace:
			inner, ok := here.Text.(TextVal)
			if !ok {
				return here
			}
			next = inner.Plan
		default:
			return here
		}
		here = next
	}
}

// stagesOf are the stages of a plan from the input outward.
func stagesOf(plan *Plan) []*Plan {
	var out []*Plan
	here := plan
	for here != nil {
		out = append(out, here)
		var next *Plan
		switch here.Kind {
		case PlanInput, PlanLit:
		case PlanRoute, PlanSelect, PlanEvents, PlanScanEmit, PlanMap, PlanFilter,
			PlanTableFromJSON, PlanRecords, PlanCsvTable, PlanCsv, PlanJSON:
			next = here.Source
		case PlanConcatMap, PlanJoin:
			next = here.Seq.Stream
		case PlanConcat:
			if here.Live >= 0 {
				switch x := here.Items[here.Live].(type) {
				case TextVal:
					next = x.Plan
				case StreamVal:
					next = x.Plan
				}
			}
		case PlanReplace:
			if inner, ok := here.Text.(TextVal); ok {
				next = inner.Plan
			}
		}
		here = next
	}
	for i, j := 0, len(out)-1; i < j; i, j = i+1, j-1 {
		out[i], out[j] = out[j], out[i]
	}
	return out
}

// isLibraryTable is whether a scan is the standard library's table, the
// twin of table-from-json: the library's table-step, bare or partially
// applied, from the state before the metadata (no-schema). Every path
// through it that binds columns builds their schema in the same step,
// which holds them to max_columns; a scan seeded with columns already
// bound builds none, so the cap is not claimed for it.
func isLibraryTable(init Val, step Fn) bool {
	var isTableStep func(f Fn) bool
	isTableStep = func(f Fn) bool {
		switch x := f.(type) {
		case *Partial:
			return isTableStep(x.F)
		case *Closure:
			return x.Scope == ScopeStdlib && x.Name == "table-step"
		}
		return false
	}
	t, ok := init.(*TaggedVal)
	return ok && t.Tag == "no-schema" && len(t.Fields) == 0 && isTableStep(step)
}

// stageOf is one stage of a plan as the summary reads it.
func stageOf(p *Plan) Stage {
	kinds := map[PlanKind]StageKind{
		PlanInput: StageInput, PlanLit: StageLit, PlanRoute: StageRoute, PlanSelect: StageSelect,
		PlanEvents: StageEvents, PlanScanEmit: StageScanEmit, PlanMap: StageMap, PlanFilter: StageFilter,
		PlanTableFromJSON: StageTableFromJSON, PlanRecords: StageRecords, PlanCsvTable: StageCsvTable,
		PlanCsv: StageCsv, PlanJSON: StageJSON, PlanConcatMap: StageConcatMap, PlanJoin: StageJoin,
		PlanConcat: StageConcat, PlanReplace: StageReplace,
	}
	s := Stage{Kind: kinds[p.Kind]}
	switch p.Kind {
	case PlanTableFromJSON:
		selector := func(key string) Selector {
			v, _ := Field(p.Binding, key)
			if sv, ok := v.(*SelectorVal); ok {
				return sv.Selector
			}
			return nil
		}
		s.Columns = selector("columns")
		s.Rows = selector("rows")
		s.Inferred = isInferred(p.Binding)
	case PlanRoute:
		for _, spec := range p.Specs {
			c := Capture{Selector: spec.Selector}
			if spec.Budget != nil {
				c.Budget = spec.Budget.Name
			}
			s.Captures = append(s.Captures, c)
		}
	case PlanSelect:
		s.Selector = p.Selector
	case PlanScanEmit:
		s.LibraryTable = isLibraryTable(p.Init, p.Step)
	case PlanCsv:
		// The fast path builds a native csv only for options that map onto
		// the renderer's dialect, and lowering builds the renderer with
		// them.
		options, ok := csvOptions(p.Options)
		if !ok {
			options = shared.DefaultCSVOptions()
		}
		s.Csv = csvProfile(options)
	}
	return s
}

// PlanView is what the report reads from the program: the output, whether
// the result reaches the input, its stages from the input outward, and
// the duplicate-member policy.
func (p *Program) PlanView() PlanView {
	plan := p.plan()
	view := PlanView{Output: p.output, Live: plan != nil && plan.IsLive(), Duplicates: p.duplicates}
	if view.Live {
		for _, stage := range stagesOf(plan) {
			view.Stages = append(view.Stages, stageOf(stage))
		}
	}
	return view
}

// Summary is the effect summary of the program's plan.
func (p *Program) Summary() *EffectSummary { return Summarize(p.resolved, p.PlanView()) }

// Explain is the plan report of spec section 15.5: the chain of calls, the
// protocols, what is retained and under which limits, the ordering
// contract, the renderer, the guarantee and its qualification.
func (p *Program) Explain() string { return p.Summary().Text() }

// ExplainJSON is the same report as one JSON object, for a host's
// `--explain`; EncodeJSON writes it.
func (p *Program) ExplainJSON() *JSONObject { return p.Summary().JSON() }

// Sink is the sink for one run: the host pushes the source's events into
// it and one End, and the output reaches out through a coalescing writer
// (the renderers' WriteOut) with limits.MaxOutputBytes enforced and
// OutputBytes counted in metrics (a fresh set when nil). render chooses how
// a table or JSON-events result is rendered (RenderCSV or RenderJSON;
// RenderDefault for the host's default); a program that renders its own
// text takes none (`render_of_text`).
func (p *Program) Sink(out io.Writer, render Renderer, limits shared.Limits, metrics *shared.Metrics) (shared.Sink, *Fail) {
	if metrics == nil {
		metrics = shared.NewMetrics()
	}
	w := p.renderers.WriteOut(out, limits, metrics)
	return p.SinkOut(w, render, limits, metrics)
}

// SinkOut is Sink over any text output: for a host that has its own writer
// stage, and for tests that read the text back.
func (p *Program) SinkOut(out shared.TextOut, render Renderer, limits shared.Limits, metrics *shared.Metrics) (shared.Sink, *Fail) {
	rt := NewRuntime(p.resolved, p.sources).
		WithNative(p.native).
		WithDuplicates(p.duplicates).
		WithLimits(limits).
		WithAbort(p.abort).
		WithRouters(p.routers).
		WithRenderers(p.renderers)
	return NewLowering(rt, limits, metrics).Sink(p.result, out, render)
}
