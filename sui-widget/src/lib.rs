//! TUI widgets for the sui coding agent.
//!
//! This crate provides [`PromptWidget`], a text-input widget with a configurable
//! prefix, scrollable input, and cursor tracking built on [ratatui].

mod prompt;

pub use prompt::PromptWidget;
