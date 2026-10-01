// Copyright (c) 2026 tabnas, MIT License

// Command alchemy is the alchemy language's command line, the Go port of
// rs/src/bin/alchemy.rs:
//
//	alchemy canon FILE       print the program in canonical form
//	alchemy format FILE      print the program in layout form
//	alchemy check FILE       parse, desugar, resolve, check and build the plan; print nothing and exit 0
//	alchemy explain FILE     print the plan report
//	alchemy run [--render csv|json] [--no-native] [--max-output-bytes N] PROGRAM INPUT
//	                         run the program over the JSON document INPUT
//
// FILE, PROGRAM and INPUT may be - for standard input (one of them per
// run). Nothing but the answer goes to standard output. A failure is the
// Fail as one JSON object on standard error (code, message, and path,
// limit, row, col when they apply, and output: "partial" when bytes had
// reached standard output). The exit status follows the code: 2 for a
// program that does not read, resolve or check (DSL_PARSE_ERROR,
// DSL_TYPE_ERROR, STREAM_REUSED, STREAMABILITY_UNKNOWN), for a usage error
// and for an unreadable file; 1 for an input or protocol failure (from
// check too, when building the plan reaches a fail); 5 for
// RESOURCE_LIMIT_EXCEEDED; 3 for OUTPUT_FAILED; 6 for ABORTED. A standard
// error that cannot be written loses the report, not the status.
//
// --max-output-bytes sets the writer's limit; the others are transduce's
// defaults.
//
// run parses INPUT with the tabnas JSON grammar through transduce's
// ParserSource, incrementally, pruning the parsed tree under the program's
// row selector when the plan knows one; other input formats are aless's
// business. The output goes through a coalescing writer to standard output
// and is flushed once, at the end; a failure found after bytes were
// written says so.
//
// The incremental source needs the engine's node-cell identity, which
// transduce builds only with the tabnas_nodecell tag. A build without it
// has no verified grammar, and run reports the source's
// STREAMABILITY_UNKNOWN (status 2) without reading the document, rather
// than read it some other way: the materialized walk would lose number
// lexemes (1.50 written 1.5), keep one of a repeated member where the
// stream reports both, and hold the whole document, which is not the run
// the plan report promises. Build the command with -tags tabnas_nodecell.
package main

import (
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"os/signal"
	"strconv"
	"strings"
	"syscall"

	alchemy "github.com/tabnas/alchemy/go"
	tabnasjson "github.com/tabnas/json/go"
	tt "github.com/tabnas/transduce/go"
)

const usage = "usage: alchemy canon|format|check|explain FILE\n       alchemy run [--render csv|json] [--no-native] [--max-output-bytes N] PROGRAM INPUT\n       (a FILE may be - for standard input)"

func main() {
	// A write to a closed standard output or standard error fails with an
	// error the command reports (OUTPUT_FAILED, status 3) or drops (the
	// report on standard error), as the Rust binary's does, rather than
	// ending the process on SIGPIPE, which Go does for those two
	// descriptors unless the signal is ignored.
	signal.Ignore(syscall.SIGPIPE)
	os.Exit(command(os.Args[1:], os.Stdin, os.Stdout, os.Stderr))
}

// command runs one command line and answers its exit status.
func command(args []string, stdin io.Reader, stdout, stderr io.Writer) int {
	text, exit := run(args, stdin, stdout)
	if exit != nil {
		report(stderr, exit.fail)
		return exit.status
	}
	if _, err := io.WriteString(stdout, text); err != nil {
		return 3
	}
	return 0
}

// report writes the failure as one JSON object on standard error. A
// standard error that cannot be written (a closed pipe) loses the report,
// not the status.
func report(stderr io.Writer, f *tt.Fail) {
	data, err := f.MarshalJSON()
	if err != nil {
		return
	}
	_, _ = stderr.Write(append(data, '\n'))
}

// exit is a failure and the status it exits with: the code's, except that
// a usage error and an unreadable file exit 2 although their code is
// INPUT_INVALID, since they are the command line's fault, not the
// document's.
type exit struct {
	fail   *tt.Fail
	status int
}

func fromFail(f *tt.Fail) *exit {
	status := 1
	switch f.Code {
	case tt.CodeDSLParseError, tt.CodeDSLTypeError, tt.CodeStreamReused, tt.CodeStreamabilityUnknown:
		status = 2
	case tt.CodeResourceLimitExceeded:
		status = 5
	case tt.CodeOutputFailed:
		status = 3
	case tt.CodeAborted:
		status = 6
	}
	return &exit{fail: f, status: status}
}

func usageError() *exit { return &exit{fail: tt.InputFail(usage), status: 2} }

// read is a file's text, or standard input's for "-".
func read(file string, stdin io.Reader) (string, *exit) {
	var data []byte
	var err error
	if file == "-" {
		data, err = io.ReadAll(stdin)
	} else {
		data, err = os.ReadFile(file)
	}
	if err != nil {
		return "", &exit{fail: tt.InputFail(fmt.Sprintf("cannot read %s: %s", file, ioMessage(err))), status: 2}
	}
	return string(data), nil
}

