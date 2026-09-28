//! The layout lex matcher: indentation as tokens, and the word tokens of
//! the language.
//!
//! The engine's lexer emits whole tokens and drops the `IGNORE` set
//! (spaces, line ends, comments) between rules, so a rule never sees
//! where a line begins. Layout needs exactly that, so this matcher runs
//! ahead of the engine's built-in bands (`options.lex.match` order `1e5`,
//! below the first band at `1e6`) and, at each line end outside explicit
//! delimiters, reads the indentation of the next non-blank, non-comment
//! line and turns it into a structural token:
//!
//! - `#IN` -- the line is exactly two spaces deeper than the enclosing
//!   level: a block of children opens;
//! - `#NL` -- the line is at the same level: a new logical line;
//! - `#DE` -- the line is shallower: one token per level closed.
//!
//! One matcher call returns one token, and the engine does not call a
//! matcher again for the same position unless the cursor stayed put, so a
//! dedent of several levels is issued one token per call: the first `#DE`
//! consumes the line end and the indentation, the count still owed is kept
//! in the context bag, and the following calls, made at the first content
//! character of the line, return one `#DE` each with an empty source until
//! the count is zero. This is the mechanism `yaml` uses for its queued
//! tokens (a token whose source is empty and whose cursor does not move
//! is a legitimate result of an imperative matcher).
//!
//! Levels still open at the end of the source are not closed here. The
//! lexer answers `#ZZ` as soon as the cursor reaches the end without
//! consulting any matcher, so a source whose last line has no trailing
//! newline never gives this matcher a turn to emit them; the grammar
//! therefore closes every open block on `#ZZ` as well as on `#DE`, and a
//! trailing run of blank and comment lines is left to the engine's own
//! trivia matchers.
//!
//! A line ends at a line feed, alone or after a carriage return. A
//! carriage return anywhere else is whitespace with no layout meaning,
//! because the engine counts rows by line feeds alone (`line.row_chars`)
//! and the position a diagnostic names has to agree with the structure the
//! layout saw: were a lone `\r` a line end here, `a\r  b\r c` would be
//! three layout lines whose errors all report row 1. In a line's leading
//! whitespace a lone `\r` restarts the indentation, as it restarts the
//! engine's column: the indentation of a line is its run of spaces after
//! the last lone `\r` before its first form, so the column a diagnostic
//! names for that form is the indentation the layout read plus one, and a
//! line holding only whitespace and comments after a lone `\r` is a blank
//! line. (Before this was so, such a line was a layout line with no form
//! on it, and a block indented under it reached the reader as a malformed
//! node.) The engine restarts the column in its own line matcher, which
//! a layout token that swallowed the `\r` would bypass (a matcher moving
//! the cursor restarts it at a line feed alone), so a layout token ends
//! at the first lone `\r` on its form's row and the engine consumes the
//! rest as the whitespace it is. The engine does end a `;` comment at a
//! lone `\r` (its `line.chars`), and the delimiter scan below follows it
//! there.
//!
//! Layout is suspended while the explicit-delimiter depth (`(` and `[`
//! opened minus `)` and `]` closed, strings and comments excluded) is
//! above zero: there, line ends and indentation are ordinary whitespace
//! and fall through to the engine's space and line matchers. The depth is
//! computed from the source by position, in an incremental scan cached in
//! the bag, rather than by counting the delimiter tokens this matcher is
//! asked about, so it cannot drift should a position ever be lexed twice.
//!
//! Nesting is bounded here, where the depth is known before anything is
//! built: a program nests at most [`MAX_NESTING`] levels, counting a
//! layout line, each indentation level and each open delimiter as one, and
//! the opener or the indented line that would pass the bound is
//! `too_deep`. Every tree the reader builds therefore stays within the
//! bound the `ast` module documents, and a two-kilobyte program of nested
//! parentheses fails as a `DSL_PARSE_ERROR` rather than overflowing the
//! stack in the recursive stages that follow the parse.
//!
//! The same matcher owns the word tokens, because their boundaries are
//! the symbol alphabet and not the engine's delimiter set: a maximal run
//! of `[A-Za-z0-9_\-?!*+/<>=.$%&|^~@]` is a JSON number (`#NR`, the
//! lexeme kept as the token source), one of `true`, `false`, `null`
//! (`#VL`), or a symbol (`#TX`); `:name` is a keyword (`#KW`, the name as
//! the value). Strings and comments stay with the engine's string and
//! comment matchers, configured in the grammar document.
//!
//! Errors this matcher raises, by code: `tab_indent` (a tab in the
//! indentation of a content line), `bad_indent` (an indented first line,
//! or a line deeper than its parent by anything but two spaces),
//! `bad_dedent` (a dedent to a column no open block has), `unbalanced` (a
//! closing delimiter with nothing open) and `too_deep`. The grammar
//! document declares their messages and hints.

