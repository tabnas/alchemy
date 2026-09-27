//! The `alchemy` command.
//!
//! ```text
//! alchemy canon FILE       print the program in canonical form
//! alchemy format FILE      print the program in layout form
//! alchemy check FILE       parse, desugar, resolve and check; print nothing and exit 0
//! alchemy explain FILE     print the plan report
//! alchemy run [--render csv|json] [--no-native] PROGRAM INPUT
//!                          run the program over the JSON document INPUT
//! ```
//!
//! `FILE`, `PROGRAM` and `INPUT` may be `-` for standard input (one of
//! them per run). Nothing but the answer goes to standard output. A
//! failure is the `Fail` as one JSON object on standard error (`code`,
//! `message`, and `path`, `limit`, `row`, `col` when they apply, and
//! `output`: `"partial"` when bytes had reached standard output). The
//! exit status follows the code: 2 for a program that does not read,
//! resolve or check (`DSL_PARSE_ERROR`, `DSL_TYPE_ERROR`, `STREAM_REUSED`,
//! `STREAMABILITY_UNKNOWN`), for a usage error and for an unreadable
//! file; 1 for an input or protocol failure; 5 for
//! `RESOURCE_LIMIT_EXCEEDED`; 3 for `OUTPUT_FAILED`; 6 for `ABORTED`.
//!
//! `run` parses `INPUT` with the tabnas JSON grammar through transduce's
//! `ParserSource`, incrementally, pruning the parsed tree under the
//! program's row selector when the plan knows one; other input formats
//! are aless's business. The output goes through a coalescing writer to
//! standard output and is flushed once, at the end; a failure found after
//! bytes were written says so.

use std::io::{self, Read, Write};
use std::process::ExitCode;

use tabnas_alchemy::{canonical, compile, format, parse_file, Renderer};
use tabnas_transduce::{Code, Fail, Limits, Metrics, ParserSource, Prune, SourceMode};

const USAGE: &str = "usage: alchemy canon|format|check|explain FILE\n       alchemy run [--render csv|json] [--no-native] PROGRAM INPUT\n       (a FILE may be - for standard input)";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(text) => {
            let mut out = io::stdout().lock();
            if out
                .write_all(text.as_bytes())
                .and_then(|()| out.flush())
                .is_err()
            {
                return ExitCode::from(3);
            }
            ExitCode::SUCCESS
        }
        Err(exit) => {
            eprintln!("{}", exit.fail.to_json());
            ExitCode::from(exit.status)
        }
    }
}

/// A failure and the status it exits with: the code's, except that a
/// usage error and an unreadable file exit 2 although their code is
/// `INPUT_INVALID`, since they are the command line's fault, not the
/// document's.
struct Exit {
    fail: Fail,
    status: u8,
}

impl From<Fail> for Exit {
    fn from(fail: Fail) -> Exit {
        let status = match fail.code {
            Code::DslParseError
            | Code::DslTypeError
            | Code::StreamReused
            | Code::StreamabilityUnknown => 2,
            Code::ResourceLimitExceeded => 5,
            Code::OutputFailed => 3,
            Code::Aborted => 6,
            _ => 1,
        };
        Exit { fail, status }
    }
}

fn usage() -> Exit {
    Exit {
        fail: Fail::input(USAGE),
        status: 2,
    }
}

fn read(file: &str) -> Result<String, Exit> {
    let mut text = String::new();
    let outcome = if file == "-" {
        io::stdin().lock().read_to_string(&mut text).map(|_| ())
    } else {
        std::fs::read_to_string(file).map(|read| text = read)
    };
    outcome.map_err(|error| Exit {
        fail: Fail::input(format!("cannot read {file}: {error}")),
        status: 2,
    })?;
    Ok(text)
}

/// The text to print for the command line, or the failure. `run` writes
/// its answer itself and answers nothing here.
fn run(args: &[String]) -> Result<String, Exit> {
    // The command is judged before any file is read, so an unknown one is
    // the usage error whatever the files hold, and never waits on standard
    // input to say so.
    let (command, rest) = args.split_first().ok_or_else(usage)?;
    match command.as_str() {
        "canon" | "format" | "check" | "explain" => {
            let [file] = rest else {
                return Err(usage());
            };
            let src = read(file)?;
            match command.as_str() {
                "canon" => {
                    let text = canonical(&parse_file(&src, file)?);
                    Ok(if text.is_empty() { text } else { text + "\n" })
                }
                "format" => Ok(format(&parse_file(&src, file)?)),
                "check" => Ok(compile(&src, file).map(|_| String::new())?),
                _ => Ok(compile(&src, file).map(|program| program.explain())?),
            }
        }
        "run" => {
            let options = RunOptions::parse(rest)?;
            let src = read(&options.program)?;
            let mut program = compile(&src, &options.program)?;
            if !options.native {
                program = program.with_native(false)?;
            }
            let input = read(&options.input)?;
            execute(&program, &input, options.render)?;
            Ok(String::new())
        }
        _ => Err(usage()),
    }
}

struct RunOptions {
    render: Option<Renderer>,
    native: bool,
    program: String,
    input: String,
}

impl RunOptions {
    fn parse(args: &[String]) -> Result<RunOptions, Exit> {
        let mut render = None;
        let mut native = true;
        let mut files = Vec::new();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--render" => {
                    let name = args.get(i + 1).ok_or_else(usage)?;
                    render = Some(Renderer::named(name).ok_or_else(|| Exit {
                        fail: Fail::input(format!("--render takes csv or json, not {name:?}")),
                        status: 2,
                    })?);
                    i += 2;
                }
                "--no-native" => {
                    native = false;
                    i += 1;
                }
                other if other.starts_with("--") => return Err(usage()),
                other => {
                    files.push(other.to_string());
                    i += 1;
                }
            }
        }
        let [program, input] = files.as_slice() else {
            return Err(usage());
        };
        if program == "-" && input == "-" {
            return Err(Exit {
                fail: Fail::input("only one of PROGRAM and INPUT may be - (standard input)"),
                status: 2,
            });
        }
        Ok(RunOptions {
            render,
            native,
            program: program.clone(),
            input: input.clone(),
        })
    }
}

/// Run the program over one JSON document, writing to standard output.
fn execute(
    program: &tabnas_alchemy::Program,
    input: &str,
    render: Option<Renderer>,
) -> Result<(), Fail> {
    let limits = Limits::default();
    let metrics = Metrics::new();
    let sink = program.sink(Box::new(io::stdout()), render, &limits, metrics.clone())?;
    let prune = match program.row_selector() {
        Some(selector) => Prune::Under(selector.clone()),
        None => Prune::Never,
    };
    let (outcome, _) = ParserSource::new(tabnas_json::make(), input)
        .grammar("json")
        .mode(SourceMode::Incremental { prune })
        .limits(limits)
        .metrics(metrics.clone())
        .run_owned(sink);
    outcome.map(|_| ()).map_err(|fail| {
        // A failure the source found (bad JSON after the rows) knows
        // nothing of the output; the writer's count says whether bytes
        // had reached standard output.
        if Metrics::get(&metrics.output_bytes) > 0 && !fail.committed_output {
            fail.committed()
        } else {
            fail
        }
    })
}
