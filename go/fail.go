// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

import (
	"fmt"
	"strconv"
	"strings"

	tabnas "github.com/tabnas/parser/go"
)

// fail.go: the failure type every stage of the front end returns.
//
// The Rust crate raises `tabnas_transduce::Fail` with codes from
// `tabnas_transduce::Code`, the one set every renderer, transducer and
// host shares. The Go port of transduce is not published yet, so this
// file mirrors its API exactly (the type, the constants, NewFail, At,
// InFile, Error): when the interpreter arrives with transduce, this file
// becomes a set of aliases (`type Fail = tabnastransduce.Fail`, and the
// same for Code and its constants) and nothing that calls it changes.

// Code is a stable failure code. The code is the contract: scripts and
// agents branch on it. A code is never renamed, removed or repurposed.
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

// String is the code as it is written in every output: SCREAMING_SNAKE_CASE.
func (c Code) String() string {
	if int(c) < len(codeNames) {
		return codeNames[c]
	}
	return "CODE(" + strconv.Itoa(int(c)) + ")"
}

// Fail is a failure: a code, a message, and the 1-based position the
// failure names (0 when it names none), with the file the position is in
// when the program was compiled from several sources.
type Fail struct {
	Code    Code
	Message string
	Row     uint64
	Column  uint64
	File    string
}

// NewFail is a failure with a code and a message.
func NewFail(code Code, message string) *Fail {
	return &Fail{Code: code, Message: message}
}

// At sets the failure's 1-based position, and returns f.
func (f *Fail) At(row, column uint64) *Fail {
	f.Row = row
	f.Column = column
	return f
}

// InFile sets the source file the position is in, and returns f.
func (f *Fail) InFile(file string) *Fail {
	f.File = file
	return f
}

// Error is the failure as text: the code, the message, then the position
// (with its file) when there is one.
func (f *Fail) Error() string {
	var b strings.Builder
	b.WriteString(f.Code.String())
	b.WriteString(": ")
	b.WriteString(f.Message)
	switch {
	case f.File != "" && f.Row > 0:
		fmt.Fprintf(&b, " (%s:%d:%d)", f.File, f.Row, f.Column)
	case f.Row > 0:
		fmt.Fprintf(&b, " (%d:%d)", f.Row, f.Column)
	case f.File != "":
		fmt.Fprintf(&b, " (in %s)", f.File)
	}
	return b.String()
}

// FinerCode is the code a failure of this package pins: the first word of
// its message, before `: ` (`bad_dedent`, `type_mismatch`, `reused`), the
// convention every stage follows; a message without it pins the code
// itself.
func (f *Fail) FinerCode() string {
	if i := strings.Index(f.Message, ": "); i >= 0 {
		return f.Message[:i]
	}
	return f.Code.String()
}

// failFromTabnas is the engine's error as this package reports it:
// DSL_PARSE_ERROR, the engine's code leading the message, and the 1-based
// position when the engine has one.
func failFromTabnas(e *tabnas.TabnasError) *Fail {
	f := NewFail(CodeDSLParseError, e.Code+": "+strings.TrimRight(e.Detail, " \t\r\n"))
	if e.Row > 0 {
		f.At(uint64(e.Row), uint64(e.Col))
	}
	return f
}
