use std::pin::Pin;

use async_openai::{
    Client,
    config::OpenAIConfig,
    types::chat::{
        ChatCompletionRequestAssistantMessageContent, ChatCompletionRequestMessage,
        ChatCompletionRequestSystemMessageContent, ChatCompletionRequestUserMessageContent,
        CreateChatCompletionRequestArgs, CreateChatCompletionStreamResponse,
    },
};

use futures::{Stream, StreamExt};

use crate::{ChatMessage, LlmConfig, LlmError, Role, config::require_non_empty_model};

/// Successful non-streaming chat completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatResponse {
    /// Assistant text from the first choice.
    pub content: String,
    /// Model id returned by the Proxy.
    pub model: String,
}

/// One streamed text delta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatChunk {
    /// Incremental assistant text (may be empty on control chunks).
    pub delta: String,
}

/// Owned stream of chat chunks from [`LlmClient::chat_stream`].
pub type ChatStream = Pin<Box<dyn Stream<Item = Result<ChatChunk, LlmError>> + Send>>;

/// Thin chat client aimed at a `LiteLLM` Proxy (or any `OpenAI`-compatible base).
///
/// `async-openai` may emit `tracing` events that include request/response
/// details depending on subscriber filters; treat log sinks as trusted.
#[derive(Clone, Debug)]
pub struct LlmClient {
    inner: Client<OpenAIConfig>,
    default_model: String,
}

impl LlmClient {
    /// Builds a client from [`LlmConfig`].
    ///
    /// Clears inherited `OPENAI_ORG_ID` / `OPENAI_PROJECT_ID` from
    /// [`OpenAIConfig::new`] so Proxy calls do not forward unrelated org/project
    /// headers.
    #[must_use]
    pub fn new(config: &LlmConfig) -> Self {
        let openai = OpenAIConfig::new()
            .with_api_base(config.base_url())
            .with_api_key(config.api_key())
            .with_org_id("")
            .with_project_id("");
        Self {
            inner: Client::with_config(openai),
            default_model: config.model().to_owned(),
        }
    }

    /// Builds a client from `LITELLM_*` environment variables.
    ///
    /// # Errors
    ///
    /// Propagates [`LlmConfig::from_env`] failures.
    pub fn from_env() -> Result<Self, LlmError> {
        Ok(Self::new(&LlmConfig::from_env()?))
    }

    /// Default model used when callers omit an override.
    #[must_use]
    pub fn default_model(&self) -> &str {
        &self.default_model
    }

    /// Non-streaming chat using the configured default model.
    ///
    /// # Errors
    ///
    /// Returns transport/API errors, [`LlmError::EmptyResponse`], or
    /// [`LlmError::Refused`].
    pub async fn chat(
        &self,
        messages: &[ChatMessage],
    ) -> Result<ChatResponse, LlmError> {
        self.chat_with_model(&self.default_model, messages).await
    }

    /// Non-streaming chat with an explicit logical model name.
    ///
    /// # Errors
    ///
    /// Returns [`LlmError::InvalidConfig`] for an empty/whitespace model,
    /// transport/API errors, [`LlmError::EmptyResponse`], or
    /// [`LlmError::Refused`].
    pub async fn chat_with_model(
        &self,
        model: &str,
        messages: &[ChatMessage],
    ) -> Result<ChatResponse, LlmError> {
        let model = require_non_empty_model(model)?;
        let request = CreateChatCompletionRequestArgs::default()
            .model(model)
            .messages(to_openai_messages(messages))
            .build()?;

        let response = self.inner.chat().create(request).await?;
        let choice = response.choices.first().ok_or(LlmError::EmptyResponse)?;
        if let Some(content) = choice
            .message
            .content
            .clone()
            .filter(|text| !text.is_empty())
        {
            return Ok(ChatResponse {
                content,
                model: response.model,
            });
        }
        if let Some(refusal) = choice
            .message
            .refusal
            .clone()
            .filter(|text| !text.is_empty())
        {
            return Err(LlmError::Refused(refusal));
        }
        Err(LlmError::EmptyResponse)
    }

