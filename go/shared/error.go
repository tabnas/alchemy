// Copyright (c) 2026 tabnas, MIT License

package shared

import (
	"bytes"
	"fmt"
	"strconv"
	"strings"
)

// Code is a stable failure code. The code is the contract: scripts and
// agents branch on it, and every renderer, transducer and host uses the
// same set. A code is never renamed, removed or repurposed; one may be
// added.
type Code uint8

// The stable failure codes, in the Rust crate's declaration order.
const (
	// CodeDSLParseError: the DSL source does not parse (reader or layout error).
	CodeDSLParseError Code = iota
	// CodeDSLTypeError: the DSL program does not type check, or names something unknown.
	CodeDSLTypeError
	// CodeStreamReused: a one-shot stream was consumed twice.
	CodeStreamReused
	// CodeStreamabilityUnknown: the plan's streamability could not be established.
	CodeStreamabilityUnknown
	// CodeInputOrderViolation: input arrived in an order the plan forbids.
	CodeInputOrderViolation
	// CodeCaptureOverlapUnsupported: two captures select overlapping scopes.
	CodeCaptureOverlapUnsupported
	// CodeMissingValue: a required value is absent and no policy maps it.
	CodeMissingValue
	// CodeDuplicateMember: an object repeats a member name under a policy that rejects it.
	CodeDuplicateMember
	// CodeInvalidNumber: a number lexeme is not valid for the target.
	CodeInvalidNumber
	// CodeProtocolOrderError: a protocol event arrived out of sequence.
	CodeProtocolOrderError
	// CodeTargetValueUnrepresentable: the target format cannot represent the value.
	CodeTargetValueUnrepresentable
	// CodeResourceLimitExceeded: a configured limit was exceeded.
	CodeResourceLimitExceeded
	// CodeInputInvalid: the input did not parse, or is invalid for the source.
	CodeInputInvalid
	// CodeOutputFailed: writing the output failed.
	CodeOutputFailed
	// CodeAborted: the run was cancelled.
	CodeAborted
)

var codeNames = [...]string{
	"DSL_PARSE_ERROR",
	"DSL_TYPE_ERROR",
	"STREAM_REUSED",
	"STREAMABILITY_UNKNOWN",
	"INPUT_ORDER_VIOLATION",
	"CAPTURE_OVERLAP_UNSUPPORTED",
	"MISSING_VALUE",
	"DUPLICATE_MEMBER",
	"INVALID_NUMBER",
	"PROTOCOL_ORDER_ERROR",
	"TARGET_VALUE_UNREPRESENTABLE",
	"RESOURCE_LIMIT_EXCEEDED",
	"INPUT_INVALID",
	"OUTPUT_FAILED",
	"ABORTED",
}

// AllCodes is every code, in declaration order (Rust's Code::ALL).
var AllCodes = [...]Code{
	CodeDSLParseError,
	CodeDSLTypeError,
	CodeStreamReused,
	CodeStreamabilityUnknown,
	CodeInputOrderViolation,
	CodeCaptureOverlapUnsupported,
	CodeMissingValue,
	CodeDuplicateMember,
	CodeInvalidNumber,
	CodeProtocolOrderError,
	CodeTargetValueUnrepresentable,
	CodeResourceLimitExceeded,
	CodeInputInvalid,
	CodeOutputFailed,
	CodeAborted,
}

// String is the code as it is written in every output: SCREAMING_SNAKE_CASE.
func (c Code) String() string {
	if int(c) < len(codeNames) {
		return codeNames[c]
	}
	return "CODE(" + strconv.Itoa(int(c)) + ")"
}

// ParseCode finds a code by its written form.
func ParseCode(text string) (Code, bool) {
	for _, c := range AllCodes {
		if c.String() == text {
			return c, true
		}
	}
	return 0, false
}

// Limit is the limit a CodeResourceLimitExceeded failure names.
type Limit struct {
	// Name is the Limits field, as written there (max_record_bytes).
	Name  string
	Value uint64
}

