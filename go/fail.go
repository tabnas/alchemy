// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

import (
	"strings"

	tabnas "github.com/tabnas/parser/go"
	tt "github.com/tabnas/transduce/go"
)

// fail.go: the failure type every stage returns.
//
// The Rust crate raises `tabnas_transduce::Fail` with codes from
// `tabnas_transduce::Code`, the one set every renderer, transducer and
// host shares. This port does the same with the Go port of transduce: the
// names here are aliases of its types and constants, so a failure this
// package returns is the one transduce's stages and render's renderers
// return, and a host handles them alike.

// Code is a stable failure code: transduce's.
type Code = tt.Code

// Fail is a failure: transduce's. Row and Column are 1-based, 0 when the
// failure names no position; File is the source the position is in, when
// the program was compiled from several.
type Fail = tt.Fail

// The stable failure codes, transduce's.
const (
	CodeDSLParseError              = tt.CodeDSLParseError
	CodeDSLTypeError               = tt.CodeDSLTypeError
	CodeStreamReused               = tt.CodeStreamReused
	CodeStreamabilityUnknown       = tt.CodeStreamabilityUnknown
	CodeInputOrderViolation        = tt.CodeInputOrderViolation
	CodeCaptureOverlapUnsupported  = tt.CodeCaptureOverlapUnsupported
	CodeMissingValue               = tt.CodeMissingValue
	CodeDuplicateMember            = tt.CodeDuplicateMember
	CodeInvalidNumber              = tt.CodeInvalidNumber
	CodeProtocolOrderError         = tt.CodeProtocolOrderError
	CodeTargetValueUnrepresentable = tt.CodeTargetValueUnrepresentable
	CodeResourceLimitExceeded      = tt.CodeResourceLimitExceeded
	CodeInputInvalid               = tt.CodeInputInvalid
	CodeOutputFailed               = tt.CodeOutputFailed
	CodeAborted                    = tt.CodeAborted
)

// NewFail is a failure with a code and a message.
func NewFail(code Code, message string) *Fail { return tt.NewFail(code, message) }

// FinerCode is the code a failure of this package pins: the first word of
// its message, before `: ` (`bad_dedent`, `type_mismatch`, `reused`), the
// convention every stage follows; a message without it pins the code
// itself.
func FinerCode(f *Fail) string {
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
