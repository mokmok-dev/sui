//! Interaction mode — what the prompt is addressing.
//!
//! The border title, color, and Enter semantics follow the active mode.
//! [`Mode::Shell`] is one-shot: Enter runs a command then returns to
//! [`Mode::Prompt`]; Esc cancels without running. Add variants here when
//! subagent / workflow surfaces exist; do not infer mode from input prefixes.

use ratatui::style::{Color, Style};

/// Active interaction mode.
///
/// Marked `non_exhaustive` so new surfaces (subagent, workflow, …) can land
/// without breaking downstream `match` expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Mode {
    /// Chat / slash-command prompt (default).
    #[default]
    Prompt,
    /// One-shot shell: `!` on an empty prompt; Enter runs then returns to Prompt.
    Shell,
}

impl Mode {
    /// Border title shown on the prompt widget for this mode.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Prompt => " prompt ",
            Self::Shell => " shell ",
        }
    }

    /// Border style (foreground color) for the prompt widget in this mode.
    #[must_use]
    pub fn border_style(self) -> Style {
        Style::default().fg(self.border_color())
    }

    /// Border foreground color for the prompt widget in this mode.
    #[must_use]
    pub const fn border_color(self) -> Color {
        match self {
            Self::Prompt => Color::Cyan,
            Self::Shell => Color::Magenta,
        }
    }
}
