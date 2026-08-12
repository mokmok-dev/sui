/// Author of a chat message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// System / developer instruction.
    System,
    /// End-user turn.
    User,
    /// Prior assistant turn.
    Assistant,
}

/// A single text chat message.
///
/// Prefer [`ChatMessage::system`], [`ChatMessage::user`], and
/// [`ChatMessage::assistant`] over struct literals so new fields remain
/// non-breaking (`#[non_exhaustive]` rejects crate-external literals).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ChatMessage {
    /// Who authored the message.
    pub role: Role,
    /// Plain-text content.
    pub content: String,
}

impl ChatMessage {
    /// Builds a system message.
    #[must_use]
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    /// Builds a user message.
    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    /// Builds an assistant message.
    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}
