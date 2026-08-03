//! Small unicode-width-aware word wrapper.
//!
//! We intentionally don't use `ratatui::widgets::Paragraph`'s built-in
//! wrapping for the history area: we need to know the exact wrapped line
//! count *before* render time (to compute scroll offsets, "stick to
//! bottom", half-page jumps, etc.), and Paragraph only wraps internally at
//! render time. So each `HistoryItem` wraps its own text into plain-string
//! lines using this helper, then turns those into styled `Line`s.
//!
//! This is a simple greedy word-wrapper, not a full Unicode line-breaking
//! algorithm (see UAX #14) -- it's intentionally minimal. If more correct
//! line-breaking (e.g. CJK, hyphenation) is ever needed, swap this out for
//! the `textwrap` crate without touching call sites.

use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

/// Wraps `text` to fit within `width` columns, respecting existing newlines.
/// Always returns at least one line (possibly empty).
pub fn wrap(text: &str, width: u16) -> Vec<String> {
    let width = width.max(1) as usize;
    let mut out = Vec::new();

    for input_line in text.split('\n') {
        if input_line.is_empty() {
            out.push(String::new());
            continue;
        }
        wrap_single_line(input_line, width, &mut out);
    }

    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn wrap_single_line(line: &str, width: usize, out: &mut Vec<String>) {
    let mut current = String::new();
    let mut current_width = 0usize;

    for word in line.split(' ') {
        // `split(' ')` yields an empty string for every leading, trailing,
        // or consecutive space.  Treat each one as a literal space character
        // so that indentation and intra-line spacing are preserved.
        if word.is_empty() {
            if current_width + 1 > width && !current.is_empty() {
                out.push(std::mem::take(&mut current));
                current_width = 0;
            }
            current.push(' ');
            current_width += 1;
            continue;
        }

        let word_width = UnicodeWidthStr::width(word);

        // A single word longer than the whole line width has to be
        // hard-broken character by character (e.g. a long path or hash).
        if word_width > width {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
                current_width = 0;
            }
            hard_break(word, width, out);
            continue;
        }

        // Add separator space if current is not empty and doesn't already end with a space
        let needs_separator = !current.is_empty() && !current.ends_with(' ');
        let sep_width = if needs_separator { 1 } else { 0 };

        if current_width + sep_width + word_width > width {
            out.push(std::mem::take(&mut current));
            current_width = 0;
            current.push_str(word);
            current_width = word_width;
        } else {
            if needs_separator {
                current.push(' ');
                current_width += 1;
            }
            current.push_str(word);
            current_width += word_width;
        }
    }

    out.push(current);
}

fn hard_break(word: &str, width: usize, out: &mut Vec<String>) {
    let mut current = String::new();
    let mut current_width = 0usize;
    for ch in word.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(1);
        if current_width + ch_width > width && !current.is_empty() {
            out.push(std::mem::take(&mut current));
            current_width = 0;
        }
        current.push(ch);
        current_width += ch_width;
    }
    if !current.is_empty() {
        out.push(current);
    }
}
