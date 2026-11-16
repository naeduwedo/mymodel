//! Video generation through stable-diffusion.cpp's async job API.
//!
//! Unlike images, video is asynchronous: `POST /sdcpp/v1/vid_gen` queues a job and
//! answers 202 with a job id, and the engine polls `/sdcpp/v1/jobs/<id>` until it
//! completes. That is the runtime's design, not a choice made here - a clip takes
//! minutes, so a synchronous request would time out long before it finished.

use serde::{Deserialize, Serialize};

use crate::error::MultimodalError;

/// What to generate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoRequest {
    /// The prompt.
    pub prompt: String,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Frames to generate.
    pub frames: u32,
    /// Frames per second of the output.
    pub fps: u32,
    /// Sampling steps.
    ///
    /// Without this the runtime falls back to its own default (20), which on this
    /// machine tripled the render time over the 6 steps a first clip needs.
    pub steps: u32,
    /// Seed; `None` lets the runtime choose.
    pub seed: Option<i64>,
}

impl VideoRequest {
    /// A small clip by default: video is expensive, and a first request that
    /// takes ten minutes is a bad first impression.
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            width: 512,
            height: 320,
            frames: 17,
            fps: 8,
            steps: 8,
            seed: None,
        }
    }
}

/// A generated clip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedVideo {
    /// Encoded container bytes.
    pub bytes: Vec<u8>,
    /// MIME type the runtime reported, e.g. `video/mp4`.
    pub mime_type: String,
    /// Frames actually produced.
    pub frames: u32,
}

/// How far along a clip is.
///
/// Mirrors what the runtime reports, because inventing a percentage from elapsed
/// time would be a guess that gets more wrong the slower the machine is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoProgress {
    /// Runtime status: queued, generating, encoding, ...
    pub status: String,
    /// Stage the runtime is in, e.g. "denoise".
    pub stage: String,
    /// Free-form detail from the runtime.
    pub detail: String,
    /// Units done in this stage, when the runtime counts them.
    pub completed: Option<u32>,
    /// Units in this stage, when the runtime knows them.
    pub total: Option<u32>,
    /// What the units are, e.g. "steps".
    pub unit: Option<String>,
    /// Whole-job elapsed milliseconds.
    pub elapsed_ms: u64,
    /// Place in the queue (1 = next) while waiting.
    pub queue_position: u32,
}

impl VideoProgress {
    /// Fraction complete, when the runtime reports both counts.
    pub fn fraction(&self) -> Option<f64> {
        match (self.completed, self.total) {
            (Some(done), Some(total)) if total > 0 => {
                Some((f64::from(done) / f64::from(total)).clamp(0.0, 1.0))
            }
            _ => None,
        }
    }
}

/// How long to wait between polls.
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// How long to wait for a clip before giving up.
pub const VIDEO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3600);

