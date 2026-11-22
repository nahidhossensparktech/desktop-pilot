use std::fmt;

/// Whether a feature works on this machine, so the app can degrade gracefully (FR-27).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Capability {
    Available,
    Degraded {
        reason: String,
        workaround: Option<String>,
    },
    Unavailable {
        reason: String,
        workaround: Option<String>,
    },
}

impl Capability {
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self::Unavailable {
            reason: reason.into(),
            workaround: None,
        }
    }

    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Available => f.write_str("available"),
            Self::Degraded { reason, .. } => write!(f, "degraded: {reason}"),
            Self::Unavailable { reason, .. } => write!(f, "unavailable: {reason}"),
        }
    }
}

/// Named capabilities of every backend, shown in Settings ▸ About.
#[derive(Clone, Debug, Default)]
pub struct CapabilityReport {
    pub entries: Vec<(&'static str, Capability)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_includes_reason() {
        let c = Capability::unavailable("no portal");
        assert_eq!(c.to_string(), "unavailable: no portal");
        assert!(!c.is_available());
        assert!(Capability::Available.is_available());
    }
}
