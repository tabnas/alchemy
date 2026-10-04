// Copyright (c) 2026 tabnas, MIT License

package shared

// CaptureMode is what a capture keeps of its match.
type CaptureMode uint8

const (
	// Materialize builds the value and delivers it whole.
	Materialize CaptureMode = iota
	// Observe delivers only that the value occurred, and where, when it
	// ends.
	Observe
)

// Budget is the byte budget one materialized capture may not exceed,
// named after the Limits field the failure reports.
type Budget struct {
	Bytes int
	Name  string
}

// CaptureSpec is one capture: a tag for the consumer, the selector to
// match, the mode, and for a Materialize capture an optional budget (nil
// takes the router's max_capture_bytes; a stage with a more specific
// limit, a table's rows under max_record_bytes, sets it so the failure
// names that).
type CaptureSpec struct {
	Tag      string
	Selector Selector
	Mode     CaptureMode
	Budget   *Budget
}

// MaterializeSpec is a Materialize capture.
func MaterializeSpec(tag string, selector Selector) CaptureSpec {
	return CaptureSpec{Tag: tag, Selector: selector, Mode: Materialize}
}

// ObserveSpec is an Observe capture.
func ObserveSpec(tag string, selector Selector) CaptureSpec {
	return CaptureSpec{Tag: tag, Selector: selector, Mode: Observe}
}

// WithBudget is the spec with a budget of its own.
func (c CaptureSpec) WithBudget(bytes int, name string) CaptureSpec {
	c.Budget = &Budget{Bytes: bytes, Name: name}
	return c
}

// Selected is one completed match. ID is the spec's position in the
// router's list; Value is the materialized value, nil for Observe.
type Selected struct {
	ID    CaptureID
	Tag   string
	Path  Path
	Value *Datum
}

// RouteSink consumes a router's matches. Selected delivers each
// completed match in source order; End is called exactly once, when the
// document's End arrives, after the last match.
//
// A RouteSink may also implement RouteBeginner, to hear of a capture's
// value as it begins.
type RouteSink interface {
	Selected(s Selected) (Flow, *Fail)
	End() (Flow, *Fail)
}

// RouteBeginner is the optional half of a RouteSink: Began is called
// when a capture's value begins, before anything of it is retained, so a
// consumer that knows the value is out of order can refuse it there at
// no cost; the router adds the value's path to a failure that has none.
type RouteBeginner interface {
	Began(id CaptureID, tag string) *Fail
}
