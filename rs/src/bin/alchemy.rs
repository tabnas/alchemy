//! The `alchemy` command.
//!
//! ```text
//! alchemy canon FILE     print the program in canonical form
//! alchemy format FILE    print the program in layout form
//! alchemy check FILE     parse and desugar; print nothing and exit 0
//! ```
//!
//! `FILE` may be `-` for standard input. A program that does not read or
//! desugar prints its failure as JSON (the `Fail` shape: `code`,
//! `message`, `row`, `col`, `output`) on standard error and exits 2, the
//! usage status, as does a usage error or an unreadable file; nothing but
//! the answer goes to standard output. `check` at this stage means the
//! program parses and desugars; the checker joins it when it exists.

use std::io::{self, Read, Write};
use std::process::ExitCode;

use tabnas_alchemy::{canonical, desugar, format, parse_file};
use tabnas_transduce::Fail;

const USAGE: &str = "usage: alchemy canon|format|check FILE   (FILE may be - for standard input)";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [command, file] = args.as_slice() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match run(command, file) {
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
        Err(fail) => {
            eprintln!("{}", fail.to_json());
            ExitCode::from(2)
        }
    }
}

enum Command {
    Canon,
    Format,
    Check,
}

impl Command {
    fn named(name: &str) -> Option<Command> {
        match name {
            "canon" => Some(Command::Canon),
            "format" => Some(Command::Format),
            "check" => Some(Command::Check),
            _ => None,
        }
    }
}

fn read(file: &str) -> Result<String, Fail> {
    let mut text = String::new();
    let outcome = if file == "-" {
        io::stdin().lock().read_to_string(&mut text).map(|_| ())
    } else {
        std::fs::read_to_string(file).map(|read| text = read)
    };
    outcome.map_err(|error| Fail::input(format!("cannot read {file}: {error}")))?;
    Ok(text)
}

/// The text to print for `command` over `file`, or the failure.
fn run(command: &str, file: &str) -> Result<String, Fail> {
    // The command is judged before the file is read, so an unknown one is
    // the usage error whatever the file holds, and never waits on standard
    // input to say so.
    let command = Command::named(command).ok_or_else(|| Fail::input(USAGE))?;
    let src = read(file)?;
    let program = parse_file(&src, file)?;
    match command {
        Command::Canon => {
            let text = canonical(&program);
            Ok(if text.is_empty() { text } else { text + "\n" })
        }
        Command::Format => Ok(format(&program)),
        Command::Check => {
            desugar::program(program, &src)?;
            Ok(String::new())
        }
    }
}