// ioMessage is an I/O error as the Rust binary's message has it, from the
// operating system's own text and number: `No such file or directory (os
// error 2)`.
func ioMessage(err error) string {
	var pathError *fs.PathError
	if errors.As(err, &pathError) {
		err = pathError.Err
	}
	var errno syscall.Errno
	if errors.As(err, &errno) {
		text := errno.Error()
		if text != "" {
			text = strings.ToUpper(text[:1]) + text[1:]
		}
		return fmt.Sprintf("%s (os error %d)", text, int(errno))
	}
	return err.Error()
}

// run is the text to print for the command line, or the failure. `run`
// writes its answer itself and answers nothing here.
func run(args []string, stdin io.Reader, stdout io.Writer) (string, *exit) {
	// The command is judged before any file is read, so an unknown one is
	// the usage error whatever the files hold, and never waits on standard
	// input to say so.
	if len(args) == 0 {
		return "", usageError()
	}
	switch command, rest := args[0], args[1:]; command {
	case "canon", "format", "check", "explain":
		if len(rest) != 1 {
			return "", usageError()
		}
		file := rest[0]
		src, e := read(file, stdin)
		if e != nil {
			return "", e
		}
		switch command {
		case "canon":
			program, f := alchemy.ParseFile(src, file)
			if f != nil {
				return "", fromFail(f)
			}
			text := alchemy.Canonical(program)
			if text != "" {
				text += "\n"
			}
			return text, nil
		case "format":
			program, f := alchemy.ParseFile(src, file)
			if f != nil {
				return "", fromFail(f)
			}
			return alchemy.Format(program), nil
		case "check":
			if _, f := alchemy.Compile(src, file); f != nil {
				return "", fromFail(f)
			}
			return "", nil
		}
		program, f := alchemy.Compile(src, file)
		if f != nil {
			return "", fromFail(f)
		}
		return program.Explain(), nil
	case "run":
		options, e := parseRunOptions(rest)
		if e != nil {
			return "", e
		}
		src, e := read(options.program, stdin)
		if e != nil {
			return "", e
		}
		program, f := alchemy.Compile(src, options.program)
		if f == nil && !options.native {
			program, f = program.WithNative(false)
		}
		if f != nil {
			return "", fromFail(f)
		}
		input, e := read(options.input, stdin)
		if e != nil {
			return "", e
		}
		limits := tt.DefaultLimits()
		limits.MaxOutputBytes = options.maxOutputBytes
		if f := execute(program, input, options.render, limits, stdout); f != nil {
			return "", fromFail(f)
		}
		return "", nil
	}
	return "", usageError()
}

type runOptions struct {
	render         alchemy.Renderer
	native         bool
	maxOutputBytes *uint64
	program        string
	input          string
}

// parseU64 reads a byte count as Rust's u64 parse does: decimal digits,
// with an optional leading +.
func parseU64(text string) (uint64, bool) {
	digits := strings.TrimPrefix(text, "+")
	if digits == "" || strings.ContainsAny(digits[:1], "+-") {
		return 0, false
	}
	n, err := strconv.ParseUint(digits, 10, 64)
	return n, err == nil
}

func parseRunOptions(args []string) (*runOptions, *exit) {
	options := &runOptions{native: true}
	var files []string
	for i := 0; i < len(args); {
		switch arg := args[i]; {
		case arg == "--render":
			if i+1 >= len(args) {
				return nil, usageError()
			}
			name := args[i+1]
			render, ok := alchemy.RendererNamed(name)
			if !ok {
				return nil, &exit{fail: tt.InputFail("--render takes csv or json, not " + strconv.Quote(name)), status: 2}
			}
			options.render = render
			i += 2
		case arg == "--no-native":
			options.native = false
			i++
		case arg == "--max-output-bytes":
			if i+1 >= len(args) {
				return nil, usageError()
			}
			n, ok := parseU64(args[i+1])
			if !ok {
				return nil, &exit{fail: tt.InputFail("--max-output-bytes takes a number of bytes, not " + strconv.Quote(args[i+1])), status: 2}
			}
			options.maxOutputBytes = &n
			i += 2
		case strings.HasPrefix(arg, "--"):
			return nil, usageError()
		default:
			files = append(files, arg)
			i++
		}
	}
	if len(files) != 2 {
		return nil, usageError()
	}
	if files[0] == "-" && files[1] == "-" {
		return nil, &exit{fail: tt.InputFail("only one of PROGRAM and INPUT may be - (standard input)"), status: 2}
	}
	options.program, options.input = files[0], files[1]
	return options, nil
}

// execute runs the program over one JSON document, writing to stdout.
func execute(program *alchemy.Program, input string, render alchemy.Renderer, limits tt.Limits, stdout io.Writer) *tt.Fail {
	metrics := tt.NewMetrics()
	sink, f := program.Sink(stdout, render, limits, metrics)
	if f != nil {
		return f
	}
	prune := tt.Prune{}
	if selector, ok := program.RowSelector(); ok {
		prune = tt.PruneUnderSelector(selector)
	}
	_, f = tt.NewParserSource(tabnasjson.Make(), input).
		Grammar("json").
		Mode(tt.IncrementalMode(prune)).
		Limits(limits).
		Metrics(metrics).
		Run(sink)
	// A failure the source found (bad JSON after the rows) knows nothing
	// of the output; the writer's count says whether bytes had reached
	// standard output.
	if f != nil && metrics.OutputBytes.Load() > 0 && !f.CommittedOutput {
		f.Committed()
	}
	return f
}
