//! Finds the wake phrase ("Hey Mouse", also Bangla "হেই মাউস") in a transcript and
//! returns the command after it; recognizes spoken "stop" commands.
//!
//! Whisper spells the same sound in different ways ("Hey, Mouse.", "Hi Mouse", "Hey
//! Maus", "হে মাউস"), so matching is word-based and fuzzy.

/// Greeting words that start the wake phrase. Kept short on purpose: "a", "ok", "এই"
/// made everyday sentences ("I need a mouse") wake the assistant.
const GREETINGS: &[&str] = &["hey", "hi", "hay", "hei", "he", "হে", "হেই", "হেয়", "হাই"];
/// Spellings of "mouse" seen in transcripts (plus close misspellings starting with m).
const MOUSE: &[&str] = &[
    "mouse",
    "maus",
    "mause",
    "mous",
    "mowse",
    "mouce",
    "মাউস",
    "মাউজ",
    "মাউস্",
    "মাওস",
];
/// Filler words allowed before the wake phrase ("uh, hey mouse").
const FILLERS: &[&str] = &["uh", "um", "so", "oh", "ok", "okay", "and", "আচ্ছা"];

/// Lowercases, strips punctuation around a word (keeps letters of any script) and
/// drops accents from Latin letters, since Whisper writes Bangla phonetically
/// ("hē māus").
fn clean(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'ā' | 'á' | 'à' | 'â' | 'ä' => 'a',
            'ē' | 'é' | 'è' | 'ê' | 'ë' => 'e',
            'ī' | 'í' | 'ì' | 'î' | 'ï' => 'i',
            'ō' | 'ó' | 'ò' | 'ô' | 'ö' => 'o',
            'ū' | 'ú' | 'ù' | 'û' | 'ü' => 'u',
            other => other,
        })
        .collect()
}

/// Edit distance between two short words (in characters).
fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

fn is_greeting(w: &str) -> bool {
    GREETINGS.contains(&w)
}

/// "mouse" or a close misspelling of it. Misspellings must start with "m", so
/// "house", "louse" and "rouse" do not count.
fn is_mouse(w: &str) -> bool {
    MOUSE.contains(&w)
        || (w.is_ascii()
            && w.starts_with('m')
            && (4..=6).contains(&w.len())
            && levenshtein(w, "mouse") <= 1)
}

/// Where the wake phrase is: the index of the first word after it, if found. It must
/// be the first thing said (one filler word like "uh" may come before it), and it must
/// be a greeting followed by "mouse": "I need a mouse" or a bare "Mouse, …" do not
/// wake the assistant.
fn wake_end(words: &[String]) -> Option<usize> {
    let start = usize::from(words.first().is_some_and(|w| FILLERS.contains(&w.as_str())));
    let w = words.get(start)?;
    // One word: "heymouse" / "হেইমাউস".
    if let Some(rest) = w.strip_prefix("hey").or_else(|| w.strip_prefix("হেই"))
        && is_mouse(rest)
    {
        return Some(start + 1);
    }
    if is_greeting(w) && words.get(start + 1).is_some_and(|m| is_mouse(m)) {
        return Some(start + 2);
    }
    None
}

/// If `transcript` starts with the wake phrase, returns the command after it (may be
/// empty when only "Hey Mouse" was said). `None` when there is no wake phrase.
pub fn find_wake(transcript: &str) -> Option<String> {
    let raw: Vec<&str> = transcript.split_whitespace().collect();
    let words: Vec<String> = raw.iter().map(|w| clean(w)).collect();
    let end = wake_end(&words)?;
    let rest = raw[end..].join(" ");
    Some(
        rest.trim_start_matches(|c: char| !c.is_alphanumeric())
            .trim()
            .to_owned(),
    )
}

/// The command after the wake phrase, for an utterance known to start with it (the
/// short first pass heard "Hey Mouse"). If the full transcript spells the wake phrase
/// differently, drops words up to one that sounds like "mouse", else the first two.
pub fn strip_wake(transcript: &str) -> String {
    if let Some(rest) = find_wake(transcript) {
        return rest;
    }
    let raw: Vec<&str> = transcript.split_whitespace().collect();
    let words: Vec<String> = raw.iter().map(|w| clean(w)).collect();
    let end = words
        .iter()
        .take(3)
        .position(|w| w.is_ascii() && levenshtein(w, "mouse") <= 2)
        .map_or(2, |i| i + 1)
        .min(raw.len());
    raw[end..]
        .join(" ")
        .trim_start_matches(|c: char| !c.is_alphanumeric())
        .trim()
        .to_owned()
}

/// Whether `command` asks to stop the running task.
pub fn is_stop_command(command: &str) -> bool {
    let words: Vec<String> = command.split_whitespace().map(clean).collect();
    if words.is_empty() || words.len() > 4 {
        return false;
    }
    let text = words.join(" ");
    const STOPS: &[&str] = &[
        "stop",
        "stop it",
        "stop now",
        "cancel",
        "abort",
        "halt",
        "থামো",
        "থামাও",
        "থাম",
        "বন্ধ করো",
        "বন্ধ কর",
        "বন্ধ",
        "স্টপ",
        "থেমে যাও",
    ];
    STOPS
        .iter()
        .any(|s| text == *s || text.starts_with(&format!("{s} ")))
}

