//! `MediaService`: image, video and speech over the local transport.
//!
//! The shells have no HTTP client and should not grow one: they already speak
//! gRPC to the core, so the media operations ride the same transport. The
//! implementation itself is `gguf_engine_multimodal::MediaEngine`, shared with
//! the OpenAI-compatible gateway so the two cannot disagree about what is
//! available or why something failed.

use std::pin::Pin;

use gguf_engine_multimodal::MediaEngine;
use gguf_engine_protocol::pb;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};
use tracing::{debug, warn};

use crate::state::CoreState;

/// `MediaService` implementation.
#[derive(Debug)]
pub struct MediaService {
    engine: MediaEngine,
}

impl MediaService {
    /// Create the service from the core's configuration.
    pub fn new(state: &CoreState) -> Self {
        let config = &state.config.multimodal;
        Self {
            engine: MediaEngine::new(
                config.image_model.clone(),
                config.diffusion_url.clone(),
                config.asr_model.clone(),
                config.tts_voice.clone(),
                config.music_model.clone(),
                config.audio_backend.clone(),
            ),
        }
    }

    /// Translate a multimodal failure into a gRPC status.
    ///
    /// A missing runtime is `FAILED_PRECONDITION`, not `INTERNAL`: nothing is
    /// broken, the machine is not set up, and the message says what to install.
    fn status(error: gguf_engine_multimodal::MultimodalError) -> Status {
        match error {
            gguf_engine_multimodal::MultimodalError::Invalid(message) => {
                Status::invalid_argument(message)
            }
            gguf_engine_multimodal::MultimodalError::NotAvailable(message) => {
                Status::failed_precondition(message)
            }
            gguf_engine_multimodal::MultimodalError::Runtime(message) => Status::internal(message),
        }
    }
}

/// How a video job event reaches the caller.
///
/// The render runs in its own task and reports through this channel: the
/// adapter's progress callback is synchronous but the stream is not, so a channel
/// is the seam between them.
enum JobEvent {
    Progress(gguf_engine_multimodal::VideoProgress),
    Finished(
        Result<gguf_engine_multimodal::GeneratedVideo, gguf_engine_multimodal::MultimodalError>,
    ),
}

/// Map a runtime progress report onto the wire type.
fn to_wire_progress(progress: &gguf_engine_multimodal::VideoProgress) -> pb::VideoProgressEvent {
    pb::VideoProgressEvent {
        status: progress.status.clone(),
        stage: progress.stage.clone(),
        detail: progress.detail.clone(),
        completed: progress.completed,
        total: progress.total,
        unit: progress.unit.clone(),
        elapsed_ms: progress.elapsed_ms,
        queue_position: progress.queue_position,
        clip: None,
        error: None,
    }
}

#[tonic::async_trait]
impl pb::media_service_server::MediaService for MediaService {
    async fn generate_image(
        &self,
        request: Request<pb::ImageRequest>,
    ) -> Result<Response<pb::ImageReply>, Status> {
        let request = request.into_inner();

        let mut image = gguf_engine_multimodal::ImageRequest::new(&request.prompt);
        if let Some(negative) = request.negative_prompt {
            image.negative_prompt = negative;
        }
        if request.width > 0 {
            image.width = request.width;
        }
        if request.height > 0 {
            image.height = request.height;
        }
        if request.steps > 0 {
            image.steps = request.steps;
        }
        image.seed = request.seed;

        debug!(
            width = image.width,
            height = image.height,
            "generating an image"
        );

        let generated = self.engine.image(&image).await.map_err(|error| {
            warn!(error = %error, "image generation failed");
            Self::status(error)
        })?;

        Ok(Response::new(pb::ImageReply {
            png: generated.bytes,
            width: image.width,
            height: image.height,
        }))
    }

    type GenerateVideoStream =
        Pin<Box<dyn Stream<Item = Result<pb::VideoProgressEvent, Status>> + Send>>;

