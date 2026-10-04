// Copyright (c) 2026 tabnas, MIT License

package shared

import (
	"io"
)

// Renderers is what alchemy's runtime builds and calls on the renderer
// side: one method per renderer, text stage and number function it uses,
// each tabnas-render's constructor or function, with its parameters.
// Render's Renderers answers the implementation, and a host hands it to
// alchemy, which depends on no implementation.
type Renderers interface {
	// JSON renders JsonEvents/1 to out as JSON text (render's
	// NewJSONRenderer).
	JSON(out TextOut, options JSONOptions) Sink
	// CSV renders TableRows/1 to out as CSV, or refuses a delimiter no CSV
	// reader could take with TARGET_VALUE_UNREPRESENTABLE (render's
	// NewCSVRenderer).
	CSV(out TextOut, options CSVOptions) (TableSink, *Fail)
	// RecordsToJSON turns TableRows/1 into JsonEvents/1 for sink, an array
	// of objects keyed by label, Missing cells skipped (render's
	// NewRecordsToJSON).
	RecordsToJSON(sink Sink) TableSink
	// Join writes separator to out between logical items (render's
	// NewJoin).
	Join(out TextOut, separator string) JoinOut
	// ReplaceText replaces every occurrence of from with to, across
	// fragment boundaries (render's NewReplaceText).
	ReplaceText(out TextOut, from, to string) TextOut
	// WriteOut coalesces fragments and writes them to w, enforcing
	// limits.MaxOutputBytes and counting OutputBytes in metrics (render's
	// NewWriteOut, with WithLimits and WithMetrics).
	WriteOut(w io.Writer, limits Limits, metrics *Metrics) TextOut
	// WriteValue is the text a renderer writes for a finite number with
	// no lexeme, and "" for a non-finite one (render's FormatValue).
	WriteValue(value float64) string
}

// JoinOut is the output Renderers.Join answers (render's Join): a TextOut
// whose logical items ItemStart and ItemEnd mark, with the separator
// written before every item but the first.
type JoinOut interface {
	TextOut
	ItemStart() *Fail
	ItemEnd() *Fail
}
