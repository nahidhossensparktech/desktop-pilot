//! Talk-back with the built-in `say` command: a Bangla voice (e.g. "Piya", bn_IN) for
//! Bangla, the system default voice for English.

use std::process::Command;

use dp_platform::Capability;
use dp_platform::speech::CommandSpeech;

const SAY: &str = "/usr/bin/say";

/// Installed voices as (name, locale), from `say -v ?`. Read each time (≈0.1 s), so a
/// voice downloaded in System Settings is used at once.
fn voices() -> Vec<(String, String)> {
    Command::new(SAY)
        .args(["-v", "?"])
        .output()
        .map(|o| parse_voices(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

/// Parses `say -v ?` lines: `Piya                bn_IN    # Hello! …`.
fn parse_voices(list: &str) -> Vec<(String, String)> {
    list.lines()
        .filter_map(|line| {
            let (head, _) = line.split_once('#')?;
            let mut parts: Vec<&str> = head.split_whitespace().collect();
            let locale = parts.pop()?;
            (locale.contains('_') && !parts.is_empty())
                .then(|| (parts.join(" "), locale.to_owned()))
        })
        .collect()
}

/// How natural a voice sounds: Premium (2) > Enhanced (1) > basic (0).
fn quality(name: &str) -> u8 {
    if name.contains("(Premium)") {
        2
    } else if name.contains("(Enhanced)") {
        1
    } else {
        0
    }
}

/// The most natural installed voice for a language, or `None` to use the system
/// default. English uses the system voice unless a Premium/Enhanced one is installed
/// (the basic English list is full of novelty voices).
fn voice_for(list: &[(String, String)], lang: &str) -> Option<String> {
    let prefix = format!("{lang}_");
    let best = list
        .iter()
        .filter(|(_, locale)| locale.starts_with(&prefix))
        .max_by_key(|(name, _)| quality(name))?;
    (lang != "en" || quality(&best.0) > 0).then(|| best.0.clone())
}

pub(crate) fn speech() -> CommandSpeech {
    let capability = if std::path::Path::new(SAY).exists() {
        Capability::Available
    } else {
        Capability::unavailable("`say` not found")
    };
    CommandSpeech::new(
        Box::new(|lang| {
            let mut cmd = Command::new(SAY);
            // The most natural installed voice for the language (Premium > Enhanced >
            // basic). Better voices: System Settings ▸ Accessibility ▸ Spoken Content ▸
            // System voice ▸ Manage Voices.
            if let Some(v) = voice_for(&voices(), lang) {
                cmd.args(["-v", &v]);
            }
            Some(cmd)
        }),
        capability,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_voice_list_and_picks_bangla() {
        let list = parse_voices(
            "Albert              en_US    # Hello! My name is Albert.\n\
             Piya                bn_IN    # Hello! My name is Piya.\n\
             Eddy (German (Germany)) de_DE    # Hallo! Ich heiße Eddy.\n",
        );
        assert_eq!(list.len(), 3);
        assert_eq!(voice_for(&list, "bn").as_deref(), Some("Piya"));
        assert_eq!(
            voice_for(&list, "de").as_deref(),
            Some("Eddy (German (Germany))")
        );
        assert_eq!(voice_for(&list, "ja"), None);
        // Basic English voices only: use the system default.
        assert_eq!(voice_for(&list, "en"), None);
    }

    #[test]
    fn natural_voices_are_preferred() {
        let list = parse_voices(
            "Piya                bn_IN    # Hello!\n\
             Piya (Enhanced)     bn_IN    # Hello!\n\
             Albert              en_US    # Hello!\n\
             Ava (Enhanced)      en_US    # Hello!\n\
             Zoe (Premium)       en_US    # Hello!\n",
        );
        assert_eq!(voice_for(&list, "bn").as_deref(), Some("Piya (Enhanced)"));
        assert_eq!(voice_for(&list, "en").as_deref(), Some("Zoe (Premium)"));
    }
}
