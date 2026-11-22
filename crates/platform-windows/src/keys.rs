//! Key names (computer-use / xdotool keysym style) → Windows virtual-key codes.

use windows::Win32::UI::Input::KeyboardAndMouse::VkKeyScanW;

/// A resolved key: virtual-key code and whether it needs `KEYEVENTF_EXTENDEDKEY`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Vk {
    pub(crate) code: u16,
    pub(crate) extended: bool,
}

const fn vk(code: u16) -> Option<Vk> {
    Some(Vk {
        code,
        extended: false,
    })
}

const fn ext(code: u16) -> Option<Vk> {
    Some(Vk {
        code,
        extended: true,
    })
}

/// Resolves one key name (case-insensitive for named keys). Returns `None` if unknown.
pub(crate) fn resolve(name: &str) -> Option<Vk> {
    let lower = name.to_ascii_lowercase();
    let named = match lower.as_str() {
        // Modifiers
        "ctrl" | "control" | "control_l" | "ctrl_l" => vk(0xA2),
        "control_r" | "ctrl_r" => ext(0xA3),
        "alt" | "alt_l" | "option" => vk(0xA4),
        "alt_r" | "altgr" => ext(0xA5),
        "shift" | "shift_l" => vk(0xA0),
        "shift_r" => vk(0xA1),
        "super" | "super_l" | "win" | "windows" | "meta" | "cmd" | "command" => ext(0x5B),
        "super_r" => ext(0x5C),
        // Editing / navigation
        "return" | "enter" | "kp_enter" => vk(0x0D),
        "tab" => vk(0x09),
        "backspace" | "back_space" => vk(0x08),
        "escape" | "esc" => vk(0x1B),
        "space" => vk(0x20),
        "delete" | "del" => ext(0x2E),
        "insert" | "ins" => ext(0x2D),
        "home" => ext(0x24),
        "end" => ext(0x23),
        "page_up" | "pageup" | "prior" => ext(0x21),
        "page_down" | "pagedown" | "next" => ext(0x22),
        "up" => ext(0x26),
        "down" => ext(0x28),
        "left" => ext(0x25),
        "right" => ext(0x27),
        "caps_lock" | "capslock" => vk(0x14),
        "num_lock" | "numlock" => ext(0x90),
        "scroll_lock" => vk(0x91),
        "print" | "printscreen" | "print_screen" | "sys_req" => ext(0x2C),
        "pause" | "break" => vk(0x13),
        "menu" | "apps" => ext(0x5D),
        // Punctuation names
        "minus" => vk(0xBD),
        "plus" | "equal" | "equals" => vk(0xBB),
        "comma" => vk(0xBC),
        "period" | "dot" => vk(0xBE),
        "slash" => vk(0xBF),
        "backslash" => vk(0xDC),
        "semicolon" => vk(0xBA),
        "apostrophe" | "quote" => vk(0xDE),
        "bracketleft" => vk(0xDB),
        "bracketright" => vk(0xDD),
        "grave" | "backtick" => vk(0xC0),
        // Media
        "xf86audiomute" | "volumemute" => ext(0xAD),
        "xf86audiolowervolume" | "volumedown" => ext(0xAE),
        "xf86audioraisevolume" | "volumeup" => ext(0xAF),
        "xf86audionext" | "medianext" => ext(0xB0),
        "xf86audioprev" | "mediaprev" => ext(0xB1),
        "xf86audiostop" | "mediastop" => ext(0xB2),
        "xf86audioplay" | "xf86audiopause" | "mediaplaypause" => ext(0xB3),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    // F1..F24
    if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u16>().ok())
        && (1..=24).contains(&n)
    {
        return vk(0x70 + n - 1);
    }
    // Keypad digits
    if let Some(d) = lower
        .strip_prefix("kp_")
        .and_then(|d| d.parse::<u16>().ok())
        && d <= 9
    {
        return vk(0x60 + d);
    }
    // Single character: letters and digits map directly; other characters through the
    // current keyboard layout.
    let mut chars = name.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return None;
    };
    if c.is_ascii_alphabetic() {
        return vk(c.to_ascii_uppercase() as u16);
    }
    if c.is_ascii_digit() {
        return vk(c as u16);
    }
    let mut buf = [0_u16; 2];
    if c.encode_utf16(&mut buf).len() != 1 {
        return None;
    }
    // SAFETY: VkKeyScanW only reads the given character and the current keyboard layout.
    let scan = unsafe { VkKeyScanW(buf[0]) };
    if scan == -1 {
        return None;
    }
    vk(u16::from((scan & 0xFF) as u8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_names_resolve() {
        assert_eq!(resolve("Return").unwrap().code, 0x0D);
        assert_eq!(resolve("ctrl").unwrap().code, 0xA2);
        assert!(resolve("Page_Down").unwrap().extended);
        assert_eq!(resolve("F5").unwrap().code, 0x74);
        assert_eq!(resolve("F24").unwrap().code, 0x87);
        assert_eq!(resolve("a").unwrap().code, u16::from(b'A'));
        assert_eq!(resolve("7").unwrap().code, u16::from(b'7'));
        assert_eq!(resolve("KP_5").unwrap().code, 0x65);
        assert!(resolve("super").unwrap().extended);
        assert!(resolve("no_such_key").is_none());
        assert!(resolve("F25").is_none());
    }
}
