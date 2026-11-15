//! Gateway tests: the security rules and the HTTP surface.
//!
//! The route tests bind a real loopback socket and make real HTTP requests. A
//! router exercised through a mock transport would not prove that the headers,
//! the status codes or the SSE framing are right.

use std::net::SocketAddr;
use std::sync::Arc;

use gguf_engine_api_server::{
    ApiConfig, ApiError, Backend, ChatMessage, ChunkStream, ConfigError, MIN_TOKEN_LEN, ModelEntry,
    backend::EmbeddingsOutcome, serve,
};

// ---------------------------------------------------------------- config ----

#[test]
fn the_default_bind_is_loopback() {
    let config = ApiConfig::default();
    assert!(config.bind.ip().is_loopback(), "{}", config.bind);
    assert_eq!(
        config.exposure(),
        gguf_engine_api_server::Exposure::Loopback
    );
    assert!(config.token.is_none(), "loopback needs no token");
}

#[test]
fn a_network_bind_without_a_token_is_refused() {
    let config = ApiConfig {
        bind: "0.0.0.0:8720".parse().unwrap(),
        token: None,
        allow_browser_origins: false,
    };

    let err = config.validate().unwrap_err();
    assert!(matches!(err, ConfigError::TokenRequired { .. }));
    assert!(
        err.to_string().contains("bearer token"),
        "the message must say what to do: {err}"
    );
}

#[test]
fn a_short_token_is_refused() {
    let config = ApiConfig {
        bind: "0.0.0.0:8720".parse().unwrap(),
        token: Some("short".to_owned()),
        allow_browser_origins: false,
    };
    assert!(matches!(
        config.validate().unwrap_err(),
        ConfigError::TokenTooShort { .. }
    ));
}

#[test]
fn a_long_enough_token_allows_a_network_bind() {
    let config = ApiConfig {
        bind: "0.0.0.0:8720".parse().unwrap(),
        token: Some("x".repeat(MIN_TOKEN_LEN)),
        allow_browser_origins: false,
    };
    assert!(config.validate().is_ok());
}

#[test]
fn authorisation_requires_the_exact_token() {
    let config = ApiConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        token: Some("0123456789abcdef".to_owned()),
        allow_browser_origins: false,
    };

    assert!(config.authorise(Some("0123456789abcdef")));
    assert!(!config.authorise(Some("0123456789abcde")));
    assert!(!config.authorise(Some("0123456789abcdefg")));
    assert!(!config.authorise(Some("")));
    assert!(!config.authorise(None));
}

#[test]
fn a_tokenless_loopback_config_accepts_a_request() {
    let config = ApiConfig::default();
    assert!(config.authorise(None));
}

// --------------------------------------------------------------- backend ----

/// A backend that answers without a model, so the HTTP layer can be tested
/// without a GPU, a worker process or a 27 GB download.
struct FakeBackend {
    models: Vec<ModelEntry>,
    deltas: Vec<String>,
    fail: Option<String>,
}

#[async_trait::async_trait]
impl Backend for FakeBackend {
    async fn models(&self) -> Result<Vec<ModelEntry>, ApiError> {
        Ok(self.models.clone())
    }

    async fn chat(
        &self,
        model: &str,
        _messages: Vec<ChatMessage>,
        _max_tokens: u32,
        _temperature: f32,
    ) -> Result<ChunkStream, ApiError> {
        if model == "missing" {
            return Err(ApiError::ModelNotFound(model.to_owned()));
        }
        if let Some(message) = &self.fail {
            let message = message.clone();
            return Ok(Box::pin(async_stream::stream! {
                yield Err(ApiError::Backend(message));
            }));
        }

        let deltas = self.deltas.clone();
        Ok(Box::pin(async_stream::stream! {
            for delta in deltas {
                yield Ok(delta);
            }
        }))
    }

    async fn embeddings(
        &self,
        model: &str,
        inputs: Vec<String>,
    ) -> Result<EmbeddingsOutcome, ApiError> {
        if model == "missing" {
            return Err(ApiError::ModelNotFound(model.to_owned()));
        }

        // Two dimensions is enough to prove ordering and shape; the real vector
        // width comes from the runtime.
        Ok(EmbeddingsOutcome {
            vectors: inputs
                .iter()
                .map(|input| vec![input.len() as f32, 0.5])
                .collect(),
        })
    }
}

fn fake() -> Arc<dyn Backend> {
    Arc::new(FakeBackend {
        models: vec![ModelEntry::new("test/model", Some("llama".to_owned()))],
        deltas: vec!["Hello".to_owned(), ", ".to_owned(), "world".to_owned()],
        fail: None,
    })
}

/// Start a gateway on an ephemeral loopback port.
async fn start(
    config: ApiConfig,
    backend: Arc<dyn Backend>,
) -> (SocketAddr, gguf_engine_api_server::ApiServer) {
    let server = serve(config, backend, no_media())
        .await
        .expect("the gateway starts");
    (server.address(), server)
}

