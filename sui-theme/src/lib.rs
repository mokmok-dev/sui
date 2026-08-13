//! Central colour palette and derived styles for the sui TUI.
//!
//! Colours that appear on prompt / shell widgets and suggestion panels are
//! defined here as consts so they read as one cohesive palette. Keeping them
//! behind a single [`Theme`] value means they can later be overridden from
//! configuration without touching call sites.

use ratatui::style::{Color, Style};

/// Named colours shared across the sui TUI widgets.
///
/// The [`DEFAULT`](Self::DEFAULT) value is applied throughout the app. Call
/// sites should consume the derived style methods rather than destructuring
/// the colour fields directly, so swapping the palette later stays localised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Border colour of the interactive prompt widget.
    pub prompt_border: Color,
    /// Border colour of the one-shot shell widget.
    pub shell_border: Color,
    /// Background of a flushed prompt line in the scrollback.
    pub prompt_background: Color,
    /// Foreground of the highlighted suggestion row.
    pub selection_fg: Color,
    /// Background of the highlighted suggestion row.
    pub selection_bg: Color,
}

impl Theme {
    /// The default palette.
    ///
    /// `prompt_background` is a mid grey — a lighter value reads as white on
    /// many terminals and fails to separate past prompts from the scrollback.
    pub const DEFAULT: Self = Self {
        prompt_border: Color::Cyan,
        shell_border: Color::Magenta,
        prompt_background: Color::DarkGray,
        selection_fg: Color::Black,
        selection_bg: Color::Yellow,
    };

    /// Foreground-only border style for the interactive prompt widget.
    #[must_use]
    pub fn prompt_style(self) -> Style {
        Style::default().fg(self.prompt_border)
    }

    /// Foreground-only border style for the one-shot shell widget.
    #[must_use]
    pub fn shell_style(self) -> Style {
        Style::default().fg(self.shell_border)
    }

    /// Background style for flushed prompt lines in the scrollback.
    #[must_use]
    pub fn prompt_flush_style(self) -> Style {
        Style::default().bg(self.prompt_background)
    }

    /// Filled style for the highlighted suggestion row.
    #[must_use]
    pub fn selected_style(self) -> Style {
        Style::default().fg(self.selection_fg).bg(self.selection_bg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_styles_carry_the_expected_colors() {
        assert_eq!(Theme::DEFAULT.prompt_style().fg, Some(Color::Cyan));
        assert_eq!(Theme::DEFAULT.shell_style().fg, Some(Color::Magenta));
        assert_eq!(
            Theme::DEFAULT.prompt_flush_style().bg,
            Some(Color::DarkGray)
        );
        assert_eq!(
            Theme::DEFAULT.selected_style(),
            Style::default().fg(Color::Black).bg(Color::Yellow)
        );
    }
}
