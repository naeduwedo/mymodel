//! Non-LLM runtimes: image generation, video, speech and music (spec §2.3, §17 Task 8).
//!
//! # Why this is a separate crate
//!
//! These runtimes share a shape the LLM adapter does not: each is a separate
//! upstream project with its own manifest, its own executable and its own
//! protocol. Keeping them together makes the shared part - declare, probe, gate -
//! visible, and stops each adapter growing its own version of it.
//!
//! # The rule this crate enforces
//!
//! A modality is offered because a runtime that claims it was found on this
//! machine, not because someone wrote the modality into a list. Every status
//! reported here traces back to a probe.

#![warn(missing_docs)]

pub mod engine;
pub mod error;
pub mod image;
pub mod manifest;
pub mod modality;
pub mod music;
pub mod probe;
pub mod speech;
pub mod video;

pub use engine::{MediaEngine, runtime_binary};
pub use error::MultimodalError;
pub use image::{GeneratedImage, ImageRequest, generate as generate_image};
pub use manifest::{Invocation, RuntimeKind, RuntimeManifest, builtin_manifests};
pub use modality::{ModalityReport, ModalityState, modality_report};
pub use music::{AudioBackend, GeneratedAudio, MusicRequest, generate as generate_music};
pub use probe::{
    RUNTIME_ENV, RuntimeProbe, http_base, locate, probe, probe_all, runtime_for, search_dirs,
};
pub use speech::{Speech, Transcription, clean_transcript, synthesise, transcribe};
pub use video::{
    GeneratedVideo, VideoProgress, VideoRequest, generate as generate_video,
    generate_with_progress as generate_video_with_progress,
};
