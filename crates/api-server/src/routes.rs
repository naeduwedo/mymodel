//! The HTTP surface (spec §9.2).
//!
//! Three endpoints, each backed by something that actually exists:
//!
//! | Endpoint | Backed by |
//! | --- | --- |
//! | `GET /health` | the gateway's own liveness |
//! | `GET /v1/models` | the core's model registry |
//! | `POST /v1/chat/completions` | `InferenceService` over the local transport |
//!
//! Nothing else is exposed. A route that returned a placeholder would be a fake
//! feature (spec rule 4).

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use tokio_stream::StreamExt;

use axum::extract::Multipart;
use axum::http::header as http_header;

use crate::backend::Backend;
use crate::config::ApiConfig;
use crate::error::ApiError;
use crate::media::MediaBackend;
use crate::openai::{
    AudioSpeechRequest, ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse,
    ChatMessage, Choice, ChunkChoice, Delta, EmbeddingData, EmbeddingsRequest, EmbeddingsResponse,
    ImageGenerationRequest, ModelList, TranscriptionResponse, Usage,
};

/// Shared state for every handler.
#[derive(Clone)]
pub struct AppState {
    /// Configuration (token, exposure).
    pub config: Arc<ApiConfig>,
    /// The engine.
    pub backend: Arc<dyn Backend>,
    /// The media runtimes behind the image and audio endpoints.
    pub media: Arc<dyn MediaBackend>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The backend is a trait object and is not itself Debug; naming the
        // configuration is enough to identify a state in a log line.
        f.debug_struct("AppState")
            .field("bind", &self.config.bind)
            .field("exposure", &self.config.exposure())
            .finish_non_exhaustive()
    }
}

/// Build the router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/models", get(list_models))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/embeddings", post(embeddings))
        .route("/v1/images/generations", post(images_generations))
        .route("/v1/audio/speech", post(audio_speech))
        .route("/v1/audio/transcriptions", post(audio_transcriptions))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            authorise_and_set_headers,
        ))
        .with_state(state)
}

/// Reject unauthorised calls and add the headers a browser client needs.
async fn authorise_and_set_headers(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim);

    if !state.config.authorise(presented) {
        let mut response = ApiError::Unauthorized.into_response();
        // A 401 without this header leaves clients guessing.
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        return response;
    }

    let mut response = next.run(request).await;
    apply_cors(&state.config, response.headers_mut());
    response
}

