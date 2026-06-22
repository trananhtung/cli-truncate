//! # cli-truncate — width-aware string truncation for terminals
//!
//! Truncate a string to a maximum terminal **display width** with an ellipsis,
//! correctly accounting for **wide (CJK) characters** (which take two columns)
//! and **ANSI escape sequences** (which take none). Like Node's `cli-truncate`.
//!
//! ```
//! assert_eq!(cli_truncate::truncate("unicorn", 4), "uni…");
//! assert_eq!(cli_truncate::truncate("古池や蛙", 6), "古池…"); // wide chars
//! assert_eq!(cli_truncate::width("\u{1b}[31mhi\u{1b}[0m"), 2); // ANSI is zero-width
//! ```
//!
//! ## Behavior
//!
//! - Output never exceeds `max_width` columns; `max_width == 0` yields `""`. If the
//!   ellipsis itself is wider than `max_width`, it is clamped (so the bound always holds).
//! - Escape sequences are recognized and treated as zero width: CSI (`ESC [ … m`,
//!   incl. 8-bit `0x9B`), string sequences (OSC hyperlinks, DCS/SOS/PM/APC and their
//!   terminators), and two-byte / nF escapes (`ESC c`, `ESC ( B`). They are preserved
//!   intact and never split across the cut.
//! - A reset (`\x1b[0m`) is appended only when the kept text leaves a style open, so
//!   color can't leak past the cut and resets are never duplicated.
//! - Other C0/C1 control characters (newline, tab, carriage return, NUL, …) have no
//!   defined column width and are dropped from truncated output so they can't break
//!   single-line layout.
//! - For `Start`/`Middle`, styling that began in the *dropped* region isn't carried
//!   over, and a combining mark whose base was dropped is not orphaned onto the ellipsis.
//! - Truncation respects character boundaries, not grapheme clusters: a multi-codepoint
//!   cluster (a ZWJ emoji or flag) may be cut between codepoints. The column budget is
//!   always honored regardless.

#![doc(html_root_url = "https://docs.rs/cli-truncate/0.1.0")]

use unicode_width::UnicodeWidthChar;

/// The SGR sequence that resets all styling to its default.
const RESET: &str = "\u{1b}[0m";

/// Where to remove characters when truncating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Position {
    /// Drop the start, keep the end: `"…corn"`.
    Start,
    /// Drop the middle, keep both ends: `"un…rn"`.
    Middle,
    /// Drop the end, keep the start: `"uni…"`. The default.
    #[default]
    End,
}

/// Truncation options: ellipsis string and [`Position`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    ellipsis: String,
    position: Position,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            ellipsis: String::from("\u{2026}"),
            position: Position::End,
        }
    }
}

impl Options {
    /// Default options: a `…` ellipsis truncated at the [`Position::End`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the ellipsis string appended (or inserted) where text is removed.
    #[must_use]
    pub fn ellipsis(mut self, ellipsis: &str) -> Self {
        ellipsis.clone_into(&mut self.ellipsis);
        self
    }

    /// Set where characters are removed.
    #[must_use]
    pub fn position(mut self, position: Position) -> Self {
        self.position = position;
        self
    }

    /// Truncate `text` to `max_width` display columns with these options.
    #[must_use]
    pub fn truncate(&self, text: &str, max_width: usize) -> String {
        if max_width == 0 {
            return String::new();
        }
        if width(text) <= max_width {
            return text.to_owned();
        }

        let toks = tokenize(text);
        let ellipsis_width = width(&self.ellipsis);

        // The ellipsis alone doesn't fit: emit a width-clamped ellipsis, no source text.
        if ellipsis_width >= max_width {
            return clamp_visible(&self.ellipsis, max_width);
        }
        let avail = max_width - ellipsis_width;

        match self.position {
            Position::End => {
                let cut = prefix_cut(&toks, avail);
                let mut out = String::new();
                let mut open = false;
                emit(&mut out, &toks[..cut], &mut open);
                push_tracked(&mut out, &self.ellipsis, &mut open);
                close(&mut out, open);
                out
            }
            Position::Start => {
                let keep = drop_leading_zero_width(&toks, suffix_start(&toks, avail));
                let mut out = String::new();
                let mut open = false;
                push_tracked(&mut out, &self.ellipsis, &mut open);
                emit(&mut out, &toks[keep..], &mut open);
                close(&mut out, open);
                out
            }
            Position::Middle => {
                let left = avail / 2;
                let right = avail - left;
                let cut = prefix_cut(&toks, left);
                let keep = drop_leading_zero_width(&toks, suffix_start(&toks, right).max(cut));
                let mut out = String::new();
                let mut open = false;
                emit(&mut out, &toks[..cut], &mut open);
                push_tracked(&mut out, &self.ellipsis, &mut open);
                emit(&mut out, &toks[keep..], &mut open);
                close(&mut out, open);
                out
            }
        }
    }
}

