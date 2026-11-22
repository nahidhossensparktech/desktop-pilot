//! Key names (computer-use / xdotool keysym style) → macOS virtual key codes (`kVK_*`,
//! ANSI layout positions).

use objc2_core_graphics::{CGEventFlags, CGKeyCode};

/// Resolves one key name (case-insensitive for named keys). Returns `None` if unknown.
pub(crate) fn resolve(name: &str) -> Option<CGKeyCode> {
    let lower = name.to_ascii_lowercase();
    let named = match lower.as_str() {
        // Modifiers. `super`/`win`/`meta` mean Command, as the toolset's key syntax says.
        "ctrl" | "control" | "control_l" | "ctrl_l" => Some(0x3B),
        "control_r" | "ctrl_r" => Some(0x3E),
        "alt" | "alt_l" | "option" | "opt" => Some(0x3A),
        "alt_r" | "altgr" | "option_r" => Some(0x3D),
        "shift" | "shift_l" => Some(0x38),
        "shift_r" => Some(0x3C),
        "super" | "super_l" | "win" | "windows" | "meta" | "cmd" | "command" => Some(0x37),
        "super_r" | "cmd_r" | "command_r" => Some(0x36),
        "fn" | "function" => Some(0x3F),
        // Editing / navigation
        "return" | "enter" => Some(0x24),
        "kp_enter" => Some(0x4C),
        "tab" => Some(0x30),
        "backspace" | "back_space" => Some(0x33),
        "escape" | "esc" => Some(0x35),
        "space" => Some(0x31),
        "delete" | "del" => Some(0x75),
        "insert" | "ins" | "help" => Some(0x72),
        "home" => Some(0x73),
        "end" => Some(0x77),
        "page_up" | "pageup" | "prior" => Some(0x74),
        "page_down" | "pagedown" | "next" => Some(0x79),
        "up" => Some(0x7E),
        "down" => Some(0x7D),
        "left" => Some(0x7B),
        "right" => Some(0x7C),
        "caps_lock" | "capslock" => Some(0x39),
        // Punctuation names
        "minus" => Some(0x1B),
        "plus" | "equal" | "equals" => Some(0x18),
        "comma" => Some(0x2B),
        "period" | "dot" => Some(0x2F),
        "slash" => Some(0x2C),
        "backslash" => Some(0x2A),
        "semicolon" => Some(0x29),
        "apostrophe" | "quote" => Some(0x27),
        "bracketleft" => Some(0x21),
        "bracketright" => Some(0x1E),
        "grave" | "backtick" => Some(0x32),
        // Media
        "xf86audiomute" | "volumemute" => Some(0x4A),
        "xf86audiolowervolume" | "volumedown" => Some(0x49),
        "xf86audioraisevolume" | "volumeup" => Some(0x48),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    // F1..F20
    if let Some(n) = lower
        .strip_prefix('f')
        .and_then(|n| n.parse::<usize>().ok())
        && (1..=20).contains(&n)
    {
        const F: [CGKeyCode; 20] = [
            0x7A, 0x78, 0x63, 0x76, 0x60, 0x61, 0x62, 0x64, 0x65, 0x6D, 0x67, 0x6F, 0x69, 0x6B,
            0x71, 0x6A, 0x40, 0x4F, 0x50, 0x5A,
        ];
        return Some(F[n - 1]);
    }
    // Keypad digits
    if let Some(d) = lower
        .strip_prefix("kp_")
        .and_then(|d| d.parse::<usize>().ok())
        && d <= 9
    {
        const KP: [CGKeyCode; 10] = [0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5B, 0x5C];
        return Some(KP[d]);
    }
    let mut chars = name.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return None;
    };
    char_key(c)
}

/// The ANSI key that produces `c` (shifted characters map to their base key, as
/// Windows' `VkKeyScanW` does).
fn char_key(c: char) -> Option<CGKeyCode> {
    let code = match c.to_ascii_lowercase() {
        'a' => 0x00,
        's' => 0x01,
        'd' => 0x02,
        'f' => 0x03,
        'h' => 0x04,
        'g' => 0x05,
        'z' => 0x06,
        'x' => 0x07,
        'c' => 0x08,
        'v' => 0x09,
        'b' => 0x0B,
        'q' => 0x0C,
        'w' => 0x0D,
        'e' => 0x0E,
        'r' => 0x0F,
        'y' => 0x10,
        't' => 0x11,
        '1' | '!' => 0x12,
        '2' | '@' => 0x13,
        '3' | '#' => 0x14,
        '4' | '$' => 0x15,
        '6' | '^' => 0x16,
        '5' | '%' => 0x17,
        '=' | '+' => 0x18,
        '9' | '(' => 0x19,
        '7' | '&' => 0x1A,
        '-' | '_' => 0x1B,
        '8' | '*' => 0x1C,
        '0' | ')' => 0x1D,
        ']' | '}' => 0x1E,
        'o' => 0x1F,
        'u' => 0x20,
        '[' | '{' => 0x21,
        'i' => 0x22,
        'p' => 0x23,
        'l' => 0x25,
        'j' => 0x26,
        '\'' | '"' => 0x27,
        'k' => 0x28,
        ';' | ':' => 0x29,
        '\\' | '|' => 0x2A,
        ',' | '<' => 0x2B,
        '/' | '?' => 0x2C,
        'n' => 0x2D,
        'm' => 0x2E,
        '.' | '>' => 0x2F,
        '`' | '~' => 0x32,
        ' ' => 0x31,
        _ => return None,
    };
    Some(code)
}

/// The event flag a modifier key sets while held, if it is a modifier.
pub(crate) fn modifier_flag(code: CGKeyCode) -> Option<CGEventFlags> {
    match code {
        0x37 | 0x36 => Some(CGEventFlags::MaskCommand),
        0x38 | 0x3C => Some(CGEventFlags::MaskShift),
        0x3A | 0x3D => Some(CGEventFlags::MaskAlternate),
        0x3B | 0x3E => Some(CGEventFlags::MaskControl),
        0x3F => Some(CGEventFlags::MaskSecondaryFn),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_names_resolve() {
        assert_eq!(resolve("Return"), Some(0x24));
        assert_eq!(resolve("cmd"), Some(0x37));
        assert_eq!(resolve("super"), Some(0x37));
        assert_eq!(resolve("ctrl"), Some(0x3B));
        assert_eq!(resolve("Page_Down"), Some(0x79));
        assert_eq!(resolve("F5"), Some(0x60));
        assert_eq!(resolve("F20"), Some(0x5A));
        assert_eq!(resolve("a"), Some(0x00));
        assert_eq!(resolve("A"), Some(0x00));
        assert_eq!(resolve("7"), Some(0x1A));
        assert_eq!(resolve("KP_5"), Some(0x57));
        assert!(resolve("no_such_key").is_none());
        assert!(resolve("F21").is_none());
        assert!(resolve("é").is_none());
    }

    #[test]
    fn modifiers_have_flags() {
        assert_eq!(
            modifier_flag(resolve("cmd").unwrap()),
            Some(CGEventFlags::MaskCommand)
        );
        assert_eq!(modifier_flag(resolve("a").unwrap()), None);
    }
}
