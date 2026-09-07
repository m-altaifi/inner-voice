//! Who the far end is, remembered between calls.
//!
//! `voiceid` clusters the voices on one call; this is where a cluster that
//! earned a name survives to the next one. Without it every call starts blank,
//! which is fine when the same three people dial in every morning and useless
//! when they are different people every time — the case that actually happens.
//!
//! **The book replaces `attendees.csv` as the naming path, it does not extend
//! it.** A roster is a list someone writes in advance; this is a list the call
//! writes for you. `attendees.csv` still pins its contents into the prompt as
//! briefing material and still supplies canonical *spellings*, but it is no
//! longer the only way a voice can acquire a name.
//!
//! Shared between two threads on purpose. The THEM whisper worker owns the
//! embeddings and route owns the names — CLAUDE.md is emphatic that naming must
//! stay in `route`, because "Ahmed, what do you think?" is spoken by YOU on the
//! other capture thread — so the one structure they both need sits behind a
//! mutex rather than either side owning it and the other asking. Contention is
//! one lock per utterance, seconds apart.
//!
//! ponytail: whole-file rewrite on every change, no index. A book is one entry
//! per person you have ever spoken to; at a thousand entries it is 2 MB and
//! still rewrites in milliseconds. Revisit if that stops being true.

use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// One voice: a unit-length CAM++ centroid, what it is called, and how many
/// utterances went into it. `name` is `None` for a cluster this call has heard
/// but nobody has named — those are real speakers and worth telling apart on
/// screen, they are just not worth keeping once the call ends.
#[derive(Clone, Debug, PartialEq)]
pub struct Person {
    pub name: Option<String>,
    pub centroid: Vec<f32>,
    pub turns: u32,
    /// Unix seconds when this voice was last heard, and the freshness half of
    /// `forget` -- `turns` is the importance half. Seconds rather than the
    /// millis `log` uses because the resolution that matters here is a day.
    pub last_seen: u64,
}

/// Unix seconds. A clock that has gone backwards past 1970 is a zero, which
/// `forget` reads as "ancient" -- the safe direction is to keep, and a person
/// with `turns` behind them is kept regardless.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// How long unwritten history is allowed to sit before a rewrite is worth it.
const FLUSH: std::time::Duration = std::time::Duration::from_secs(60);

/// Drop the people who are both stale **and** rarely heard.
///
/// Two factors OR-ed to *keep*, not one to evict, and the difference is the
/// whole design: a client spoken to twice a year is exactly what this book
/// exists for, so silence alone must never forget them. What earns eviction is
/// silence with nothing behind it -- a voice heard three times, eight months
/// ago, whose centroid still competes for every match on evidence nobody
/// remembers making.
///
/// A free function taking `now` rather than a method reading the clock, for the
/// same reason `assign` is split out of `VoiceId`: the code that can delete a
/// year-old record has to be testable without a file and without a wall clock.
///
/// `keep_days` of 0 turns forgetting off entirely.
pub fn forget(people: &mut Vec<Person>, now: u64, keep_days: u64, keep_turns: u32) -> usize {
    if keep_days == 0 {
        return 0;
    }
    let cutoff = now.saturating_sub(keep_days.saturating_mul(86_400));
    let before = people.len();
    people.retain(|p| p.last_seen >= cutoff || p.turns >= keep_turns);
    before - people.len()
}

pub struct Book {
    path: Option<PathBuf>,
    /// Something worth writing has happened since the last `save`.
    dirty: bool,
    /// When the file was last written. `turns` and `last_seen` move on every
    /// turn, but the book is 6 KB a person -- rewriting it per turn to record
    /// "still listening" is real IO for no new fact, so `stale` debounces.
    written: std::time::Instant,
    /// Loaded people first, in file order, then whoever this call turns up.
    /// The order is the *index* `Msg::Turn` carries, so it must not be sorted
    /// or compacted while a call is running.
    pub people: Vec<Person>,
}

impl Book {
    /// A missing or unreadable file is an empty book, not an error: not
    /// recognising anyone is a working call, and refusing to start over a
    /// corrupt cache would be the worse failure.
    pub fn load(path: Option<PathBuf>) -> Book {
        Book {
            people: path.as_deref().and_then(read).unwrap_or_default(),
            path,
            dirty: false,
            written: std::time::Instant::now(),
        }
    }

    /// Sweep the book. **Load time only.** `people`'s order *is* the index
    /// `Msg::Turn` carries (see the field), so compacting once a call is
    /// running would silently rename whoever is speaking mid-sentence.
    pub fn sweep(&mut self, keep_days: u64, keep_turns: u32) -> usize {
        let dropped = forget(&mut self.people, now(), keep_days, keep_turns);
        self.dirty |= dropped > 0;
        dropped
    }

    /// Note that a voice was just heard. `assign` bumps `turns` and is pure, so
    /// it has no clock; this is the other half of the same event.
    pub fn heard(&mut self, voice: usize) {
        let Some(p) = self.people.get_mut(voice) else {
            return;
        };
        p.last_seen = now();
        // Only named people are written, so an anonymous cluster moving is not
        // a reason to rewrite the file -- the rewrite would be byte-identical.
        self.dirty |= p.name.is_some();
    }

