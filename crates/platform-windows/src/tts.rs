//! Talk-back with the in-box `System.Speech` synthesizer through PowerShell. Uses an
//! installed Bangla voice for Bangla if there is one, else the default voice.

use std::os::windows::process::CommandExt as _;
use std::process::Command;

use dp_platform::Capability;
use dp_platform::speech::CommandSpeech;

/// `CREATE_NO_WINDOW`: no console window flashes up.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn script(lang: &str) -> String {
    let pick = if lang == "bn" {
        "try { $v = $s.GetInstalledVoices() | Where-Object { $_.VoiceInfo.Culture.Name -like 'bn-*' } | Select-Object -First 1; if ($v) { $s.SelectVoice($v.VoiceInfo.Name) } } catch {}"
    } else {
        ""
    };
    format!(
        "[Console]::InputEncoding = [Text.Encoding]::UTF8; \
         Add-Type -AssemblyName System.Speech; \
         $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; {pick} \
         $s.Speak([Console]::In.ReadToEnd())"
    )
}

pub(crate) fn speech() -> CommandSpeech {
    CommandSpeech::new(
        Box::new(|lang| {
            let mut cmd = Command::new("powershell");
            cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script(lang)])
                .creation_flags(CREATE_NO_WINDOW);
            Some(cmd)
        }),
        Capability::Available,
    )
}
