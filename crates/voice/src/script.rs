//! Writes Whisper's Bangla output in Bangla script.
//!
//! Tested with Whisper small on Bangla speech: even with the language set to Bangla it
//! writes the words in Devanagari (Hindi script), e.g. "अखुन त्ता बाजे" for
//! "এখন কটা বাজে". The sound is right, only the script is wrong. Devanagari and
//! Bengali share the Unicode layout (Bengali = Devanagari + 0x80), so this converts
//! letter by letter, with special cases for letters Bangla does not have.

/// Bengali code points that are unassigned (no letter at Devanagari + 0x80).
const UNASSIGNED: &[u32] = &[
    0x0984, 0x098D, 0x098E, 0x0991, 0x0992, 0x09A9, 0x09B1, 0x09B3, 0x09B4, 0x09B5, 0x09BA, 0x09BB,
    0x09C5, 0x09C6, 0x09C9, 0x09CA, 0x09CF, 0x09D0, 0x09D1, 0x09D2, 0x09D3, 0x09D4, 0x09D5, 0x09D6,
    0x09D8, 0x09D9, 0x09DA, 0x09DB, 0x09DE, 0x09E4, 0x09E5,
];

/// Devanagari letters without a Bangla twin, and their closest Bangla letter.
fn special(c: char) -> Option<&'static str> {
    Some(match c {
        'व' => "ব",
        'ळ' | 'ऴ' => "ল",
        'ऱ' => "র",
        'ऩ' => "ন",
        'ऑ' | 'ॲ' => "অ",
        'ऍ' | 'ऎ' => "এ",
        'ऒ' => "ও",
        'ॉ' | 'ॊ' => "ো",
        'ॅ' | 'ॆ' => "ে",
        'ॐ' => "ওঁ",
        'ऀ' => "",
        _ => return None,
    })
}

/// Converts Devanagari letters in `text` to Bangla script; everything else (Latin,
/// Bangla, punctuation, the shared danda "।") is kept.
pub fn devanagari_to_bangla(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        let code = u32::from(c);
        if !(0x0900..=0x097F).contains(&code) || matches!(code, 0x0964 | 0x0965) {
            out.push(c);
            continue;
        }
        if let Some(s) = special(c) {
            out.push_str(s);
            continue;
        }
        let target = code + 0x80;
        match char::from_u32(target) {
            Some(b) if !UNASSIGNED.contains(&target) => out.push(b),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whisper_outputs_become_bangla() {
        // Real Whisper outputs for Bangla speech (see module docs).
        assert_eq!(devanagari_to_bangla("अखुन त्ता बाजे"), "অখুন ত্তা বাজে");
        assert_eq!(
            devanagari_to_bangla("शाखारी खुले गुबले आब हाँजा देखो"),
            "শাখারী খুলে গুবলে আব হাঁজা দেখো"
        );
        assert_eq!(devanagari_to_bangla("इक नुखर ता बाजे"), "ইক নুখর তা বাজে");
    }

    #[test]
    fn special_letters_and_other_text() {
        assert_eq!(devanagari_to_bangla("वन"), "বন");
        assert_eq!(devanagari_to_bangla("कॉफ़ी"), "কোফ়ী");
        assert_eq!(devanagari_to_bangla("१२३"), "১২৩");
        assert_eq!(devanagari_to_bangla("Open Safari।"), "Open Safari।");
        assert_eq!(devanagari_to_bangla("এখন কটা বাজে"), "এখন কটা বাজে");
    }
}
