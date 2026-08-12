//! Interaction mode — what the prompt is addressing.
//!
//! Modes are sticky (vim-like): you enter one, stay until you leave. The border
//! title and Enter semantics follow the active mode. Add variants here when
//! subagent / workflow surfaces exist; do not infer mode from input prefixes.

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
    /// One-shot shell commands (entered with `!` on an empty prompt).
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
}
