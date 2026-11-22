//! User settings stored as TOML in the OS config directory (TRD §7).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{APPLICATION, ORGANIZATION, QUALIFIER};

/// How often the agent asks before acting (SECURITY §4.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmMode {
    /// Ask once when a task starts acting, then only for security-sensitive actions
    /// (installers, permission dialogs, system settings, terminals, deleting, secrets,
    /// sending messages). See `dp_core::risk`.
    #[default]
    RiskyOnly,
    /// Ask before every action that changes something (clicks, typing, keys, drags).
    EveryAction,
    /// Never ask. The user watches and can STOP at any time.
    Automatic,
}

/// Which Claude runs the task.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    /// The user's Claude Code CLI (`claude -p`), using its own login, e.g. a Claude
    /// plan. Desktop Pilot provides the computer tools over a local MCP server.
    #[default]
    ClaudeCode,
    /// The Claude API directly (API key or `ant` login; needs Console credit).
    ClaudeApi,
    /// The Gemini API directly with an AI Studio key. Only Desktop Pilot's own tools.
    GeminiApi,
    /// Google Antigravity (`agy`) with the user's Google account. Its own built-in
    /// tools cannot be fully switched off; runs sandboxed.
    Antigravity,
}

impl Engine {
    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::ClaudeApi => "Claude API",
            Self::GeminiApi => "Gemini API",
            Self::Antigravity => "Antigravity (Gemini)",
        }
    }

    /// Engines that use the built-in Claude model list.
    pub fn uses_claude_models(self) -> bool {
        matches!(self, Self::ClaudeCode | Self::ClaudeApi)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub schema_version: u32,
    pub engine: Engine,
    pub default_model: String,
    pub confirm_mode: ConfirmMode,
    /// Max model responses per task.
    pub max_steps: u32,
    /// Pause after each action so the user can follow along.
    pub action_delay_ms: u64,
    /// Long edge of screenshots sent to Claude (px). Smaller = fewer tokens.
    pub screenshot_long_edge: u32,
    /// Global FORCE STOP hotkey.
    pub stop_hotkey: String,
    /// Glide the mouse to its target in this many ms (0 = jump). Visible to the user;
    /// grabbing the mouse during the glide stops the task.
    pub mouse_motion_ms: u64,
    /// Attach a fresh screenshot to the result of every action that changes something,
    /// so Claude needs fewer turns (fewer tokens overall).
    pub screenshot_after_actions: bool,
    /// Save each task (prompt, actions, Claude's replies, result; no screenshots) as
    /// training data in [`training_dir`]. Local only; never uploaded.
    pub save_training_data: bool,
    /// Voice assistant on (local Whisper). See `voice_mode` for when the mic is open.
    pub voice_enabled: bool,
    /// "tap": the microphone opens only on the orb, mic button or `voice_shortcut`.
    /// "wake": always listening for "Hey Mouse" (the mic stays open).
    pub voice_mode: String,
    /// Global shortcut that opens the microphone for one command.
    pub voice_shortcut: String,
    /// Whisper model: "turbo" (best Bangla), "small", "medium" or "base".
    pub voice_model: String,
    /// Spoken language: "auto" (Bangla or English), "bn" or "en".
    pub voice_language: String,
    /// Speak replies to voice commands aloud.
    pub voice_talk_back: bool,
    /// Closing the window keeps Desktop Pilot running (tray / menu-bar icon).
    pub run_in_background: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            engine: Engine::ClaudeCode,
            default_model: "claude-sonnet-5-5".to_owned(),
            confirm_mode: ConfirmMode::RiskyOnly,
            max_steps: 40,
            action_delay_ms: 150,
            screenshot_long_edge: 1024,
            stop_hotkey: "ctrl+alt+Escape".to_owned(),
            mouse_motion_ms: 250,
            screenshot_after_actions: true,
            save_training_data: true,
            voice_enabled: false,
            voice_mode: "tap".to_owned(),
            voice_shortcut: "alt+Space".to_owned(),
            voice_model: "turbo".to_owned(),
            voice_language: "auto".to_owned(),
            voice_talk_back: true,
            run_in_background: true,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("no config directory available on this system")]
    NoConfigDir,
    #[error("config I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("config format: {0}")]
    Format(String),
}