use std::sync::Arc;

use tabnas::{Context, Lexer, Rule, Tabnas, Token, Value, TIN_NR, TIN_TX, TIN_VL};

use crate::ast::MAX_NESTING;

/// The reference name the grammar document's `options.lex.match` uses.
pub(crate) const MATCHER: &str = "@alchemy-layout";

/// The structural tokens, registered by name before the grammar names
/// them.
pub(crate) const IN: &str = "#IN";
pub(crate) const DE: &str = "#DE";
pub(crate) const NL: &str = "#NL";
/// A keyword token; its value is the name without the colon.
pub(crate) const KW: &str = "#KW";

/// The one key this matcher owns in the context bag, prefixed so a grammar
/// layered on this one cannot collide with it by accident.
///
/// Its value is an array of the state's fields, at the `S_*` indexes, and
/// not an object with a key per field: this matcher is called for every
/// token and every run of whitespace, so its state is loaded with one
/// lookup and stored with one in-place write, and a call allocates
/// nothing once the array exists.
const K_STATE: &str = "alchemyLayout";
const S_SEEN: usize = 0;
const S_LEVELS: usize = 1;
const S_PENDING: usize = 2;
const S_DEPTH: usize = 3;
const S_DEPTH_POS: usize = 4;
const S_IN_STRING: usize = 5;
const S_IN_COMMENT: usize = 6;
const S_LEN: usize = 7;

/// Register the tokens and the matcher on `parser`, ahead of the grammar
/// document that names them.
pub(crate) fn register(parser: &mut Tabnas) {
    for name in [IN, DE, NL, KW] {
        parser.token(name);
    }
    parser.imperative_lex_match_ref(MATCHER, layout_matcher);
}

/// Whether `c` belongs to the symbol alphabet
/// `[A-Za-z0-9_\-?!*+/<>=.$%&|^~@]`.
pub fn is_symbol_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(
            c,
            '_' | '-'
                | '?'
                | '!'
                | '*'
                | '+'
                | '/'
                | '<'
                | '>'
                | '='
                | '.'
                | '$'
                | '%'
                | '&'
                | '|'
                | '^'
                | '~'
                | '@'
        )
}

/// Whether `text` is exactly a JSON number:
/// `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`. Written out rather
/// than as a regular expression so the crate takes no regex dependency
/// for one pattern.
pub fn is_json_number(text: &str) -> bool {
    let b = text.as_bytes();
    let mut i = 0;
    if b.get(i) == Some(&b'-') {
        i += 1;
    }
    match b.get(i) {
        Some(b'0') => i += 1,
        Some(c) if c.is_ascii_digit() => {
            while b.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
        }
        _ => return false,
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let start = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    i == b.len()
}

/// The layout state of one parse, as this matcher keeps it between calls.
///
/// It lives in `Context::u` because [`Tabnas::parse`] takes `&self` and
/// the instance is shared between parses, so nothing written during a
/// parse may live on the instance. The bag holds engine [`Value`]s, so the
/// state is loaded before each call and stored after the calls that
/// changed it. Seven scalars, `Copy`, so that comparison costs nothing;
/// the initial state, before any token, is every field at zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LayoutState {
    /// Whether any content token has been lexed yet. Before the first one
    /// there is no line for a layout token to end, so indentation may only
    /// be zero and no `#NL`/`#IN`/`#DE` is issued.
    pub(crate) seen: bool,
    /// The open indentation levels below the top one. A block opens
    /// exactly two spaces deeper than its parent and a dedent returns to a
    /// level that is open, so the open levels are always `0, 2, ...,
    /// 2 * levels`, and the count is the whole stack.
    pub(crate) levels: usize,
    /// `#DE` tokens still owed for the dedent in progress.
    pub(crate) pending: usize,
    /// The delimiter-depth scan: the depth at `depth_pos`, and whether
    /// that position is inside a string or a comment.
    pub(crate) depth: usize,
    pub(crate) depth_pos: usize,
    pub(crate) in_string: bool,
    pub(crate) in_comment: bool,
}