/// Truncate `text` to `max_width` display columns, dropping the end and adding a
/// `…` ellipsis. Shorthand for [`Options::new`]`().truncate(text, max_width)`.
#[must_use]
pub fn truncate(text: &str, max_width: usize) -> String {
    Options::new().truncate(text, max_width)
}

/// The display width of `text` in terminal columns, ignoring escape sequences and
/// control characters and counting wide characters as two.
#[must_use]
pub fn width(text: &str) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut total = 0;
    let mut i = 0;
    while i < chars.len() {
        let (scanned, next) = scan(&chars, i);
        if let Scanned::Visible(c) = scanned {
            total += char_width(c);
        }
        i = next;
    }
    total
}

// ---------------------------------------------------------------------------
// Tokens
// ---------------------------------------------------------------------------

/// A parsed token: a zero-width escape sequence (preserved verbatim) or a visible
/// char with its column width. Control characters are dropped during tokenization.
enum Tok {
    Esc(String),
    Ch(char, usize),
}

/// Tokenize `text`, preserving escape sequences and dropping control characters.
fn tokenize(text: &str) -> Vec<Tok> {
    let chars: Vec<char> = text.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let (scanned, next) = scan(&chars, i);
        match scanned {
            Scanned::Escape(seq) => toks.push(Tok::Esc(seq)),
            Scanned::Visible(c) => toks.push(Tok::Ch(c, char_width(c))),
            Scanned::Control => {}
        }
        i = next;
    }
    toks
}

