use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OsKind {
    Windows,
    MacOs,
    Linux,
    Other,
}

/// The kind of graphical session. On Linux this decides the backend (FR-23).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionKind {
    Windows,
    MacOs,
    X11,
    Wayland,
    Unknown,
}

/// Facts about the running OS, detected by the OS crate at startup.
#[derive(Clone, Debug)]
pub struct PlatformInfo {
    pub os: OsKind,
    pub os_version: String,
    pub arch: &'static str,
    pub session: SessionKind,
    /// Desktop environment / compositor, e.g. "GNOME", "KDE" (Linux only).
    pub desktop: Option<String>,
}

impl fmt::Display for OsKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Windows => "windows",
            Self::MacOs => "macos",
            Self::Linux => "linux",
            Self::Other => "other",
        })
    }
}

impl fmt::Display for SessionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Windows => "windows",
            Self::MacOs => "macos",
            Self::X11 => "x11",
            Self::Wayland => "wayland",
            Self::Unknown => "unknown",
        })
    }
}

impl fmt::Display for PlatformInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "os={} os_version={} arch={} session={}",
            self.os, self.os_version, self.arch, self.session
        )?;
        if let Some(d) = &self.desktop {
            write!(f, " desktop={d}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_format_is_stable() {
        let info = PlatformInfo {
            os: OsKind::Linux,
            os_version: "24.04".into(),
            arch: "x86_64",
            session: SessionKind::Wayland,
            desktop: Some("GNOME".into()),
        };
        assert_eq!(
            info.to_string(),
            "os=linux os_version=24.04 arch=x86_64 session=wayland desktop=GNOME"
        );
    }
}