fn flag(fields: &[Value], index: usize) -> bool {
    matches!(fields.get(index), Some(Value::Bool(true)))
}

fn count(fields: &[Value], index: usize) -> usize {
    match fields.get(index) {
        // Every value stored here is a small count written by `store`, so
        // the conversion is exact; the guard only keeps a foreign write
        // from turning into a huge count.
        Some(Value::Number(n)) if n.is_finite() && *n >= 0.0 => *n as usize,
        _ => 0,
    }
}

impl LayoutState {
    /// The indentation, in spaces, of the innermost open level.
    pub(crate) fn top(&self) -> usize {
        2 * self.levels
    }

    /// The state stored in `context`, or the initial state on the first
    /// call of a parse (the bag starts empty).
    pub(crate) fn load(context: &Context) -> Self {
        let Some(Value::Array(fields)) = context.u.get(K_STATE) else {
            return LayoutState::default();
        };
        LayoutState {
            seen: flag(fields, S_SEEN),
            levels: count(fields, S_LEVELS),
            pending: count(fields, S_PENDING),
            depth: count(fields, S_DEPTH),
            depth_pos: count(fields, S_DEPTH_POS),
            in_string: flag(fields, S_IN_STRING),
            in_comment: flag(fields, S_IN_COMMENT),
        }
    }

    /// Write the state into `context`, over the array a previous call
    /// left when there is one.
    pub(crate) fn store(&self, context: &mut Context) {
        let fields = [
            Value::Bool(self.seen),
            Value::Number(self.levels as f64),
            Value::Number(self.pending as f64),
            Value::Number(self.depth as f64),
            Value::Number(self.depth_pos as f64),
            Value::Bool(self.in_string),
            Value::Bool(self.in_comment),
        ];
        if let Some(Value::Array(current)) = context.u.get_mut(K_STATE) {
            if current.len() == S_LEN {
                // The bag is the array's only holder, so this is in place.
                for (slot, field) in Arc::make_mut(current).iter_mut().zip(fields) {
                    *slot = field;
                }
                return;
            }
        }
        context
            .u
            .insert(K_STATE.to_string(), Value::array(fields.into()));
    }

    /// The explicit-delimiter depth just before byte `target` of `src`:
    /// `(` and `[` opened minus `)` and `]` closed, ignoring delimiters
    /// inside a string (JSON escapes honoured) or a `;` comment. The scan
    /// continues from where it last stopped; asking about an earlier
    /// position restarts it from the beginning.
    pub(crate) fn depth_at(&mut self, src: &str, target: usize) -> usize {
        let target = target.min(src.len());
        if target < self.depth_pos {
            self.depth = 0;
            self.depth_pos = 0;
            self.in_string = false;
            self.in_comment = false;
        }
        let bytes = src.as_bytes();
        let mut i = self.depth_pos;
        while i < target {
            let c = bytes[i];
            if self.in_string {
                match c {
                    b'\\' => i += 1,
                    b'"' | b'\n' | b'\r' => self.in_string = false,
                    _ => {}
                }
            } else if self.in_comment {
                // The engine ends a line comment at either character.
                if c == b'\n' || c == b'\r' {
                    self.in_comment = false;
                }
            } else {
                match c {
                    b'(' | b'[' => self.depth += 1,
                    b')' | b']' => self.depth = self.depth.saturating_sub(1),
                    b'"' => self.in_string = true,
                    b';' => self.in_comment = true,
                    _ => {}
                }
            }
            i += 1;
        }
        self.depth_pos = target;
        self.depth
    }
}

/// What one matcher call decided.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Decision {
    /// Not this matcher's: leave the position to the engine's bands.
    Pass,
    /// A word or keyword token over `len` bytes of source.
    Word {
        name: &'static str,
        tin: tabnas::Tin,
        value: Value,
        len: usize,
    },
    /// A layout token whose source is the `len` bytes of line end and
    /// indentation it consumed; `pending` further `#DE` are owed.
    Layout {
        name: &'static str,
        len: usize,
        pending: usize,
    },
    /// An error with this matcher's code, reported at byte `at`. When a
    /// lone `\r` at `row_cr` precedes `at` on its row, the column the
    /// error names counts from the character after it, as the engine's
    /// own line matcher would have had it.
    Bad {
        code: &'static str,
        at: usize,
        row_cr: Option<usize>,
    },
}

