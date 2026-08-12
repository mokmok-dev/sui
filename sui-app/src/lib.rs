//! Application layer for the sui coding agent.
//!
//! Provides [`App`], which owns the prompt state, message history, and the
//! terminal run-loop (event → update → render).
//!
//! The interactive UI uses an inline [`ratatui::Viewport`] so only the prompt
//! (and slash suggestions) occupy the screen; submitted output is inserted
//! above it and scrolls into the terminal scrollback.
//!
//! # Modes
//!
//! Interaction is sticky [`Mode`] state (vim-like), not inferred from prefixes:
//! - [`Mode::Prompt`] (default) — chat text; `/` opens slash commands
//! - [`Mode::Shell`] — entered with `!` on an empty prompt; Enter runs bash
//!   via [`sui_tools::run_line`]; Esc returns to [`Mode::Prompt`]; output is
//!   flushed as dim ghost text (no prompt prefix)
//!
//! Future surfaces (subagent, workflow) should add [`Mode`] variants rather
//! than new prefix heuristics.

pub mod app;
pub(crate) mod bang;
pub mod input;
pub mod mode;
pub mod slash;

pub use app::{App, PROMPT_HEIGHT};
pub use mode::Mode;
pub use slash::SlashCommand;

/// Converts a char-based index into a byte offset within `s`.
///
/// Returns `None` when `char_idx` is past the end of the string.
#[inline]
pub(crate) fn char_index_to_byte(
    s: &str,
    char_idx: usize,
) -> Option<usize> {
    s.char_indices().nth(char_idx).map(|(i, _)| i)
}

#[cfg(test)]
mod tests;
