use std::fmt;

use thiserror::Error;

/// Errors from configuration or `LiteLLM` Proxy chat calls.
///
/// [`LlmError::Api`] keeps the underlying `async-openai` error for
/// [`std::error::Error::source`], but [`Display`] and [`Debug`] stay opaque so
/// response bodies and URLs are not printed by default logging.
#[derive(Error)]
pub enum LlmError {
    /// A required environment variable was not set.
    #[error("missing environment variable `{0}`")]
    MissingEnv(&'static str),
    /// Configuration values failed validation.
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
    /// The OpenAI-compatible HTTP/API layer failed.
    #[error("LLM API error")]
    Api(#[source] async_openai::error::OpenAIError),
    /// The Proxy returned a completion with no usable assistant text.
    #[error("empty chat completion response")]
    EmptyResponse,
    /// The model refused to produce assistant content.
    #[error("model refused: {0}")]
    Refused(String),
}

impl From<async_openai::error::OpenAIError> for LlmError {
    fn from(value: async_openai::error::OpenAIError) -> Self {
        Self::Api(value)
    }
}

impl fmt::Debug for LlmError {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        match self {
            Self::MissingEnv(name) => f.debug_tuple("MissingEnv").field(name).finish(),
            Self::InvalidConfig(msg) => f.debug_tuple("InvalidConfig").field(msg).finish(),
            Self::Api(_) => f.write_str("Api(/* redacted */)"),
            Self::EmptyResponse => f.write_str("EmptyResponse"),
            Self::Refused(msg) => f.debug_tuple("Refused").field(msg).finish(),
        }
    }
}
