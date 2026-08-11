use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::Text,
    widgets::{Block, Borders, Paragraph, Widget},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// A text-input prompt widget with a prefix, scrollable input, and cursor
/// tracking.
///
/// The widget renders a bordered box containing the prompt prefix followed by
/// the current input text.  When the input text is wider than the available
/// space the widget scrolls horizontally so that the cursor stays visible.
///
/// Display widths are computed via [`unicode_width`] so that full-width
/// characters (CJK, emoji, etc.) contribute 2 columns each.  This keeps the
/// cursor correctly positioned even when the input contains Japanese, Chinese,
/// or Korean text.
///
/// After rendering the caller should position the terminal cursor by calling
/// [`screen_cursor`](PromptWidget::screen_cursor) with the same [`Rect`] that
/// was used for rendering.
///
/// # Example
///
/// ```ignore
/// let prompt = PromptWidget::new(input, cursor_pos, "❯ ");
/// let cursor_pos = prompt.screen_cursor(area);
/// frame.render_widget(prompt, area);
/// frame.set_cursor_position(cursor_pos);
/// ```
///
/// Note: call `screen_cursor` *before* `render_widget` because [`Widget::render`]
/// consumes `self`.
pub struct PromptWidget<'a> {
    block: Block<'a>,
    input: &'a str,
    /// Char-based position of the cursor within `input`.
    cursor_position: usize,
    prompt_prefix: &'a str,
}

impl<'a> PromptWidget<'a> {
    /// Creates a new prompt widget.
    ///
    /// * `input` — the current input text.
    /// * `cursor_position` — char-based cursor index into `input`.
    /// * `prompt_prefix` — prefix displayed before the input (e.g. `"❯ "`).
    #[must_use]
    pub fn new(
        input: &'a str,
        cursor_position: usize,
        prompt_prefix: &'a str,
    ) -> Self {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" prompt ")
            .style(Style::default().fg(Color::Cyan));

        Self {
            block,
            input,
            cursor_position,
            prompt_prefix,
        }
    }

    /// Returns the (x, y) terminal coordinates where the cursor should be
    /// placed so that it sits immediately after the visible portion of input up
    /// to `cursor_position`.
    ///
    /// `area` must be the same [`Rect`] that will be (or was) passed to
    /// [`Widget::render`].
    #[must_use]
    pub fn screen_cursor(
        &self,
        area: Rect,
    ) -> (u16, u16) {
        let (scroll, _visible_chars) =
            visible_range(self.input, self.cursor_position, self.prompt_prefix, area);

        let input_chars: Vec<char> = self.input.chars().collect();
        let cursor_pos = self.cursor_position.min(input_chars.len());

        // Display width from scroll start to cursor position.
        let cursor_display_offset: usize = input_chars[scroll..cursor_pos]
            .iter()
            .map(|c| c.width().unwrap_or(0))
            .sum();

        let prefix_width = self.prompt_prefix.width();
        let inner_x = 1usize + prefix_width + cursor_display_offset;
        let x = area.x + u16::try_from(inner_x).unwrap_or(u16::MAX);
        // Clamp so the cursor is never placed on or past the right border.
        let max_x = area.x + area.width.saturating_sub(2);
        let y = area.y + 1;
        (x.min(max_x), y)
    }
}

impl Widget for PromptWidget<'_> {
    fn render(
        self,
        area: Rect,
        buf: &mut Buffer,
    ) {
        let (scroll, visible_chars) =
            visible_range(self.input, self.cursor_position, self.prompt_prefix, area);

        let visible_input: String = self
            .input
            .chars()
            .skip(scroll)
            .take(visible_chars)
            .collect();

        let display_text = format!("{}{}", self.prompt_prefix, visible_input);
        let paragraph = Paragraph::new(Text::from(display_text)).block(self.block);

        paragraph.render(area, buf);
    }
}

// ── scroll helpers ──────────────────────────────────────────────────────────

