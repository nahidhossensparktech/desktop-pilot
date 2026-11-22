//! Whisper model files: which ones exist, where they live, and downloading them once
//! (with a SHA-256 check) from the official whisper.cpp repository on Hugging Face.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::VoiceError;

const BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

/// Model size: bigger understands Bangla better but is slower.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModelSize {
    Base,
    Small,
    Medium,
    /// Whisper large-v3-turbo: far better Bangla (writes Bangla script, gets more words
    /// right) at about 3 s per command on Apple Silicon.
    Turbo,
}

impl ModelSize {
    pub const ALL: [Self; 4] = [Self::Turbo, Self::Small, Self::Medium, Self::Base];

    /// File name in the whisper.cpp repository (8-bit / 5-bit quantized: about half
    /// the size of the full models with nearly the same accuracy).
    pub fn file_name(self) -> &'static str {
        match self {
            Self::Base => "ggml-base-q8_0.bin",
            Self::Small => "ggml-small-q8_0.bin",
            Self::Medium => "ggml-medium-q5_0.bin",
            Self::Turbo => "ggml-large-v3-turbo-q5_0.bin",
        }
    }

    /// Exact download size in bytes.
    pub fn bytes(self) -> u64 {
        match self {
            Self::Base => 81_768_585,
            Self::Small => 264_464_607,
            Self::Medium => 539_212_467,
            Self::Turbo => 574_041_195,
        }
    }

    /// SHA-256 published by Hugging Face for the file (checked after download).
    pub fn sha256(self) -> &'static str {
        match self {
            Self::Base => "c577b9a86e7e048a0b7eada054f4dd79a56bbfa911fbdacf900ac5b567cbb7d9",
            Self::Small => "49c8fb02b65e6049d5fa6c04f81f53b867b5ec9540406812c643f177317f779f",
            Self::Medium => "19fea4b380c3a618ec4723c3eef2eb785ffba0d0538cf43f8f235e7b3b34220f",
            Self::Turbo => "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Base => "Base (82 MB, fastest, weak Bangla)",
            Self::Small => "Small (264 MB, fast, basic Bangla)",
            Self::Medium => "Medium (539 MB, slower)",
            Self::Turbo => "Large v3 Turbo (574 MB, best Bangla, recommended)",
        }
    }

    /// Settings key.
    pub fn id(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Turbo => "turbo",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id() == id)
    }

    pub fn path_in(self, dir: &Path) -> PathBuf {
        dir.join(self.file_name())
    }

    /// Downloaded completely (the hash was checked when it was downloaded).
    pub fn is_downloaded(self, dir: &Path) -> bool {
        std::fs::metadata(self.path_in(dir)).is_ok_and(|m| m.len() == self.bytes())
    }
}

/// Downloads `size` into `dir` unless it is already there. `progress(done, total)` is
/// called as bytes arrive. The file appears under its final name only after the hash
/// matched.
pub async fn download(
    size: ModelSize,
    dir: &Path,
    progress: impl Fn(u64, u64),
) -> Result<PathBuf, VoiceError> {
    let path = size.path_in(dir);
    if size.is_downloaded(dir) {
        return Ok(path);
    }
    tokio::fs::create_dir_all(dir).await?;
    let part = dir.join(format!("{}.part", size.file_name()));
    let url = format!("{BASE_URL}/{}", size.file_name());
    let client = reqwest::Client::builder()
        .user_agent(concat!("desktop-pilot/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| VoiceError::Download(e.to_string()))?;
    let mut response = client
        .get(&url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| VoiceError::Download(e.to_string()))?;
    let total = response.content_length().unwrap_or(size.bytes());
    let mut file = tokio::fs::File::create(&part).await?;
    let mut hasher = Sha256::new();
    let mut done = 0_u64;
    progress(0, total);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| VoiceError::Download(e.to_string()))?
    {
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        done += chunk.len() as u64;
        progress(done, total);
    }
    file.flush().await?;
    drop(file);
    let hash: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if hash != size.sha256() || done != size.bytes() {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(VoiceError::Download(format!(
            "the downloaded file is damaged (got {done} bytes, hash {hash}); try again"
        )));
    }
    tokio::fs::rename(&part, &path).await?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_hashes_look_right() {
        for m in ModelSize::ALL {
            assert_eq!(ModelSize::from_id(m.id()), Some(m));
            assert_eq!(m.sha256().len(), 64);
            assert!(m.file_name().ends_with(".bin"));
        }
        assert_eq!(ModelSize::from_id("huge"), None);
    }

    #[test]
    fn missing_model_is_not_downloaded() {
        let dir = std::env::temp_dir().join("dp-voice-no-such-dir");
        assert!(!ModelSize::Small.is_downloaded(&dir));
    }
}
