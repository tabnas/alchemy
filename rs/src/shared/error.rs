//! Failures, with stable codes.
//!
//! The code is the contract: scripts and agents branch on it, and every
//! renderer, transducer and host uses the same set. The message, path and
//! limit are informative. A code is never renamed, removed or repurposed;
//! one may be added.

use std::fmt;

/// The stable failure codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Code {
    /// The DSL source does not parse (reader or layout error).
    DslParseError,
    /// The DSL program does not type check, or names something unknown.
    DslTypeError,
    /// A one-shot stream was consumed twice.
    StreamReused,
    /// The plan's streamability could not be established in strict mode.
    StreamabilityUnknown,
    /// Input arrived in an order the plan's contract forbids (a row before
    /// its metadata).
    InputOrderViolation,
    /// Two captures select overlapping scopes.
    CaptureOverlapUnsupported,
    /// A required value is absent and no policy maps it.
    MissingValue,
    /// An object repeats a member name under a policy that rejects that.
    DuplicateMember,
    /// A number lexeme is not a valid number for the target.
    InvalidNumber,
    /// A protocol event arrived out of sequence (a row before the schema,
    /// two schemas, a row of the wrong width, a missing end).
    ProtocolOrderError,
    /// The target format cannot represent the value (NaN in JSON, a table
    /// with no columns in CSV).
    TargetValueUnrepresentable,
    /// A configured limit was exceeded.
    ResourceLimitExceeded,
    /// The input did not parse, or is invalid for the source.
    InputInvalid,
    /// Writing the output failed.
    OutputFailed,
    /// The run was cancelled.
    Aborted,
}

impl Code {
    /// The code as it is written in every output: `SCREAMING_SNAKE_CASE`.
    pub fn as_str(self) -> &'static str {
        match self {
            Code::DslParseError => "DSL_PARSE_ERROR",
            Code::DslTypeError => "DSL_TYPE_ERROR",
            Code::StreamReused => "STREAM_REUSED",
            Code::StreamabilityUnknown => "STREAMABILITY_UNKNOWN",
            Code::InputOrderViolation => "INPUT_ORDER_VIOLATION",
            Code::CaptureOverlapUnsupported => "CAPTURE_OVERLAP_UNSUPPORTED",
            Code::MissingValue => "MISSING_VALUE",
            Code::DuplicateMember => "DUPLICATE_MEMBER",
            Code::InvalidNumber => "INVALID_NUMBER",
            Code::ProtocolOrderError => "PROTOCOL_ORDER_ERROR",
            Code::TargetValueUnrepresentable => "TARGET_VALUE_UNREPRESENTABLE",
            Code::ResourceLimitExceeded => "RESOURCE_LIMIT_EXCEEDED",
            Code::InputInvalid => "INPUT_INVALID",
            Code::OutputFailed => "OUTPUT_FAILED",
            Code::Aborted => "ABORTED",
        }
    }

    /// Every code, in declaration order (for documentation checks).
    pub const ALL: [Code; 15] = [
        Code::DslParseError,
        Code::DslTypeError,
        Code::StreamReused,
        Code::StreamabilityUnknown,
        Code::InputOrderViolation,
        Code::CaptureOverlapUnsupported,
        Code::MissingValue,
        Code::DuplicateMember,
        Code::InvalidNumber,
        Code::ProtocolOrderError,
        Code::TargetValueUnrepresentable,
        Code::ResourceLimitExceeded,
        Code::InputInvalid,
        Code::OutputFailed,
        Code::Aborted,
    ];

    /// A code by its written form.
    pub fn parse(text: &str) -> Option<Code> {
        Code::ALL.iter().copied().find(|c| c.as_str() == text)
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The limit a [`Code::ResourceLimitExceeded`] names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limit {
    /// The `Limits` field, as written there (`max_record_bytes`).
    pub name: &'static str,
    pub value: u64,
}

/// A failure: a code, and what is known about where and why.
///
/// `committed_output` says whether bytes had already been written when
/// the failure was found: an incremental export cannot take them back, and
/// the caller must be told the output may be partial.
#[derive(Clone, Debug, PartialEq)]
pub struct Fail {
    pub code: Code,
    pub message: String,
    /// The input path the failure concerns, in jq syntax, when one applies.
    pub path: Option<String>,
    /// Boxed, with the file below, so that a `Fail` stays small enough
    /// to return by value on every failure path without a warning.
    pub limit: Option<Box<Limit>>,
    /// 1-based source row and column, when the failure has a position.
    pub row: Option<u64>,
    pub column: Option<u64>,
    /// The source file the position is in, when the program the failure
    /// came from was compiled from several (a format's parts linked with
    /// a program); absent, the position is the one source's.
    pub file: Option<Box<str>>,
    pub committed_output: bool,
}

impl Fail {
    pub fn new(code: Code, message: impl Into<String>) -> Fail {
        Fail {
            code,
            message: message.into(),
            path: None,
            limit: None,
            row: None,
            column: None,
            file: None,
            committed_output: false,
        }
    }

    /// The failure with the source file its position is in.
    pub fn in_file(mut self, file: impl Into<Box<str>>) -> Fail {
        self.file = Some(file.into());
        self
    }

    pub fn at_path(mut self, path: impl Into<String>) -> Fail {
        self.path = Some(path.into());
        self
    }

    pub fn at(mut self, row: u64, column: u64) -> Fail {
        self.row = Some(row);
        self.column = Some(column);
        self
    }

    pub fn committed(mut self) -> Fail {
        self.committed_output = true;
        self
    }

    /// A limit failure, named after the `Limits` field that was passed.
    pub fn limit(name: &'static str, value: u64, message: impl Into<String>) -> Fail {
        Fail {
            limit: Some(Box::new(Limit { name, value })),
            ..Fail::new(Code::ResourceLimitExceeded, message)
        }
    }

    pub fn protocol(message: impl Into<String>) -> Fail {
        Fail::new(Code::ProtocolOrderError, message)
    }

    pub fn input(message: impl Into<String>) -> Fail {
        Fail::new(Code::InputInvalid, message)
    }

    pub fn output(message: impl Into<String>) -> Fail {
        Fail::new(Code::OutputFailed, message)
    }

    pub fn aborted() -> Fail {
        Fail::new(Code::Aborted, "the run was cancelled")
    }

    /// The engine's own error, as an input failure carrying its code,
    /// position and report. With the `tabnas` feature, which the
    /// `language` feature turns on.
    #[cfg(feature = "tabnas")]
    pub fn from_tabnas(e: &tabnas::TabnasError) -> Fail {
        let mut f = Fail::new(
            Code::InputInvalid,
            format!("{}: {}", e.code, e.detail.trim_end()),
        );
        if e.row > 0 {
            f.row = Some(e.row as u64);
            f.column = Some(e.col as u64);
        }
        f
    }

    /// The failure as a JSON object: `code`, `message`, and `path`,
    /// `limit` (`{name, value}`), `row`, `col`, `output` ("partial" or
    /// "none") when they apply. This is the shape hosts print.
    pub fn to_json(&self) -> serde_json::Value {
        let mut m = serde_json::Map::new();
        m.insert("code".into(), self.code.as_str().into());
        m.insert("message".into(), self.message.clone().into());
        if let Some(p) = &self.path {
            m.insert("path".into(), p.clone().into());
        }
        if let Some(l) = &self.limit {
            m.insert(
                "limit".into(),
                serde_json::json!({ "name": l.name, "value": l.value }),
            );
        }
        if let Some(r) = self.row {
            m.insert("row".into(), r.into());
        }
        if let Some(c) = self.column {
            m.insert("col".into(), c.into());
        }
        if let Some(file) = &self.file {
            m.insert("file".into(), file.to_string().into());
        }
        m.insert(
            "output".into(),
            if self.committed_output {
                "partial"
            } else {
                "none"
            }
            .into(),
        );
        serde_json::Value::Object(m)
    }
}

impl fmt::Display for Fail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)?;
        if let Some(p) = &self.path {
            write!(f, " at {p}")?;
        }
        match (&self.file, self.row, self.column) {
            (Some(file), Some(r), Some(c)) => write!(f, " ({file}:{r}:{c})")?,
            (None, Some(r), Some(c)) => write!(f, " ({r}:{c})")?,
            (Some(file), _, _) => write!(f, " (in {file})")?,
            (None, _, _) => {}
        }
        if let Some(l) = &self.limit {
            write!(f, " [{} = {}]", l.name, l.value)?;
        }
        Ok(())
    }
}

