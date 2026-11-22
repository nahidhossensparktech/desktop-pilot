//! Where the Claude credential comes from.
//!
//! Order (same as Anthropic's SDKs, plus our saved key):
//! 1. `ANTHROPIC_API_KEY` environment variable
//! 2. `ANTHROPIC_AUTH_TOKEN` environment variable (OAuth bearer token)
//! 3. API key saved in the OS secret store (Settings)
//! 4. The `ant` CLI login (`ant auth login`): a short-lived OAuth token from
//!    `ant auth print-credentials --access-token`, which also refreshes it.
//!
//! Claude Code's own login is deliberately NOT used: it belongs to Claude Code.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use secrecy::{ExposeSecret, SecretString};

use crate::LlmError;

/// How long a token from `ant` is reused before asking `ant` again.
const ANT_TOKEN_TTL: Duration = Duration::from_secs(5 * 60);
const ANT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
pub enum Credential {
    /// Sent as `x-api-key`.
    ApiKey(SecretString),
    /// Sent as `Authorization: Bearer` plus the OAuth beta header.
    OAuthToken(SecretString),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CredentialSource {
    EnvApiKey,
    EnvAuthToken,
    SavedApiKey,
    AntCli { path: PathBuf },
}

impl std::fmt::Display for CredentialSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EnvApiKey => f.write_str("ANTHROPIC_API_KEY environment variable"),
            Self::EnvAuthToken => f.write_str("ANTHROPIC_AUTH_TOKEN environment variable"),
            Self::SavedApiKey => f.write_str("API key saved in Credential Manager"),
            Self::AntCli { .. } => f.write_str("ant CLI login (ant auth login)"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedCredential {
    pub credential: Credential,
    pub source: CredentialSource,
}

/// Resolves and caches the credential.
#[derive(Debug, Default)]
pub struct Credentials {
    saved_key: Mutex<Option<SecretString>>,
    ant_cache: Mutex<Option<(SecretString, Instant)>>,
}

fn env_secret(name: &str) -> Option<SecretString> {
    std::env::var(name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(SecretString::from)
}

/// Finds the `ant` executable on PATH or in common install folders.
pub fn find_ant() -> Option<PathBuf> {
    let exe = format!("ant{}", std::env::consts::EXE_SUFFIX);
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        let home = PathBuf::from(home);
        dirs.push(home.join("go").join("bin"));
        dirs.push(home.join(".local").join("bin"));
    }
    dirs.into_iter().map(|d| d.join(&exe)).find(|p| p.is_file())
}

async fn ant_token(path: &Path) -> Result<SecretString, LlmError> {
    let mut cmd = tokio::process::Command::new(path);
    cmd.args(["auth", "print-credentials", "--access-token"])
        // Make `ant` use its own login, not an env key.
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("ANTHROPIC_AUTH_TOKEN")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(ANT_TIMEOUT, cmd.output())
        .await
        .map_err(|_| LlmError::NoCredentials("`ant auth print-credentials` timed out".into()))?
        .map_err(|e| LlmError::NoCredentials(format!("could not run ant: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(LlmError::NoCredentials(format!(
            "ant has no usable login ({}). Run `ant auth login` in a terminal.",
            stderr.trim()
        )));
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if token.is_empty() || token.contains(char::is_whitespace) || token.starts_with('{') {
        return Err(LlmError::NoCredentials(
            "ant printed no access token. Run `ant auth login` in a terminal.".into(),
        ));
    }
    Ok(SecretString::from(token))
}

impl Credentials {
    pub fn new(saved_key: Option<SecretString>) -> Self {
        Self {
            saved_key: Mutex::new(saved_key),
            ant_cache: Mutex::new(None),
        }
    }

    /// Replaces the saved API key (after the user saves or removes it in Settings).
    pub fn set_saved_key(&self, key: Option<SecretString>) {
        *self
            .saved_key
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = key;
    }

    /// Which source would be used, without contacting `ant` or the API.
    pub fn preferred_source(&self) -> Option<CredentialSource> {
        if env_secret("ANTHROPIC_API_KEY").is_some() {
            return Some(CredentialSource::EnvApiKey);
        }
        if env_secret("ANTHROPIC_AUTH_TOKEN").is_some() {
            return Some(CredentialSource::EnvAuthToken);
        }
        if self
            .saved_key
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
        {
            return Some(CredentialSource::SavedApiKey);
        }
        find_ant().map(|path| CredentialSource::AntCli { path })
    }

    /// Returns the credential to use. `refresh` forces a new token from `ant`
    /// (after a 401).
    pub async fn resolve(&self, refresh: bool) -> Result<ResolvedCredential, LlmError> {
        if let Some(key) = env_secret("ANTHROPIC_API_KEY") {
            return Ok(ResolvedCredential {
                credential: Credential::ApiKey(key),
                source: CredentialSource::EnvApiKey,
            });
        }
        if let Some(token) = env_secret("ANTHROPIC_AUTH_TOKEN") {
            return Ok(ResolvedCredential {
                credential: Credential::OAuthToken(token),
                source: CredentialSource::EnvAuthToken,
            });
        }
        let saved = self
            .saved_key
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(key) = saved {
            return Ok(ResolvedCredential {
                credential: Credential::ApiKey(key),
                source: CredentialSource::SavedApiKey,
            });
        }
        let Some(path) = find_ant() else {
            return Err(LlmError::NoCredentials(
                "save an API key in Settings, or install the ant CLI and run `ant auth login`"
                    .into(),
            ));
        };
        if !refresh {
            let cached = self
                .ant_cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            if let Some((token, at)) = cached
                && at.elapsed() < ANT_TOKEN_TTL
            {
                return Ok(ResolvedCredential {
                    credential: Credential::OAuthToken(token),
                    source: CredentialSource::AntCli { path },
                });
            }
        }
        let token = ant_token(&path).await?;
        *self
            .ant_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some((token.clone(), Instant::now()));
        Ok(ResolvedCredential {
            credential: Credential::OAuthToken(token),
            source: CredentialSource::AntCli { path },
        })
    }
}

/// Masks a secret for display: `sk-ant-…a9F2`.
pub fn mask(secret: &SecretString) -> String {
    let s = secret.expose_secret();
    let tail: String = s
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if s.len() <= 8 {
        "••••".to_owned()
    } else {
        format!("••••{tail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_shows_only_last_four() {
        let m = mask(&SecretString::from(
            "sk-ant-api03-abcdefghijklmnopa9F2".to_owned(),
        ));
        assert_eq!(m, "••••a9F2");
        assert!(!m.contains("sk-ant"));
        assert_eq!(mask(&SecretString::from("short".to_owned())), "••••");
    }

    #[test]
    fn credential_debug_does_not_leak() {
        let c = Credential::ApiKey(SecretString::from("sk-ant-secret-value".to_owned()));
        assert!(!format!("{c:?}").contains("secret-value"));
    }
}
