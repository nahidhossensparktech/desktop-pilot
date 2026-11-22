//! Extra fonts. egui's built-in fonts have no Bengali letters, so Bangla text (Claude's
//! answers, your typed or spoken prompts) showed as empty boxes. A system Bangla font
//! is added as a fallback; egui shapes it properly (joined letters such as "ক্ষ",
//! vowel signs such as "ি" drawn before the consonant).
//!
//! The font is read from the system instead of shipped with the app: every desktop OS
//! has one, and it keeps the binary small.

use eframe::egui::{
    self, FontData, FontFamily,
    epaint::text::{FontInsert, FontPriority, InsertFontFamily},
};

/// Bangla fonts in the order we prefer them (missing files are skipped).
const BANGLA_FONTS: &[&str] = &[
    // macOS
    "/System/Library/Fonts/KohinoorBangla.ttc",
    "/System/Library/Fonts/Supplemental/Bangla Sangam MN.ttc",
    "/System/Library/Fonts/Supplemental/Bangla MN.ttc",
    // Windows
    "C:\\Windows\\Fonts\\Nirmala.ttc",
    "C:\\Windows\\Fonts\\Nirmala.ttf",
    "C:\\Windows\\Fonts\\vrinda.ttf",
    // Linux
    "/usr/share/fonts/truetype/noto/NotoSansBengali-Regular.ttf",
    "/usr/share/fonts/noto/NotoSansBengali-Regular.ttf",
    "/usr/share/fonts/google-noto/NotoSansBengali-Regular.ttf",
    "/usr/share/fonts/truetype/lohit-bengali/Lohit-Bengali.ttf",
];

/// Adds the first Bangla font found as a fallback for all text. Returns its path.
pub(super) fn add_bangla(ctx: &egui::Context) -> Option<&'static str> {
    let (path, bytes) = BANGLA_FONTS
        .iter()
        .find_map(|p| std::fs::read(p).ok().map(|b| (*p, b)))?;
    let fallback = |family| InsertFontFamily {
        family,
        priority: FontPriority::Lowest,
    };
    ctx.add_font(FontInsert::new(
        "bangla",
        FontData::from_owned(bytes),
        vec![
            fallback(FontFamily::Proportional),
            fallback(FontFamily::Monospace),
        ],
    ));
    Some(path)
}

/// Whether `text` contains Bengali script (U+0980–U+09FF).
pub(super) fn has_bangla(text: &str) -> bool {
    text.chars().any(|c| ('\u{0980}'..='\u{09FF}').contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_bengali_script() {
        assert!(has_bangla("কাজ শেষ"));
        assert!(has_bangla("Done, কাজ শেষ"));
        assert!(!has_bangla("Safari kholo"));
        assert!(!has_bangla(""));
    }
}