/// Generate a clip, reporting progress through `on_progress` as it advances.
///
/// The callback is invoked once per poll with the runtime's own figures. A caller
/// that does not care can pass `|_| {}`; there is no second implementation to
/// drift from this one.
pub async fn generate_with_progress<F>(
    base_url: &str,
    request: &VideoRequest,
    mut on_progress: F,
) -> Result<GeneratedVideo, MultimodalError>
where
    F: FnMut(VideoProgress),
{
    if request.prompt.trim().is_empty() {
        return Err(MultimodalError::Invalid(
            "the prompt must not be empty".to_owned(),
        ));
    }
    if request.frames == 0 {
        return Err(MultimodalError::Invalid(
            "frames must not be zero".to_owned(),
        ));
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|err| MultimodalError::Runtime(format!("building the HTTP client: {err}")))?;

    let body = serde_json::json!({
        "prompt": request.prompt,
        "width": request.width,
        "height": request.height,
        "video_frames": request.frames,
        "fps": request.fps,
        "seed": request.seed,
        // Step count lives under sample_params in the runtime's schema; passing
        // it at the top level silently does nothing.
        "sample_params": { "sample_steps": request.steps },
    });

    let response = client
        .post(format!("{base_url}/sdcpp/v1/vid_gen"))
        .json(&body)
        .send()
        .await
        .map_err(|err| MultimodalError::Runtime(format!("queueing the clip: {err}")))?;

    let status = response.status();
    let queued: serde_json::Value = response
        .json()
        .await
        .map_err(|err| MultimodalError::Runtime(format!("reading the job reply: {err}")))?;

    if !status.is_success() {
        let message = queued
            .get("error")
            .and_then(|value| value.as_str())
            .unwrap_or("the runtime refused the job");
        return Err(MultimodalError::NotAvailable(format!(
            "this runtime cannot generate video: {message}"
        )));
    }

    let job_id = queued
        .get("id")
        .and_then(|value| value.as_str())
        .ok_or_else(|| MultimodalError::Runtime("the runtime returned no job id".to_owned()))?;

    let deadline = tokio::time::Instant::now() + VIDEO_TIMEOUT;
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(MultimodalError::Runtime(format!(
                "the clip did not finish within {} seconds",
                VIDEO_TIMEOUT.as_secs()
            )));
        }

        tokio::time::sleep(POLL_INTERVAL).await;

        let response = client
            .get(format!("{base_url}/sdcpp/v1/jobs/{job_id}"))
            .send()
            .await
            .map_err(|err| MultimodalError::Runtime(format!("polling the job: {err}")))?;

        let job: serde_json::Value = response
            .json()
            .await
            .map_err(|err| MultimodalError::Runtime(format!("reading the job: {err}")))?;

        let status = job
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown")
            .to_owned();

        on_progress(read_progress(&job, &status));

        match status.as_str() {
            "completed" => return decode(&job),
            "failed" | "cancelled" => {
                return Err(MultimodalError::Runtime(format!(
                    "the runtime could not make the clip: {}",
                    job_error(&job)
                )));
            }
            // queued and running are both "keep waiting".
            _ => continue,
        }
    }
}

/// Read the progress block out of a job document.
fn read_progress(job: &serde_json::Value, status: &str) -> VideoProgress {
    let progress = job.get("progress");
    let number = |key: &str| {
        progress
            .and_then(|value| value.get(key))
            .and_then(|value| value.as_u64())
    };
    let text = |key: &str| {
        progress
            .and_then(|value| value.get(key))
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_owned()
    };

    VideoProgress {
        status: status.to_owned(),
        stage: text("stage"),
        detail: text("detail"),
        completed: number("completed").map(|value| value as u32),
        total: number("total").map(|value| value as u32),
        unit: progress
            .and_then(|value| value.get("unit"))
            .and_then(|value| value.as_str())
            .map(str::to_owned),
        elapsed_ms: number("elapsed_ms").unwrap_or_default(),
        queue_position: job
            .get("queue_position")
            .and_then(|value| value.as_u64())
            .unwrap_or_default() as u32,
    }
}

/// Generate a clip without reporting progress.
pub async fn generate(
    base_url: &str,
    request: &VideoRequest,
) -> Result<GeneratedVideo, MultimodalError> {
    generate_with_progress(base_url, request, |_| {}).await
}

/// Pull the clip out of a completed job.
fn decode(job: &serde_json::Value) -> Result<GeneratedVideo, MultimodalError> {
    let result = job
        .get("result")
        .ok_or_else(|| MultimodalError::Runtime("a completed job had no result".to_owned()))?;

    let encoded = result
        .get("b64_json")
        .and_then(|value| value.as_str())
        .ok_or_else(|| MultimodalError::Runtime("the job produced no video bytes".to_owned()))?;

    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|err| MultimodalError::Runtime(format!("decoding the clip: {err}")))?;

    if bytes.is_empty() {
        return Err(MultimodalError::Runtime(
            "the runtime returned an empty clip".to_owned(),
        ));
    }

    Ok(GeneratedVideo {
        bytes,
        mime_type: result
            .get("mime_type")
            .and_then(|value| value.as_str())
            .unwrap_or("video/mp4")
            .to_owned(),
        frames: result
            .get("frame_count")
            .and_then(|value| value.as_u64())
            .unwrap_or(0) as u32,
    })
}