/// Decide what `src` holds at byte `si`, updating `state`.
pub(crate) fn decide(src: &str, si: usize, state: &mut LayoutState) -> Decision {
    if state.pending > 0 {
        state.pending -= 1;
        return Decision::Layout {
            name: DE,
            len: 0,
            pending: state.pending,
        };
    }
    let rest = &src[si..];
    let Some(first) = rest.chars().next() else {
        return Decision::Pass;
    };

    // The start of the source is a line start with no line end before it.
    if si == 0 && !state.seen && (first == ' ' || first == '\t') {
        return line_start(src, 0, false, state);
    }

    match first {
        '\n' => line_end(src, si, state),
        '\r' if rest.as_bytes().get(1) == Some(&b'\n') => line_end(src, si, state),
        // A carriage return with no line feed after it is whitespace: the
        // engine does not count it as a row (see the module docs). Before
        // the first line it restarts the indentation the first line must
        // not have, so it is read as a line start there.
        '\r' if !state.seen => line_start(src, si, false, state),
        ' ' | '\t' | '\r' | ';' => Decision::Pass,
        '(' | '[' => {
            state.seen = true;
            // The depth just after this opener, with the layout line and
            // the indentation levels around it, is the nesting it opens.
            let nesting = 1 + state.levels + state.depth_at(src, si + 1);
            if nesting > MAX_NESTING {
                Decision::Bad {
                    code: "too_deep",
                    at: si,
                    row_cr: None,
                }
            } else {
                Decision::Pass
            }
        }
        ')' | ']' => {
            state.seen = true;
            if state.depth_at(src, si) == 0 {
                Decision::Bad {
                    code: "unbalanced",
                    at: si,
                    row_cr: None,
                }
            } else {
                Decision::Pass
            }
        }
        ':' => {
            state.seen = true;
            let name_len = symbol_run(&rest[1..]);
            if name_len == 0 {
                // A bare colon is nothing in this language; the engine
                // reports it as `unexpected` when no band claims it.
                return Decision::Pass;
            }
            Decision::Word {
                name: KW,
                tin: -1,
                value: Value::String(rest[1..1 + name_len].to_string()),
                len: 1 + name_len,
            }
        }
        c if is_symbol_char(c) => {
            state.seen = true;
            let len = symbol_run(rest);
            let word = &rest[..len];
            if is_json_number(word) {
                // `parse` accepts every JSON number; an exponent out of
                // range saturates to an infinity, and the AST keeps the
                // lexeme rather than this value.
                let value = word.parse::<f64>().unwrap_or(f64::NAN);
                Decision::Word {
                    name: "#NR",
                    tin: TIN_NR,
                    value: Value::Number(value),
                    len,
                }
            } else {
                match word {
                    "true" | "false" => Decision::Word {
                        name: "#VL",
                        tin: TIN_VL,
                        value: Value::Bool(word == "true"),
                        len,
                    },
                    "null" => Decision::Word {
                        name: "#VL",
                        tin: TIN_VL,
                        value: Value::Null,
                        len,
                    },
                    _ => Decision::Word {
                        name: "#TX",
                        tin: TIN_TX,
                        value: Value::String(word.to_string()),
                        len,
                    },
                }
            }
        }
        _ => {
            // A quote, or something no band will claim: the engine's
            // matchers decide, but a line has begun either way.
            state.seen = true;
            Decision::Pass
        }
    }
}

/// A line end at `si`: a layout decision outside delimiters, whitespace
/// inside them.
fn line_end(src: &str, si: usize, state: &mut LayoutState) -> Decision {
    if state.depth_at(src, si) > 0 {
        Decision::Pass
    } else {
        line_start(src, si, true, state)
    }
}

/// The byte length of the symbol-alphabet run at the start of `text`.
fn symbol_run(text: &str) -> usize {
    text.find(|c: char| !is_symbol_char(c))
        .unwrap_or(text.len())
}

/// The byte after the line terminator at `pos` (`\n`, or `\r\n`), or
/// `pos` when there is none.
fn after_terminator(bytes: &[u8], pos: usize) -> usize {
    match (bytes.get(pos), bytes.get(pos + 1)) {
        (Some(b'\n'), _) => pos + 1,
        (Some(b'\r'), Some(b'\n')) => pos + 2,
        _ => pos,
    }
}