/// Display width of a single visible char (zero-width chars → 0).
fn char_width(c: char) -> usize {
    UnicodeWidthChar::width(c).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Escape / control scanner (single source of truth for width() and tokenize())
// ---------------------------------------------------------------------------

/// The classification of the token starting at a given index.
enum Scanned {
    /// A complete escape sequence (zero display width), preserved verbatim.
    Escape(String),
    /// A visible character.
    Visible(char),
    /// A C0/C1 control character with no defined column width.
    Control,
}

/// Scan one token starting at `chars[i]`, returning it and the index just past it.
/// Always advances (`next > i`).
fn scan(chars: &[char], i: usize) -> (Scanned, usize) {
    let c = chars[i];
    if c == '\u{1b}' {
        return scan_escape(chars, i);
    }
    // 8-bit C1 introducers behave like their 7-bit ESC-prefixed forms.
    match c {
        '\u{9b}' => return scan_csi(chars, i + 1, i),
        '\u{90}' | '\u{9d}' | '\u{98}' | '\u{9e}' | '\u{9f}' => {
            return scan_string_sequence(chars, i + 1, i);
        }
        _ => {}
    }
    if c.is_control() {
        return (Scanned::Control, i + 1);
    }
    (Scanned::Visible(c), i + 1)
}

/// Scan a 7-bit escape beginning with `ESC` at `chars[i]`.
fn scan_escape(chars: &[char], i: usize) -> (Scanned, usize) {
    match chars.get(i + 1).copied() {
        // Control Sequence Introducer.
        Some('[') => scan_csi(chars, i + 2, i),
        // String sequences: OSC / DCS / SOS / PM / APC.
        Some(']' | 'P' | 'X' | '^' | '_') => scan_string_sequence(chars, i + 2, i),
        // nF escape: intermediate bytes (0x20–0x2F) then a final byte (0x30–0x7E).
        Some(c) if ('\u{20}'..='\u{2f}').contains(&c) => {
            let mut j = i + 2;
            while j < chars.len() && ('\u{20}'..='\u{2f}').contains(&chars[j]) {
                j += 1;
            }
            if j < chars.len() && ('\u{30}'..='\u{7e}').contains(&chars[j]) {
                j += 1;
            }
            (Scanned::Escape(slice(chars, i, j)), j)
        }
        // Two-byte Fe/Fs escape: ESC followed by a single final byte (0x30–0x7E).
        Some(c) if ('\u{30}'..='\u{7e}').contains(&c) => {
            (Scanned::Escape(slice(chars, i, i + 2)), i + 2)
        }
        // Lone ESC or ESC + an unexpected byte: treat the ESC as a zero-width sequence.
        _ => (Scanned::Escape(slice(chars, i, i + 1)), i + 1),
    }
}

/// Scan a CSI body (params 0x30–0x3F, intermediates 0x20–0x2F, final 0x40–0x7E),
/// starting at `body` and reporting the sequence from `start`.
fn scan_csi(chars: &[char], body: usize, start: usize) -> (Scanned, usize) {
    let mut j = body;
    while j < chars.len() && ('\u{30}'..='\u{3f}').contains(&chars[j]) {
        j += 1;
    }
    while j < chars.len() && ('\u{20}'..='\u{2f}').contains(&chars[j]) {
        j += 1;
    }
    if j < chars.len() && ('\u{40}'..='\u{7e}').contains(&chars[j]) {
        j += 1;
    }
    (Scanned::Escape(slice(chars, start, j)), j)
}

/// Scan a string sequence (OSC/DCS/SOS/PM/APC) up to and including its terminator —
/// `BEL` (0x07), 8-bit `ST` (0x9C), or 7-bit `ST` (`ESC \`).
fn scan_string_sequence(chars: &[char], body: usize, start: usize) -> (Scanned, usize) {
    let mut j = body;
    while j < chars.len() {
        let c = chars[j];
        if c == '\u{07}' || c == '\u{9c}' {
            j += 1;
            break;
        }
        if c == '\u{1b}' && chars.get(j + 1) == Some(&'\\') {
            j += 2;
            break;
        }
        j += 1;
    }
    (Scanned::Escape(slice(chars, start, j)), j)
}

/// Collect `chars[start..end]` into a `String`.
fn slice(chars: &[char], start: usize, end: usize) -> String {
    chars[start..end].iter().collect()
}

// ---------------------------------------------------------------------------
// Emission helpers
// ---------------------------------------------------------------------------

/// Index just past the last visible char that fits within `avail` columns. Escape
/// tokens do not advance the cut, so trailing escapes that style only dropped text
/// are excluded.
fn prefix_cut(toks: &[Tok], avail: usize) -> usize {
    let mut used = 0;
    let mut cut = 0;
    for (idx, t) in toks.iter().enumerate() {
        if let Tok::Ch(_, w) = t {
            if used + w > avail {
                break;
            }
            used += w;
            cut = idx + 1;
        }
    }
    cut
}

/// Index of the earliest token such that `toks[i..]` has visible width ≤ `avail`.
fn suffix_start(toks: &[Tok], avail: usize) -> usize {
    let mut used = 0;
    let mut keep_from = toks.len();
    for (idx, t) in toks.iter().enumerate().rev() {
        match t {
            Tok::Ch(_, w) => {
                if used + w > avail {
                    break;
                }
                used += w;
                keep_from = idx;
            }
            Tok::Esc(_) => keep_from = idx,
        }
    }
    keep_from
}

/// Advance past leading zero-width chars so a kept suffix never starts with a
/// combining mark (or ZWJ) whose base character was dropped.
fn drop_leading_zero_width(toks: &[Tok], mut from: usize) -> usize {
    while let Some(Tok::Ch(_, 0)) = toks.get(from) {
        from += 1;
    }
    from
}

/// Append a token slice to `out`, tracking whether a style is left open.
fn emit(out: &mut String, toks: &[Tok], open: &mut bool) {
    for t in toks {
        match t {
            Tok::Ch(c, _) => out.push(*c),
            Tok::Esc(s) => {
                out.push_str(s);
                update_style(open, s);
            }
        }
    }
}

/// Append a raw string to `out`, tracking any style its escape sequences set.
fn push_tracked(out: &mut String, s: &str, open: &mut bool) {
    out.push_str(s);
    for t in tokenize(s) {
        if let Tok::Esc(seq) = t {
            update_style(open, &seq);
        }
    }
}

/// Append a reset if a non-default style is still open.
fn close(out: &mut String, open: bool) {
    if open {
        out.push_str(RESET);
    }
}

/// Update the "style open" flag from an SGR sequence (non-SGR escapes are ignored).
fn update_style(open: &mut bool, seq: &str) {
    if let Some(is_reset) = sgr_is_reset(seq) {
        *open = !is_reset;
    }
}

/// For an SGR sequence (`CSI … m`), `Some(true)` if it resets all attributes,
/// `Some(false)` otherwise; `None` if `seq` is not an SGR sequence.
fn sgr_is_reset(seq: &str) -> Option<bool> {
    let body = match seq.strip_prefix('\u{1b}') {
        Some(rest) => rest.strip_prefix('[')?,
        None => seq.strip_prefix('\u{9b}')?,
    };
    let params = body.strip_suffix('m')?;
    Some(
        params
            .split(';')
            .all(|p| p.is_empty() || p.bytes().all(|b| b == b'0')),
    )
}

/// Keep escapes plus visible chars from `s` up to `max` columns, closing any open
/// style. Used when the ellipsis alone exceeds `max_width`.
fn clamp_visible(s: &str, max: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    let mut open = false;
    for t in tokenize(s) {
        match t {
            Tok::Ch(c, w) => {
                if used + w > max {
                    break;
                }
                out.push(c);
                used += w;
            }
            Tok::Esc(seq) => {
                out.push_str(&seq);
                update_style(&mut open, &seq);
            }
        }
    }
    close(&mut out, open);
    out
}
