//! Who is on the other end, by name.
//!
//! The names come from `knowledge/attendees.csv` and nowhere else. The
//! transcript is only ever evidence *that* a roster name was said — the
//! spelling always comes back out of the file, because whisper writes "Sarah
//! Chan" and the roster is the source of truth. A name that is not on the list
//! is never returned at all: a wrong name is worse than no name.
//!
//! ponytail: plain substring scan, no regex crate and no fuzzy matching.
//! Mangled surnames are covered by matching on the first name instead; reach
//! for edit distance or phonetics only if a real call proves that too thin.

use std::path::Path;

/// Cues whose next words are the speaker's own name.
const INTRO_BEFORE: &[&str] = &[
    "i'm",
    "im",
    "i am",
    "this is",
    "it's",
    "my name is",
    "my name's",
];

/// The mirror image: "Sarah here", "Marcus speaking".
const INTRO_AFTER: &[&str] = &["here", "speaking"];

/// Punctuation that sets a name off from the sentence around it — the written
/// trace of the pause in "Marcus, your call". Without one of these a name is
/// being mentioned, not addressed. '.' is deliberately absent: "Ask Sarah. Then
/// we ship" is a mention.
const VOCATIVE: &str = ",-–—:;?!";

#[derive(Default)]
pub struct Roster {
    /// Canonical spellings, exactly as written in the file.
    names: Vec<String>,
    /// Lowercase needle → index into `names`. Full names, plus first names that
    /// point at exactly one attendee.
    keys: Vec<(String, usize)>,
}

impl Roster {
    /// Reads `<dir>/attendees.csv`. A missing file is the normal case for an
    /// unbriefed call, not an error — the coach just runs without names.
    pub fn load(dir: &Path) -> Roster {
        let mut r = Roster::default();
        let Ok(mut rdr) = csv::ReaderBuilder::new()
            .has_headers(false)
            .flexible(true)
            .from_path(dir.join("attendees.csv"))
        else {
            return r;
        };

        // Roles are not kept: knowledge::load already pins the whole file,
        // roles included, into the system prompt. This module only answers
        // "what is this voice called".
        for rec in rdr.records().flatten() {
            let name = rec.get(0).unwrap_or_default().trim();
            // has_headers(false) plus this skip, so a file written without the
            // header row keeps its first attendee instead of losing one.
            if name.is_empty() || name.eq_ignore_ascii_case("name") {
                continue;
            }
            if !r.names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
                r.names.push(name.to_string());
            }
        }

        for (i, n) in r.names.iter().enumerate() {
            r.keys.push((n.to_lowercase(), i));
        }
        // A first name earns a key only when one attendee answers to it. Two
        // Sarahs on the roster and "I'm Sarah" is a coin flip, so it stays
        // unmatched until the surname is heard.
        for i in 0..r.names.len() {
            let first = first_word(&r.names[i]);
            let taken = r.keys.iter().any(|(k, _)| *k == first)
                || (0..r.names.len()).any(|j| j != i && first_word(&r.names[j]) == first);
            if !first.is_empty() && !taken {
                r.keys.push((first, i));
            }
        }
        r
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Who the panel is prepared to call the far end, for the startup notice.
    ///
    /// `naming: on` used to be the whole message, which made a name arriving
    /// mid-call unattributable to anything the user could see — and the sample
    /// `attendees.csv` this project shipped meant the names were not even his.
    /// A roster is a small list by nature, so it is printed rather than counted.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Speaker naming themselves: "I'm Sarah", "Sarah here", "This is Marcus
    /// Webb", "my name is Sarah". Returns the roster's canonical spelling.
    pub fn self_intro(&self, text: &str) -> Option<&str> {
        let lower = text.to_lowercase();
        self.hits(&lower).into_iter().find_map(|(who, start, end)| {
            let claimed =
                !possessive(&lower, end) && (cue_before(&lower, start) || cue_after(&lower, end));
            claimed.then(|| self.names[who].as_str())
        })
    }

