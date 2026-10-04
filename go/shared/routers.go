// Copyright (c) 2026 tabnas, MIT License

package shared

// Routers is what alchemy's runtime builds on the transducer side: one
// method per stage it constructs, each tabnas-transduce's constructor of
// that stage, with its parameters. Transduce's Routers answers the
// implementation, and a host hands it to alchemy, which depends on no
// implementation.
type Routers interface {
	// Router recognizes every capture in one pass and delivers the
	// completed matches to downstream (transduce's NewRouter); two specs
	// that may overlap are refused with CAPTURE_OVERLAP_UNSUPPORTED
	// unless both observe.
	Router(specs []CaptureSpec, limits Limits, duplicates Duplicates, metrics *Metrics, downstream RouteSink) (Sink, *Fail)
	// TableFromJSON is the metadata-first table transducer over one
	// document's events, its TableRows/1 going to sink (transduce's
	// NewTableFromJSON).
	TableFromJSON(binding TableBinding, limits Limits, duplicates Duplicates, metrics *Metrics, sink TableSink) (Sink, *Fail)
	// ScanEmit is the scan-emit operator over an initial state and its
	// functions (transduce's NewScanEmit), over values of any type, since
	// a method takes no type parameters.
	ScanEmit(initial any, step func(any, any) (Transition[any, any], *Fail), finish func(any) ([]any, *Fail), out func(any) (Flow, *Fail)) ScanEmitter
	// Guarded wraps inner with the three limits a source owns, the abort
	// flag and the source metrics (transduce's NewGuarded); a nil abort or
	// metrics gets a fresh one.
	Guarded(inner Sink, limits Limits, abort *AbortFlag, metrics *Metrics) Sink
}

// ScanEmitter is the operator Routers.ScanEmit answers (transduce's
// ScanEmit): feed it items with Item, then call Finish exactly once when
// the input completed successfully, never after a failure or a
// cancellation.
type ScanEmitter interface {
	Item(item any) (Flow, *Fail)
	Finish() (Flow, *Fail)
}
