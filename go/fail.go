// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

import (
	"strings"

	"github.com/tabnas/alchemy/go/shared"
	tabnas "github.com/tabnas/parser/go"
)

// fail.go: the failure type every stage returns.
//
// The Rust crate raises `tabnas_transduce::Fail` with codes from
// `tabnas_transduce::Code`, the one set every renderer, transducer and
// host shares. This port declares that set in its shared package, which
// the Go ports of transduce and render build on: the names here are
// aliases of its types and constants, so a failure this package returns
// is the one transduce's stages and render's renderers return, and a host
// handles them alike.

// Code is a stable failure code: the shared set.
type Code = shared.Code

// Fail is a failure: the shared one. Row and Column are 1-based, 0 when
// the failure names no position; File is the source the position is in,
// when the program was compiled from several.
type Fail = shared.Fail

// The stable failure codes, the shared set.
const (
	CodeDSLParseError              = shared.CodeDSLParseError
	CodeDSLTypeError               = shared.CodeDSLTypeError
	CodeStreamReused               = shared.CodeStreamReused
	CodeStreamabilityUnknown       = shared.CodeStreamabilityUnknown
	CodeInputOrderViolation        = shared.CodeInputOrderViolation
	CodeCaptureOverlapUnsupported  = shared.CodeCaptureOverlapUnsupported
	CodeMissingValue               = shared.CodeMissingValue
	CodeDuplicateMember            = shared.CodeDuplicateMember
	CodeInvalidNumber              = shared.CodeInvalidNumber
	CodeProtocolOrderError         = shared.CodeProtocolOrderError
	CodeTargetValueUnrepresentable = shared.CodeTargetValueUnrepresentable
	CodeResourceLimitExceeded      = shared.CodeResourceLimitExceeded
	CodeInputInvalid               = shared.CodeInputInvalid
	CodeOutputFailed               = shared.CodeOutputFailed
	CodeAborted                    = shared.CodeAborted
)

// NewFail is a failure with a code and a message.
func NewFail(code Code, message string) *Fail { return shared.NewFail(code, message) }

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
