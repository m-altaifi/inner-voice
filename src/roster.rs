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

/// Cues this module will accept from a *stranger*, as word sequences.
///
/// A deliberate subset of `INTRO_BEFORE`. The roster path can afford "it's",
/// because whatever follows still has to be a name someone wrote down; with no
/// list to check against, "it's Tuesday" and "it's Chrome" would both enrol a
/// person. What is left is the phrasing that is only ever followed by a name.
const OPENERS: &[&[&str]] = &[
    &["i'm"],
    &["im"],
    &["i", "am"],
    &["this", "is"],
    &["my", "name", "is"],
    &["my", "name's"],
];

/// Capitalised words that open sentences, answer questions, or name days — the
/// false positives an unlisted name has no roster to be checked against.
/// Everything here would otherwise be enrolled as a person by "I'm Sorry" or
/// "This is Great".
const NOT_A_NAME: &[&str] = &[
    "a", "and", "but", "so", "the", "then", "there", "this", "that", "these", "those", "here",
    "just", "not", "no", "yes", "yeah", "yep", "nope", "ok", "okay", "right", "sure", "sorry",
    "good", "great", "fine", "well", "hi", "hello", "hey", "thanks", "thank", "please", "we",
    "you", "i", "he", "she", "it", "they", "my", "your", "our", "his", "her", "their", "monday",
    "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday", "january", "february",
    "march", "april", "may", "june", "july", "august", "september", "october", "november",
    "december", "today", "tomorrow", "yesterday", "morning", "afternoon", "evening", "everyone",
    "everybody", "all", "both", "one", "two", "actually", "basically", "still", "about",
];

/// A name someone spoke about themselves, whether or not anybody listed it.
///
/// This is what makes the far end nameable when it is different people every
/// time, which is the normal case — `attendees.csv` only ever worked for a
/// standing meeting. It is deliberately the *weakest-evidence* path in this
/// module and the narrowest: a self-introduction only, never being addressed.
/// "Ahmed, can you take this?" is far more common in a call than "I'm Ahmed",
/// and is exactly the sentence that would enrol a person who is not on the call
/// at all. `Roster::addressed` still handles that case, because there a name
/// has to appear on a list a human wrote.
///
/// Capitalisation is the only signal separating "I'm Ahmed" from "I'm sorry",
/// so it is required — and taken from the original text, never from `lower`.
///
/// ponytail: word-sequence scan over `split_whitespace`, no byte offsets. The
/// existing `hits` path indexes into the lowercased string, which is safe only
/// because those offsets never touch the original; capitalisation has to read
/// the original, so this walks words instead of risking the two drifting.
pub fn introduced(text: &str) -> Option<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let low: Vec<String> = words.iter().map(|w| bare(w).to_lowercase()).collect();

    for (i, _) in words.iter().enumerate() {
        // "Ahmed here", "Ahmed speaking" — the mirror image, and as safe as the
        // openers because the cue follows rather than precedes the name.
        if i > 0 && INTRO_AFTER.contains(&low[i].as_str())
            && let Some(name) = name_at(&words, &low, i.saturating_sub(1))
        {
            return Some(name);
        }
        for opener in OPENERS {
            // Not `starts_with`: `low` is `Vec<String>` and the openers are
            // `&[&str]`, which are different enough types to need the compare
            // spelled out rather than an allocation per word per cue.
            if i + opener.len() <= low.len()
                && opener.iter().zip(&low[i..]).all(|(a, b)| *a == b.as_str())
                && let Some(name) = name_at(&words, &low, i + opener.len())
            {
                return Some(name);
            }
        }
    }
    None
}

/// One or two capitalised words at `at`, as a name. Two only when both look
/// like one: "I'm Ada Lovelace" is a full name, "I'm Ada and this is" is not.
fn name_at(words: &[&str], low: &[String], at: usize) -> Option<String> {
    let one = |i: usize| -> Option<&str> {
        let word = bare(words.get(i)?);
        // A possessive is about somebody else: "I'm Ahmed's manager" names the
        // manager, not Ahmed, and "this is Monday's number" names nobody. The
        // roster path guards the same case with `possessive`; here the name is
        // not on a list, so the apostrophe is the only warning there is.
        if word.to_lowercase().ends_with("'s") {
            return None;
        }
        let capitalised = word.chars().next()?.is_uppercase();
        // Interior hyphens and apostrophes are part of the name -- Anne-Marie,
        // O'Brien. `bare` has already taken any off the ends, so what is left
        // cannot be punctuation pretending to be a word.
        let plausible = word.chars().count() >= 2
            && word.chars().all(|c| c.is_alphabetic() || "-'".contains(c))
            && !NOT_A_NAME.contains(&low.get(i)?.as_str());
        (capitalised && plausible).then_some(word)
    };
    let first = one(at)?;
    // "I'm Priya, Marcus asked me to join" is one person's name followed by a
    // different person's, not a two-word name -- and it named the speaker
    // "Priya Marcus". `bare` strips the comma before `one` ever sees it, so the
    // boundary can only be read off the raw word. Any clause-closing mark ends
    // the name: a surname never follows one.
    let closed = words
        .get(at)
        .is_some_and(|w| w.trim().ends_with([',', '.', ';', ':', '!', '?']));
    Some(match (!closed).then(|| one(at + 1)).flatten() {
        Some(second) => format!("{first} {second}"),
        None => first.to_string(),
    })
}

