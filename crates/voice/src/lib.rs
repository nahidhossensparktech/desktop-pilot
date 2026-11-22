//! Voice assistant building blocks (ROADMAP M7): microphone capture, speech
//! detection, the "Hey Mouse" wake phrase and local Whisper speech-to-text.
//!
//! Everything runs on this computer; audio is never stored or sent anywhere. Only the
//! recognized text leaves this crate. No OS-specific code: `cpal` and whisper.cpp
//! handle the platforms.

pub mod assistant;
pub mod audio;
pub mod model;
pub mod script;
pub mod stt;
pub mod vad;
pub mod wake;

pub use assistant::{Assistant, AssistantConfig, VoiceEvent};
pub use model::{ModelSize, download};
pub use stt::{Language, Transcriber, Transcript};
pub use wake::{find_wake, is_meaningful_command, is_stop_command, strip_wake};

/// Errors from the voice assistant.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VoiceError {
    #[error("microphone: {0}")]
    Microphone(String),
    #[error("speech model: {0}")]
    Model(String),
    #[error("model download failed: {0}")]
    Download(String),
    #[error("speech recognition failed: {0}")]
    Transcribe(String),
    #[error("file error: {0}")]
    Io(#[from] std::io::Error),
}
