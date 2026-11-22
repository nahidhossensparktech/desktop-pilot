use dp_platform::SessionKind;

/// Picks X11 or Wayland from the environment (FR-23, TRD TR-X1 / TR-L1).
///
/// `XDG_SESSION_TYPE` wins when it is `wayland` or `x11`. Otherwise a set
/// `WAYLAND_DISPLAY` means Wayland (even if `DISPLAY` is also set for XWayland,
/// X11 APIs cannot reach native Wayland windows), and a set `DISPLAY` means X11.
pub fn detect_session(env: impl Fn(&str) -> Option<String>) -> SessionKind {
    let non_empty = |name: &str| env(name).filter(|v| !v.trim().is_empty());

    match non_empty("XDG_SESSION_TYPE")
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("wayland") => return SessionKind::Wayland,
        Some("x11") => return SessionKind::X11,
        _ => {}
    }
    if non_empty("WAYLAND_DISPLAY").is_some() {
        SessionKind::Wayland
    } else if non_empty("DISPLAY").is_some() {
        SessionKind::X11
    } else {
        SessionKind::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[test]
    fn session_type_wins() {
        let e = env_of(&[
            ("XDG_SESSION_TYPE", "x11"),
            ("WAYLAND_DISPLAY", "wayland-0"),
        ]);
        assert_eq!(detect_session(e), SessionKind::X11);
        let e = env_of(&[("XDG_SESSION_TYPE", "Wayland"), ("DISPLAY", ":0")]);
        assert_eq!(detect_session(e), SessionKind::Wayland);
    }

    #[test]
    fn wayland_display_beats_xwayland_display() {
        let e = env_of(&[
            ("XDG_SESSION_TYPE", "tty"),
            ("WAYLAND_DISPLAY", "wayland-0"),
            ("DISPLAY", ":0"),
        ]);
        assert_eq!(detect_session(e), SessionKind::Wayland);
    }

    #[test]
    fn display_only_is_x11_and_nothing_is_unknown() {
        assert_eq!(
            detect_session(env_of(&[("DISPLAY", ":1")])),
            SessionKind::X11
        );
        assert_eq!(
            detect_session(env_of(&[("DISPLAY", "  ")])),
            SessionKind::Unknown
        );
        assert_eq!(detect_session(env_of(&[])), SessionKind::Unknown);
    }
}