/// Apply CORS only when the operator opted in.
///
/// The default is no CORS headers at all: a local model endpoint that any web
/// page can call is a confused-deputy hole, and silence is the safe default.
fn apply_cors(config: &ApiConfig, headers: &mut axum::http::HeaderMap) {
    if !config.allow_browser_origins {
        return;
    }
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("authorization, content-type"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

async fn list_models(State(state): State<AppState>) -> Result<Json<ModelList>, ApiError> {
    let models = state.backend.models().await?;
    Ok(Json(ModelList::new(models)))
}

async fn chat_completions(
    State(state): State<AppState>,
    body: Result<Json<ChatCompletionRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Response, ApiError> {
    let Json(request) = body?;

    if request.messages.is_empty() {
        return Err(ApiError::Invalid("messages must not be empty".to_owned()));
    }
    if request.model.trim().is_empty() {
        return Err(ApiError::Invalid("model must not be empty".to_owned()));
    }

    let max_tokens = request.max_tokens.unwrap_or(512).max(1);
    let temperature = request.temperature.unwrap_or(0.7);
    let created = unix_seconds();
    let id = format!("chatcmpl-{created}-{}", short_hash(&request.model));

    let stream = state
        .backend
        .chat(
            &request.model,
            request.messages.clone(),
            max_tokens,
            temperature,
        )
        .await?;

    if request.stream {
        return Ok(streaming_response(id, created, request.model, stream));
    }

    // Non-streaming drains the same stream, so the two modes cannot disagree.
    let mut stream = stream;
    let mut text = String::new();
    let mut tokens = 0u32;
    while let Some(item) = stream.next().await {
        text.push_str(&item?);
        tokens += 1;
    }

    Ok(Json(ChatCompletionResponse {
        id,
        object: "chat.completion".to_owned(),
        created,
        model: request.model,
        choices: vec![Choice {
            index: 0,
            message: ChatMessage {
                role: "assistant".to_owned(),
                content: text,
            },
            finish_reason: "stop".to_owned(),
        }],
        usage: Usage {
            // The runtime does not report prompt tokens; 0 is honest, a guess
            // would not be.
            prompt_tokens: 0,
            completion_tokens: tokens,
            total_tokens: tokens,
        },
    })
    .into_response())
}

async fn embeddings(
    State(state): State<AppState>,
    body: Result<Json<EmbeddingsRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<EmbeddingsResponse>, ApiError> {
    let Json(request) = body?;

    if request.model.trim().is_empty() {
        return Err(ApiError::Invalid("model must not be empty".to_owned()));
    }

    // Validate the input shape before touching the runtime: a malformed request
    // must not cost a model load.
    let model = request.model.clone();
    let inputs = request.into_inputs()?;
    if inputs.is_empty() {
        return Err(ApiError::Invalid("input must not be empty".to_owned()));
    }

    let outcome = state.backend.embeddings(&model, inputs.clone()).await?;

    let data: Vec<EmbeddingData> = outcome
        .vectors
        .into_iter()
        .enumerate()
        .map(|(index, embedding)| EmbeddingData {
            object: "embedding".to_owned(),
            index: index as u32,
            embedding,
        })
        .collect();

    Ok(Json(EmbeddingsResponse {
        object: "list".to_owned(),
        data,
        model,
        usage: Usage {
            // The runtime reports no prompt tokens for embedding input; 0 is
            // honest, a guess would not be.
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
        },
    }))
}

async fn images_generations(
    State(state): State<AppState>,
    body: Result<Json<ImageGenerationRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<crate::openai::ImageGenerationResponse>, ApiError> {
    let Json(request) = body?;

    if request.prompt.trim().is_empty() {
        return Err(ApiError::Invalid("prompt must not be empty".to_owned()));
    }
    // One image per request: the endpoint accepts `n`, but the runtime is asked
    // for one and repeating it would multiply a slow operation silently.
    if request.n.is_some_and(|n| n != 1) {
        return Err(ApiError::Invalid(
            "n > 1 is not supported by this gateway; send separate requests".to_owned(),
        ));
    }

    let model = request
        .model
        .clone()
        .unwrap_or_else(|| "diffusion".to_owned());
    let bytes = state.media.image(&request).await?;

    Ok(Json(crate::media::image_response(bytes, model)))
}

async fn audio_speech(
    State(state): State<AppState>,
    body: Result<Json<AudioSpeechRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Response, ApiError> {
    let Json(request) = body?;

    if request.input.trim().is_empty() {
        return Err(ApiError::Invalid("input must not be empty".to_owned()));
    }
    // Only wav is produced. Claiming mp3 by re-encoding would need another
    // dependency and would lie about the bytes if it failed halfway.
    if let Some(format) = &request.response_format
        && !format.eq_ignore_ascii_case("wav")
        && !format.eq_ignore_ascii_case("pcm")
    {
        return Err(ApiError::Invalid(format!(
            "response_format '{format}' is not supported; use wav"
        )));
    }

    let bytes = state.media.speech(&request).await?;

    let mut response = (StatusCode::OK, bytes).into_response();
    response.headers_mut().insert(
        http_header::CONTENT_TYPE,
        HeaderValue::from_static("audio/wav"),
    );
    Ok(response)
}

async fn audio_transcriptions(
    State(state): State<AppState>,
    // Multipart is what OpenAI clients send, so it is what is accepted.
    mut multipart: Multipart,
) -> Result<Json<TranscriptionResponse>, ApiError> {
    let mut audio: Option<(Vec<u8>, String)> = None;
    let mut language: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|err| ApiError::Invalid(format!("reading the upload: {err}")))?
    {
        match field.name() {
            Some("file") => {
                let filename = field.file_name().unwrap_or("audio.wav").to_owned();
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|err| ApiError::Invalid(format!("reading the file: {err}")))?;
                if bytes.is_empty() {
                    return Err(ApiError::Invalid("the uploaded file is empty".to_owned()));
                }
                audio = Some((bytes.to_vec(), filename));
            }
            Some("language") => {
                language = Some(
                    field
                        .text()
                        .await
                        .map_err(|err| ApiError::Invalid(format!("reading language: {err}")))?,
                );
            }
            _ => {}
        }
    }

    let Some((bytes, filename)) = audio else {
        return Err(ApiError::Invalid(
            "a 'file' part with the audio is required".to_owned(),
        ));
    };

    let text = state.media.transcribe(bytes, &filename, language).await?;
    Ok(Json(TranscriptionResponse { text }))
}

fn streaming_response(
    id: String,
    created: u64,
    model: String,
    stream: crate::backend::ChunkStream,
) -> Response {
    let mut stream = stream;
    let model_for_stream = model.clone();
    let id_for_stream = id.clone();

    let events = async_stream::stream! {
        // The first chunk carries the role, as OpenAI clients expect.
        yield Ok::<Event, ApiError>(Event::default().data(
            serde_json::to_string(&ChatCompletionChunk {
                id: id_for_stream.clone(),
                object: "chat.completion.chunk".to_owned(),
                created,
                model: model_for_stream.clone(),
                choices: vec![ChunkChoice {
                    index: 0,
                    delta: Delta {
                        role: Some("assistant".to_owned()),
                        content: None,
                    },
                    finish_reason: None,
                }],
            })
            .unwrap_or_default(),
        ));

        while let Some(item) = stream.next().await {
            match item {
                Ok(delta) => {
                    let chunk = ChatCompletionChunk {
                        id: id_for_stream.clone(),
                        object: "chat.completion.chunk".to_owned(),
                        created,
                        model: model_for_stream.clone(),
                        choices: vec![ChunkChoice {
                            index: 0,
                            delta: Delta {
                                role: None,
                                content: Some(delta),
                            },
                            finish_reason: None,
                        }],
                    };
                    yield Ok(Event::default().data(
                        serde_json::to_string(&chunk).unwrap_or_default(),
                    ));
                }
                Err(err) => {
                    // Errors mid-stream cannot change the HTTP status, so they
                    // are sent as a final data event the client can recognise.
                    let body = serde_json::json!({
                        "error": {
                            "message": err.to_string(),
                            "type": "api_error",
                        }
                    });
                    yield Ok(Event::default().data(body.to_string()));
                    break;
                }
            }
        }

        // The final chunk: finish_reason and the [DONE] sentinel.
        let last = ChatCompletionChunk {
            id: id_for_stream.clone(),
            object: "chat.completion.chunk".to_owned(),
            created,
            model: model_for_stream,
            choices: vec![ChunkChoice {
                index: 0,
                delta: Delta {
                    role: None,
                    content: None,
                },
                finish_reason: Some("stop".to_owned()),
            }],
        };
        yield Ok(Event::default().data(
            serde_json::to_string(&last).unwrap_or_default(),
        ));
        yield Ok(Event::default().data("[DONE]"));
    };

    Sse::new(events).into_response()
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// A short stable suffix so response ids differ per model.
fn short_hash(value: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:016x}", hash)
}
