// Copyright (c) 2026 tabnas, MIT License

package shared

import (
	"fmt"
	"math"
	"strconv"
	"strings"
)

// CellKind names one kind of projected value.
type CellKind uint8

const (
	CellNull CellKind = iota
	CellBool
	CellNumber
	CellString
	// CellMissing: the source had no value at the column's path. Not
	// null, not the empty string, not zero: a policy maps or rejects it
	// later.
	CellMissing
)

// Cell is one projected value of TableRows/1: Bool for CellBool, Value
// and, when HasLexeme is set, Lexeme for CellNumber (as on Event:
// HasLexeme false is no lexeme, and an empty Lexeme with it set is the
// empty one), Text for CellString.
type Cell struct {
	Kind      CellKind
	Bool      bool
	HasLexeme bool
	Value     float64
	Lexeme    string
	Text      string
}

// CellFromDatum is the cell for a retained value. A container is not a
// cell; it is written as compact JSON text, which is the lossy but
// unambiguous choice the standard binding makes and documents.
func CellFromDatum(d *Datum) Cell {
	switch d.Kind {
	case DatumBool:
		return Cell{Kind: CellBool, Bool: d.Bool}
	case DatumNumber:
		return Cell{Kind: CellNumber, HasLexeme: d.HasLexeme, Value: d.Value, Lexeme: d.Lexeme}
	case DatumString:
		return Cell{Kind: CellString, Text: d.Text}
	case DatumArray, DatumObject:
		return Cell{Kind: CellString, Text: d.String()}
	}
	return Cell{Kind: CellNull}
}

// IsMissing reports whether the cell is CellMissing.
func (c Cell) IsMissing() bool { return c.Kind == CellMissing }

// ByteSize is the bytes this cell retains, on the same basis as
// Datum.ByteSize.
func (c Cell) ByteSize() int {
	switch c.Kind {
	case CellNumber:
		if !c.HasLexeme {
			return NodeBytes + 8
		}
		return NodeBytes + len(c.Lexeme)
	case CellString:
		return NodeBytes + len(c.Text)
	}
	return NodeBytes
}

// Equal reports whether two cells are the same value, lexeme included.
func (c Cell) Equal(o Cell) bool {
	if c.Kind != o.Kind {
		return false
	}
	switch c.Kind {
	case CellBool:
		return c.Bool == o.Bool
	case CellNumber:
		return math.Float64bits(c.Value) == math.Float64bits(o.Value) &&
			c.HasLexeme == o.HasLexeme && c.Lexeme == o.Lexeme
	case CellString:
		return c.Text == o.Text
	}
	return true
}

// String is the cell as JSON text; CellMissing prints as `missing`.
func (c Cell) String() string {
	switch c.Kind {
	case CellBool:
		return strconv.FormatBool(c.Bool)
	case CellNumber:
		var b strings.Builder
		WriteJSONNumber(c.Value, c.Lexeme, c.HasLexeme, &b)
		return b.String()
	case CellString:
		var b strings.Builder
		writeJSONString(c.Text, &b)
		return b.String()
	case CellMissing:
		return "missing"
	}
	return "null"
}

// PublicColumn is what a renderer knows about a column.
type PublicColumn struct {
	Label string
}

// TableEventKind names one kind of TableRows/1 event.
type TableEventKind uint8

const (
	// TableSchema: exactly one, first.
	TableSchema TableEventKind = iota
	// TableRow: as many as there are rows, each exactly as wide as the schema.
	TableRow
	// TableEnd: exactly one, last, and only after the source validated to
	// its end.
	TableEnd
)

// TableEvent is one event of TableRows/1: the schema's Columns, or a
// row's Cells. The slices belong to the sender for the call only; a
// sink that keeps them copies them.
type TableEvent struct {
	Kind    TableEventKind
	Columns []PublicColumn
	Cells   []Cell
}

// TableSink consumes TableRows/1.
type TableSink interface {
	TableEvent(ev TableEvent) (Flow, *Fail)
}

// Table is an owned recording of a table, for tests and small results.
type Table struct {
	Columns []PublicColumn
	Rows    [][]Cell
	Ended   bool
}