// Fail is a failure: a code, and what is known about where and why. It
// is this package's error type; every stage returns a *Fail, nil for
// none.
//
// Path is the input path the failure concerns, in jq syntax, or "" when
// none applies (a jq path is never empty: the root is "."). Row and
// Column are the 1-based source position, 0 when the failure has none.
// File is the source the position is in, when the program the failure
// came from was compiled from several. CommittedOutput says whether bytes
// had already been written when the failure was found: an incremental
// export cannot take them back.
type Fail struct {
	Code            Code
	Message         string
	Path            string
	Limit           *Limit
	Row             uint64
	Column          uint64
	File            string
	CommittedOutput bool
}

// NewFail is a failure with a code and a message.
func NewFail(code Code, message string) *Fail {
	return &Fail{Code: code, Message: message}
}

// LimitFail is a limit failure, named after the Limits field that was
// passed.
func LimitFail(name string, value uint64, message string) *Fail {
	return &Fail{Code: CodeResourceLimitExceeded, Message: message, Limit: &Limit{Name: name, Value: value}}
}

// ProtocolFail is a PROTOCOL_ORDER_ERROR.
func ProtocolFail(message string) *Fail { return NewFail(CodeProtocolOrderError, message) }

// InputFail is an INPUT_INVALID.
func InputFail(message string) *Fail { return NewFail(CodeInputInvalid, message) }

// OutputFail is an OUTPUT_FAILED.
func OutputFail(message string) *Fail { return NewFail(CodeOutputFailed, message) }

// AbortedFail is the failure for a cancelled run.
func AbortedFail() *Fail { return NewFail(CodeAborted, "the run was cancelled") }

// InFile sets the source file the position is in, and returns f.
func (f *Fail) InFile(file string) *Fail {
	f.File = file
	return f
}

// AtPath sets the failure's path, and returns f.
func (f *Fail) AtPath(path string) *Fail {
	f.Path = path
	return f
}

// At sets the failure's 1-based position, and returns f.
func (f *Fail) At(row, column uint64) *Fail {
	f.Row = row
	f.Column = column
	return f
}

// Committed marks the output as already partly written, and returns f.
func (f *Fail) Committed() *Fail {
	f.CommittedOutput = true
	return f
}

// Error is the failure as text: the code, the message, then the path,
// the position (with its file) and the limit when they apply.
func (f *Fail) Error() string {
	var b strings.Builder
	b.WriteString(f.Code.String())
	b.WriteString(": ")
	b.WriteString(f.Message)
	if f.Path != "" {
		b.WriteString(" at ")
		b.WriteString(f.Path)
	}
	switch {
	case f.File != "" && f.Row > 0:
		fmt.Fprintf(&b, " (%s:%d:%d)", f.File, f.Row, f.Column)
	case f.Row > 0:
		fmt.Fprintf(&b, " (%d:%d)", f.Row, f.Column)
	case f.File != "":
		fmt.Fprintf(&b, " (in %s)", f.File)
	}
	if f.Limit != nil {
		fmt.Fprintf(&b, " [%s = %d]", f.Limit.Name, f.Limit.Value)
	}
	return b.String()
}

// MarshalJSON writes the failure as hosts print it: `code`, `message`,
// and `path`, `limit` ({name, value}), `row`, `col`, `file` when they
// apply, then `output` ("partial" or "none").
func (f *Fail) MarshalJSON() ([]byte, error) {
	var b bytes.Buffer
	var s strings.Builder
	b.WriteString(`{"code":`)
	writeJSONString(f.Code.String(), &s)
	b.WriteString(s.String())
	field := func(name, text string) {
		s.Reset()
		writeJSONString(text, &s)
		b.WriteString(`,"` + name + `":` + s.String())
	}
	field("message", f.Message)
	if f.Path != "" {
		field("path", f.Path)
	}
	if f.Limit != nil {
		s.Reset()
		writeJSONString(f.Limit.Name, &s)
		fmt.Fprintf(&b, `,"limit":{"name":%s,"value":%d}`, s.String(), f.Limit.Value)
	}
	if f.Row > 0 {
		fmt.Fprintf(&b, `,"row":%d,"col":%d`, f.Row, f.Column)
	}
	if f.File != "" {
		field("file", f.File)
	}
	if f.CommittedOutput {
		b.WriteString(`,"output":"partial"}`)
	} else {
		b.WriteString(`,"output":"none"}`)
	}
	return b.Bytes(), nil
}
