//! Secrets in Windows Credential Manager (ADR-010), via the Win32 Cred* API directly.
//!
//! Entries are generic credentials named `com.desktoppilot.DesktopPilot/<account>`,
//! visible in Control Panel ▸ Credential Manager ▸ Windows Credentials.

use dp_platform::{PlatformError, SecretKey, SecretStore};
use secrecy::{ExposeSecret, SecretString};
use windows::Win32::Foundation::ERROR_NOT_FOUND;
use windows::Win32::Security::Credentials::{
    CRED_FLAGS, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree,
    CredReadW, CredWriteW,
};
use windows::core::{HSTRING, PWSTR};

const SERVICE: &str = "com.desktoppilot.DesktopPilot";

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct WindowsSecrets;

fn target(key: SecretKey) -> HSTRING {
    HSTRING::from(format!("{SERVICE}/{}", key.account()))
}

fn is_not_found(e: &windows::core::Error) -> bool {
    e.code() == ERROR_NOT_FOUND.to_hresult()
}

impl SecretStore for WindowsSecrets {
    fn get(&self, key: SecretKey) -> Result<Option<SecretString>, PlatformError> {
        let name = target(key);
        let mut cred: *mut CREDENTIALW = std::ptr::null_mut();
        // SAFETY: `cred` receives a buffer owned by Windows that we free with CredFree.
        match unsafe { CredReadW(&name, CRED_TYPE_GENERIC, None, &mut cred) } {
            Ok(()) => {}
            Err(e) if is_not_found(&e) => return Ok(None),
            Err(e) => return Err(PlatformError::Os(format!("Credential Manager read: {e}"))),
        }
        // SAFETY: CredReadW succeeded, so `cred` points to a valid CREDENTIALW whose
        // blob is CredentialBlobSize bytes long. We copy it out before freeing.
        let value = unsafe {
            let c = &*cred;
            let bytes = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize);
            let text = String::from_utf8_lossy(bytes).into_owned();
            CredFree(cred.cast());
            text
        };
        Ok(Some(SecretString::from(value)))
    }

    fn set(&self, key: SecretKey, value: &SecretString) -> Result<(), PlatformError> {
        let name = target(key);
        let mut blob = value.expose_secret().as_bytes().to_vec();
        let mut user: Vec<u16> = "Desktop Pilot".encode_utf16().chain(Some(0)).collect();
        let mut target_name: Vec<u16> = name.to_string().encode_utf16().chain(Some(0)).collect();
        let cred = CREDENTIALW {
            Flags: CRED_FLAGS(0),
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR(target_name.as_mut_ptr()),
            CredentialBlobSize: u32::try_from(blob.len())
                .map_err(|_| PlatformError::Os("secret too large".to_owned()))?,
            CredentialBlob: blob.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            UserName: PWSTR(user.as_mut_ptr()),
            ..Default::default()
        };
        // SAFETY: all pointers in `cred` point into buffers that live until after the call.
        let result = unsafe { CredWriteW(&cred, 0) };
        blob.fill(0);
        result.map_err(|e| PlatformError::Os(format!("Credential Manager write: {e}")))
    }

    fn delete(&self, key: SecretKey) -> Result<(), PlatformError> {
        let name = target(key);
        // SAFETY: deletes by name; no buffers are passed.
        match unsafe { CredDeleteW(&name, CRED_TYPE_GENERIC, None) } {
            Ok(()) => Ok(()),
            Err(e) if is_not_found(&e) => Ok(()),
            Err(e) => Err(PlatformError::Os(format!("Credential Manager delete: {e}"))),
        }
    }
}
