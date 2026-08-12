//! Blocking LLM chat for [`crate::Mode::Prompt`].
//!
//! The TUI event loop is synchronous, so each chat call runs on a short-lived
//! worker thread with its own current-thread Tokio runtime (same pattern as
//! [`crate::bang`]). That avoids `block_in_place` on current-thread runtimes
//! used by many tests.

use std::time::Duration;

use sui_llm::{ChatMessage, ChatResponse, LlmClient};

/// Default deadline for a single non-streaming chat completion.
pub const DEFAULT_CHAT_TIMEOUT: Duration = Duration::from_mins(2);

/// Runs a non-streaming chat to completion (timeout [`DEFAULT_CHAT_TIMEOUT`]).
///
/// # Errors
///
/// Returns a display-oriented message when the worker cannot build a runtime,
/// the request times out, the worker panics, or the Proxy chat call fails.
pub fn chat_blocking(
    client: &LlmClient,
    messages: &[ChatMessage],
) -> Result<ChatResponse, String> {
    chat_blocking_with_timeout(client, messages, DEFAULT_CHAT_TIMEOUT)
}

fn chat_blocking_with_timeout(
    client: &LlmClient,
    messages: &[ChatMessage],
    timeout: Duration,
) -> Result<ChatResponse, String> {
    let client = client.clone();
    let messages = messages.to_vec();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("failed to create tokio runtime: {error}"))?;
        runtime.block_on(async {
            tokio::time::timeout(timeout, client.chat(&messages))
                .await
                .map_or_else(
                    |_| Err(format!("llm request timed out after {}s", timeout.as_secs())),
                    |result| result.map_err(|error| error.to_string()),
                )
        })
    })
    .join()
    .unwrap_or_else(|_| Err("llm worker thread panicked".into()))
}