    /// Someone addressing another person by name: "Sarah, what do you think?",
    /// "over to you, Marcus", "Marcus - your call".
    pub fn addressed(&self, text: &str) -> Option<&str> {
        let lower = text.to_lowercase();
        let hits = self.hits(&lower);
        let who = hits.first()?.0;
        // Two attendees named in one breath: which one is being spoken to is a
        // guess, and guessing is the one thing this module must not do.
        if hits.iter().any(|&(other, _, _)| other != who) {
            return None;
        }
        hits.into_iter().find_map(|(_, start, end)| {
            if possessive(&lower, end) || cue_before(&lower, start) || cue_after(&lower, end) {
                return None; // naming themselves, not addressing anyone
            }
            let before = lower[..start].trim_end();
            let after = lower[end..].trim_start();
            let set_off = before.ends_with(|c| VOCATIVE.contains(c))
                || after.is_empty()
                || after.starts_with(|c| VOCATIVE.contains(c));
            set_off.then(|| self.names[who].as_str())
        })
    }

    /// Every roster name in `lower`, as (attendee, start, end) byte ranges into
    /// the lowercased text, earliest first. Offsets are only ever used against
    /// `lower`, never the original, so lowercasing changing byte lengths is
    /// harmless.
    fn hits(&self, lower: &str) -> Vec<(usize, usize, usize)> {
        let mut out = Vec::new();
        for (key, who) in &self.keys {
            let mut from = 0;
            while let Some(rel) = lower[from..].find(key.as_str()) {
                let (start, end) = (from + rel, from + rel + key.len());
                if boundary(lower[..start].chars().next_back())
                    && boundary(lower[end..].chars().next())
                {
                    out.push((*who, start, end));
                }
                from = end;
            }
        }
        out.sort_by_key(|&(_, start, _)| start);
        out
    }
}

fn first_word(name: &str) -> String {
    name.split_whitespace().next().unwrap_or("").to_lowercase()
}

/// Letters and digits bind, everything else separates — so "Marcus" does not
/// fire inside "Marcuses". An apostrophe separates too, on purpose: "Sarah's"
/// still finds "Sarah", which is then rejected below as a possessive rather
/// than silently missed here and mistaken for an address.
fn boundary(c: Option<char>) -> bool {
    !c.is_some_and(char::is_alphanumeric)
}

/// "Sarah's project" is about her, not from her. Curly apostrophe included —
/// whisper emits it.
fn possessive(lower: &str, end: usize) -> bool {
    lower[end..].starts_with(['\'', '\u{2019}'])
}

fn cue_before(lower: &str, start: usize) -> bool {
    let before = lower[..start].trim_end();
    INTRO_BEFORE.iter().any(|cue| {
        before.ends_with(cue) && boundary(before[..before.len() - cue.len()].chars().next_back())
    })
}