    async fn generate_video(
        &self,
        request: Request<pb::VideoRequest>,
    ) -> Result<Response<Self::GenerateVideoStream>, Status> {
        let request = request.into_inner();

        let mut video = gguf_engine_multimodal::VideoRequest::new(&request.prompt);
        if request.width > 0 {
            video.width = request.width;
        }
        if request.height > 0 {
            video.height = request.height;
        }
        if request.frames > 0 {
            video.frames = request.frames;
        }
        if request.fps > 0 {
            video.fps = request.fps;
        }
        if request.steps > 0 {
            video.steps = request.steps;
        }
        video.seed = request.seed;
        let engine = self.engine.clone();

        // An unbounded channel is safe here because the producer emits at most one
        // event per poll interval (seconds), so a slow consumer cannot grow the
        // queue without bound in any realistic render.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<JobEvent>();

        tokio::spawn(async move {
            let result = engine
                .video_with_progress(&video, |progress| {
                    // A closed receiver means the caller went away; the send
                    // simply fails and the render is left to finish or fail on
                    // its own.
                    let _ = tx.send(JobEvent::Progress(progress));
                })
                .await;
            let _ = tx.send(JobEvent::Finished(result));
        });

        let stream = async_stream::stream! {
            while let Some(event) = rx.recv().await {
                match event {
                    JobEvent::Progress(progress) => {
                        yield Ok(to_wire_progress(&progress));
                    }
                    JobEvent::Finished(Ok(clip)) => {
                        yield Ok(pb::VideoProgressEvent {
                            status: "completed".to_owned(),
                            stage: String::new(),
                            detail: String::new(),
                            completed: None,
                            total: None,
                            unit: None,
                            elapsed_ms: 0,
                            queue_position: 0,
                            clip: Some(pb::VideoReply {
                                video: clip.bytes,
                                mime_type: clip.mime_type,
                                frames: clip.frames,
                            }),
                            error: None,
                        });
                        return;
                    }
                    JobEvent::Finished(Err(error)) => {
                        warn!(error = %error, "video generation failed");
                        // The failure is reported in the stream rather than as a
                        // status: the call already succeeded, and the client is
                        // watching events.
                        yield Ok(pb::VideoProgressEvent {
                            status: "failed".to_owned(),
                            stage: String::new(),
                            detail: String::new(),
                            completed: None,
                            total: None,
                            unit: None,
                            elapsed_ms: 0,
                            queue_position: 0,
                            clip: None,
                            error: Some(error.to_string()),
                        });
                        return;
                    }
                }
            }
        };

        Ok(Response::new(Box::pin(stream) as Self::GenerateVideoStream))
    }

    async fn synthesise(
        &self,
        request: Request<pb::SpeechRequest>,
    ) -> Result<Response<pb::SpeechReply>, Status> {
        let request = request.into_inner();

        // Piper blocks while it synthesises, so it runs off the reactor.
        let engine = self.engine.clone();
        let text = request.text;
        let speech = tokio::task::spawn_blocking(move || engine.speech(&text))
            .await
            .map_err(|error| Status::internal(format!("the synthesis task failed: {error}")))
            .and_then(|result| result.map_err(Self::status))?;

        Ok(Response::new(pb::SpeechReply { wav: speech.wav }))
    }

    async fn transcribe(
        &self,
        request: Request<pb::TranscriptionRequest>,
    ) -> Result<Response<pb::TranscriptionReply>, Status> {
        let request = request.into_inner();

        if request.audio.is_empty() {
            return Err(Status::invalid_argument("the audio is empty"));
        }

        // whisper.cpp reads a file, so the bytes are staged and removed. The
        // extension is kept because the tool uses it to pick the container.
        let extension = std::path::Path::new(&request.filename)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("wav")
            .to_owned();
        let path = std::env::temp_dir().join(format!(
            "gguf-grpc-audio-{}.{extension}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));

        tokio::fs::write(&path, &request.audio)
            .await
            .map_err(|error| Status::internal(format!("staging the audio: {error}")))?;

        let engine = self.engine.clone();
        let language = request.language.clone();
        let staged = path.clone();
        let result =
            tokio::task::spawn_blocking(move || engine.transcribe(&staged, language.as_deref()))
                .await
                .map_err(|error| {
                    Status::internal(format!("the transcription task failed: {error}"))
                });

        let _ = tokio::fs::remove_file(&path).await;

        let transcription = result.and_then(|inner| inner.map_err(Self::status))?;

        Ok(Response::new(pb::TranscriptionReply {
            text: transcription.text,
        }))
    }
}
