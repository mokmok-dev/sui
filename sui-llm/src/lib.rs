//! Thin OpenAI-compatible LLM client for a `LiteLLM` Proxy.
//!
//! Provider routing, retries, and upstream keys live in the Proxy. This crate
//! only talks `OpenAI` chat completions at a configurable `base_url` with a
//! Proxy-issued `api_key` and a logical `model` name. Tool calling uses the
//! same wire format: advertise [`ToolSpec`]s, read [`ChatResponse::tool_calls`],
//! append [`ChatMessage::tool`] results, sample again.
//!
//! # Configuration
//!
//! Environment variables (see [`LlmConfig::from_env`]):
//! - `LITELLM_BASE_URL` — Proxy `OpenAI` base (with or without `/v1`)
//! - `LITELLM_API_KEY` — Proxy credential (virtual key); may be empty
//! - `LITELLM_MODEL` — default logical model name
//!
//! `base_url` is trusted operator config (see [`LlmConfig`]). Do not pass
//! untrusted URLs.
//!
//! Request timeouts are not configured by this crate; wrap calls with your
//! runtime's timeout helper (for example Tokio's `timeout`) at the call site if
//! you need one.
//!
//! # Example
//!
//! ```no_run
//! # async fn demo() -> Result<(), sui_llm::LlmError> {
//! use sui_llm::{ChatMessage, LlmClient, LlmConfig};
//!
//! let config = LlmConfig::new("http://localhost:4000", "sk-litellm-...", "gpt-4o")?;
//! let client = LlmClient::new(&config);
//! let reply = client.chat(&[ChatMessage::user("hello")]).await?;
//! println!("{}", reply.content);
//! # let _ = reply;
//! # Ok(())
//! # }
//! ```

mod client;
mod config;
mod error;
mod message;

pub use client::{ChatChunk, ChatResponse, ChatStream, LlmClient};
pub use config::LlmConfig;
pub use error::LlmError;
pub use message::{ChatMessage, Role, ToolCall, ToolSpec};