/// From a line start at `from` (just before its terminator when
/// `after_newline`), skip blank and comment-only lines to the next content
/// line and decide the layout token its indentation calls for.
///
/// The indentation of a line is its run of leading spaces after the last
/// lone carriage return before its first other character. A lone `\r` is
/// whitespace that restarts the engine's column, so it restarts the count
/// too, and what follows it on the same row is read as the row's leading
/// whitespace was: a comment there, which the engine ends at the next
/// lone `\r` or at the line end, counts for nothing. The first lone `\r`
/// on the content line's own row is where the layout token ends (see the
/// module docs), and where an error's column counts from.
fn line_start(src: &str, from: usize, after_newline: bool, state: &mut LayoutState) -> Decision {
    let bytes = src.as_bytes();
    let mut pos = if after_newline {
        after_terminator(bytes, from)
    } else {
        from
    };
    loop {
        let mut spaces = 0;
        let mut tab_at = None;
        let mut row_cr = None;
        loop {
            while let Some(c) = bytes.get(pos) {
                match c {
                    b' ' => spaces += 1,
                    b'\t' => {
                        tab_at.get_or_insert(pos);
                    }
                    _ => break,
                }
                pos += 1;
            }
            // A comment runs to the line end and counts for nothing.
            if bytes.get(pos) == Some(&b';') {
                while bytes.get(pos).is_some_and(|c| *c != b'\n' && *c != b'\r') {
                    pos += 1;
                }
            }
            // A lone carriage return restarts the indentation.
            if bytes.get(pos) == Some(&b'\r') && bytes.get(pos + 1) != Some(&b'\n') {
                row_cr.get_or_insert(pos);
                pos += 1;
                spaces = 0;
                tab_at = None;
                continue;
            }
            break;
        }
        match bytes.get(pos) {
            // Only trivia to the end: the engine's own matchers eat it,
            // and `#ZZ` closes what is open.
            None => return Decision::Pass,
            Some(b'\n') => pos += 1,
            Some(b'\r') if bytes.get(pos + 1) == Some(&b'\n') => pos += 2,
            Some(_) => {
                if let Some(at) = tab_at {
                    return Decision::Bad {
                        code: "tab_indent",
                        at,
                        row_cr,
                    };
                }
                return layout_token(spaces, pos, from, row_cr, state);
            }
        }
    }
}

/// The layout token for a content line indented by `indent` spaces whose
/// first character is at `content`, when the matcher was called at `from`.
/// The token runs to `content`, or to the first lone `\r` on its row
/// (`row_cr`), which the engine consumes with what follows it.
fn layout_token(
    indent: usize,
    content: usize,
    from: usize,
    row_cr: Option<usize>,
    state: &mut LayoutState,
) -> Decision {
    if !state.seen {
        return if indent == 0 {
            Decision::Pass
        } else {
            Decision::Bad {
                code: "bad_indent",
                at: content,
                row_cr,
            }
        };
    }
    let len = row_cr.unwrap_or(content) - from;
    let top = state.top();
    if indent == top {
        return Decision::Layout {
            name: NL,
            len,
            pending: 0,
        };
    }
    if indent == top + 2 {
        // The new level, with the layout line it opens, is the nesting.
        if 1 + state.levels + 1 > MAX_NESTING {
            return Decision::Bad {
                code: "too_deep",
                at: content,
                row_cr,
            };
        }
        state.levels += 1;
        return Decision::Layout {
            name: IN,
            len,
            pending: 0,
        };
    }
    if indent > top {
        return Decision::Bad {
            code: "bad_indent",
            at: content,
            row_cr,
        };
    }
    // A dedent returns to an open level: an even indentation below the
    // top, since the open levels are exactly the even indentations up to
    // it.
    if indent % 2 != 0 {
        return Decision::Bad {
            code: "bad_dedent",
            at: content,
            row_cr,
        };
    }
    let pops = (top - indent) / 2;
    state.levels -= pops;
    // The first `#DE` goes out now with the consumed trivia; the rest are
    // owed and issued on the following calls, which stand at `content`.
    state.pending = pops - 1;
    Decision::Layout {
        name: DE,
        len,
        pending: state.pending,
    }
}