// TableEvent records ev.
func (t *Table) TableEvent(ev TableEvent) (Flow, *Fail) {
	switch ev.Kind {
	case TableSchema:
		t.Columns = append([]PublicColumn(nil), ev.Columns...)
	case TableRow:
		t.Rows = append(t.Rows, append([]Cell(nil), ev.Cells...))
	case TableEnd:
		t.Ended = true
	}
	return Continue, nil
}

// MissingPolicy is what to do when a row has no value at a column's path.
type MissingPolicy uint8

const (
	// MissingCell delivers CellMissing; the renderer's policy decides.
	// The default.
	MissingCell MissingPolicy = iota
	// MissingNull delivers null.
	MissingNull
	// MissingError fails the run with MISSING_VALUE.
	MissingError
)

// BoundColumn is a column as the transducer binds it: the public label,
// and the source path projected from each row. It never crosses into a
// renderer.
type BoundColumn struct {
	Label   string
	Source  []Segment
	Missing MissingPolicy
}

// NewBoundColumn is a column with the default missing policy.
func NewBoundColumn(label string, source []Segment) BoundColumn {
	return BoundColumn{Label: label, Source: source}
}

// Public is the column as a renderer sees it.
func (c BoundColumn) Public() PublicColumn { return PublicColumn{Label: c.Label} }

// ColumnMapper maps one metadata descriptor to a bound column.
type ColumnMapper func(meta *Datum) (BoundColumn, *Fail)

// SchemaKind names where a table's columns come from.
type SchemaKind uint8

const (
	// SchemaStatic: declared by the caller; no metadata is read.
	SchemaStatic SchemaKind = iota
	// SchemaFromMetadata: selected from the source and mapped one
	// descriptor at a time; the metadata must complete before the first
	// row begins.
	SchemaFromMetadata
	// SchemaInfer: the first row's member names, in its order.
	// Data-dependent, and documented as such: a later row's extra members
	// are dropped, its absent ones are Missing.
	SchemaInfer
)

// Schema is where a table's columns come from: Static for SchemaStatic,
// Columns and Column for SchemaFromMetadata.
type Schema struct {
	Kind    SchemaKind
	Static  []BoundColumn
	Columns Selector
	Column  ColumnMapper
}

// StaticSchema is a schema the caller declares.
func StaticSchema(columns ...BoundColumn) Schema {
	return Schema{Kind: SchemaStatic, Static: columns}
}

// MetadataSchema is a schema read from the descriptors the selector
// names, each mapped by column.
func MetadataSchema(columns Selector, column ColumnMapper) Schema {
	return Schema{Kind: SchemaFromMetadata, Columns: columns, Column: column}
}

// InferSchema is a schema taken from the first row's member names.
func InferSchema() Schema { return Schema{Kind: SchemaInfer} }

// TableBinding is a table transducer's source binding: the schema, and
// the selector each of whose locations is one row.
type TableBinding struct {
	Schema Schema
	Rows   Selector
}

// ColumnFromMeta is the standard mapping from a metadata descriptor to a
// column, the spec's `column-from-meta`: a string `title` and a `path`
// of segments (strings and non-negative integers).
func ColumnFromMeta(meta *Datum) (BoundColumn, *Fail) {
	if meta.Kind != DatumObject {
		return BoundColumn{}, InputFail("a column descriptor is not an object")
	}
	title, ok := meta.Get("title")
	if !ok || title.Kind != DatumString {
		return BoundColumn{}, InputFail(`a column descriptor has no string "title"`)
	}
	label := title.Text
	path, ok := meta.Get("path")
	if !ok || path.Kind != DatumArray {
		return BoundColumn{}, InputFail(fmt.Sprintf(`column %s has no "path" array`, strconv.Quote(label)))
	}
	source := make([]Segment, 0, len(path.Items))
	for i := range path.Items {
		seg := &path.Items[i]
		switch {
		case seg.Kind == DatumString:
			source = append(source, KeySegment(seg.Text))
		case seg.Kind == DatumNumber && seg.Value >= 0 && seg.Value == math.Trunc(seg.Value) && seg.Value <= math.MaxUint32:
			source = append(source, IndexSegment(int(seg.Value)))
		default:
			return BoundColumn{}, InputFail(fmt.Sprintf(
				"column %s has a path segment that is neither a string nor a non-negative integer: %s",
				strconv.Quote(label), seg.String()))
		}
	}
	return NewBoundColumn(label, source), nil
}
