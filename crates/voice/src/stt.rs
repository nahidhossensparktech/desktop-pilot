//! Local speech-to-text with Whisper (whisper.cpp through `whisper-rs`).

use std::path::Path;

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::VoiceError;
use crate::vad::SAMPLE_RATE;

/// Which language to expect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Language {
    /// Bangla or English, detected per utterance.
    #[default]
    Auto,
    Bangla,
    English,
}

impl Language {
    pub fn id(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Bangla => "bn",
            Self::English => "en",
        }
    }

    pub fn from_id(id: &str) -> Self {
        match id {
            "bn" => Self::Bangla,
            "en" => Self::English,
            _ => Self::Auto,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Bangla + English (auto)",
            Self::Bangla => "Bangla",
            Self::English => "English",
        }
    }
}

/// What Whisper heard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transcript {
    pub text: String,
    /// "bn" or "en".
    pub lang: &'static str,
}

/// A loaded Whisper model. Not thread-safe; use it from one thread.
pub struct Transcriber {
    ctx: WhisperContext,
    threads: i32,
}

impl std::fmt::Debug for Transcriber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transcriber").finish_non_exhaustive()
    }
}

impl Transcriber {
    /// Loads a model file (takes about a second).
    pub fn load(model: &Path) -> Result<Self, VoiceError> {
        // Route whisper.cpp's own logging into `tracing` instead of stderr.
        whisper_rs::install_logging_hooks();
        let ctx = WhisperContext::new_with_params(model, WhisperContextParameters::default())
            .map_err(|e| VoiceError::Model(format!("{}: {e}", model.display())))?;
        let threads = std::thread::available_parallelism()
            .map(|n| n.get().min(6))
            .unwrap_or(4) as i32;
        Ok(Self { ctx, threads })
    }

    /// Picks Bangla or English for `pcm` with Whisper's language detector. Hindi and
    /// Assamese (close to Bangla in sound) count as Bangla.
    fn detect(&self, pcm: &[f32]) -> Result<&'static str, VoiceError> {
        let mut state = self.ctx.create_state().map_err(stt_err)?;
        state
            .pcm_to_mel(pcm, self.threads as usize)
            .map_err(stt_err)?;
        let (_, probs) = state
            .lang_detect(0, self.threads as usize)
            .map_err(stt_err)?;
        let p = |code: &str| {
            whisper_rs::get_lang_id(code)
                .and_then(|i| probs.get(i as usize).copied())
                .unwrap_or(0.0)
        };
        let bangla = p("bn") + p("hi") + p("as") + p("ur");
        Ok(if bangla > p("en") { "bn" } else { "en" })
    }

    /// Transcribes 16 kHz mono audio.
    ///
    /// No prompt is given: with "Hey Mouse." as a prompt, Whisper wrote "Hey Mouse." for
    /// plain room noise (it echoes its prompt), which woke the assistant by itself; and
    /// any prompt made Bangla worse. Bangla usually comes out as phonetic Latin
    /// ("shafari kholo"), which Claude understands.
    pub fn transcribe(
        &mut self,
        pcm: &[f32],
        language: Language,
    ) -> Result<Transcript, VoiceError> {
        // whisper.cpp wants at least 1 s of audio; pad short clips with silence.
        let mut audio = pcm.to_vec();
        if audio.len() < SAMPLE_RATE + SAMPLE_RATE / 10 {
            audio.resize(SAMPLE_RATE + SAMPLE_RATE / 10, 0.0);
        }
        let lang = match language {
            Language::Auto => self.detect(&audio)?,
            Language::Bangla => "bn",
            Language::English => "en",
        };
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(lang));
        params.set_n_threads(self.threads);
        params.set_no_context(true);
        params.set_no_timestamps(true);
        params.set_single_segment(false);
        params.set_suppress_blank(true);
        params.set_suppress_nst(true);
        params.set_temperature(0.0);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        let mut state = self.ctx.create_state().map_err(stt_err)?;
        state.full(params, &audio).map_err(stt_err)?;
        let mut text = String::new();
        for segment in state.as_iter() {
            // Skip segments Whisper itself thinks are not speech.
            if segment.no_speech_probability() > 0.5 {
                continue;
            }
            if let Ok(s) = segment.to_str_lossy() {
                text.push_str(&s);
                text.push(' ');
            }
        }
        let mut text = crate::wake::clean_transcript(&text);
        if lang == "bn" {
            // Whisper writes Bangla speech in Hindi script; put it in Bangla script.
            text = crate::script::devanagari_to_bangla(&text);
        }
        Ok(Transcript { text, lang })
    }
}

fn stt_err(e: whisper_rs::WhisperError) -> VoiceError {
    VoiceError::Transcribe(e.to_string())
}
