//! The only file outside `platform-*` allowed to use `cfg(target_os)`:
//! it picks the backend crate for the current OS (ARCHITECTURE §2), opens folders in
//! the OS file manager, and owns the tray / menu-bar icon (not available on Linux yet).

use dp_platform::{Platform, PlatformError};

#[cfg(target_os = "windows")]
pub(crate) fn init() -> Result<Platform, PlatformError> {
    dp_platform_windows::init()
}

#[cfg(target_os = "macos")]
pub(crate) fn init() -> Result<Platform, PlatformError> {
    dp_platform_macos::init()
}

#[cfg(target_os = "linux")]
pub(crate) fn init() -> Result<Platform, PlatformError> {
    dp_platform_linux::init()
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
pub(crate) fn init() -> Result<Platform, PlatformError> {
    Ok(Platform::stub(dp_platform::PlatformInfo {
        os: dp_platform::OsKind::Other,
        os_version: String::from("unknown"),
        arch: std::env::consts::ARCH,
        session: dp_platform::SessionKind::Unknown,
        desktop: None,
    }))
}

/// Opens `dir` in the OS file manager, creating it first if needed.
pub(crate) fn open_folder(dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(target_os = "windows")]
    let program = "explorer";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let program = "xdg-open";
    std::process::Command::new(program)
        .arg(dir)
        .spawn()
        .map(|_| ())
}

/// Opens the OS page where natural text-to-speech voices are downloaded.
pub(crate) fn open_voice_settings() -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let (program, arg) = (
        "open",
        "x-apple.systempreferences:com.apple.preference.universalaccess?SpokenContent",
    );
    #[cfg(target_os = "windows")]
    let (program, arg) = ("explorer", "ms-settings:speech");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let (program, arg) = ("xdg-open", "https://github.com/espeak-ng/espeak-ng");
    std::process::Command::new(program)
        .arg(arg)
        .spawn()
        .map(|_| ())
}

/// What the user picked in the tray / menu-bar menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TrayAction {
    Show,
    /// Open the microphone for one command.
    Talk,
    /// Listening mode: the microphone opens only on the shortcut / orb / mic button.
    ModeShortcut,
    /// Listening mode: always listening for "Hey Mouse".
    ModeWake,
    Stop,
    Quit,
}

/// The tray / menu-bar icon with the app logo: click it to open the chat window, turn
/// the voice assistant on or off, stop a task or turn Desktop Pilot off (Windows and macOS; on Linux it would need GTK, so there
/// is none yet and closing the window quits as before).
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(crate) struct Tray {
    _icon: tray_icon::TrayIcon,
    status: tray_icon::menu::MenuItem,
    mode_shortcut: tray_icon::menu::CheckMenuItem,
    mode_wake: tray_icon::menu::CheckMenuItem,
    stop: tray_icon::menu::MenuItem,
    ids: Vec<(tray_icon::menu::MenuId, TrayAction)>,
    events: std::sync::mpsc::Receiver<tray_icon::menu::MenuId>,
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
impl std::fmt::Debug for Tray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tray").finish_non_exhaustive()
    }
}

/// Even-odd point-in-polygon test.
#[cfg(any(target_os = "windows", target_os = "macos"))]
fn inside(poly: &[(f32, f32)], x: f32, y: f32) -> bool {
    let mut c = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let ((xi, yi), (xj, yj)) = (poly[i], poly[j]);
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            c = !c;
        }
        j = i;
    }
    c
}

