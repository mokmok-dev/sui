//! Non-blocking streaming LLM chat for [`crate::Mode::Prompt`].
//!
//! The TUI event loop stays responsive while a chat request runs: the worker
//! thread owns a current-thread Tokio runtime (same pattern as [`crate::bang`])
//! and forwards stream deltas on a channel. The app polls that channel between
//! draw ticks so Markdown can render incrementally above the prompt.

use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use futures::StreamExt;
use sui_llm::{ChatMessage, ChatResponse, LlmClient};

/// Default deadline for a single streaming chat completion.
pub const DEFAULT_CHAT_TIMEOUT: Duration = Duration::from_mins(2);

/// Spinner frame interval while an LLM request is in flight.
pub const SPINNER_TICK: Duration = Duration::from_millis(80);

/// Braille spinner glyphs (clockwise).
const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Incremental update from the background chat worker.
pub enum LlmStreamMsg {
    /// A text delta from the Proxy stream.
    Chunk(String),
    /// Stream finished successfully.
    Done(ChatResponse),
    /// Stream failed or timed out.
    Failed(String),
}

/// Glyph for a spinner that started `elapsed` ago.
#[must_use]
pub fn spinner_glyph(elapsed: Duration) -> char {
    let tick = SPINNER_TICK.as_millis().max(1);
    let idx = (elapsed.as_millis() / tick) as usize % SPINNER_FRAMES.len();
    SPINNER_FRAMES[idx]
}

/// Spawns a streaming chat and returns a receiver for incremental updates.
///
/// The worker uses timeout [`DEFAULT_CHAT_TIMEOUT`]. Dropping the receiver does
/// not cancel the request; sends are best-effort.
#[must_use]
pub fn chat_stream_spawn(
    client: &LlmClient,
    messages: &[ChatMessage],
) -> Receiver<LlmStreamMsg> {
    chat_stream_spawn_with_timeout(client, messages, DEFAULT_CHAT_TIMEOUT)
}

fn chat_stream_spawn_with_timeout(
    client: &LlmClient,
    messages: &[ChatMessage],
    timeout: Duration,
) -> Receiver<LlmStreamMsg> {
    let (tx, rx) = mpsc::channel();
    let client = client.clone();
    let messages = messages.to_vec();
    let default_model = client.default_model().to_owned();
    std::thread::spawn(move || {
        let result = (|| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| format!("failed to create tokio runtime: {error}"))?;
            runtime.block_on(async {
                tokio::time::timeout(timeout, async {
                    let mut stream = client
                        .chat_stream(&messages)
                        .await
                        .map_err(|error| error.to_string())?;
                    let mut content = String::new();
                    while let Some(item) = stream.next().await {
                        let chunk = item.map_err(|error| error.to_string())?;
                        if chunk.delta.is_empty() {
                            continue;
                        }
                        content.push_str(&chunk.delta);
                        if tx.send(LlmStreamMsg::Chunk(chunk.delta)).is_err() {
                            break;
                        }
                    }
                    Ok(ChatResponse::new(content, &default_model))
                })
                .await
                .unwrap_or_else(|_| {
                    Err(format!(
                        "llm request timed out after {}s",
                        timeout.as_secs()
                    ))
                })
            })
        })();
        match result {
            Ok(response) => {
                let _ = tx.send(LlmStreamMsg::Done(response));
            },
            Err(error) => {
                let _ = tx.send(LlmStreamMsg::Failed(error));
            },
        }
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