fn job_error(job: &serde_json::Value) -> String {
    job.get("error")
        .and_then(|error| {
            error
                .get("message")
                .and_then(|value| value.as_str())
                .or_else(|| error.as_str())
        })
        .unwrap_or("no reason given")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_empty_prompt_is_refused_before_any_request() {
        let request = VideoRequest::new("  ");
        let error = generate("http://127.0.0.1:1", &request).await.unwrap_err();
        assert!(matches!(error, MultimodalError::Invalid(_)), "{error}");
    }

    #[tokio::test]
    async fn zero_frames_is_refused() {
        let mut request = VideoRequest::new("a fox");
        request.frames = 0;
        let error = generate("http://127.0.0.1:1", &request).await.unwrap_err();
        assert!(matches!(error, MultimodalError::Invalid(_)), "{error}");
    }

    #[test]
    fn the_defaults_are_a_short_small_clip() {
        let request = VideoRequest::new("a fox");
        assert!(
            request.frames <= 32,
            "a first clip must not take ten minutes"
        );
        assert_eq!(request.fps, 8);
    }

    #[test]
    fn a_completed_job_is_decoded_from_its_result() {
        let job = serde_json::json!({
            "status": "completed",
            "result": {
                "b64_json": "AQID",
                "mime_type": "video/mp4",
                "frame_count": 17
            }
        });
        let clip = decode(&job).expect("decoded");
        assert_eq!(clip.bytes, vec![1, 2, 3]);
        assert_eq!(clip.mime_type, "video/mp4");
        assert_eq!(clip.frames, 17);
    }

    #[test]
    fn a_completed_job_without_bytes_is_an_error_not_an_empty_clip() {
        let job = serde_json::json!({ "status": "completed", "result": {} });
        assert!(decode(&job).is_err());
    }

    #[test]
    fn progress_is_read_from_the_job_document() {
        let job = serde_json::json!({
            "status": "running",
            "queue_position": 0,
            "progress": {
                "stage": "denoise",
                "detail": "sampling",
                "pass": 1,
                "completed": 3,
                "total": 6,
                "unit": "steps",
                "elapsed_ms": 12_345
            }
        });

        let progress = read_progress(&job, "running");
        assert_eq!(progress.stage, "denoise");
        assert_eq!(progress.completed, Some(3));
        assert_eq!(progress.total, Some(6));
        assert_eq!(progress.unit.as_deref(), Some("steps"));
        assert_eq!(progress.elapsed_ms, 12_345);
        assert!((progress.fraction().unwrap() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_stage_without_counts_has_no_fraction() {
        // Encoding has no step counter; reporting a fraction there would be a
        // guess presented as a measurement.
        let job = serde_json::json!({
            "status": "encoding",
            "progress": { "stage": "encoding", "completed": null, "total": null }
        });
        let progress = read_progress(&job, "encoding");
        assert_eq!(progress.fraction(), None);
    }

    #[test]
    fn a_queued_job_reports_its_position() {
        let job = serde_json::json!({ "status": "queued", "queue_position": 3 });
        let progress = read_progress(&job, "queued");
        assert_eq!(progress.queue_position, 3);
        assert_eq!(progress.status, "queued");
    }

    #[test]
    fn a_failure_reason_is_read_in_both_shapes() {
        let object = serde_json::json!({ "error": { "message": "out of memory" } });
        assert_eq!(job_error(&object), "out of memory");
        let text = serde_json::json!({ "error": "cancelled" });
        assert_eq!(job_error(&text), "cancelled");
        assert_eq!(job_error(&serde_json::json!({})), "no reason given");
    }
}
