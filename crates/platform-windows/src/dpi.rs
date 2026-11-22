use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};

/// Opts the process into per-monitor DPI awareness v2. Must run before any window is
/// created. If awareness was already set (e.g. by a manifest or by winit), Windows
/// returns an error that is safe to ignore.
pub(crate) fn make_process_dpi_aware() {
    // SAFETY: plain Win32 call with a predefined constant context; no pointers involved.
    let result =
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if let Err(err) = result {
        tracing::debug!(%err, "DPI awareness already set");
    }
}