    /// Streaming chat using the configured default model.
    ///
    /// Drive the returned stream with [`futures::StreamExt`] (for example
    /// `.next().await`).
    ///
    /// Unlike [`Self::chat`], streaming does **not** map an all-empty delta
    /// sequence to [`LlmError::EmptyResponse`]: control chunks often carry an
    /// empty `delta`, and callers assemble text themselves. Per-chunk transport
    /// errors are still yielded on the stream.
    ///
    /// # Errors
    ///
    /// Returns transport/API errors while opening the stream. Per-chunk errors
    /// are yielded on the returned stream.
    pub async fn chat_stream(
        &self,
        messages: &[ChatMessage],
    ) -> Result<ChatStream, LlmError> {
        self.chat_stream_with_model(&self.default_model, messages)
            .await
    }

    /// Streaming chat with an explicit logical model name.
    ///
    /// Drive the returned stream with [`futures::StreamExt`] (for example
    /// `.next().await`).
    ///
    /// See [`Self::chat_stream`] for intentional `EmptyResponse` asymmetry vs
    /// non-streaming chat.
    ///
    /// # Errors
    ///
    /// Returns [`LlmError::InvalidConfig`] for an empty/whitespace model, or
    /// transport/API errors while opening the stream. Per-chunk errors are
    /// yielded on the returned stream.
    pub async fn chat_stream_with_model(
        &self,
        model: &str,
        messages: &[ChatMessage],
    ) -> Result<ChatStream, LlmError> {
        let model = require_non_empty_model(model)?;
        let request = CreateChatCompletionRequestArgs::default()
            .model(model)
            .messages(to_openai_messages(messages))
            .build()?;

        let stream = self.inner.chat().create_stream(request).await?;
        Ok(Box::pin(stream.map(map_stream_item)))
    }
}

fn map_stream_item(
    item: Result<CreateChatCompletionStreamResponse, async_openai::error::OpenAIError>
) -> Result<ChatChunk, LlmError> {
    let chunk = item?;
    let Some(choice) = chunk.choices.first() else {
        return Ok(ChatChunk {
            delta: String::new(),
        });
    };
    if let Some(refusal) = choice
        .delta
        .refusal
        .as_deref()
        .filter(|text| !text.is_empty())
    {
        return Err(LlmError::Refused(refusal.to_owned()));
    }
    Ok(ChatChunk {
        delta: choice.delta.content.clone().unwrap_or_default(),
    })
}