/// The matcher itself, in the shape [`Tabnas::imperative_lex_match_ref`]
/// takes: decide against the immutable source, then move the cursor and
/// build the token at the point captured before the move.
///
/// The state is stored only when the call changed it, so the common calls
/// (a word, a run of spaces) load it and write nothing.
fn layout_matcher(lexer: &mut Lexer<'_>, _rule: &mut Rule, context: &mut Context) -> Option<Token> {
    let point = lexer.point();
    let si = point.site.si;
    let loaded = LayoutState::load(context);
    let mut state = loaded;
    let decision = decide(lexer.source(), si, &mut state);
    if state != loaded {
        state.store(context);
    }

    let advance = |lexer: &mut Lexer<'_>, len: usize| -> String {
        let text = lexer.source()[si..si + len].to_string();
        let chars = text.chars().count();
        if chars > 0 {
            lexer.advance_chars(chars);
        }
        text
    };

    match decision {
        Decision::Pass => None,
        Decision::Word {
            name,
            tin,
            value,
            len,
        } => {
            let text = advance(lexer, len);
            Some(Token::new(name, tin, value, text, point))
        }
        Decision::Layout { name, len, .. } => {
            let text = advance(lexer, len);
            // A tin of -1 asks the engine to resolve the identity from the
            // name; the names are registered before the grammar installs.
            Some(Token::new(name, -1, Value::Undefined, text, point))
        }
        Decision::Bad { code, at, row_cr } => {
            advance(lexer, at - si);
            let mut token = lexer.bad(code);
            // The cursor's column counts the lone `\r` it moved over as a
            // character; the engine's line matcher restarts it there.
            if let Some(cr) = row_cr {
                token.site.ci = lexer.source()[cr + 1..at].chars().count() + 1;
            }
            Some(token)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(src: &str) -> Vec<(String, String)> {
        // Drive `decide` the way the lexer would over a source with no
        // strings or comments: this matcher's tokens by name and source,
        // and `_` for a position it passed on (advanced by one byte).
        let mut state = LayoutState::default();
        let mut out = Vec::new();
        let mut si = 0;
        while si < src.len() {
            match decide(src, si, &mut state) {
                Decision::Pass => {
                    out.push(("_".to_string(), src[si..si + 1].to_string()));
                    si += 1;
                }
                Decision::Word { name, len, .. } => {
                    out.push((name.to_string(), src[si..si + len].to_string()));
                    si += len;
                }
                Decision::Layout { name, len, .. } => {
                    out.push((name.to_string(), src[si..si + len].to_string()));
                    si += len;
                }
                Decision::Bad { code, .. } => {
                    out.push(("BAD".to_string(), code.to_string()));
                    break;
                }
            }
        }
        out
    }

    fn names(src: &str) -> String {
        words(src)
            .into_iter()
            .map(|(name, text)| match name.as_str() {
                "_" => text,
                "#IN" | "#DE" | "#NL" => format!(" {name} "),
                "BAD" => format!(" BAD:{text} "),
                _ => format!("{name}({text})"),
            })
            .collect::<Vec<_>>()
            .join("")
    }

    /// `levels + 1` lines, each two spaces deeper than the one before.
    fn staircase(levels: usize) -> String {
        (0..=levels)
            .map(|level| format!("{}x", "  ".repeat(level)))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn json_numbers_are_recognised_exactly() {
        for ok in ["0", "-0", "12", "1.5", "-1.5e10", "2E-3", "0.0"] {
            assert!(is_json_number(ok), "{ok} is a JSON number");
        }
        for bad in ["01", "1.", ".5", "+1", "1e", "-", "1_000", "0x1f", "1a"] {
            assert!(!is_json_number(bad), "{bad} is not a JSON number");
        }
    }

    #[test]
    fn words_split_into_numbers_values_and_symbols() {
        assert_eq!(
            names("def a-b? -1 1.5e3 true null :key x"),
            "#TX(def) #TX(a-b?) #NR(-1) #NR(1.5e3) #VL(true) #VL(null) #KW(:key) #TX(x)"
        );
    }

    #[test]
    fn indentation_becomes_structural_tokens() {
        assert_eq!(
            names("a\n  b\n    c\n  d\ne"),
            "#TX(a) #IN #TX(b) #IN #TX(c) #DE #TX(d) #DE #TX(e)"
        );
    }

    #[test]
    fn a_dedent_of_two_levels_is_issued_one_token_per_call() {
        let mut state = LayoutState::default();
        let src = "a\n  b\n    c\nd";
        // Lex to the line end after `c`.
        let at = src.find("c\n").unwrap() + 1;
        for (si, _) in src.char_indices().take_while(|(si, _)| *si < at) {
            let _ = decide(src, si, &mut state);
        }
        // What the calls so far built: the two levels `b` and `c` opened.
        assert_eq!(state.levels, 2);
        let first = decide(src, at, &mut state);
        assert_eq!(
            first,
            Decision::Layout {
                name: DE,
                len: 1,
                pending: 1
            }
        );
        let second = decide(src, at + 1, &mut state);
        assert_eq!(
            second,
            Decision::Layout {
                name: DE,
                len: 0,
                pending: 0
            }
        );
        assert_eq!(state.levels, 0);
        assert!(matches!(
            decide(src, at + 1, &mut state),
            Decision::Word { name: "#TX", .. }
        ));
    }

    #[test]
    fn blank_and_comment_lines_do_not_count() {
        assert_eq!(
            names("a\n\n  ; note\n   \n  b\n\n"),
            "#TX(a) #IN #TX(b)\n\n"
        );
    }

    #[test]
    fn a_line_ends_at_a_line_feed_and_a_lone_carriage_return_is_whitespace() {
        assert_eq!(names("a\r\n  b\r\nc"), "#TX(a) #IN #TX(b) #DE #TX(c)");
        // No layout token: one line, as the engine counts it.
        assert_eq!(names("a\r  b\r c"), "#TX(a)\r  #TX(b)\r #TX(c)");
        assert_eq!(names("a \r b"), "#TX(a) \r #TX(b)");
        // In leading whitespace it restarts the indentation, as it restarts
        // the engine's column: the indentation is what follows it. The
        // layout token ends at the first lone `\r` on the row, left to the
        // engine (here, passed byte by byte).
        assert_eq!(names("a\n\r  b"), "#TX(a) #IN \r  #TX(b)");
        assert_eq!(names("a\n  \r  b"), "#TX(a) #IN \r  #TX(b)");
        assert_eq!(names("a\n  b\n  \rc"), "#TX(a) #IN #TX(b) #DE \r#TX(c)");
        assert_eq!(
            names("a\n  b\n  \r  \r  c"),
            "#TX(a) #IN #TX(b) #NL \r  \r  #TX(c)"
        );
        assert_eq!(names("a\n\t\r  b"), "#TX(a) #IN \r  #TX(b)");
        assert_eq!(names("\r  a"), " BAD:bad_indent ");
        assert_eq!(names("\ra"), "\r#TX(a)");
        assert_eq!(names("a\r\n\r  b"), "#TX(a) #IN \r  #TX(b)");
    }

    #[test]
    fn a_line_holding_only_trivia_after_a_lone_carriage_return_is_blank() {
        // No layout token for a row with nothing on it but whitespace and
        // comments after a lone `\r`: the next content line decides.
        assert_eq!(names("f x\n\r"), "#TX(f) #TX(x)\n\r");
        assert_eq!(names("f x\n  \r  "), "#TX(f) #TX(x)\n  \r  ");
        assert_eq!(names("a\n  b\n  \r;c\nd"), "#TX(a) #IN #TX(b) #DE #TX(d)");
        assert_eq!(names("a\n  b\n\r;x\n  c"), "#TX(a) #IN #TX(b) #NL #TX(c)");
        assert_eq!(names("a\n;x\r  c"), "#TX(a) #IN \r  #TX(c)");
        assert_eq!(names("\r;x\n  a"), " BAD:bad_indent ");
    }

    #[test]
    fn an_error_after_a_lone_carriage_return_counts_its_column_from_it() {
        let mut state = LayoutState::default();
        assert!(matches!(
            decide("  \r  a", 0, &mut state),
            Decision::Bad {
                code: "bad_indent",
                at: 5,
                row_cr: Some(2)
            }
        ));
        let mut state = LayoutState {
            seen: true,
            ..LayoutState::default()
        };
        assert!(matches!(
            decide("a\n\r \tb", 1, &mut state),
            Decision::Bad {
                code: "tab_indent",
                at: 4,
                row_cr: Some(2)
            }
        ));
        let mut state = LayoutState {
            seen: true,
            ..LayoutState::default()
        };
        assert!(matches!(
            decide("a\n  \r   b", 1, &mut state),
            Decision::Bad {
                code: "bad_indent",
                at: 8,
                row_cr: Some(4)
            }
        ));
    }

    #[test]
    fn layout_is_suspended_inside_delimiters() {
        assert_eq!(
            names("a (b\n  c)\n  d"),
            "#TX(a) (#TX(b)\n  #TX(c)) #IN #TX(d)"
        );
        assert_eq!(names("[1\n2]"), "[#NR(1)\n#NR(2)]");
    }

    #[test]
    fn a_stray_closer_is_unbalanced() {
        assert_eq!(names("a )"), "#TX(a)  BAD:unbalanced ");
        assert_eq!(names("(a))"), "(#TX(a)) BAD:unbalanced ");
    }

    #[test]
    fn indentation_errors_have_their_own_codes() {
        assert_eq!(names("a\n\tb"), "#TX(a) BAD:tab_indent ");
        assert_eq!(names("a\n   b"), "#TX(a) BAD:bad_indent ");
        assert_eq!(names("a\n  b\n c"), "#TX(a) #IN #TX(b) BAD:bad_dedent ");
        assert_eq!(names("  a"), " BAD:bad_indent ");
        // Decided at the first line end, before any of the trivia is lexed.
        assert_eq!(names("\n\n  a"), " BAD:bad_indent ");
    }

    #[test]
    fn nesting_past_the_bound_is_too_deep_at_the_opener_or_the_line() {
        // A layout line and 255 open parens are 256 levels; the 256th
        // paren is one more.
        let at_limit = "(".repeat(MAX_NESTING - 1);
        assert_eq!(names(&at_limit), at_limit);
        assert_eq!(
            names(&"(".repeat(MAX_NESTING)),
            format!("{at_limit} BAD:too_deep ")
        );
        // Indentation levels count the same way, together with delimiters.
        assert!(!names(&staircase(MAX_NESTING - 1)).contains("BAD"));
        assert!(names(&staircase(MAX_NESTING)).ends_with(" BAD:too_deep "));
        let mixed = format!("{}\n{}[", staircase(2), "  ".repeat(3));
        assert!(names(&format!("{mixed}{}", "(".repeat(MAX_NESTING - 5))).ends_with('('));
        assert!(
            names(&format!("{mixed}{}", "(".repeat(MAX_NESTING - 4))).ends_with(" BAD:too_deep ")
        );
    }

    #[test]
    fn leading_trivia_before_the_first_line_issues_nothing() {
        assert_eq!(names("\n\n  \na"), "\n\n  \n#TX(a)");
    }

    #[test]
    fn the_depth_scan_ignores_strings_and_comments() {
        let mut state = LayoutState::default();
        let src = "(\"a)\" ; )\n)";
        assert_eq!(state.depth_at(src, src.len() - 1), 1);
        assert_eq!(state.depth_at(src, src.len()), 0);
        // Asking about an earlier position restarts the scan.
        assert_eq!(state.depth_at(src, 1), 1);
    }

    #[test]
    fn state_round_trips_through_the_context_bag() {
        // The bag belongs to a live parse, so the state is observed from a
        // lexer subscriber over the real grammar: after every token, what
        // `store` left is what `load` reads back.
        use std::sync::Mutex;
        let seen: Arc<Mutex<Vec<LayoutState>>> = Arc::new(Mutex::new(Vec::new()));
        let mut parser = crate::make();
        let record = Arc::clone(&seen);
        parser.subscribe_lex(move |_token, _rule, context| {
            if let Ok(mut states) = record.lock() {
                states.push(LayoutState::load(context));
            }
        });
        parser.parse("a\n  b\n    c\nd").expect("parses");
        let states = seen.lock().expect("no panic held the lock");
        assert!(
            states.iter().any(|state| state.levels == 2 && state.seen),
            "some token was lexed three levels deep: {states:?}"
        );
        let last = states.last().expect("tokens were lexed");
        assert_eq!(
            (last.seen, last.levels, last.pending, last.depth),
            (true, 0, 0, 0)
        );
    }

    #[test]
    fn the_store_writes_in_place_and_only_what_changed() {
        // A stored state is one array under one key, overwritten rather
        // than replaced: the same allocation before and after.
        use std::sync::Mutex;
        let identities: Arc<Mutex<Vec<(usize, usize)>>> = Arc::new(Mutex::new(Vec::new()));
        let mut parser = crate::make();
        let record = Arc::clone(&identities);
        parser.subscribe_lex(move |_token, _rule, context| {
            if let (Ok(mut seen), Some(Value::Array(fields))) =
                (record.lock(), context.u.get(K_STATE))
            {
                seen.push((Arc::as_ptr(fields) as usize, context.u.len()));
            }
        });
        parser.parse("a b\n  c d\n(e\n f)\nx").expect("parses");
        let seen = identities.lock().expect("no panic held the lock");
        assert!(seen.len() > 4, "several tokens were lexed: {seen:?}");
        assert!(
            seen.windows(2).all(|pair| pair[0] == pair[1]),
            "one array, one key, throughout: {seen:?}"
        );
    }
}
