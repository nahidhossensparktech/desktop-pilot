//! Parsing of computer-use key strings ("ctrl+s", "alt+Tab", "ctrl+a Delete").

/// Splits a key string into combos pressed one after another; each combo is a list of
/// keys held together, e.g. `"ctrl+shift+t Return"` → `[[ctrl, shift, t], [Return]]`.
pub fn parse_key_sequence(text: &str) -> Result<Vec<Vec<String>>, String> {
    let mut combos = Vec::new();
    for token in text.split_whitespace() {
        combos.push(parse_combo(token)?);
    }
    if combos.is_empty() {
        return Err("empty key string".to_owned());
    }
    Ok(combos)
}

/// Parses one combo. A literal plus key can be written `plus`, `+`, or as a trailing
/// `++` (e.g. `ctrl++`).
pub fn parse_combo(token: &str) -> Result<Vec<String>, String> {
    if token == "+" {
        return Ok(vec!["plus".to_owned()]);
    }
    let (base, plus) = match token.strip_suffix("++") {
        Some(b) => (b, true),
        None => (token, false),
    };
    let mut keys: Vec<String> = Vec::new();
    for part in base.split('+') {
        if part.is_empty() {
            return Err(format!("malformed key combo `{token}`"));
        }
        keys.push(part.to_owned());
    }
    if plus {
        keys.push("plus".to_owned());
    }
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_combos_and_sequences() {
        assert_eq!(parse_key_sequence("Return").unwrap(), vec![vec!["Return"]]);
        assert_eq!(
            parse_key_sequence("ctrl+shift+t").unwrap(),
            vec![vec!["ctrl", "shift", "t"]]
        );
        assert_eq!(
            parse_key_sequence("ctrl+a Delete").unwrap(),
            vec![vec!["ctrl", "a"], vec!["Delete"]]
        );
        assert_eq!(
            parse_key_sequence("ctrl++").unwrap(),
            vec![vec!["ctrl", "plus"]]
        );
        assert_eq!(parse_key_sequence("+").unwrap(), vec![vec!["plus"]]);
        assert!(parse_key_sequence("ctrl+").is_err());
        assert!(parse_key_sequence("  ").is_err());
    }
}