fn to_openai_messages(messages: &[ChatMessage]) -> Vec<ChatCompletionRequestMessage> {
    messages
        .iter()
        .map(|message| match message.role {
            Role::System => ChatCompletionRequestMessage::System(
                ChatCompletionRequestSystemMessageContent::Text(message.content.clone()).into(),
            ),
            Role::User => ChatCompletionRequestMessage::User(
                ChatCompletionRequestUserMessageContent::Text(message.content.clone()).into(),
            ),
            Role::Assistant => ChatCompletionRequestMessage::Assistant(
                ChatCompletionRequestAssistantMessageContent::Text(message.content.clone()).into(),
            ),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use futures::StreamExt;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_partial_json, header, method, path},
    };

    use super::*;

    fn client_for(
        server: &MockServer,
        model: &str,
    ) -> LlmClient {
        let config = LlmConfig::new(server.uri(), "test-key", model).expect("config");
        LlmClient::new(&config)
    }

    fn client_with_key(
        server: &MockServer,
        api_key: &str,
        model: &str,
    ) -> LlmClient {
        let config = LlmConfig::new(server.uri(), api_key, model).expect("config");
        LlmClient::new(&config)
    }

    fn ok_completion(content: &str) -> serde_json::Value {
        json!({
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "created": 1,
            "model": "proxy-model",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": content
                },
                "finish_reason": "stop"
            }]
        })
    }

    #[test]
    fn llm_client_debug_hides_api_key() {
        let config =
            LlmConfig::new("http://localhost:4000", "super-secret-key", "m").expect("config");
        let client = LlmClient::new(&config);
        let rendered = format!("{client:?}");
        assert!(!rendered.contains("super-secret-key"), "{rendered}");
    }

    #[tokio::test]
    async fn chat_posts_and_reads_assistant_content() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(ok_completion("hello from proxy")),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = client_for(&server, "proxy-model");
        let response = client.chat(&[ChatMessage::user("hi")]).await.expect("chat");

        assert_eq!(response.content, "hello from proxy");
        assert_eq!(response.model, "proxy-model");

        let requests = server.received_requests().await.expect("requests");
        assert_eq!(requests.len(), 1);
        assert!(!requests[0].headers.contains_key("openai-organization"));
        assert!(!requests[0].headers.contains_key("openai-project"));
    }

    #[tokio::test]
    async fn empty_api_key_sends_bearer_with_empty_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_completion("ok")))
            .expect(1)
            .mount(&server)
            .await;

        let client = client_with_key(&server, "", "proxy-model");
        let response = client.chat(&[ChatMessage::user("hi")]).await.expect("chat");
        assert_eq!(response.content, "ok");

        let requests = server.received_requests().await.expect("requests");
        assert_eq!(requests.len(), 1);
        let auth = requests[0]
            .headers
            .get("authorization")
            .expect("authorization")
            .to_str()
            .expect("auth utf8");
        // HTTP may trim trailing OWS from `Bearer ` (empty token).
        assert!(
            auth == "Bearer" || auth == "Bearer ",
            "unexpected authorization: {auth:?}"
        );
    }

    #[tokio::test]
    async fn chat_with_model_override_is_sent() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(json!({ "model": "other-model" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "chatcmpl-2",
                "object": "chat.completion",
                "created": 1,
                "model": "other-model",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "ok"
                    },
                    "finish_reason": "stop"
                }]
            })))
            .mount(&server)
            .await;

        let client = client_for(&server, "default-model");
        let response = client
            .chat_with_model("other-model", &[ChatMessage::user("x")])
            .await
            .expect("chat");
        assert_eq!(response.content, "ok");
        assert_eq!(response.model, "other-model");
    }

    #[tokio::test]
    async fn chat_with_model_rejects_blank_override() {
        let server = MockServer::start().await;
        let client = client_for(&server, "default-model");
        let err = client
            .chat_with_model("  ", &[ChatMessage::user("x")])
            .await
            .expect_err("blank model");
        assert!(matches!(err, LlmError::InvalidConfig(_)));
    }

    #[tokio::test]
    async fn chat_sends_multi_role_messages() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(json!({
                "messages": [
                    { "role": "system", "content": "sys" },
                    { "role": "user", "content": "usr" },
                    { "role": "assistant", "content": "prev" }
                ]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_completion("done")))
            .expect(1)
            .mount(&server)
            .await;

        let client = client_for(&server, "proxy-model");
        let response = client
            .chat(&[
                ChatMessage::system("sys"),
                ChatMessage::user("usr"),
                ChatMessage::assistant("prev"),
            ])
            .await
            .expect("chat");
        assert_eq!(response.content, "done");
    }

    #[tokio::test]
    async fn chat_stream_concatenates_deltas() {
        let server = MockServer::start().await;
        let sse = concat!(
            "data: {\"id\":\"chatcmpl-s\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"proxy-model\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"hel\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-s\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"proxy-model\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-s\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"proxy-model\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(json!({ "stream": true })))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse),
            )
            .mount(&server)
            .await;

        let client = client_for(&server, "proxy-model");
        let mut stream = client
            .chat_stream(&[ChatMessage::user("hi")])
            .await
            .expect("open stream");

        let mut text = String::new();
        while let Some(item) = stream.next().await {
            let chunk = item.expect("chunk");
            text.push_str(&chunk.delta);
        }
        assert_eq!(text, "hello");
    }

    #[tokio::test]
    async fn chat_stream_with_model_sends_model_and_stream_flag() {
        let server = MockServer::start().await;
        let sse = concat!(
            "data: {\"id\":\"chatcmpl-s\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"other-model\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"x\"},\"finish_reason\":null}]}\n\n",
            "data: [DONE]\n\n",
        );
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(json!({
                "model": "other-model",
                "stream": true
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = client_for(&server, "default-model");
        let mut stream = client
            .chat_stream_with_model("other-model", &[ChatMessage::user("hi")])
            .await
            .expect("open stream");
        let chunk = stream.next().await.expect("item").expect("chunk");
        assert_eq!(chunk.delta, "x");
    }

    #[tokio::test]
    async fn chat_stream_maps_http_error_to_api() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "error": {
                    "message": "invalid key",
                    "type": "auth_error",
                    "param": null,
                    "code": null
                }
            })))
            .mount(&server)
            .await;

        let client = client_for(&server, "proxy-model");
        let err = client
            .chat_stream(&[ChatMessage::user("hi")])
            .await
            .err()
            .expect("api error");
        assert!(matches!(err, LlmError::Api(_)), "{err}");
        assert_eq!(err.to_string(), "LLM API error");
    }

    #[tokio::test]
    async fn chat_empty_choices_is_empty_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "chatcmpl-empty",
                "object": "chat.completion",
                "created": 1,
                "model": "proxy-model",
                "choices": []
            })))
            .mount(&server)
            .await;

        let client = client_for(&server, "proxy-model");
        let err = client
            .chat(&[ChatMessage::user("hi")])
            .await
            .expect_err("empty");
        assert!(matches!(err, LlmError::EmptyResponse));
    }

    #[tokio::test]
    async fn chat_empty_content_string_is_empty_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_completion("")))
            .mount(&server)
            .await;

        let client = client_for(&server, "proxy-model");
        let err = client
            .chat(&[ChatMessage::user("hi")])
            .await
            .expect_err("empty content");
        assert!(matches!(err, LlmError::EmptyResponse));
    }

    #[tokio::test]
    async fn chat_refusal_maps_to_refused() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "chatcmpl-refusal",
                "object": "chat.completion",
                "created": 1,
                "model": "proxy-model",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "refusal": "I cannot help with that."
                    },
                    "finish_reason": "stop"
                }]
            })))
            .mount(&server)
            .await;

        let client = client_for(&server, "proxy-model");
        let err = client
            .chat(&[ChatMessage::user("hi")])
            .await
            .expect_err("refused");
        assert!(
            matches!(err, LlmError::Refused(ref msg) if msg == "I cannot help with that."),
            "{err}"
        );
    }

    #[tokio::test]
    async fn chat_stream_with_model_rejects_blank_override() {
        let server = MockServer::start().await;
        let client = client_for(&server, "default-model");
        let result = client
            .chat_stream_with_model("  ", &[ChatMessage::user("x")])
            .await;
        assert!(
            matches!(result, Err(LlmError::InvalidConfig(_))),
            "expected InvalidConfig, got Ok stream"
        );
    }

    #[tokio::test]
    async fn chat_stream_refusal_maps_to_refused() {
        let server = MockServer::start().await;
        let sse = concat!(
            "data: {\"id\":\"chatcmpl-s\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"proxy-model\",\"choices\":[{\"index\":0,\"delta\":{\"refusal\":\"nope\"},\"finish_reason\":null}]}\n\n",
            "data: [DONE]\n\n",
        );
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse),
            )
            .mount(&server)
            .await;

        let client = client_for(&server, "proxy-model");
        let mut stream = client
            .chat_stream(&[ChatMessage::user("hi")])
            .await
            .expect("open stream");
        let err = stream.next().await.expect("item").expect_err("refused");
        assert!(
            matches!(err, LlmError::Refused(ref msg) if msg == "nope"),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn chat_maps_http_error_to_api_without_body_in_display() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "error": {
                    "message": "invalid key secret-body",
                    "type": "auth_error",
                    "param": null,
                    "code": null
                }
            })))
            .mount(&server)
            .await;

        let client = client_for(&server, "proxy-model");
        let err = client
            .chat(&[ChatMessage::user("hi")])
            .await
            .expect_err("api error");
        assert!(matches!(err, LlmError::Api(_)), "{err:?}");
        assert_eq!(err.to_string(), "LLM API error");
        assert!(!err.to_string().contains("secret-body"));
        assert!(!format!("{err:?}").contains("secret-body"));
        assert!(err.source().is_some());
    }
}
