//! Application layer for the sui coding agent.
//!
//! Provides [`App`], which owns the prompt state, message history, and the
//! terminal run-loop (event → update → render).

pub mod app;
pub mod input;

pub use app::App;

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