    /// Is there unwritten history worth a rewrite yet?
    ///
    /// The file used to be written only when a *name* changed, which froze
    /// `turns` and `last_seen` at binding time -- so the number meant to prove
    /// how well a voice is known recorded only the moment it was named, and
    /// `forget` would have evicted on it.
    pub fn stale(&self) -> bool {
        self.dirty && self.written.elapsed() >= FLUSH
    }

    /// Everyone with a name, as (index, name) — what `route` seeds its
    /// per-call bindings from and what the startup notice lists.
    pub fn named(&self) -> Vec<(usize, String)> {
        self.people
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.name.clone().map(|n| (i, n)))
            .collect()
    }

    /// Name a voice, keeping the higher-evidence spelling when one already
    /// exists. Returns whether anything changed, so the caller only writes the
    /// file when it has to.
    pub fn name_it(&mut self, voice: usize, name: &str) -> bool {
        match self.people.get_mut(voice) {
            Some(p) if p.name.as_deref() == Some(name) => false,
            Some(p) => {
                p.name = Some(name.to_string());
                true
            }
            None => false,
        }
    }

    /// Only named people are written. An anonymous cluster is a within-call
    /// convenience — it cannot be recognised again without a name to offer, so
    /// keeping it would grow the file with rows that can never match anything a
    /// user would notice.
    pub fn save(&mut self) -> Result<()> {
        self.dirty = false;
        self.written = std::time::Instant::now();
        let Some(path) = &self.path else {
            return Ok(());
        };
        let people: Vec<Value> = self
            .people
            .iter()
            .filter(|p| p.name.is_some())
            .map(|p| {
                json!({
                    "name": p.name,
                    "turns": p.turns,
                    "last_seen": p.last_seen,
                    "centroid": p.centroid,
                })
            })
            .collect();
        let body = serde_json::to_string(&json!({ "version": 1, "people": people }))?;
        // Write beside, then rename. The book is overwritten whole on every
        // new name, so a crash mid-write would otherwise lose every person
        // learned so far rather than the one being added.
        let tmp = path.with_extension("json.tmp");
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::write(&tmp, body).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }
}

