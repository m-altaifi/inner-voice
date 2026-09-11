//! Cheap interruption control. Judgment stays with the configured coach;
//! local rules only suppress exact filler/repetition and preserve short decisions.
pub fn silent(text: &str) -> bool {
    matches!(normalized(text).as_str(), "silent" | "note nothing needed")
}
fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub fn worth_asking(text: &str, previous: &str, min_words: usize) -> bool {
    let lower = normalized(text);
    if lower.is_empty() {
        return false;
    }
    let answer = lower.trim_matches(|c: char| !c.is_alphanumeric());
    let prior = previous.to_lowercase();
    let consequential = [
        "approv",
        "commit",
        "launch",
        "rollout",
        "sign ",
        "budget",
        "cancel",
        "deploy",
        "guarantee",
        "hire",
        "fire ",
    ]
    .iter()
    .any(|s| prior.contains(s));
    if consequential
        && [
            "yes", "no", "sure", "okay", "ok", "agreed", "approved", "stop", "go",
        ]
        .contains(&answer)
    {
        return true;
    }
    if [
        "let me share my screen",
        "can you hear me",
        "thank you everyone",
        "thanks everyone",
        "good morning everyone",
        "give me a moment",
    ]
    .contains(&answer)
    {
        return false;
    }
    text.split_whitespace().count() >= min_words && crate::worth_asking(text)
}

/// Withhold only a possible silence marker. Real advice still streams immediately.
#[derive(Default)]
pub struct Filter {
    pending: String,
    released: bool,
}
impl Filter {
    pub fn push(&mut self, text: &str) -> Option<String> {
        if self.released {
            return Some(text.to_string());
        }
        self.pending.push_str(text);
        let n = normalized(&self.pending);
        if ["silent", "note nothing needed"]
            .iter()
            .any(|s| s.starts_with(&n))
        {
            return None;
        }
        self.released = true;
        Some(std::mem::take(&mut self.pending))
    }
    pub fn finish(&mut self) -> Option<String> {
        if self.released || silent(&self.pending) || self.pending.trim().is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.pending))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_silence_markers_never_reach_the_panel() {
        for parts in [vec!["SI", "LE", "NT"], vec!["NOTE ", " nothing ", "needed"]] {
            let mut filter = Filter::default();
            for p in parts {
                assert_eq!(filter.push(p), None);
            }
            assert_eq!(filter.finish(), None);
        }
        let mut filter = Filter::default();
        assert_eq!(filter.push("NOTE "), None);
        assert_eq!(
            filter.push("Runway is six months."),
            Some("NOTE Runway is six months.".into())
        );
    }
    #[test]
    fn short_consequential_answers_survive_but_routine_chatter_does_not() {
        assert!(worth_asking("Sure.", "Approve the acquisition budget?", 3));
        assert!(worth_asking("No.", "Shall we deploy?", 3));
        assert!(!worth_asking("Sure.", "Good morning", 3));
        assert!(!worth_asking("Let me share my screen.", "", 3));
        assert!(worth_asking("Thanks, but the budget is wrong.", "", 3));
    }
}
