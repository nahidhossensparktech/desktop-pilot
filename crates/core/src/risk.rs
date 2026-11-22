//! Deterministic risk rules for "ask only for risky actions" mode (SECURITY §4.2,
//! ADR-009). They look at the action, what it types or presses, and the title of the
//! focused window. Nothing the model or the screen says can switch them off.
//!
//! Limitation (until the accessibility tree arrives, M4): we cannot see the name of
//! the button under a click, so clicks are judged by the window they happen in.

use serde_json::Value;

/// Windows where any click, key or typing needs approval.
const SENSITIVE_WINDOWS: &[(&str, &str)] = &[
    ("user account control", "a Windows permission (UAC) prompt"),
    ("windows security", "Windows Security"),
    ("credential", "a credentials dialog"),
    ("password", "a password dialog"),
    ("sign in", "a sign-in page"),
    ("log in", "a sign-in page"),
    ("login", "a sign-in page"),
    ("setup", "an installer"),
    ("install", "an installer"),
    ("uninstall", "an uninstaller"),
    ("wizard", "a setup wizard"),
    ("microsoft store", "the Microsoft Store"),
    ("control panel", "Control Panel"),
    ("registry editor", "the Registry Editor"),
    ("regedit", "the Registry Editor"),
    ("group policy", "Group Policy"),
    ("device manager", "Device Manager"),
    ("disk management", "Disk Management"),
    ("services", "Windows Services"),
    ("task scheduler", "Task Scheduler"),
    ("command prompt", "a terminal"),
    ("cmd.exe", "a terminal"),
    ("powershell", "a terminal"),
    ("windows terminal", "a terminal"),
    ("terminal", "a terminal"),
    ("bash", "a terminal"),
    ("payment", "a payment page"),
    ("checkout", "a checkout page"),
    ("billing", "a billing page"),
    ("bank", "a banking page"),
    ("paypal", "a payment page"),
];

/// Windows of messaging apps: pressing Enter there usually sends something.
const MESSAGING_WINDOWS: &[&str] = &[
    "gmail",
    "outlook",
    "mail",
    "inbox",
    "whatsapp",
    "telegram",
    "messenger",
    "slack",
    "teams",
    "discord",
    "signal",
    "skype",
    "linkedin",
    "facebook",
    "twitter",
    " x ",
    "instagram",
    "chatgpt",
    "message",
];

/// Typed text that looks like a command that can change or destroy things.
const DANGEROUS_TEXT: &[&str] = &[
    "rm -rf",
    "rm -r ",
    "del ",
    "erase ",
    "rmdir",
    "rd /s",
    "format ",
    "remove-item",
    "diskpart",
    "reg delete",
    "reg add",
    "bcdedit",
    "shutdown",
    "net user",
    "icacls",
    "takeown",
    "set-executionpolicy",
    "invoke-webrequest",
    "iwr ",
    "curl ",
    "wget ",
    "powershell",
    "cmd /c",
    "msiexec",
    "winget install",
    "choco install",
    "pip install",
    "npm install",
    "sudo ",
];

fn is_settings_title(title: &str) -> bool {
    // The Windows Settings app is titled exactly "Settings".
    title.trim() == "settings" || title.starts_with("settings -") || title.ends_with("- settings")
}

