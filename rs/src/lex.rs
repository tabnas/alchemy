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
//! Layout is suspended while the explicit-delimiter depth (`(` and `[`
//! opened minus `)` and `]` closed, strings and comments excluded) is
//! above zero: there, line ends and indentation are ordinary whitespace
//! and fall through to the engine's space and line matchers. The depth is
//! computed from the source by position, in an incremental scan cached in
//! the bag, rather than by counting the delimiter tokens this matcher is
//! asked about, so it cannot drift should a position ever be lexed twice.
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
//! `bad_dedent` (a dedent to a column no open block has) and `unbalanced`
//! (a closing delimiter with nothing open). The grammar document declares
//! their messages and hints.

use std::sync::Arc;

use tabnas::{Context, Lexer, Rule, Tabnas, Token, Value, TIN_NR, TIN_TX, TIN_VL};

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
    /// An error with this matcher's code, reported at byte `at`.
    Bad { code: &'static str, at: usize },
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
        '\n' | '\r' => {
            if state.depth_at(src, si) > 0 {
                Decision::Pass
            } else {
                line_start(src, si, true, state)
            }
        }
        ' ' | '\t' | ';' => Decision::Pass,
        ')' | ']' => {
            state.seen = true;
            if state.depth_at(src, si) == 0 {
                Decision::Bad {
                    code: "unbalanced",
                    at: si,
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
            // A delimiter, a quote, or something no band will claim: the
            // engine's matchers decide, but a line has begun either way.
            state.seen = true;
            Decision::Pass
        }
    }
}

/// The byte length of the symbol-alphabet run at the start of `text`.
fn symbol_run(text: &str) -> usize {
    text.find(|c: char| !is_symbol_char(c))
        .unwrap_or(text.len())
}

/// One line terminator at `pos` (`\r\n`, `\n` or `\r`), or `pos` when
/// there is none.
fn after_terminator(bytes: &[u8], pos: usize) -> usize {
    match bytes.get(pos) {
        Some(b'\r') if bytes.get(pos + 1) == Some(&b'\n') => pos + 2,
        Some(b'\r' | b'\n') => pos + 1,
        _ => pos,
    }
}

/// From a line start at `from` (just before its terminator when
/// `after_newline`), skip blank and comment-only lines to the next content
/// line and decide the layout token its indentation calls for.
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
        match bytes.get(pos) {
            // Only trivia to the end: the engine's own matchers eat it,
            // and `#ZZ` closes what is open.
            None => return Decision::Pass,
            Some(b'\n' | b'\r') => pos = after_terminator(bytes, pos),
            Some(b';') => {
                while bytes.get(pos).is_some_and(|c| *c != b'\n' && *c != b'\r') {
                    pos += 1;
                }
            }
            Some(_) => {
                if let Some(at) = tab_at {
                    return Decision::Bad {
                        code: "tab_indent",
                        at,
                    };
                }
                return layout_token(spaces, pos, from, state);
            }
        }
    }
}

/// The layout token for a content line indented by `indent` spaces whose
/// first character is at `content`, when the matcher was called at `from`.
fn layout_token(indent: usize, content: usize, from: usize, state: &mut LayoutState) -> Decision {
    if !state.seen {
        return if indent == 0 {
            Decision::Pass
        } else {
            Decision::Bad {
                code: "bad_indent",
                at: content,
            }
        };
    }
    let len = content - from;
    let top = state.top();
    if indent == top {
        return Decision::Layout {
            name: NL,
            len,
            pending: 0,
        };
    }
    if indent == top + 2 {
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
        };
    }
    // A dedent returns to an open level: an even indentation below the
    // top, since the open levels are exactly the even indentations up to
    // it.
    if indent % 2 != 0 {
        return Decision::Bad {
            code: "bad_dedent",
            at: content,
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
        Decision::Bad { code, at } => {
            advance(lexer, at - si);
            Some(lexer.bad(code))
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
