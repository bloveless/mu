//! Multiline prompt editor: emacs-ish editing, history recall, paste.

use std::borrow::Cow;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthChar;

/// Multiline text buffer with a byte-indexed cursor (always on a char
/// boundary). Soft-wrap math lives here — shared by rendering and cursor
/// placement so the two can never disagree.
#[derive(Debug, Default)]
pub struct Input {
    text: String,
    cursor: usize,
    scroll: usize,
    history: Vec<String>,
    history_index: Option<usize>,
    /// Stash of the in-progress edit while recalling history.
    draft: String,
}

impl Input {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// Display rows at `width`: byte ranges of `text`, one per row. Newlines
    /// force a row break; long lines hard-wrap at the edge.
    pub fn rows(&self, width: usize) -> Vec<(usize, usize)> {
        let width = width.max(1);
        let mut rows = Vec::new();
        let mut start = 0;
        let mut column = 0;
        for (index, ch) in self.text.char_indices() {
            if ch == '\n' {
                rows.push((start, index));
                start = index + 1;
                column = 0;
                continue;
            }
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if column + ch_width > width && index > start {
                rows.push((start, index));
                start = index;
                column = 0;
            }
            column += ch_width;
        }
        rows.push((start, self.text.len()));
        rows
    }

    /// Cursor as (column, row) in display rows at `width`.
    pub fn cursor_pos(&self, width: usize) -> (u16, u16) {
        let rows = self.rows(width);
        for (index, &(start, end)) in rows.iter().enumerate() {
            let broke_on_newline = self.text.as_bytes().get(end) == Some(&b'\n');
            if self.cursor < end
                || (self.cursor == end && (broke_on_newline || index + 1 == rows.len()))
            {
                return (column_of(&self.text[start..self.cursor]), index as u16);
            }
        }
        (0, 0)
    }

    /// Keep the cursor row inside the visible window of `height` rows.
    pub fn ensure_cursor_visible(&mut self, height: usize, width: usize) {
        let height = height.max(1);
        let (_, row) = self.cursor_pos(width);
        let row = row as usize;
        if row < self.scroll {
            self.scroll = row;
        } else if row >= self.scroll + height {
            self.scroll = row + 1 - height;
        }
        // Re-wrapping on a resize can leave scroll past the last valid
        // window position (rows were computed at the old width, then the
        // text reflowed at the new width); clamp so `rows - height -
        // scroll` can never underflow.
        self.scroll = self.scroll.min(self.rows(width).len().saturating_sub(height));
    }

    /// Take the submitted text, pushing it onto the recall history. `None`
    /// when empty or whitespace-only.
    pub fn take(&mut self) -> Option<String> {
        let text = std::mem::take(&mut self.text);
        self.cursor = 0;
        self.scroll = 0;
        self.history_index = None;
        self.draft.clear();
        if text.trim().is_empty() {
            return None;
        }
        self.history.push(text.clone());
        Some(text)
    }

