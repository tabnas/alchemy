// Copyright (c) 2026 tabnas, MIT License

package shared

import (
	"encoding/json"
	"math"
	"sync/atomic"
)

// Limits are per-run limits. Limits are part of a plan, not advice: a
// stage that would exceed one fails with RESOURCE_LIMIT_EXCEEDED naming
// the field (as its Rust name, max_record_bytes), rather than switching
// to a slower or larger algorithm. Sizes are payload bytes (UTF-8
// lengths of keys and scalars, and for a retained value their sum plus
// NodeBytes per node), a stable and portable measure, not a heap one.
type Limits struct {
	// MaxDepth is the container nesting the source may reach.
	MaxDepth int
	// MaxKeyBytes is the bytes in one object key.
	MaxKeyBytes int
	// MaxScalarBytes is the bytes in one scalar (a string's text, a number's lexeme).
	MaxScalarBytes int
	// MaxMetadataBytes is the bytes retained for metadata (the column descriptors).
	MaxMetadataBytes int
	// MaxColumns is the columns a schema may declare.
	MaxColumns int
	// MaxRecordBytes is the bytes retained for one row before projection.
	MaxRecordBytes int
	// MaxCaptureBytes is the bytes retained for any one materialized capture.
	MaxCaptureBytes int
	// MaxOutputBytes is the bytes the run may write; nil for no limit.
	MaxOutputBytes *uint64
}

// DefaultLimits is 256 levels, 64 KiB keys, 16 MiB scalars and metadata,
// 10 000 columns, 64 MiB records and captures, and no output limit.
func DefaultLimits() Limits {
	return Limits{
		MaxDepth:         256,
		MaxKeyBytes:      64 * 1024,
		MaxScalarBytes:   16 * 1024 * 1024,
		MaxMetadataBytes: 16 * 1024 * 1024,
		MaxColumns:       10_000,
		MaxRecordBytes:   64 * 1024 * 1024,
		MaxCaptureBytes:  64 * 1024 * 1024,
	}
}

// UnlimitedLimits is no limit on anything that can be unlimited, and the
// largest values otherwise. For tests and trusted, measured inputs only.
func UnlimitedLimits() Limits {
	return Limits{
		MaxDepth:         math.MaxInt,
		MaxKeyBytes:      math.MaxInt,
		MaxScalarBytes:   math.MaxInt,
		MaxMetadataBytes: math.MaxInt,
		MaxColumns:       math.MaxInt,
		MaxRecordBytes:   math.MaxInt,
		MaxCaptureBytes:  math.MaxInt,
	}
}

// NodeBytes is the fixed allowance counted for every retained node (a
// container, a member, an element) on top of its payload bytes.
const NodeBytes = 16

// Metrics are the counters and high-water marks a run reports, shared
// between stages through a pointer; every update is one atomic.
type Metrics struct {
	// Events is the source events seen.
	Events atomic.Uint64
	// Keys is the object keys seen.
	Keys atomic.Uint64
	// Scalars is the scalars seen.
	Scalars atomic.Uint64
	// Rows is the rows delivered to the table protocol.
	Rows atomic.Uint64
	// CapturedBytes is the bytes currently held by materialized captures.
	CapturedBytes atomic.Uint64
	// CapturedBytesHigh is the most bytes ever held by captures at once.
	CapturedBytesHigh atomic.Uint64
	// RetainedBytesHigh is the most bytes ever retained at once across
	// every retaining stage.
	RetainedBytesHigh atomic.Uint64
	// OutputBytes is the bytes written to the output.
	OutputBytes atomic.Uint64
}

// NewMetrics is a fresh set of metrics.
func NewMetrics() *Metrics { return &Metrics{} }

// raise lifts a high-water mark to value if it is higher.
func raise(high *atomic.Uint64, value uint64) {
	for {
		old := high.Load()
		if value <= old || high.CompareAndSwap(old, value) {
			return
		}
	}
}

// Capture accounts bytes as captured now, and raises the high-water marks.
func (m *Metrics) Capture(bytes uint64) {
	now := m.CapturedBytes.Add(bytes)
	raise(&m.CapturedBytesHigh, now)
	raise(&m.RetainedBytesHigh, now)
}

// Release gives back bytes captured earlier.
func (m *Metrics) Release(bytes uint64) {
	m.CapturedBytes.Add(^(bytes - 1))
}

// MarshalJSON writes the metrics as one JSON object, a field per counter.
func (m *Metrics) MarshalJSON() ([]byte, error) {
	return json.Marshal(struct {
		Events            uint64 `json:"events"`
		Keys              uint64 `json:"keys"`
		Scalars           uint64 `json:"scalars"`
		Rows              uint64 `json:"rows"`
		CapturedBytes     uint64 `json:"captured_bytes"`
		CapturedBytesHigh uint64 `json:"captured_bytes_high"`
		RetainedBytesHigh uint64 `json:"retained_bytes_high"`
		OutputBytes       uint64 `json:"output_bytes"`
	}{
		m.Events.Load(), m.Keys.Load(), m.Scalars.Load(), m.Rows.Load(),
		m.CapturedBytes.Load(), m.CapturedBytesHigh.Load(),
		m.RetainedBytesHigh.Load(), m.OutputBytes.Load(),
	})
}

// AbortFlag is a cancellation flag shared by the caller, the source and
// the stages, by pointer. The source polls it between parse steps
// through the engine's parse budget and stops with ABORTED; long loops
// in stages poll it too.
type AbortFlag struct {
	v atomic.Bool
}

// NewAbortFlag is a flag not yet set.
func NewAbortFlag() *AbortFlag { return &AbortFlag{} }

// Abort sets the flag.
func (a *AbortFlag) Abort() { a.v.Store(true) }

// IsAborted reports whether the flag is set.
func (a *AbortFlag) IsAborted() bool { return a.v.Load() }