/// A word without the punctuation around it. Interior marks stay, so "name's"
/// survives as a cue and "Anne-Marie" survives as a name.
fn bare(word: &str) -> &str {
    word.trim_matches(|c: char| !c.is_alphanumeric())
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
    /// `introduced` is the path that works with no list at all, so it has no
    /// roster to check a name against — capitalisation and the cue are the only
    /// evidence there is. These are the cases that decide whether the far end
    /// gets named or gets libelled.
    mod introduced {
        use super::super::introduced;

        #[test]
        fn a_stranger_naming_themselves_is_learned() {
            for (said, expected) in [
                ("Hi, I'm Ahmed.", "Ahmed"),
                ("I'm Ada Lovelace, platform lead", "Ada Lovelace"),
                ("I am Grace", "Grace"),
                ("this is Ahmed from the Kinshasa office", "Ahmed"),
                ("My name is Ada", "Ada"),
                ("my name's Grace Hopper", "Grace Hopper"),
                ("Ahmed here", "Ahmed"),
                ("Grace Hopper speaking", "Hopper"),
            ] {
                assert_eq!(introduced(said).as_deref(), Some(expected), "{said:?}");
            }
        }

        /// The whole risk of naming without a list. Every one of these is a
        /// sentence a real call contains, and each would enrol a person.
        #[test]
        fn ordinary_speech_is_never_a_name() {
            for said in [
                "I'm sorry, could you repeat that?",
                "I'm not sure about the migration",
                "I'm good thanks",
                "This is great",
                "this is the part I wanted to ask about",
                "I am here",
                "I'm Sorry",
                "I'm ok",
                "This is Monday's number",
                "I'm Ahmed's manager",
                "this is Ada's call",
                "I'm A",
                "it's Ahmed",
                "so I'm on the call with the vendor",
            ] {
                assert_eq!(introduced(said), None, "{said:?}");
            }
        }

        /// "it's" is the cue the roster path can afford and this one cannot:
        /// with a list, whatever follows still has to be a name someone wrote
        /// down; without one, "it's Chrome" enrols a browser.
        #[test]
        fn the_riskier_roster_cues_are_not_borrowed() {
            assert_eq!(introduced("it's Tuesday"), None);
            assert_eq!(introduced("It's Chrome again"), None);
        }

        /// Two words only when both look like a name. Whisper writes a stream
        /// of clauses with no punctuation, so "I'm Ada and Grace is on mute"
        /// must not become one person called "Ada And".
        #[test]
        fn a_second_word_joins_only_when_it_is_one() {
            assert_eq!(introduced("I'm Ada and Grace is on mute").as_deref(), Some("Ada"));
            assert_eq!(introduced("I'm Ada, the platform lead").as_deref(), Some("Ada"));
            assert_eq!(introduced("I'm Anne-Marie").as_deref(), Some("Anne-Marie"));
            // A comma ends the name. "I'm Ada, the platform lead" above only
            // passed because "the" is in NOT_A_NAME -- a *capitalised* word
            // after the comma was never tried, and ran straight on into the
            // name, enrolling one person under two people's names.
            assert_eq!(
                introduced("Actually, I'm Priya, Marcus asked me to join").as_deref(),
                Some("Priya")
            );
            assert_eq!(introduced("I'm Drew, Sarah's colleague").as_deref(), Some("Drew"));
            // Still one name when nothing separates the two words.
            assert_eq!(introduced("I'm Ada Lovelace").as_deref(), Some("Ada Lovelace"));
        }

        /// The cue has to be a word. "Trim" ends in "im" and "him is" contains
        /// neither cue as a word, but a substring scan would find both.
        #[test]
        fn a_cue_inside_a_word_is_not_a_cue() {
            assert_eq!(introduced("Trim Ahmed's list"), None);
            assert_eq!(introduced("Whim Ahmed"), None);
        }
    }

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
        assert!(!r.names().is_empty());
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
        assert!(r.names().is_empty());
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
