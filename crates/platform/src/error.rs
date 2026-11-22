use crate::types::PermissionKind;

/// Errors returned by platform backends. Backends never panic on OS failures.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PlatformError {
    #[error("permission missing: {0:?}")]
    PermissionDenied(PermissionKind),
    #[error("not supported on this platform or session: {0}")]
    Unsupported(String),
    #[error("target not found: {0}")]
    NotFound(String),
    #[error(
        "target window runs as administrator; restart Desktop Pilot as administrator to control it"
    )]
    ElevatedTarget,
    #[error("secure desktop active (UAC prompt, lock screen or similar)")]
    SecureDesktop,
    #[error("desktop portal error: {0}")]
    Portal(String),
    #[error("cancelled")]
    Cancelled,
    #[error("os error: {0}")]
    Os(String),
}