    /// `width` is the display width the rows are computed at; needed so Up/Down
    /// can walk the cursor across display rows.
    pub fn handle_key(&mut self, key: KeyEvent, width: usize) {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Left => self.move_left(),
            KeyCode::Right => self.move_right(),
            KeyCode::Home => self.cursor = self.current_line_start(),
            KeyCode::End => self.cursor = self.current_line_end(),
            KeyCode::Up => self.line_up_or_recall(width),
            KeyCode::Down => self.line_down_or_recall(width),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Char('a') if control => self.cursor = self.current_line_start(),
            KeyCode::Char('e') if control => self.cursor = self.current_line_end(),
            KeyCode::Char('k') if control => self.kill_to_line_end(),
            KeyCode::Char('u') if control => self.kill_line(),
            KeyCode::Char('w') if control => self.kill_word_back(),
            KeyCode::Char(ch) if !control && !key.modifiers.contains(KeyModifiers::ALT) => {
                self.insert_char(ch);
            }
            _ => {}
        }
    }

    pub fn insert_char(&mut self, ch: char) {
        self.text.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
        self.history_index = None;
    }

    pub fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    /// Bracketed paste: inserted literally (newlines included), never
    /// submitted early.
    pub fn insert_str(&mut self, text: &str) {
        let normalized = if text.contains('\r') {
            Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
        } else {
            Cow::Borrowed(text)
        };
        self.text.insert_str(self.cursor, &normalized);
        self.cursor += normalized.len();
        self.history_index = None;
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start = self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(index, _)| index);
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
        self.history_index = None;
    }

    fn delete(&mut self) {
        if self.cursor == self.text.len() {
            return;
        }
        let end = self.cursor
            + self.text[self.cursor..]
                .chars()
                .next()
                .map_or(0, char::len_utf8);
        self.text.replace_range(self.cursor..end, "");
        self.history_index = None;
    }

    fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor = self.text[..self.cursor]
                .char_indices()
                .next_back()
                .map_or(0, |(index, _)| index);
        }
    }

    fn move_right(&mut self) {
        if self.cursor < self.text.len() {
            self.cursor += self.text[self.cursor..]
                .chars()
                .next()
                .map_or(0, char::len_utf8);
        }
    }

    fn current_line_start(&self) -> usize {
        self.text[..self.cursor]
            .rfind('\n')
            .map_or(0, |index| index + 1)
    }

    fn current_line_end(&self) -> usize {
        self.text[self.cursor..]
            .find('\n')
            .map_or(self.text.len(), |index| self.cursor + index)
    }

    fn kill_to_line_end(&mut self) {
        let end = self.current_line_end();
        if self.cursor < end {
            self.text.replace_range(self.cursor..end, "");
        } else if end < self.text.len() {
            // At a line boundary: join with the next line.
            self.text.replace_range(end..end + 1, "");
        }
        self.history_index = None;
    }

    fn kill_line(&mut self) {
        let mut start = self.current_line_start();
        let mut end = self.current_line_end();
        if end < self.text.len() {
            end += 1; // swallow the trailing newline
        } else {
            // Last line: swallow the preceding newline.
            start = start.saturating_sub(1);
        }
        self.text.replace_range(start..end, "");
        self.cursor = start;
        self.history_index = None;
    }

    fn kill_word_back(&mut self) {
        let mut start = self.cursor;
        while let Some((index, ch)) = self.text[..start].char_indices().next_back() {
            if ch.is_whitespace() {
                start = index;
            } else {
                break;
            }
        }
        while let Some((index, ch)) = self.text[..start].char_indices().next_back() {
            if !ch.is_whitespace() {
                start = index;
            } else {
                break;
            }
        }
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
        self.history_index = None;
    }

    /// Cursor moves up one display row, keeping the column (clamped to the
    /// target row's width). On the top row this recalls the previous history
    /// entry instead — zsh's `up-line-or-history`. So from empty input with
    /// history `first`, `second`, `some multi\nline\ntext`, `fourth`,
    /// repeated `↑` focuses `fourth` → `text` → `line` → `some multi` →
    /// `second` → `first`.
    fn line_up_or_recall(&mut self, width: usize) {
        let (column, row) = self.cursor_pos(width);
        if row == 0 {
            self.recall_prev();
        } else {
            self.cursor = self.offset_at(width, row - 1, column);
        }
    }

    /// Mirror of [`Self::line_up_or_recall`]: cursor down one display row,
    /// or the next history entry (restoring the draft past the newest) when
    /// recall is active and the cursor sits on the bottom row.
    fn line_down_or_recall(&mut self, width: usize) {
        let (column, row) = self.cursor_pos(width);
        let rows = self.rows(width);
        if self.history_index.is_some() && row as usize + 1 == rows.len() {
            self.recall_next();
        } else if row as usize + 1 < rows.len() {
            self.cursor = self.offset_at(width, row + 1, column);
        }
    }

    /// Byte offset of display `row`, `column` in the text; the column is
    /// clamped to the row's display width.
    fn offset_at(&self, width: usize, row: u16, column: u16) -> usize {
        let rows = self.rows(width);
        let &(start, end) = &rows[row as usize];
        let column = column.min(column_of(&self.text[start..end]));
        let mut offset = start;
        let mut consumed = 0;
        // `char_indices` are relative to the slice; add `start` back.
        for (index, ch) in self.text[start..end].char_indices() {
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
            if consumed + ch_width > column {
                break;
            }
            offset = start + index + ch.len_utf8();
            consumed += ch_width;
        }
        offset
    }

    fn recall_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let index = match self.history_index {
            None => {
                self.draft = self.text.clone();
                self.history.len() - 1
            }
            Some(0) => return,
            Some(index) => index - 1,
        };
        self.history_index = Some(index);
        self.text = self.history[index].clone();
        self.cursor = self.text.len();
        self.scroll = 0;
    }

    fn recall_next(&mut self) {
        let Some(index) = self.history_index else {
            return;
        };
        if index + 1 < self.history.len() {
            self.history_index = Some(index + 1);
            self.text = self.history[index + 1].clone();
            // Land on the first line: `↓` back into a multiline entry
            // shows its start (recall_prev, walking `↑`, shows the end).
            self.cursor = 0;
        } else {
            self.history_index = None;
            self.text = self.draft.clone();
            self.cursor = self.text.len();
        }
        self.scroll = 0;
    }
}

