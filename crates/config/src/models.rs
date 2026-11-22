/// One selectable Claude model (a `[[models]]` entry in config, TRD §3.7).
#[derive(Clone, Debug, PartialEq)]
pub struct ModelEntry {
    pub id: String,
    pub display_name: String,
    pub default: bool,
    /// Effort sent explicitly with every request.
    pub effort: String,
    /// Show a cost warning in the model selector.
    pub cost_warning: bool,
    /// `thinking.display` to request, e.g. `"updates"` for short progress notes
    /// between tool calls (needs a beta header; not every model supports it).
    pub thinking_display: Option<String>,
    /// `max_tokens` per response (streaming, so large values are fine).
    pub max_tokens: u32,
}

impl ModelEntry {
    /// A model known only by id and name (e.g. listed live by Gemini or Antigravity).
    pub fn simple(id: &str, display_name: &str) -> Self {
        Self {
            thinking_display: None,
            ..Self::new(id, display_name, "medium")
        }
    }

    fn new(id: &str, display_name: &str, effort: &str) -> Self {
        Self {
            id: id.to_owned(),
            display_name: display_name.to_owned(),
            default: false,
            effort: effort.to_owned(),
            cost_warning: false,
            thinking_display: Some("updates".to_owned()),
            max_tokens: 32_000,
        }
    }
}

/// Built-in models (TRD §3.7). Sonnet 5.5 is the default (PRD §3 "Models").
pub fn default_models() -> Vec<ModelEntry> {
    vec![
        ModelEntry {
            default: true,
            ..ModelEntry::new("claude-sonnet-5-5", "Claude Sonnet 5.5", "medium")
        },
        ModelEntry::new("claude-opus-5-5", "Claude Opus 5.5", "medium"),
        ModelEntry {
            // Progress-update display is not documented for Haiku 5.5.
            thinking_display: None,
            ..ModelEntry::new("claude-haiku-5-5", "Claude Haiku 5.5", "medium")
        },
        ModelEntry {
            cost_warning: true,
            ..ModelEntry::new("claude-fable-5-2", "Claude Fable 5.2", "high")
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_one_default_and_it_is_sonnet() {
        let models = default_models();
        let defaults: Vec<_> = models.iter().filter(|m| m.default).collect();
        assert_eq!(defaults.len(), 1);
        assert_eq!(defaults[0].id, "claude-sonnet-5-5");
    }

    #[test]
    fn ids_are_unique() {
        let models = default_models();
        let mut ids: Vec<_> = models.iter().map(|m| m.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), models.len());
    }
}
