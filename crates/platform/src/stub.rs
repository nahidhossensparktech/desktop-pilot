//! A backend that implements every trait by reporting "not implemented yet".
//! OS crates use it until their real implementations land; it never touches the OS.

use async_trait::async_trait;
use secrecy::SecretString;

use crate::traits::NativeWindowHandle;
use crate::*;

const MSG: &str = "not implemented yet (planned for milestone M1/M2)";

fn todo_err() -> PlatformError {
    PlatformError::Unsupported(MSG.to_owned())
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NotYetImplemented;

#[async_trait]
impl ScreenCapture for NotYetImplemented {
    async fn monitors(&self) -> Result<Vec<MonitorInfo>, PlatformError> {
        Err(todo_err())
    }
    async fn capture(&self, _monitor: &MonitorId) -> Result<Frame, PlatformError> {
        Err(todo_err())
    }
    fn exclude_windows(&self, _handles: &[NativeWindowHandle]) -> ExclusionSupport {
        ExclusionSupport::MustHide
    }
    fn capability(&self) -> Capability {
        Capability::unavailable(MSG)
    }
}

#[async_trait]
impl InputController for NotYetImplemented {
    fn coordinate_space(&self) -> CoordinateSpace {
        CoordinateSpace::DesktopPhysical
    }
    async fn mouse_move(&self, _p: DesktopPoint) -> Result<(), PlatformError> {
        Err(todo_err())
    }
    async fn button(&self, _b: MouseButton, _dir: Direction) -> Result<(), PlatformError> {
        Err(todo_err())
    }
    async fn scroll(&self, _dx: i32, _dy: i32) -> Result<(), PlatformError> {
        Err(todo_err())
    }
    async fn key(&self, _key: &Key, _dir: Direction) -> Result<(), PlatformError> {
        Err(todo_err())
    }
    async fn text(&self, _chunk: &str) -> Result<(), PlatformError> {
        Err(todo_err())
    }
    async fn cursor_position(&self) -> Result<Option<DesktopPoint>, PlatformError> {
        Err(todo_err())
    }
    fn release_all(&self) -> Result<(), PlatformError> {
        // Nothing can be held: this backend never presses anything.
        Ok(())
    }
    fn held_inputs(&self) -> HeldInputs {
        HeldInputs::default()
    }
    fn capability(&self) -> Capability {
        Capability::unavailable(MSG)
    }
}

#[async_trait]
impl AppCatalog for NotYetImplemented {
    async fn list(&self, _refresh: bool) -> Result<Vec<AppEntry>, PlatformError> {
        Err(todo_err())
    }
    async fn launch(&self, _app: &AppId, _args: &[String]) -> Result<LaunchResult, PlatformError> {
        Err(todo_err())
    }
}

#[async_trait]
impl WindowManager for NotYetImplemented {
    async fn list(&self) -> Result<Vec<WindowInfo>, PlatformError> {
        Err(todo_err())
    }
    async fn focus(&self, _id: &WindowId) -> Result<(), PlatformError> {
        Err(todo_err())
    }
    async fn focused(&self) -> Result<Option<WindowInfo>, PlatformError> {
        Err(todo_err())
    }
    fn capability(&self) -> Capability {
        Capability::unavailable(MSG)
    }
}

#[async_trait]
impl AccessibilityTree for NotYetImplemented {
    async fn tree(&self, _t: TreeTarget, _l: TreeLimits) -> Result<UiNode, PlatformError> {
        Err(todo_err())
    }
    fn capability(&self) -> Capability {
        Capability::unavailable(MSG)
    }
}

impl HotkeyService for NotYetImplemented {
    fn register(
        &self,
        _combo: &HotkeyCombo,
        _callback: Box<dyn Fn() + Send + Sync>,
    ) -> Result<HotkeyId, PlatformError> {
        Err(todo_err())
    }
    fn unregister(&self, _id: HotkeyId) -> Result<(), PlatformError> {
        Err(todo_err())
    }
    fn capability(&self) -> Capability {
        Capability::unavailable(MSG)
    }
}

#[async_trait]
impl PermissionManager for NotYetImplemented {
    async fn status(&self) -> Vec<PermissionStatus> {
        Vec::new()
    }
    async fn request(&self, kind: PermissionKind) -> Result<PermissionStatus, PlatformError> {
        Ok(PermissionStatus {
            kind,
            state: PermissionState::NotApplicable,
        })
    }
    fn settings_deep_link(&self, _kind: PermissionKind) -> Option<String> {
        None
    }
}

impl SecretStore for NotYetImplemented {
    fn get(&self, _key: SecretKey) -> Result<Option<SecretString>, PlatformError> {
        Err(todo_err())
    }
    fn set(&self, _key: SecretKey, _value: &SecretString) -> Result<(), PlatformError> {
        Err(todo_err())
    }
    fn delete(&self, _key: SecretKey) -> Result<(), PlatformError> {
        Err(todo_err())
    }
}

impl TextToSpeech for NotYetImplemented {
    fn speak(&self, _text: &str, _lang: &str) -> Result<(), PlatformError> {
        Err(todo_err())
    }
    fn stop(&self) {}
    fn is_speaking(&self) -> bool {
        false
    }
    fn capability(&self) -> Capability {
        Capability::unavailable(MSG)
    }
}

#[async_trait]
impl Clipboard for NotYetImplemented {
    async fn read_text(&self) -> Result<Option<String>, PlatformError> {
        Err(todo_err())
    }
    async fn write_text(&self, _text: &str) -> Result<(), PlatformError> {
        Err(todo_err())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_platform_reports_everything_unavailable() {
        let p = Platform::stub(PlatformInfo {
            os: OsKind::Other,
            os_version: "test".into(),
            arch: "test",
            session: SessionKind::Unknown,
            desktop: None,
        });
        let report = p.capability_report();
        assert!(!report.entries.is_empty());
        assert!(report.entries.iter().all(|(_, c)| !c.is_available()));
        assert!(p.input.release_all().is_ok());
        assert!(p.input.held_inputs().is_empty());
    }
}
