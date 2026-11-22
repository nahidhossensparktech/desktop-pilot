use std::fmt::Debug;

use async_trait::async_trait;
use secrecy::SecretString;

use crate::{
    AppEntry, AppId, Capability, CoordinateSpace, DesktopPoint, Direction, ExclusionSupport, Frame,
    HeldInputs, HotkeyCombo, HotkeyId, Key, LaunchResult, MonitorId, MonitorInfo, MouseButton,
    PermissionKind, PermissionStatus, PlatformError, SecretKey, TreeLimits, TreeTarget, UiNode,
    WindowId, WindowInfo,
};

/// Native window handle of one of our own windows, as an opaque integer.
pub type NativeWindowHandle = u64;

#[async_trait]
pub trait ScreenCapture: Send + Sync + Debug {
    async fn monitors(&self) -> Result<Vec<MonitorInfo>, PlatformError>;
    async fn capture(&self, monitor: &MonitorId) -> Result<Frame, PlatformError>;
    /// Exclude our own windows from captures if the OS allows it.
    fn exclude_windows(&self, handles: &[NativeWindowHandle]) -> ExclusionSupport;
    fn capability(&self) -> Capability;
    /// Makes the GUI's "Monitoring" overlay window (titled
    /// [`crate::OVERLAY_WINDOW_TITLE`]) cover the whole primary display, above menu bars
    /// and docks, where the OS needs help with that. Called on the UI (main) thread
    /// every frame while the overlay shows, so it must be cheap. Default: nothing.
    fn prepare_overlay(&self) {}
}

#[async_trait]
pub trait InputController: Send + Sync + Debug {
    fn coordinate_space(&self) -> CoordinateSpace;
    async fn mouse_move(&self, p: DesktopPoint) -> Result<(), PlatformError>;
    async fn button(&self, b: MouseButton, dir: Direction) -> Result<(), PlatformError>;
    async fn scroll(&self, dx: i32, dy: i32) -> Result<(), PlatformError>;
    async fn key(&self, key: &Key, dir: Direction) -> Result<(), PlatformError>;
    /// Types a short chunk of text (the agent sends at most 8 characters per call).
    async fn text(&self, chunk: &str) -> Result<(), PlatformError>;
    /// `None` where the OS does not expose the global cursor (Wayland).
    async fn cursor_position(&self) -> Result<Option<DesktopPoint>, PlatformError>;
    /// Releases every key/button this controller pressed. Synchronous, idempotent and
    /// thread-safe: it is called from the hotkey thread and the panic hook.
    fn release_all(&self) -> Result<(), PlatformError>;
    fn held_inputs(&self) -> HeldInputs;
    fn capability(&self) -> Capability;
}

#[async_trait]
pub trait AppCatalog: Send + Sync + Debug {
    async fn list(&self, refresh: bool) -> Result<Vec<AppEntry>, PlatformError>;
    async fn launch(&self, app: &AppId, args: &[String]) -> Result<LaunchResult, PlatformError>;
}

#[async_trait]
pub trait WindowManager: Send + Sync + Debug {
    async fn list(&self) -> Result<Vec<WindowInfo>, PlatformError>;
    async fn focus(&self, id: &WindowId) -> Result<(), PlatformError>;
    async fn focused(&self) -> Result<Option<WindowInfo>, PlatformError>;
    fn capability(&self) -> Capability;
}

#[async_trait]
pub trait AccessibilityTree: Send + Sync + Debug {
    async fn tree(&self, target: TreeTarget, limits: TreeLimits) -> Result<UiNode, PlatformError>;
    fn capability(&self) -> Capability;
}

pub trait HotkeyService: Send + Sync + Debug {
    /// Registers a global hotkey. The callback runs on an OS thread and must be cheap
    /// (e.g. `CancellationToken::cancel`).
    fn register(
        &self,
        combo: &HotkeyCombo,
        callback: Box<dyn Fn() + Send + Sync>,
    ) -> Result<HotkeyId, PlatformError>;
    fn unregister(&self, id: HotkeyId) -> Result<(), PlatformError>;
    fn capability(&self) -> Capability;
}

#[async_trait]
pub trait PermissionManager: Send + Sync + Debug {
    async fn status(&self) -> Vec<PermissionStatus>;
    async fn request(&self, kind: PermissionKind) -> Result<PermissionStatus, PlatformError>;
    /// A deep link that opens the right OS settings page, if any.
    fn settings_deep_link(&self, kind: PermissionKind) -> Option<String>;
}

pub trait SecretStore: Send + Sync + Debug {
    fn get(&self, key: SecretKey) -> Result<Option<SecretString>, PlatformError>;
    fn set(&self, key: SecretKey, value: &SecretString) -> Result<(), PlatformError>;
    fn delete(&self, key: SecretKey) -> Result<(), PlatformError>;
}

/// Speaks short replies aloud (voice assistant talk-back).
pub trait TextToSpeech: Send + Sync + Debug {
    /// Starts speaking `text`, interrupting anything already being spoken. `lang` is
    /// "bn" (Bangla) or "en"; backends fall back to the default voice.
    fn speak(&self, text: &str, lang: &str) -> Result<(), PlatformError>;
    /// Stops speaking at once.
    fn stop(&self);
    fn is_speaking(&self) -> bool;
    fn capability(&self) -> Capability;
}

#[async_trait]
pub trait Clipboard: Send + Sync + Debug {
    async fn read_text(&self) -> Result<Option<String>, PlatformError>;
    async fn write_text(&self, text: &str) -> Result<(), PlatformError>;
}