/// Phrases Whisper is known to "hear" in noise or silence (learned from subtitles).
const PHANTOMS: &[&str] = &[
    "nothing",
    "thank you",
    "thanks",
    "thank you very much",
    "thanks for watching",
    "thank you for watching",
    "please subscribe",
    "subscribe",
    "bye",
    "bye bye",
    "goodbye",
    "you",
    "okay",
    "ok",
    "oh",
    "uh",
    "um",
    "hmm",
    "so",
    "yeah",
    "yes",
    "no",
    "music",
    "silence",
    "the end",
    "ধন্যবাদ",
    "আচ্ছা",
    "হ্যাঁ",
    "না",
    "ঠিক আছে",
];

/// Whether a transcript is a real command: at least two words, and not one of the
/// phrases Whisper invents for noise ("Nothing.", "Thank you."). Stop commands
/// ("stop", "থামো") count even as one word.
pub fn is_meaningful_command(text: &str) -> bool {
    if is_stop_command(text) {
        return true;
    }
    let words: Vec<String> = text
        .split_whitespace()
        .map(clean)
        .filter(|w| !w.is_empty())
        .collect();
    if words.len() < 2 {
        return false;
    }
    !PHANTOMS.contains(&words.join(" ").as_str())
}

/// Removes Whisper's sound annotations ("[MUSIC]", "(applause)", "[BLANK_AUDIO]") and
/// collapses whitespace.
pub fn clean_transcript(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0_u32;
    for c in text.chars() {
        match c {
            '[' | '(' | '*' if depth == 0 => depth = 1,
            ']' | ')' | '*' if depth == 1 => depth = 0,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_wake_with_command() {
        assert_eq!(
            find_wake("Hey Mouse, open TextEdit.").as_deref(),
            Some("open TextEdit.")
        );
        assert_eq!(
            find_wake("Hi mouse open safari").as_deref(),
            Some("open safari")
        );
        assert_eq!(
            find_wake("Hey, Maus. What time is it?").as_deref(),
            Some("What time is it?")
        );
        assert_eq!(
            find_wake("Uh, hey mouse, play music").as_deref(),
            Some("play music")
        );
        assert_eq!(
            find_wake("Heymouse open notes").as_deref(),
            Some("open notes")
        );
        assert_eq!(find_wake("Okay, hey mouse, stop").as_deref(), Some("stop"));
    }

    #[test]
    fn everyday_speech_does_not_wake() {
        for s in [
            "I need a mouse",
            "a mouse ran by",
            "Hey house, open it",
            "Mouse, open Safari",
            "ok mouse",
            "so I said hey mouse",
            "he must go",
            "Hey Mom, are you there?",
        ] {
            assert_eq!(find_wake(s), None, "{s}");
        }
    }

    #[test]
    fn wake_without_command() {
        assert_eq!(find_wake("Hey Mouse.").as_deref(), Some(""));
        assert_eq!(find_wake("hey mouse").as_deref(), Some(""));
    }

    #[test]
    fn bangla_wake() {
        assert_eq!(
            find_wake("হেই মাউস, সাফারি খোলো").as_deref(),
            Some("সাফারি খোলো")
        );
        assert_eq!(find_wake("হে মাউস।").as_deref(), Some(""));
        assert_eq!(
            find_wake("Hey Mouse, Safari kholo").as_deref(),
            Some("Safari kholo")
        );
    }

    #[test]
    fn phonetic_bangla_wake() {
        assert_eq!(
            find_wake("hē māus, shākhāri khulu").as_deref(),
            Some("shākhāri khulu")
        );
        assert_eq!(
            find_wake("HAY MOUSE, SHAKARI KOLO").as_deref(),
            Some("SHAKARI KOLO")
        );
    }

    #[test]
    fn strip_wake_falls_back_when_spelled_differently() {
        assert_eq!(strip_wake("Hey Mouse. Open notes."), "Open notes.");
        assert_eq!(strip_wake("Hā mos, ājke kemon"), "ājke kemon");
        assert_eq!(strip_wake("Hej Muz, play music"), "play music");
    }

    #[test]
    fn no_wake() {
        assert_eq!(find_wake("I bought a new mouse yesterday"), None);
        assert_eq!(find_wake("hello there"), None);
        assert_eq!(find_wake(""), None);
        assert_eq!(find_wake("the mice are here"), None);
        assert_eq!(find_wake("Hey!"), None);
    }

    #[test]
    fn stop_commands() {
        assert!(is_stop_command("Stop."));
        assert!(is_stop_command("stop it"));
        assert!(is_stop_command("থামো"));
        assert!(is_stop_command("বন্ধ করো।"));
        assert!(!is_stop_command(
            "stop the music in Spotify and open notes now please"
        ));
        assert!(!is_stop_command("open safari"));
    }

    #[test]
    fn noise_phantoms_are_not_commands() {
        assert!(!is_meaningful_command("Nothing."));
        assert!(!is_meaningful_command("Thank you."));
        assert!(!is_meaningful_command("Thanks for watching!"));
        assert!(!is_meaningful_command("ধন্যবাদ।"));
        assert!(!is_meaningful_command("Hello"));
        assert!(!is_meaningful_command(""));
        assert!(is_meaningful_command("Stop."));
        assert!(is_meaningful_command("থামো"));
        assert!(is_meaningful_command("open Safari"));
        assert!(is_meaningful_command("shākhāri khulu"));
    }

    #[test]
    fn annotations_are_removed() {
        assert_eq!(
            clean_transcript(" [MUSIC] Hey mouse (laughs) hi "),
            "Hey mouse hi"
        );
        assert_eq!(clean_transcript("[BLANK_AUDIO]"), "");
    }
}
