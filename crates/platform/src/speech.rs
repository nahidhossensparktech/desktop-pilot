//! Text to speech through a system command that reads the text on stdin (`say` on
//! macOS, PowerShell `System.Speech` on Windows, `spd-say`/`espeak-ng` on Linux). The
//! OS crates only choose the command; this file has no OS-specific code.

use std::fmt;
use std::io::Write as _;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, PoisonError};

use crate::{Capability, PlatformError, TextToSpeech};

/// Builds the speaking command for a language ("bn" or "en"); `None` = cannot speak.
pub type CommandBuilder = Box<dyn Fn(&str) -> Option<Command> + Send + Sync>;

/// Speaks by running a command per utterance; `stop` kills it.
pub struct CommandSpeech {
    build: CommandBuilder,
    child: Mutex<Option<Child>>,
    capability: Capability,
}

impl fmt::Debug for CommandSpeech {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandSpeech")
            .field("capability", &self.capability)
            .finish_non_exhaustive()
    }
}

impl CommandSpeech {
    pub fn new(build: CommandBuilder, capability: Capability) -> Self {
        Self {
            build,
            child: Mutex::new(None),
            capability,
        }
    }

    fn child(&self) -> std::sync::MutexGuard<'_, Option<Child>> {
        self.child.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl TextToSpeech for CommandSpeech {
    fn speak(&self, text: &str, lang: &str) -> Result<(), PlatformError> {
        self.stop();
        let text = text.trim();
        if text.is_empty() {
            return Ok(());
        }
        let mut cmd = (self.build)(lang)
            .ok_or_else(|| PlatformError::Unsupported("no text-to-speech voice".to_owned()))?;
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| PlatformError::Os(format!("text to speech: {e}")))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(text.as_bytes())
                .map_err(|e| PlatformError::Os(format!("text to speech: {e}")))?;
        }
        *self.child() = Some(child);
        Ok(())
    }

    fn stop(&self) {
        if let Some(mut child) = self.child().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn is_speaking(&self) -> bool {
        let mut guard = self.child();
        match guard.as_mut().map(Child::try_wait) {
            Some(Ok(None)) => true,
            Some(_) => {
                *guard = None;
                false
            }
            None => false,
        }
    }

    fn capability(&self) -> Capability {
        self.capability.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speaks_through_a_command_and_finishes() {
        // `cat` stands in for a speech command: it reads stdin and exits.
        let tts = CommandSpeech::new(
            Box::new(|_| Some(Command::new("cat"))),
            Capability::Available,
        );
        if Command::new("cat").stdin(Stdio::null()).output().is_err() {
            return; // no `cat` (Windows CI)
        }
        tts.speak("hello", "en").unwrap();
        for _ in 0..100 {
            if !tts.is_speaking() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the command did not finish");
    }

    #[test]
    fn no_command_is_an_error() {
        let tts = CommandSpeech::new(Box::new(|_| None), Capability::unavailable("none"));
        assert!(tts.speak("hi", "en").is_err());
        assert!(!tts.is_speaking());
    }
}