fn cue_after(lower: &str, end: usize) -> bool {
    let after = lower[end..].trim_start();
    INTRO_AFTER.iter().any(|cue| {
        after.starts_with(cue)
            // "Sarah here's the deal" is not "Sarah here".
            && !possessive(after, cue.len())
            && boundary(after[cue.len()..].chars().next())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Own temp dir per test — they run in parallel.
    fn roster(tag: &str, csv: &str) -> Roster {
        let dir = std::env::temp_dir().join(format!("iv_roster_{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("attendees.csv"), csv).unwrap();
        Roster::load(&dir)
    }

    fn two(tag: &str) -> Roster {
        roster(
            tag,
            "name,role,notes\n\
             Sarah Chen,Platform Lead,\"owns the migration, joined 2024\"\n\
             Marcus Webb,CTO,\"decision maker, ex-Stripe\"\n",
        )
    }

    #[test]
    fn self_intro_forms_hit_and_return_the_written_spelling() {
        let r = two("intro");
        assert!(!r.is_empty());
        for (line, want) in [
            ("I'm Sarah", "Sarah Chen"),
            ("im sarah", "Sarah Chen"),
            ("Sarah here", "Sarah Chen"),
            ("Yeah, Sarah Chen here.", "Sarah Chen"),
            ("my name is Sarah", "Sarah Chen"),
            ("This is Marcus Webb", "Marcus Webb"),
            ("it's Marcus", "Marcus Webb"),
            ("Marcus speaking", "Marcus Webb"),
            // The whole point: whisper's spelling loses to the file's.
            ("I'm Sarah Chan", "Sarah Chen"),
            ("this is marcus web", "Marcus Webb"),
        ] {
            assert_eq!(r.self_intro(line), Some(want), "{line}");
        }
    }

    #[test]
    fn self_intro_ignores_mere_mentions() {
        let r = two("mention");
        for line in [
            "I'm talking to Sarah",
            "this is Sarah's project",
            "it's Marcus's call, not mine",
            "Sarah, what do you think?",
            "we should ask Marcus",
        ] {
            assert_eq!(r.self_intro(line), None, "{line}");
        }
    }

    #[test]
    fn direct_address_forms_hit() {
        let r = two("address");
        for (line, want) in [
            ("Sarah, what do you think?", "Sarah Chen"),
            ("over to you, Marcus", "Marcus Webb"),
            ("Marcus - your call", "Marcus Webb"),
            ("So Marcus: where does that leave us?", "Marcus Webb"),
            ("what's your read, Sarah Chen?", "Sarah Chen"),
        ] {
            assert_eq!(r.addressed(line), Some(want), "{line}");
        }
        // Self-introduction is not an address.
        assert_eq!(r.addressed("I'm Sarah"), None);
        assert_eq!(r.addressed("Marcus here"), None);
    }

    #[test]
    fn a_name_not_on_the_roster_is_never_returned() {
        let r = two("unknown");
        for line in [
            "I'm Priya",
            "This is David Chen",
            "Priya, what do you think?",
            "over to you, Dave",
            "my name is Webb Marcusson",
        ] {
            assert_eq!(r.self_intro(line), None, "{line}");
            assert_eq!(r.addressed(line), None, "{line}");
        }
    }

    #[test]
    fn two_roster_names_in_one_line_is_ambiguous() {
        let r = two("ambiguous");
        assert_eq!(r.addressed("Sarah and Marcus, thoughts?"), None);
        assert_eq!(r.addressed("Marcus - can you loop in Sarah?"), None);
        // A shared first name is ambiguous too, so it never becomes a key.
        let s = roster("sarahs", "name,role\nSarah Chen,Lead\nSarah Kim,PM\n");
        assert_eq!(s.self_intro("I'm Sarah"), None);
        assert_eq!(s.self_intro("I'm Sarah Kim"), Some("Sarah Kim"));
    }

    #[test]
    fn names_match_whole_words_only() {
        let r = two("boundary");
        for line in [
            "Marcuses everywhere",
            "the Sarahs disagree",
            "unmarcus",
            "sarahchen",
        ] {
            assert_eq!(r.self_intro(line), None, "{line}");
            assert_eq!(r.addressed(line), None, "{line}");
        }
        assert_eq!(r.self_intro("I'm Marcuses"), None);
    }

    #[test]
    fn missing_attendees_csv_yields_an_empty_roster() {
        let r = Roster::load(Path::new("does-not-exist"));
        assert!(r.is_empty());
        assert_eq!(r.self_intro("I'm Sarah"), None);
        assert_eq!(r.addressed("Sarah, what do you think?"), None);
    }

    #[test]
    fn header_row_is_optional() {
        let r = roster("noheader", "Sarah Chen,Platform Lead\nMarcus Webb,CTO\n");
        assert_eq!(r.self_intro("I'm Sarah"), Some("Sarah Chen"));
        assert_eq!(r.self_intro("this is Marcus"), Some("Marcus Webb"));
    }
}