fn text_arg(input: &Value) -> &str {
    input
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// Looks like a password, API key, token or card number.
pub fn looks_secret(text: &str) -> bool {
    let t = text.trim();
    let lower = t.to_ascii_lowercase();
    if [
        "sk-ant-",
        "sk-",
        "ghp_",
        "gho_",
        "xoxb-",
        "xoxp-",
        "akia",
        "-----begin",
    ]
    .iter()
    .any(|p| lower.starts_with(p))
    {
        return true;
    }
    // Card number: 13–19 digits (spaces/dashes allowed) passing the Luhn check.
    let digits: Vec<u32> = t.chars().filter_map(|c| c.to_digit(10)).collect();
    let only_digits = t
        .chars()
        .all(|c| c.is_ascii_digit() || c == ' ' || c == '-');
    if only_digits && (13..=19).contains(&digits.len()) {
        let sum: u32 = digits
            .iter()
            .rev()
            .enumerate()
            .map(|(i, d)| {
                if i % 2 == 1 {
                    let x = d * 2;
                    if x > 9 { x - 9 } else { x }
                } else {
                    *d
                }
            })
            .sum();
        if sum.is_multiple_of(10) {
            return true;
        }
    }
    // High-entropy single token: mixed classes, no spaces, long.
    if t.len() >= 20 && !t.contains(' ') {
        let classes = [
            t.chars().any(|c| c.is_ascii_lowercase()),
            t.chars().any(|c| c.is_ascii_uppercase()),
            t.chars().any(|c| c.is_ascii_digit()),
            t.chars().any(|c| !c.is_ascii_alphanumeric()),
        ];
        if classes.iter().filter(|b| **b).count() >= 3 && !t.contains("://") && !t.contains('.') {
            return true;
        }
    }
    false
}

/// Returns why an action needs approval, or `None` if it may run without asking.
/// `window_title` is the title of the window that has focus right now.
pub fn assess(action: &str, input: &Value, window_title: &str) -> Option<String> {
    let read_only = matches!(
        action,
        "screenshot" | "zoom" | "cursor_position" | "wait" | "mouse_move" | "scroll"
    );
    if read_only {
        return None;
    }
    let title = window_title.to_ascii_lowercase();

    if is_settings_title(&title) {
        return Some("changes Windows Settings".to_owned());
    }
    if let Some((_, what)) = SENSITIVE_WINDOWS.iter().find(|(k, _)| title.contains(k)) {
        return Some(format!("acts in {what}"));
    }

    match action {
        "type" => {
            let text = text_arg(input);
            if looks_secret(text) {
                return Some(
                    "types something that looks like a password, key or card number".to_owned(),
                );
            }
            let lower = text.to_ascii_lowercase();
            if let Some(cmd) = DANGEROUS_TEXT.iter().find(|c| lower.contains(*c)) {
                return Some(format!("types a command (`{}`)", cmd.trim()));
            }
            if (text.ends_with('\n') || text.ends_with('\r'))
                && MESSAGING_WINDOWS.iter().any(|m| title.contains(m))
            {
                return Some("may send a message".to_owned());
            }
            None
        }
        "key" | "hold_key" => {
            let keys = text_arg(input).to_ascii_lowercase().replace(' ', "+");
            let parts: Vec<&str> = keys.split('+').collect();
            let has = |k: &str| parts.contains(&k);
            let win = has("super") || has("win") || has("windows") || has("cmd") || has("meta");
            if has("delete") || has("del") {
                return Some("deletes (Delete key)".to_owned());
            }
            if (has("ctrl") || has("control")) && has("alt") && (has("delete") || has("del")) {
                return Some("opens the security screen".to_owned());
            }
            if win && (has("r") || has("x")) {
                return Some("opens the Run / power-user menu".to_owned());
            }
            if win && has("l") {
                return Some("locks the computer".to_owned());
            }
            if has("alt") && has("f4") {
                return Some("closes a window or app".to_owned());
            }
            let enter = has("return") || has("enter") || has("kp_enter");
            if enter && MESSAGING_WINDOWS.iter().any(|m| title.contains(m)) {
                return Some("may send a message".to_owned());
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn harmless_actions_run_without_asking() {
        assert_eq!(
            assess(
                "left_click",
                &json!({"coordinate": [1, 2]}),
                "Untitled - Notepad"
            ),
            None
        );
        assert_eq!(
            assess(
                "type",
                &json!({"text": "hello world"}),
                "Untitled - Notepad"
            ),
            None
        );
        assert_eq!(
            assess("key", &json!({"text": "ctrl+s"}), "Untitled - Notepad"),
            None
        );
        assert_eq!(
            assess("key", &json!({"text": "super"}), "Program Manager"),
            None
        );
        assert_eq!(
            assess("screenshot", &json!({}), "User Account Control"),
            None
        );
        assert_eq!(
            assess(
                "type",
                &json!({"text": "https://example.com/some-long-path"}),
                "Google Chrome"
            ),
            None
        );
    }

    #[test]
    fn sensitive_windows_ask() {
        assert!(assess("left_click", &json!({}), "User Account Control").is_some());
        assert!(assess("left_click", &json!({}), "Settings").is_some());
        assert!(assess("left_click", &json!({}), "Python 3.13 Setup").is_some());
        assert!(assess("type", &json!({"text": "dir"}), "Windows PowerShell").is_some());
        assert!(
            assess(
                "left_click",
                &json!({}),
                "Checkout - Amazon - Google Chrome"
            )
            .is_some()
        );
    }

    #[test]
    fn risky_keys_and_text_ask() {
        assert!(
            assess(
                "key",
                &json!({"text": "Delete"}),
                "Downloads - File Explorer"
            )
            .is_some()
        );
        assert!(assess("key", &json!({"text": "shift+Delete"}), "File Explorer").is_some());
        assert!(assess("key", &json!({"text": "super+r"}), "Desktop").is_some());
        assert!(assess("key", &json!({"text": "alt+F4"}), "Notepad").is_some());
        assert!(
            assess(
                "key",
                &json!({"text": "Return"}),
                "Inbox - Gmail - Google Chrome"
            )
            .is_some()
        );
        assert!(assess("key", &json!({"text": "Return"}), "Untitled - Notepad").is_none());
        assert!(assess("type", &json!({"text": "rm -rf ~/x"}), "Notepad").is_some());
        assert!(assess("type", &json!({"text": "hi there\n"}), "WhatsApp").is_some());
    }

    #[test]
    fn secrets_are_detected() {
        assert!(looks_secret("sk-ant-api03-abcdef"));
        assert!(looks_secret("4111 1111 1111 1111")); // Luhn-valid test card
        assert!(!looks_secret("4111 1111 1111 1112"));
        assert!(looks_secret("Tr0ub4dor&3xyzQ!wE9pL"));
        assert!(!looks_secret("hello"));
        assert!(!looks_secret("open notepad and type hello"));
        assert!(assess("type", &json!({"text": "Tr0ub4dor&3xyzQ!wE9pL"}), "Notepad").is_some());
    }
}