/// A malformed entry is skipped, not fatal: the file is a cache this program
/// wrote, and one bad row must not cost every other person in it.
fn read(path: &Path) -> Option<Vec<Person>> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    Some(
        value
            .get("people")?
            .as_array()?
            .iter()
            .filter_map(|p| {
                let centroid: Vec<f32> = p
                    .get("centroid")?
                    .as_array()?
                    .iter()
                    .filter_map(|x| x.as_f64().map(|f| f as f32))
                    .collect();
                // A centroid that is not a direction matches everybody at 0 and
                // would sit in the book forever, never claiming a turn and
                // never being replaced.
                if centroid.is_empty() || !centroid.iter().all(|x| x.is_finite()) {
                    return None;
                }
                Some(Person {
                    name: Some(p.get("name")?.as_str()?.to_string()),
                    centroid,
                    turns: p.get("turns").and_then(Value::as_u64).unwrap_or(1) as u32,
                    // A book written before this field existed has everyone
                    // last heard *now*: the alternative is reading a missing
                    // date as 1970 and forgetting the whole file on the first
                    // run after an upgrade.
                    last_seen: p.get("last_seen").and_then(Value::as_u64).unwrap_or_else(now),
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("iv_people_{}_{name}.json", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    fn person(name: Option<&str>, x: f32) -> Person {
        Person {
            name: name.map(str::to_string),
            centroid: vec![x, 1.0 - x],
            turns: 3,
            last_seen: now(),
        }
    }

    const DAY: u64 = 86_400;

    /// `(turns, days since heard)` -> a person, so each case below reads as the
    /// two facts the rule is actually about.
    fn aged(turns: u32, days_ago: u64) -> Person {
        Person {
            name: Some(format!("{turns}t/{days_ago}d")),
            centroid: vec![1.0, 0.0],
            turns,
            last_seen: (100 * DAY).saturating_sub(days_ago * DAY),
        }
    }

    fn kept(people: &[Person]) -> Vec<String> {
        let mut v = people.to_vec();
        forget(&mut v, 100 * DAY, 30, 20);
        v.into_iter().filter_map(|p| p.name).collect()
    }

    /// The rule is "forget older, keep the very important", and both halves
    /// have to hold at once: age alone must not evict, and being well known
    /// must not preserve someone forever without also being heard... except
    /// that it must, because a twice-a-year client is the case the book exists
    /// for. Age is only ever *half* a reason.
    #[test]
    fn only_the_stale_and_barely_heard_are_forgotten() {
        let people = vec![
            aged(3, 400),  // quiet and long gone
            aged(99, 400), // long gone, but this voice is well known
            aged(3, 1),    // barely heard, but heard yesterday
            aged(99, 1),   // neither
        ];
        assert_eq!(
            kept(&people),
            vec!["99t/400d", "3t/1d", "99t/1d"],
            "only the voice with nothing behind it and nothing recent goes"
        );
    }

    /// The boundary, both sides, because an off-by-one here deletes a record.
    #[test]
    fn the_edges_of_both_thresholds_keep_rather_than_drop() {
        assert_eq!(kept(&[aged(20, 400)]), vec!["20t/400d"], "turns == keep");
        assert!(kept(&[aged(19, 400)]).is_empty(), "one turn short");
        assert_eq!(kept(&[aged(3, 30)]), vec!["3t/30d"], "heard on the cutoff");
        assert!(kept(&[aged(3, 31)]).is_empty(), "a day past it");
    }

    /// `--keep-days 0` is the off switch, and a book nobody is sweeping must
    /// come through untouched however old it is.
    #[test]
    fn forgetting_can_be_turned_off_entirely() {
        let mut v = vec![aged(1, 9_999)];
        assert_eq!(forget(&mut v, 100 * DAY, 0, 20), 0);
        assert_eq!(v.len(), 1);
    }

    /// A clock before 1970, or a `keep_days` big enough to overflow the
    /// multiply, must not wrap the cutoff into the future and empty the book.
    #[test]
    fn an_impossible_clock_keeps_everyone_rather_than_forgetting_them() {
        let mut v = vec![aged(1, 0), aged(1, 400)];
        assert_eq!(forget(&mut v, 0, 30, 20), 0, "now == 0 forgets nobody");
        assert_eq!(forget(&mut v, u64::MAX, u64::MAX, 20), 0, "no overflow");
        assert_eq!(v.len(), 2);
    }

    /// Rewriting 6 KB a person to record "still listening" is IO for no new
    /// fact, and an anonymous cluster is not even written -- the rewrite would
    /// be byte-identical.
    #[test]
    fn only_a_named_voice_is_worth_rewriting_the_file_for() {
        let mut book = Book::load(None);
        book.people.push(person(None, 0.5));
        book.heard(0);
        assert!(!book.dirty, "an anonymous cluster never reaches the file");
        assert!(book.people[0].last_seen > 0, "but it is still noted on screen");

        book.people.push(person(Some("Ada"), 0.5));
        book.heard(1);
        assert!(book.dirty);
        assert!(!book.stale(), "dirty is not yet worth a write");

        book.heard(9); // no such voice
        book.save().unwrap();
        assert!(!book.dirty, "saving is what clears it");
    }

    #[test]
    fn a_named_voice_survives_the_call_and_an_anonymous_one_does_not() {
        let path = tmp("roundtrip");
        let mut book = Book::load(Some(path.clone()));
        assert!(book.people.is_empty());

        book.people.push(person(Some("Ada Lovelace"), 0.8));
        book.people.push(person(None, 0.2));
        book.save().unwrap();

        let next = Book::load(Some(path.clone()));
        assert_eq!(next.people.len(), 1, "the anonymous cluster was kept");
        assert_eq!(next.named(), vec![(0, "Ada Lovelace".to_string())]);
        // Against the same expression, not a literal: 1.0 - 0.8f32 is not 0.2,
        // and this test is about the JSON round trip rather than about f32.
        assert_eq!(next.people[0].centroid, person(None, 0.8).centroid);
        assert_eq!(next.people[0].turns, 3);
        // Without this the sweep reads every upgraded book as ancient.
        assert_eq!(next.people[0].last_seen, book.people[0].last_seen);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_corrupt_file_is_an_empty_book_rather_than_a_dead_start() {
        let path = tmp("corrupt");
        std::fs::write(&path, "{ not json at all").unwrap();
        assert!(Book::load(Some(path.clone())).people.is_empty());

        // One unusable row must not cost the rest of the file.
        std::fs::write(
            &path,
            r#"{"people":[{"name":"Ada","centroid":[1.0,0.0]},
                          {"name":"Broken"},
                          {"centroid":[0.0,1.0]},
                          {"name":"Nan","centroid":[null,"x"]},
                          {"name":"Grace","centroid":[0.0,1.0],"turns":9}]}"#,
        )
        .unwrap();
        let book = Book::load(Some(path.clone()));
        assert_eq!(
            book.named(),
            vec![(0, "Ada".to_string()), (1, "Grace".to_string())]
        );
        // A missing count is one utterance, not zero: it was heard at least once.
        assert_eq!(book.people[0].turns, 1);
        assert_eq!(book.people[1].turns, 9);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn naming_reports_whether_the_file_needs_writing() {
        let mut book = Book::load(None);
        book.people.push(person(None, 0.5));
        assert!(book.name_it(0, "Ada"));
        assert!(!book.name_it(0, "Ada"), "same name, nothing to write");
        assert!(book.name_it(0, "Ada Lovelace"), "a better spelling wins");
        assert!(!book.name_it(7, "Nobody"), "no such voice");
        // No path: saving is a no-op, not a failure. `--people ''` turns the
        // whole feature off and must not error on every turn.
        book.save().unwrap();
    }
}
