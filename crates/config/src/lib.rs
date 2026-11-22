//! Settings (TOML), model registry and app identity.

#![forbid(unsafe_code)]

mod models;
mod settings;

pub use models::{ModelEntry, default_models};
pub use settings::{
    ConfigError, ConfirmMode, Engine, Settings, config_path, models_dir, training_dir,
};

/// Identity used for config paths and the OS secret store. It must never change after
/// release: on macOS it is also the permission (TCC) identity. See TRD open question 1.
pub const QUALIFIER: &str = "com";
pub const ORGANIZATION: &str = "desktoppilot";
pub const APPLICATION: &str = "DesktopPilot";
/// Service name for secrets in the OS keychain (ADR-010).
pub const SECRET_SERVICE: &str = "com.desktoppilot.DesktopPilot";
