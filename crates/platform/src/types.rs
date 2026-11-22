//! Shared value types. Coordinate newtypes for the model/capture spaces live in
//! `dp-core::coords` (ADR-007); here are only the OS-facing desktop coordinates.

use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MonitorId(pub String);

#[derive(Clone, Debug)]
pub struct MonitorInfo {
    pub id: MonitorId,
    pub name: String,
    /// Origin in the virtual desktop, physical pixels.
    pub origin_physical: (i32, i32),
    pub size_physical: (u32, u32),
    /// Origin in the OS global logical space (macOS points, Wayland logical px).
    pub origin_logical: (f64, f64),
    /// Physical pixels per logical pixel.
    pub scale_factor: f64,
    pub is_primary: bool,
}

/// A raw capture of one monitor (RGBA8, physical pixels).
#[derive(Clone, Debug)]
pub struct Frame {
    pub monitor: MonitorId,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
    pub captured_at: Instant,
}

/// Which coordinate space an [`crate::InputController`] consumes.
#[derive(Clone, Debug, PartialEq)]
pub enum CoordinateSpace {
    /// Windows, X11: physical pixels in the virtual desktop.
    DesktopPhysical,
    /// macOS: global points.
    DesktopLogical,
    /// Wayland portals: logical coordinates of a ScreenCast stream.
    StreamLogical { stream: MonitorId },
}

/// A point in the space the input backend declared via `coordinate_space()`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DesktopPoint {
    Physical { x: i32, y: i32 },
    Logical { x: f64, y: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Press,
    Release,
    Click,
}

/// A key in the computer-use key syntax (e.g. "Return", "ctrl", "a"). Parsed in M2.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Key(pub String);

/// Keys and buttons currently held down by our own input controller.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeldInputs {
    pub keys: Vec<Key>,
    pub buttons: Vec<MouseButton>,
}

impl HeldInputs {
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty() && self.buttons.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AppId(pub String);

#[derive(Clone, Debug)]
pub struct AppEntry {
    pub id: AppId,
    pub name: String,
    pub localized_names: Vec<String>,
    pub executable: Option<String>,
}

#[derive(Clone, Debug)]
pub struct LaunchResult {
    pub pid: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WindowId(pub String);

#[derive(Clone, Debug)]
pub struct WindowInfo {
    pub id: WindowId,
    pub title: String,
    pub app: String,
    pub pid: Option<u32>,
    /// x, y, width, height in desktop physical pixels.
    pub bounds: (i32, i32, u32, u32),
    pub focused: bool,
}

#[derive(Clone, Debug)]
pub enum TreeTarget {
    FocusedWindow,
    Window(WindowId),
}

#[derive(Clone, Copy, Debug)]
pub struct TreeLimits {
    pub max_nodes: usize,
    pub max_depth: usize,
}

impl Default for TreeLimits {
    fn default() -> Self {
        Self {
            max_nodes: 400,
            max_depth: 25,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct UiNode {
    pub role: String,
    pub name: String,
    pub value: Option<String>,
    pub bounds: Option<(i32, i32, u32, u32)>,
    pub enabled: bool,
    pub focused: bool,
    pub is_password: bool,
    pub children: Vec<UiNode>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HotkeyCombo(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HotkeyId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PermissionKind {
    Accessibility,
    ScreenRecording,
    PortalSession,
    AccessibilityBus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionState {
    Granted,
    Denied,
    NotDetermined,
    RestartRequired,
    NotApplicable,
}

#[derive(Clone, Debug)]
pub struct PermissionStatus {
    pub kind: PermissionKind,
    pub state: PermissionState,
}

/// Names of secrets kept in the OS secret store (ADR-010).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SecretKey {
    AnthropicApiKey,
    GeminiApiKey,
    WaylandRestoreToken,
}

impl SecretKey {
    pub fn account(self) -> &'static str {
        match self {
            Self::AnthropicApiKey => "anthropic_api_key",
            Self::GeminiApiKey => "gemini_api_key",
            Self::WaylandRestoreToken => "wayland_restore_token",
        }
    }
}

/// Whether captures can exclude our own windows (TRD §5.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExclusionSupport {
    Excluded,
    MustHide,
}