/// A media backend with nothing configured, so the media endpoints report the
/// capability as unavailable instead of being absent from the router.
fn no_media() -> Arc<dyn gguf_engine_api_server::MediaBackend> {
    Arc::new(gguf_engine_api_server::LocalMedia::default())
}

fn loopback_config() -> ApiConfig {
    ApiConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        token: None,
        allow_browser_origins: false,
    }
}

// ---------------------------------------------------------------- routes ----

#[tokio::test]
async fn health_reports_ok() {
    let (address, server) = start(loopback_config(), fake()).await;

    let body: serde_json::Value = reqwest::get(format!("http://{address}/health"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["status"], "ok");

    server.stop();
}

#[tokio::test]
async fn models_are_listed_from_the_backend() {
    let (address, server) = start(loopback_config(), fake()).await;

    let body: serde_json::Value = reqwest::get(format!("http://{address}/v1/models"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["object"], "list");
    assert_eq!(body["data"][0]["id"], "test/model");
    assert_eq!(body["data"][0]["object"], "model");

    server.stop();
}

#[tokio::test]
async fn a_non_streaming_completion_returns_the_whole_answer() {
    let (address, server) = start(loopback_config(), fake()).await;

    let client = reqwest::Client::new();
    let body: serde_json::Value = client
        .post(format!("http://{address}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": "test/model",
            "messages": [{ "role": "user", "content": "hi" }]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["object"], "chat.completion");
    assert_eq!(body["choices"][0]["message"]["content"], "Hello, world");
    assert_eq!(body["choices"][0]["message"]["role"], "assistant");
    assert_eq!(body["choices"][0]["finish_reason"], "stop");
    assert_eq!(body["usage"]["completion_tokens"], 3);

    server.stop();
}

#[tokio::test]
async fn a_streaming_completion_emits_openai_sse_frames() {
    let (address, server) = start(loopback_config(), fake()).await;

    let client = reqwest::Client::new();
    let text = client
        .post(format!("http://{address}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": "test/model",
            "messages": [{ "role": "user", "content": "hi" }],
            "stream": true
        }))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    // The first frame announces the role, then deltas, then finish, then [DONE].
    assert!(text.contains("chat.completion.chunk"), "{text}");
    assert!(text.contains("\"role\":\"assistant\""), "{text}");
    assert!(text.contains("Hello"), "{text}");
    assert!(text.contains("\"finish_reason\":\"stop\""), "{text}");
    assert!(text.trim_end().ends_with("[DONE]"), "{text}");

    server.stop();
}

#[tokio::test]
async fn an_unknown_model_is_a_404_in_openai_shape() {
    let (address, server) = start(loopback_config(), fake()).await;

    let response = reqwest::Client::new()
        .post(format!("http://{address}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": "missing",
            "messages": [{ "role": "user", "content": "hi" }]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "model_not_found");

    server.stop();
}

#[tokio::test]
async fn an_empty_message_list_is_a_400() {
    let (address, server) = start(loopback_config(), fake()).await;

    let response = reqwest::Client::new()
        .post(format!("http://{address}/v1/chat/completions"))
        .json(&serde_json::json!({ "model": "test/model", "messages": [] }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");

    server.stop();
}

#[tokio::test]
async fn a_configured_token_is_required() {
    let config = ApiConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        token: Some("0123456789abcdef".to_owned()),
        allow_browser_origins: false,
    };
    let (address, server) = start(config, fake()).await;

    let client = reqwest::Client::new();

    let unauthorised = client
        .get(format!("http://{address}/v1/models"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorised.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert!(
        unauthorised
            .headers()
            .contains_key(reqwest::header::WWW_AUTHENTICATE),
        "a 401 must say how to authenticate"
    );

    let authorised = client
        .get(format!("http://{address}/v1/models"))
        .bearer_auth("0123456789abcdef")
        .send()
        .await
        .unwrap();
    assert_eq!(authorised.status(), reqwest::StatusCode::OK);

    server.stop();
}

#[tokio::test]
async fn no_cors_headers_are_sent_unless_opted_in() {
    let (address, server) = start(loopback_config(), fake()).await;

    let response = reqwest::get(format!("http://{address}/health"))
        .await
        .unwrap();
    assert!(
        !response
            .headers()
            .contains_key(reqwest::header::ACCESS_CONTROL_ALLOW_ORIGIN),
        "a local endpoint reachable from any web page is a confused-deputy hole"
    );

    server.stop();
}

#[tokio::test]
async fn an_error_mid_stream_is_reported_as_a_data_event() {
    let backend = Arc::new(FakeBackend {
        models: Vec::new(),
        deltas: Vec::new(),
        fail: Some("the worker died".to_owned()),
    });
    let (address, server) = start(loopback_config(), backend).await;

    let text = reqwest::Client::new()
        .post(format!("http://{address}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": "test/model",
            "messages": [{ "role": "user", "content": "hi" }],
            "stream": true
        }))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    // The status was already sent, so the failure must arrive in the stream.
    assert!(text.contains("the worker died"), "{text}");

    server.stop();
}
#[tokio::test]
async fn embeddings_accept_a_single_string_input() {
    let (address, server) = start(loopback_config(), fake()).await;

    let body: serde_json::Value = reqwest::Client::new()
        .post(format!("http://{address}/v1/embeddings"))
        .json(&serde_json::json!({ "model": "test/model", "input": "hello" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["object"], "list");
    assert_eq!(body["data"][0]["object"], "embedding");
    assert_eq!(body["data"][0]["index"], 0);
    assert_eq!(body["data"][0]["embedding"].as_array().unwrap().len(), 2);

    server.stop();
}

#[tokio::test]
async fn embeddings_accept_an_array_and_preserve_order() {
    let (address, server) = start(loopback_config(), fake()).await;

    let body: serde_json::Value = reqwest::Client::new()
        .post(format!("http://{address}/v1/embeddings"))
        .json(&serde_json::json!({
            "model": "test/model",
            "input": ["a", "bbb"]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let data = body["data"].as_array().unwrap();
    assert_eq!(data.len(), 2);
    // The fake returns the input length as the first component, so ordering is
    // observable: "a" (1) must come before "bbb" (3).
    assert_eq!(data[0]["embedding"][0], 1.0);
    assert_eq!(data[1]["embedding"][0], 3.0);

    server.stop();
}

#[tokio::test]
async fn an_empty_embedding_input_is_a_400_and_loads_nothing() {
    let (address, server) = start(loopback_config(), fake()).await;

    let response = reqwest::Client::new()
        .post(format!("http://{address}/v1/embeddings"))
        .json(&serde_json::json!({ "model": "test/model", "input": [] }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);

    server.stop();
}

#[tokio::test]
async fn a_non_string_embedding_input_is_a_400() {
    let (address, server) = start(loopback_config(), fake()).await;

    let response = reqwest::Client::new()
        .post(format!("http://{address}/v1/embeddings"))
        .json(&serde_json::json!({ "model": "test/model", "input": [1, 2, 3] }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = response.json().await.unwrap();
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("array of strings"),
        "{body}"
    );

    server.stop();
}

#[tokio::test]
async fn an_unknown_model_for_embeddings_is_a_404() {
    let (address, server) = start(loopback_config(), fake()).await;

    let response = reqwest::Client::new()
        .post(format!("http://{address}/v1/embeddings"))
        .json(&serde_json::json!({ "model": "missing", "input": "hi" }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);

    server.stop();
}
#[tokio::test]
async fn an_image_request_without_a_runtime_is_a_503_with_a_reason() {
    let (address, server) = start(loopback_config(), fake()).await;

    let response = reqwest::Client::new()
        .post(format!("http://{address}/v1/images/generations"))
        .json(&serde_json::json!({ "prompt": "a cat" }))
        .send()
        .await
        .unwrap();

    // 503, not 500: the server is fine, the capability is missing.
    assert_eq!(response.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "capability_not_available");
    assert!(
        !body["error"]["message"].as_str().unwrap().is_empty(),
        "a missing capability must say why: {body}"
    );

    server.stop();
}

#[tokio::test]
async fn an_empty_image_prompt_is_a_400() {
    let (address, server) = start(loopback_config(), fake()).await;

    let response = reqwest::Client::new()
        .post(format!("http://{address}/v1/images/generations"))
        .json(&serde_json::json!({ "prompt": "   " }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    server.stop();
}

#[tokio::test]
async fn asking_for_several_images_is_refused_rather_than_silently_reduced() {
    let (address, server) = start(loopback_config(), fake()).await;

    let response = reqwest::Client::new()
        .post(format!("http://{address}/v1/images/generations"))
        .json(&serde_json::json!({ "prompt": "a cat", "n": 4 }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = response.json().await.unwrap();
    assert!(body["error"]["message"].as_str().unwrap().contains("n > 1"));
    server.stop();
}

#[tokio::test]
async fn speech_without_a_runtime_is_a_503() {
    let (address, server) = start(loopback_config(), fake()).await;

    let response = reqwest::Client::new()
        .post(format!("http://{address}/v1/audio/speech"))
        .json(&serde_json::json!({ "input": "hello" }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    server.stop();
}

#[tokio::test]
async fn an_unsupported_speech_format_is_refused_rather_than_mislabelled() {
    let (address, server) = start(loopback_config(), fake()).await;

    let response = reqwest::Client::new()
        .post(format!("http://{address}/v1/audio/speech"))
        .json(&serde_json::json!({ "input": "hello", "response_format": "mp3" }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    server.stop();
}

#[tokio::test]
async fn a_transcription_without_a_file_is_a_400() {
    let (address, server) = start(loopback_config(), fake()).await;

    let form = reqwest::multipart::Form::new().text("model", "whisper");
    let response = reqwest::Client::new()
        .post(format!("http://{address}/v1/audio/transcriptions"))
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    server.stop();
}
