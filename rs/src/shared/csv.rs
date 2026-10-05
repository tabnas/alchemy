//! The CSV dialect: the options render's `CsvRenderer` takes, which
//! alchemy builds from a program's `csv-options` and hands to
//! [`Renderers::csv`](crate::shared::Renderers::csv).
//!
//! The standard profile quotes every field, doubles `"`, ends every record
//! with CRLF and writes numbers as their lexemes; minimal quoting and other
//! delimiters are dialects the caller selects explicitly.

/// The record terminator.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Newline {
    Lf,
    /// RFC 4180's terminator, and the standard profile's.
    #[default]
    CrLf,
}

impl Newline {
    pub fn as_str(self) -> &'static str {
        match self {
            Newline::Lf => "\n",
            Newline::CrLf => "\r\n",
        }
    }
}

/// When a field is quoted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Quoting {
    /// Every field, the standard profile.
    #[default]
    Always,
    /// Only a field holding the delimiter, `"`, CR or LF. An empty field is
    /// then written as nothing, so the empty string and an empty null text
    /// read back the same; that is the dialect's trade-off, not a defect.
    Minimal,
}

/// What a [`Cell::Missing`](crate::shared::Cell::Missing) becomes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MissingText {
    /// Fail the run with `MISSING_VALUE`: a table that promised a column
    /// and did not deliver it is not silently padded.
    #[default]
    Error,
    /// Write this text instead.
    Text(Box<str>),
}

/// The CSV dialect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CsvOptions {
    /// One character, and not `"`, CR, LF or NUL: those would make the
    /// output unreadable by construction, and are refused when the renderer
    /// is built.
    pub delimiter: char,
    pub newline: Newline,
    /// Write the labels as the first record.
    pub header: bool,
    /// The text of a [`Cell::Null`](crate::shared::Cell::Null); empty by default.
    pub null_text: Box<str>,
    pub missing: MissingText,
    pub quoting: Quoting,
}

impl Default for CsvOptions {
    fn default() -> Self {
        CsvOptions {
            delimiter: ',',
            newline: Newline::CrLf,
            header: true,
            null_text: "".into(),
            missing: MissingText::Error,
            quoting: Quoting::Always,
        }
    }
}