/// The app logo (the pointer with a spark from the chat window, `style::paint_logo`)
/// as a 32×32 icon in `shade`, anti-aliased. macOS draws it as a template image from
/// the alpha only; Windows taskbars are usually dark, so Windows gets white.
#[cfg(any(target_os = "windows", target_os = "macos"))]
fn tray_rgba(shade: u8) -> Vec<u8> {
    const N: usize = 32;
    const SAMPLES: usize = 4;
    // Logo units (see `paint_logo`) → pixels.
    let map = |pts: &[(f32, f32)]| -> Vec<(f32, f32)> {
        pts.iter()
            .map(|(x, y)| (3.0 + x * 1.12, 2.5 + y * 1.12))
            .collect()
    };
    let head = map(&[(0.0, 0.0), (17.0, 15.5), (9.5, 15.5), (0.0, 22.0)]);
    let tail = map(&[(6.5, 13.0), (10.5, 11.5), (15.0, 22.5), (11.0, 24.0)]);
    // Four-point spark, a little larger than in the window so it reads at menu-bar size.
    let (cx, cy, r) = (3.0 + 21.0 * 1.12, 2.5 + 3.5 * 1.12, 5.2);
    let spark: Vec<(f32, f32)> = (0..8)
        .map(|k| {
            let a = k as f32 * std::f32::consts::FRAC_PI_4 - std::f32::consts::FRAC_PI_2;
            let rr = if k % 2 == 0 { r } else { r * 0.32 };
            (cx + a.cos() * rr, cy + a.sin() * rr)
        })
        .collect();
    let mut rgba = vec![0_u8; N * N * 4];
    for y in 0..N {
        for x in 0..N {
            let mut hits = 0;
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let fx = x as f32 + (sx as f32 + 0.5) / SAMPLES as f32;
                    let fy = y as f32 + (sy as f32 + 0.5) / SAMPLES as f32;
                    if inside(&head, fx, fy) || inside(&tail, fx, fy) || inside(&spark, fx, fy) {
                        hits += 1;
                    }
                }
            }
            let alpha = (hits * 255 / (SAMPLES * SAMPLES)) as u8;
            rgba[(y * N + x) * 4..(y * N + x) * 4 + 4]
                .copy_from_slice(&[shade, shade, shade, alpha]);
        }
    }
    rgba
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
impl Tray {
    /// Creates the icon. Must run on the main thread after the event loop started.
    /// `wake` is called for every menu click (to wake the UI).
    pub(crate) fn create(
        voice_on: bool,
        talk_label: &str,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Option<Self> {
        use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};

        // A status line (not clickable), then the actions.
        let status = MenuItem::new("Desktop Pilot", false, None);
        let show = MenuItem::new("Open chat window", true, None);
        let talk = MenuItem::new(talk_label, true, None);
        // Listening mode: two choices, exactly one ticked (kept in sync by `set_voice`,
        // because macOS flips a check item by itself when it is clicked).
        let mode_title = MenuItem::new("Listening mode", false, None);
        let mode_shortcut = CheckMenuItem::new(
            format!(
                "    {}",
                talk_label
                    .trim_start_matches("Talk now (")
                    .trim_end_matches(')')
            ) + " to talk",
            true,
            !voice_on,
            None,
        );
        let mode_wake = CheckMenuItem::new(
            "    Say “Hey Mouse” (always listening)",
            true,
            voice_on,
            None,
        );
        let stop = MenuItem::new("Stop current task", false, None);
        let quit = MenuItem::new("Turn off Desktop Pilot", true, None);
        let menu = Menu::new();
        for item in [
            &status as &dyn tray_icon::menu::IsMenuItem,
            &PredefinedMenuItem::separator(),
            &show,
            &talk,
            &PredefinedMenuItem::separator(),
            &mode_title,
            &mode_shortcut,
            &mode_wake,
            &PredefinedMenuItem::separator(),
            &stop,
            &PredefinedMenuItem::separator(),
            &quit,
        ] {
            menu.append(item).ok()?;
        }
        let ids = vec![
            (show.id().clone(), TrayAction::Show),
            (talk.id().clone(), TrayAction::Talk),
            (mode_shortcut.id().clone(), TrayAction::ModeShortcut),
            (mode_wake.id().clone(), TrayAction::ModeWake),
            (stop.id().clone(), TrayAction::Stop),
            (quit.id().clone(), TrayAction::Quit),
        ];
        let (tx, events) = std::sync::mpsc::channel();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let _ = tx.send(e.id);
            wake();
        }));
        let builder = tray_icon::TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Desktop Pilot");
        #[cfg(target_os = "macos")]
        let builder =
            builder.with_icon_templated(tray_icon::Icon::from_rgba(tray_rgba(0), 32, 32).ok()?);
        #[cfg(target_os = "windows")]
        let builder = builder.with_icon(tray_icon::Icon::from_rgba(tray_rgba(255), 32, 32).ok()?);
        let tray = builder
            .build()
            .map_err(|e| tracing::warn!("tray icon: {e}"))
            .ok()?;
        Some(Self {
            _icon: tray,
            status,
            mode_shortcut,
            mode_wake,
            stop,
            ids,
            events,
        })
    }

    /// The next menu click, if any.
    pub(crate) fn poll(&self) -> Option<TrayAction> {
        let id = self.events.try_recv().ok()?;
        self.ids.iter().find(|(i, _)| *i == id).map(|(_, a)| *a)
    }

    /// Ticks the current listening mode (`true` = "Hey Mouse").
    pub(crate) fn set_voice(&self, wake: bool) {
        self.mode_shortcut.set_checked(!wake);
        self.mode_wake.set_checked(wake);
    }

    /// Updates the status line and whether "Stop current task" is clickable.
    pub(crate) fn set_status(&self, text: &str, running: bool) {
        self.status.set_text(text);
        self.stop.set_enabled(running);
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
#[derive(Debug)]
pub(crate) struct Tray;

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
impl Tray {
    pub(crate) fn create(
        _voice_on: bool,
        _talk_label: &str,
        _wake: impl Fn() + Send + Sync + 'static,
    ) -> Option<Self> {
        None
    }
    pub(crate) fn poll(&self) -> Option<TrayAction> {
        None
    }
    pub(crate) fn set_voice(&self, _on: bool) {}
    pub(crate) fn set_status(&self, _text: &str, _running: bool) {}
}