/// `…/config.toml` in the OS config directory.
pub fn config_path() -> Result<PathBuf, ConfigError> {
    directories::ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
        .map(|d| d.config_dir().join("config.toml"))
        .ok_or(ConfigError::NoConfigDir)
}

/// `…/training` in the OS data directory, where task records are saved
/// (macOS: `~/Library/Application Support/com.desktoppilot.DesktopPilot/training`).
pub fn training_dir() -> Result<PathBuf, ConfigError> {
    directories::ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
        .map(|d| d.data_dir().join("training"))
        .ok_or(ConfigError::NoConfigDir)
}

/// `…/models` in the OS data directory, where speech models are downloaded.
pub fn models_dir() -> Result<PathBuf, ConfigError> {
    directories::ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
        .map(|d| d.data_dir().join("models"))
        .ok_or(ConfigError::NoConfigDir)
}

impl Settings {
    /// Clamps values into their allowed ranges.
    pub fn sanitized(mut self) -> Self {
        self.max_steps = self.max_steps.clamp(1, 200);
        self.action_delay_ms = self.action_delay_ms.min(2_000);
        self.screenshot_long_edge = self.screenshot_long_edge.clamp(800, 1920);
        self.mouse_motion_ms = self.mouse_motion_ms.min(1_500);
        if self.stop_hotkey.trim().is_empty() {
            self.stop_hotkey = Self::default().stop_hotkey;
        }
        self
    }

    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        toml::from_str::<Self>(text)
            .map(Self::sanitized)
            .map_err(|e| ConfigError::Format(e.to_string()))
    }

    pub fn to_toml(&self) -> Result<String, ConfigError> {
        toml::to_string_pretty(self).map_err(|e| ConfigError::Format(e.to_string()))
    }

    /// Loads settings; a missing file gives defaults. A broken file is backed up as
    /// `config.toml.bak` and defaults are used (TRD §7).
    pub fn load() -> (Self, Option<String>) {
        let Ok(path) = config_path() else {
            return (
                Self::default(),
                Some("no config directory; using defaults".into()),
            );
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match Self::from_toml(&text) {
                Ok(s) => (s, None),
                Err(e) => {
                    let _ = std::fs::copy(&path, path.with_extension("toml.bak"));
                    (
                        Self::default(),
                        Some(format!("config.toml was invalid ({e}); using defaults")),
                    )
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Self::default(), None),
            Err(e) => (
                Self::default(),
                Some(format!("could not read config.toml: {e}")),
            ),
        }
    }

    pub fn save(&self) -> Result<PathBuf, ConfigError> {
        let path = config_path()?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, self.to_toml()?)?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_toml() {
        let s = Settings {
            confirm_mode: ConfirmMode::Automatic,
            max_steps: 25,
            ..Settings::default()
        };
        let text = s.to_toml().unwrap();
        assert!(text.contains("confirm_mode = \"automatic\""));
        assert_eq!(Settings::from_toml(&text).unwrap(), s);
    }

    #[test]
    fn missing_fields_take_defaults_and_values_are_clamped() {
        let s = Settings::from_toml("max_steps = 9999\naction_delay_ms = 99999").unwrap();
        assert_eq!(s.max_steps, 200);
        assert_eq!(s.action_delay_ms, 2_000);
        assert_eq!(s.default_model, "claude-sonnet-5-5");
        assert_eq!(s.confirm_mode, ConfirmMode::RiskyOnly);
        assert_eq!(s.screenshot_long_edge, 1024);
        assert!(s.screenshot_after_actions);
    }

    #[test]
    fn invalid_toml_is_an_error() {
        assert!(Settings::from_toml("max_steps = \"lots\"").is_err());
    }
}