/// Returns `(scroll_char_offset, visible_char_count)` for the current area.
///
/// * `scroll_char_offset` — how many chars of `input` should be skipped so that
///   `cursor_position` stays visible.
/// * `visible_char_count` — how many chars from the scroll position fit inside
///   the available display width.
///
/// Both values account for the actual terminal display width of each character
/// (e.g. CJK characters are 2 columns wide).
fn visible_range(
    input: &str,
    cursor_position: usize,
    prefix: &str,
    area: Rect,
) -> (usize, usize) {
    let inner_width = area.width.saturating_sub(2) as usize;
    let prefix_width = prefix.width();
    let available_width = inner_width.saturating_sub(prefix_width);

    if available_width == 0 {
        return (0, 0);
    }

    let input_chars: Vec<char> = input.chars().collect();
    let total_chars = input_chars.len();
    let cursor_pos = cursor_position.min(total_chars);

    // Display width of input up to (but not including) the cursor.
    let cursor_display_width: usize = input_chars[..cursor_pos]
        .iter()
        .map(|c| c.width().unwrap_or(0))
        .sum();

    // Find the scroll offset: the smallest char index such that the display
    // width from that index to `cursor_pos` fits in `available_width`.
    let scroll = if cursor_display_width <= available_width {
        0
    } else {
        let mut width = 0usize;
        let mut s = cursor_pos;
        for ch in input_chars[..cursor_pos].iter().rev() {
            let ch_width = ch.width().unwrap_or(0);
            if width + ch_width > available_width {
                break;
            }
            width += ch_width;
            s -= 1;
        }
        s
    };

    // Count how many chars from `scroll` fit in `available_width`.
    let mut visible_chars = 0usize;
    let mut used_width = 0usize;
    for ch in &input_chars[scroll..] {
        let ch_width = ch.width().unwrap_or(0);
        if used_width + ch_width > available_width {
            break;
        }
        used_width += ch_width;
        visible_chars += 1;
    }

    (scroll, visible_chars)
}

// ── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    // ── visible_range (ASCII) ──────────────────────────────────────────

    #[test]
    fn visible_range_empty_input() {
        let area = Rect::new(0, 0, 20, 3);
        let (scroll, visible) = visible_range("", 0, "❯ ", area);
        assert_eq!(scroll, 0);
        assert_eq!(visible, 0);
    }

    #[test]
    fn visible_range_scrolls_long_input() {
        let area = Rect::new(0, 0, 10, 3);
        // inner = 8, prefix "❯ " width = 2, available = 6
        // "123456789" cursor at 7 → display width before cursor = 7 > 6
        let (scroll, visible) = visible_range("123456789", 7, "❯ ", area);
        // Walk back from 7: 7(1) 6(1) 5(1) 4(1) 3(1) 2(1) = 6 cols, fits; 1 would overflow
        assert_eq!(scroll, 1);
        // From scroll=1: 2,3,4,5,6,7 = 6 chars, 6 cols
        assert_eq!(visible, 6);
    }

    #[test]
    fn visible_range_cursor_fits_no_scroll() {
        let area = Rect::new(0, 0, 30, 3);
        let (scroll, visible) = visible_range("abc", 2, "❯ ", area);
        assert_eq!(scroll, 0);
        assert_eq!(visible, 3);
    }

    #[test]
    fn visible_range_zero_inner_width() {
        let area = Rect::new(0, 0, 3, 3);
        // inner = 1, prefix width 2 → available = 0 (saturating)
        let (scroll, visible) = visible_range("hello", 2, "❯ ", area);
        assert_eq!(scroll, 0);
        assert_eq!(visible, 0);
    }

    // ── visible_range (CJK) ────────────────────────────────────────────

    #[test]
    fn visible_range_cjk_no_scroll() {
        let area = Rect::new(0, 0, 20, 3);
        // inner = 18, prefix "❯ " width = 2, available = 16
        // "あいう" = 6 cols, cursor at 2 → 4 cols ≤ 16
        let (scroll, visible) = visible_range("あいう", 2, "❯ ", area);
        assert_eq!(scroll, 0);
        // All 3 CJK chars fit: 6 cols ≤ 16
        assert_eq!(visible, 3);
    }

    #[test]
    fn visible_range_cjk_scrolls() {
        let area = Rect::new(0, 0, 10, 3);
        // inner = 8, prefix = 2, available = 6
        // "abcあいうdef": a(1)b(1)c(1)あ(2)い(2)う(2)d(1)e(1)f(1)
        // cursor at 6 (after う): display before cursor = 1+1+1+2+2+2 = 9 > 6
        let (scroll, visible) = visible_range("abcあいうdef", 6, "❯ ", area);
        // Walk back: う(2) い(2) あ(2) = 6 cols, fits; c(1) would be 7 > 6
        assert_eq!(scroll, 3);
        // From scroll=3: あ(2)い(2)う(2) = 6 cols; d(1) would overflow
        assert_eq!(visible, 3);
    }

    #[test]
    fn visible_range_mixed_ascii_cjk() {
        let area = Rect::new(0, 0, 12, 3);
        // inner = 10, prefix = 2, available = 8
        // "abあcd": a(1)b(1)あ(2)c(1)d(1) = 6 cols
        // cursor at 3 (after あ): display before = 1+1+2 = 4 ≤ 8
        let (scroll, visible) = visible_range("abあcd", 3, "❯ ", area);
        assert_eq!(scroll, 0);
        // All 5 chars fit: 6 cols ≤ 8
        assert_eq!(visible, 5);
    }

    // ── screen_cursor (ASCII) ──────────────────────────────────────────

    #[test]
    fn screen_cursor_at_start() {
        let widget = PromptWidget::new("hello", 0, "❯ ");
        let area = Rect::new(0, 0, 30, 3);
        let (x, y) = widget.screen_cursor(area);
        // x = area.x + 1 (border) + 2 (prefix "❯ ") + 0 = 3
        assert_eq!(x, 3);
        assert_eq!(y, 1);
    }

    #[test]
    fn screen_cursor_after_text() {
        let widget = PromptWidget::new("hi", 2, "> ");
        let area = Rect::new(5, 2, 30, 3);
        let (x, y) = widget.screen_cursor(area);
        // x = 5 + 1 + 2 + 2 = 10
        assert_eq!(x, 10);
        assert_eq!(y, 3);
    }

    #[test]
    fn screen_cursor_clamps_to_border() {
        // Area too narrow: even the prefix overflows
        let widget = PromptWidget::new("hello", 5, "❯ ");
        let area = Rect::new(0, 0, 5, 3);
        // max_x = 0 + 5 - 2 = 3
        let (x, _y) = widget.screen_cursor(area);
        assert!(x <= 3);
    }

    // ── screen_cursor (CJK) ────────────────────────────────────────────

    #[test]
    fn screen_cursor_after_cjk() {
        // "あいう" — each char is 2 columns wide, total 6 columns
        let widget = PromptWidget::new("あいう", 3, "❯ ");
        let area = Rect::new(0, 0, 30, 3);
        let (x, y) = widget.screen_cursor(area);
        // prefix "❯ " = 2 cols, "あいう" = 6 cols → x = 0+1+2+6 = 9
        assert_eq!(x, 9);
        assert_eq!(y, 1);
    }

    #[test]
    fn screen_cursor_mid_cjk() {
        // "あいう" cursor at 1 (after あ, before い)
        let widget = PromptWidget::new("あいう", 1, "❯ ");
        let area = Rect::new(0, 0, 30, 3);
        let (x, y) = widget.screen_cursor(area);
        // prefix = 2, "あ" = 2 → x = 0+1+2+2 = 5
        assert_eq!(x, 5);
        assert_eq!(y, 1);
    }

    #[test]
    fn screen_cursor_mixed_ascii_cjk() {
        // "abあcd" — a(1) b(1) あ(2) c(1) d(1)
        // cursor at 3 (after あ, before c)
        let widget = PromptWidget::new("abあcd", 3, "❯ ");
        let area = Rect::new(0, 0, 30, 3);
        let (x, y) = widget.screen_cursor(area);
        // prefix = 2, "abあ" = 1+1+2 = 4 → x = 0+1+2+4 = 7
        assert_eq!(x, 7);
        assert_eq!(y, 1);
    }

    #[test]
    fn screen_cursor_cjk_with_scroll() {
        // Narrow area that forces scrolling with CJK text
        let area = Rect::new(0, 0, 10, 3);
        // inner = 8, prefix = 2, available = 6
        // "abcあいうdef": cursor at 6 (after う), scroll should be 3
        let widget = PromptWidget::new("abcあいうdef", 6, "❯ ");
        let (x, y) = widget.screen_cursor(area);
        // From scroll=3: chars are あ(2)い(2)う(2), cursor at position 6
        // cursor_display_offset = width of chars[3..6] = width("あいう") = 6
        // x = 0+1+2+6 = 9, but max_x = 0+10-2 = 8, clamped to 8
        assert_eq!(x, 8);
        assert_eq!(y, 1);
    }
}