/// Display width of a text fragment.
fn column_of(text: &str) -> u16 {
    text.chars()
        .map(|ch| UnicodeWidthChar::width(ch).unwrap_or(0) as u16)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn up() -> KeyEvent {
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)
    }

    fn down() -> KeyEvent {
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)
    }

    fn commit(input: &mut Input, texts: &[&str]) {
        for text in texts {
            input.insert_str(text);
            assert_eq!(input.take().as_deref(), Some(*text));
        }
    }

    /// The exact scenario from the manual-testing plan: `↑` focuses
    /// fourth → text → line → some multi → second → first.
    #[test]
    fn up_walks_rows_then_history() {
        let mut input = Input::default();
        commit(&mut input, &["first", "second", "some multi\nline\ntext", "fourth"]);
        let width = 40;

        let mut focused = Vec::new();
        for _ in 0..6 {
            input.handle_key(up(), width);
            let (_, row) = input.cursor_pos(width);
            let (start, end) = input.rows(width)[row as usize];
            focused.push(input.text()[start..end].to_string());
        }
        assert_eq!(
            focused,
            ["fourth", "text", "line", "some multi", "second", "first"]
        );

        // `↑` at the oldest is a no-op; the buffer is untouched.
        let before = input.text().to_string();
        input.handle_key(up(), width);
        assert_eq!(input.text(), before);
    }

    /// `↓` mirrors `↑`: walks rows, then forward through history, and past
    /// the newest restores the original draft.
    #[test]
    fn down_walks_rows_then_history_then_draft() {
        let mut input = Input::default();
        commit(&mut input, &["first", "second", "some multi\nline\ntext", "fourth"]);
        let width = 40;

        // Walk to the oldest.
        for _ in 0..6 {
            input.handle_key(up(), width);
        }
        assert_eq!(input.text(), "first");

        // Down back through history; the multiline entry takes one extra
        // Down to walk off its bottom row.
        input.handle_key(down(), width);
        assert_eq!(input.text(), "second");
        input.handle_key(down(), width);
        assert_eq!(input.text(), "some multi\nline\ntext");
        input.handle_key(down(), width); // "line"
        input.handle_key(down(), width); // "text" (bottom row)
        input.handle_key(down(), width);
        assert_eq!(input.text(), "fourth");
        input.handle_key(down(), width);
        assert_eq!(input.text(), ""); // draft restored
        input.handle_key(down(), width);
        assert_eq!(input.text(), ""); // no-op past the draft
    }

    /// Down-recall lands on the first line of the recalled message, so an
    /// `↑`/`↓` round-trip through a multiline entry shows its start without
    /// walking back up.
    #[test]
    fn down_recall_lands_on_first_line() {
        let mut input = Input::default();
        commit(&mut input, &["first", "second", "some multi\nline\ntext", "fourth"]);
        let width = 40;

        // Up into the multiline entry (cursor at its end), up to the top
        // row, then up onto the previous entry.
        input.handle_key(up(), width); // "fourth"
        input.handle_key(up(), width); // multiline, cursor at "text"
        input.handle_key(up(), width); // "line"
        input.handle_key(up(), width); // "some multi" (top row)
        input.handle_key(up(), width); // "second"

        // Down back into the multiline entry: cursor lands on the first row.
        input.handle_key(down(), width);
        assert_eq!(input.text(), "some multi\nline\ntext");
        assert_eq!(input.cursor_pos(width), (0, 0));

        // Down then walks the rows down from the top.
        input.handle_key(down(), width);
        assert_eq!(input.cursor_pos(width), (0, 1));
        input.handle_key(down(), width);
        assert_eq!(input.cursor_pos(width), (0, 2));
    }

    /// Typing any char during recall resets it; the next `↑` starts over
    /// from the newest prompt.
    #[test]
    fn typing_resets_recall() {
        let mut input = Input::default();
        commit(&mut input, &["first", "second"]);
        let width = 40;

        input.handle_key(up(), width); // "second"
        input.handle_key(up(), width); // "first"
        input.insert_char('!'); // resets recall
        input.handle_key(up(), width); // back to the newest: "second"
        assert_eq!(input.text(), "second");
    }

    /// Row-walking clamps the column to the target row, including across
    /// soft wraps.
    #[test]
    fn row_walk_clamps_column_on_soft_wraps() {
        let mut input = Input::default();
        input.insert_str("abcdefghij\nxyz");
        let width = 5; // rows: "abcde", "fghij", "xyz"

        input.handle_key(up(), width); // end → row 1
        assert_eq!(input.cursor_pos(width), (3, 1));
        input.handle_key(up(), width); // row 1 → row 0
        assert_eq!(input.cursor_pos(width), (3, 0));
        input.handle_key(up(), width); // top row: no history, no-op
        assert_eq!(input.cursor_pos(width), (3, 0));

        input.handle_key(down(), width); // row 0 → row 1
        assert_eq!(input.cursor_pos(width), (3, 1));
        input.handle_key(down(), width); // row 1 → row 2, column clamps to 3
        assert_eq!(input.cursor_pos(width), (3, 2));
        input.handle_key(down(), width); // bottom row, not recalling: no-op
        assert_eq!(input.cursor_pos(width), (3, 2));
    }
}
