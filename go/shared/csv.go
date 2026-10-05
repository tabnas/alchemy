// Copyright (c) 2026 tabnas, MIT License

package shared

// Newline is the record terminator.
type Newline uint8

const (
	// NewlineCRLF is RFC 4180's terminator, and the standard profile's.
	NewlineCRLF Newline = iota
	// NewlineLF is a dialect.
	NewlineLF
)

// String is the terminator's text.
func (n Newline) String() string {
	if n == NewlineLF {
		return "\n"
	}
	return "\r\n"
}

// Quoting is when a field is quoted.
type Quoting uint8

const (
	// QuotingAlways quotes every field: the standard profile.
	QuotingAlways Quoting = iota
	// QuotingMinimal quotes only a field holding the delimiter, `"`, CR
	// or LF. An empty field is then written as nothing, so the empty
	// string and an empty null text read back the same; that is the
	// dialect's trade-off, not a defect.
	QuotingMinimal
)

// CSVOptions is the CSV dialect. Start from DefaultCSVOptions: the zero
// value's delimiter is NUL, which no reader can take and NewCSVRenderer
// refuses.
type CSVOptions struct {
	// Delimiter is one character, and not `"`, CR, LF or NUL: those would
	// make the output unreadable by construction, and are refused when
	// the renderer is built.
	Delimiter rune
	Newline   Newline
	// Header writes the labels as the first record.
	Header bool
	// NullText is the text of a CellNull; empty by default.
	NullText string
	// Missing is what a CellMissing becomes: nil fails the run with
	// MISSING_VALUE (a table that promised a column and did not deliver
	// it is not silently padded), else the text it points at.
	Missing *string
	Quoting Quoting
}

// DefaultCSVOptions is the standard profile: `,`, CRLF, a header, an
// empty null text, Missing an error, every field quoted.
func DefaultCSVOptions() CSVOptions {
	return CSVOptions{Delimiter: ',', Newline: NewlineCRLF, Header: true}
}

// MissingAs is the CSVOptions.Missing that writes text for a CellMissing.
func MissingAs(text string) *string { return &text }