impl std::error::Error for Fail {}

#[cfg(test)]
mod tests {
    use super::*;

    /// A position is written as it always was, and with its file when
    /// the failure names one.
    #[test]
    fn a_position_names_its_file_when_it_has_one() {
        let plain = Fail::new(Code::DslTypeError, "arity: one argument").at(2, 3);
        assert_eq!(
            plain.to_string(),
            "DSL_TYPE_ERROR: arity: one argument (2:3)"
        );
        let filed = Fail::new(Code::DslTypeError, "arity: one argument")
            .at(2, 3)
            .in_file("render.alc");
        assert_eq!(
            filed.to_string(),
            "DSL_TYPE_ERROR: arity: one argument (render.alc:2:3)"
        );
        let no_position = Fail::new(Code::DslParseError, "bad_def: a name").in_file("lift.alc");
        assert_eq!(
            no_position.to_string(),
            "DSL_PARSE_ERROR: bad_def: a name (in lift.alc)"
        );
    }

    #[test]
    fn codes_are_stable_names() {
        for code in Code::ALL {
            assert_eq!(Code::parse(code.as_str()), Some(code));
            assert!(code
                .as_str()
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b == b'_'));
        }
        assert_eq!(Code::parse("nope"), None);
    }

    #[test]
    fn json_shape() {
        let f = Fail::limit("max_record_bytes", 64, "a row of 65 bytes")
            .at_path(".rows[3]")
            .committed();
        let j = f.to_json();
        assert_eq!(j["code"], "RESOURCE_LIMIT_EXCEEDED");
        assert_eq!(j["limit"]["name"], "max_record_bytes");
        assert_eq!(j["limit"]["value"], 64);
        assert_eq!(j["path"], ".rows[3]");
        assert_eq!(j["output"], "partial");
        assert_eq!(
            f.to_string(),
            "RESOURCE_LIMIT_EXCEEDED: a row of 65 bytes at .rows[3] [max_record_bytes = 64]"
        );
        // No file is written when there is none; the file beside the
        // position when there is.
        assert!(j.get("file").is_none(), "{j}");
        let filed = Fail::new(Code::DslTypeError, "arity: one argument")
            .at(2, 3)
            .in_file("render.alc")
            .to_json();
        assert_eq!(filed["file"], "render.alc");
        assert_eq!(
            (filed["row"].clone(), filed["col"].clone()),
            (2.into(), 3.into())
        );
    }
}
