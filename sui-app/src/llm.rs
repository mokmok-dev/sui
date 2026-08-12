//! Non-blocking LLM chat for [`crate::Mode::Prompt`].
//!
//! The TUI event loop stays responsive while a chat request runs: the worker
//! thread owns a current-thread Tokio runtime (same pattern as [`crate::bang`])
//! and sends the result on a channel. The app polls that channel between draw
//! ticks so a working spinner can animate.

use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use sui_llm::{ChatMessage, ChatResponse, LlmClient};

/// Default deadline for a single non-streaming chat completion.
pub const DEFAULT_CHAT_TIMEOUT: Duration = Duration::from_mins(2);

/// Spinner frame interval while an LLM request is in flight.
pub const SPINNER_TICK: Duration = Duration::from_millis(80);

/// Braille spinner glyphs (clockwise).
const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Glyph for a spinner that started `elapsed` ago.
#[must_use]
pub fn spinner_glyph(elapsed: Duration) -> char {
    let tick = SPINNER_TICK.as_millis().max(1);
    let idx = (elapsed.as_millis() / tick) as usize % SPINNER_FRAMES.len();
    SPINNER_FRAMES[idx]
}

/// Spawns a non-streaming chat and returns a receiver for the result.
///
/// The worker uses timeout [`DEFAULT_CHAT_TIMEOUT`]. Dropping the receiver does
/// not cancel the request; the send is best-effort.
#[must_use]
pub fn chat_spawn(
    client: &LlmClient,
    messages: &[ChatMessage],
) -> Receiver<Result<ChatResponse, String>> {
    chat_spawn_with_timeout(client, messages, DEFAULT_CHAT_TIMEOUT)
}

fn chat_spawn_with_timeout(
    client: &LlmClient,
    messages: &[ChatMessage],
    timeout: Duration,
) -> Receiver<Result<ChatResponse, String>> {
    let (tx, rx) = mpsc::channel();
    let client = client.clone();
    let messages = messages.to_vec();
    std::thread::spawn(move || {
        let result = (|| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| format!("failed to create tokio runtime: {error}"))?;
            runtime.block_on(async {
                tokio::time::timeout(timeout, client.chat(&messages))
                    .await
                    .map_or_else(
                        |_| {
                            Err(format!(
                                "llm request timed out after {}s",
                                timeout.as_secs()
                            ))
                        },
                        |result| result.map_err(|error| error.to_string()),
                    )
            })
        })();
        let _ = tx.send(result);
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinner_glyph_cycles() {
        assert_eq!(spinner_glyph(Duration::ZERO), '⠋');
        assert_eq!(spinner_glyph(SPINNER_TICK), '⠙');
        let full_cycle = SPINNER_TICK * u32::try_from(SPINNER_FRAMES.len()).unwrap_or(1);
        assert_eq!(spinner_glyph(full_cycle), '⠋');
    }
}
