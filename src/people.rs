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
}

pub struct Book {
    path: Option<PathBuf>,
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
        }
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
    pub fn save(&self) -> Result<()> {
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
        }
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
