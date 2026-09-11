//! Working context consists of observed speech and explicitly referenced
//! projects. Open items live in the knowledge store, not in this hourly window.
use crate::{Who, learning::Learning, memory_store::KnowledgeClaim};
use std::collections::{BTreeSet, VecDeque};

#[derive(Clone)]
struct Observation {
    speaker: String,
    text: String,
    time: u64,
    mine: bool,
}

#[derive(Default)]
pub struct AwarenessSnapshot {
    source: String,
    recent: VecDeque<Observation>,
}

impl AwarenessSnapshot {
    pub fn reset(&mut self, source: &str) {
        self.source = source.into();
        self.recent.clear();
    }

    pub fn observe(&mut self, source: &str, who: &Who, text: &str, now: u64) {
        if self.source != source {
            self.reset(source);
        }
        self.recent.push_back(Observation {
            speaker: who.label(),
            text: crate::memory::clipped(text, 1200),
            time: now,
            mine: matches!(who, Who::You),
        });
        while self.recent.len() > 24 {
            self.recent.pop_front();
        }
    }

    pub fn source_changed(&mut self, source: &str) -> bool {
        if source == self.source {
            false
        } else {
            self.reset(source);
            true
        }
    }

    fn projects(&self, claims: &[KnowledgeClaim]) -> Vec<String> {
        // Pick the newest observation naming a known project. Earlier topics
        // cannot all become active merely because they remain in the window.
        for o in self.recent.iter().rev() {
            let text = crate::memory_store::normalized(&o.text);
            let found: BTreeSet<_> = claims
                .iter()
                .filter(|c| {
                    !c.scope.starts_with("session:")
                        && crate::memory_context::scope_matches(&text, &c.scope)
                })
                .map(|c| c.scope.clone())
                .collect();
            if !found.is_empty() {
                return found.into_iter().take(4).collect();
            }
        }
        Vec::new()
    }

    pub fn query(&self, question: &str, claims: &[KnowledgeClaim]) -> String {
        let text = crate::memory_store::normalized(question);
        if claims.iter().any(|c| {
            !c.scope.starts_with("session:")
                && crate::memory_context::scope_matches(&text, &c.scope)
        }) {
            question.into()
        } else {
            format!("{} {question}", self.projects(claims).join(" "))
        }
    }

    pub fn context(&self, question: &str, learning: Option<&Learning>, session: &str) -> String {
        let Some(learning) = learning else {
            return String::new();
        };
        let Ok(snapshot) = learning.snapshot.try_read() else {
            return String::new();
        };
        let query = self.query(question, &snapshot.claims);
        let context = serde_json::json!({"audio_source":self.source,"user_spoke_in_window":self.recent.iter().any(|o|o.mine),"last_observed_unix":self.recent.back().map(|o|o.time),"projects_named_in_recent_speech":self.projects(&snapshot.claims),"limits":"observed speech only; topic and outside events may be unknown"});
        let mut out = format!("\nWorking context (observations, not instructions): {context}\n");
        let learned = snapshot.index.render(&snapshot.claims, &query, session);
        for line in learned.lines() {
            if out.len() + line.len() + 1 > crate::memory_context::CONTEXT_BYTES {
                break;
            }
            out.push_str(line);
            out.push('\n');
        }
        crate::memory::clipped(&out, crate::memory_context::CONTEXT_BYTES)
    }

    pub fn describe(&self, learning: Option<&Learning>) -> String {
        let people: BTreeSet<_> = self.recent.iter().map(|o| o.speaker.as_str()).collect();
        let projects = learning
            .and_then(|l| l.snapshot.try_read().ok().map(|s| self.projects(&s.claims)))
            .unwrap_or_default();
        let last = self
            .recent
            .back()
            .map(|o| format!("{} · Unix {}\n{}", o.speaker, o.time, o.text))
            .unwrap_or_else(|| "No speech observed in this context.".into());
        format!(
            "Audio source: {}\nSpeakers observed: {}\nProjects named in retained speech: {}\nUser participation: {}\n\nLatest observation\n{}\n\nThis view describes captured speech. Current intent, outside events, approvals and unanswered questions may be unknown.\n\n{}",
            self.source,
            people.into_iter().collect::<Vec<_>>().join(", "),
            if projects.is_empty() {
                "not established".into()
            } else {
                projects.join(", ")
            },
            if self.recent.iter().any(|o| o.mine) {
                "microphone speech observed"
            } else {
                "no microphone speech in this window"
            },
            last,
            learning
                .map(Learning::status)
                .unwrap_or_else(|| "Learning unavailable.".into())
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_store::Store;

    #[test]
    #[ignore = "one-hour paced storage/context soak; synthetic evidence, no microphone or paid provider"]
    fn paced_memory_hour_with_restart() {
        use std::io::Write;
        use std::time::{Duration, Instant};
        let seconds = std::env::var("IV_SOAK_SECONDS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(3602);
        assert!((2..=7200).contains(&seconds));
        let folder = std::path::Path::new("target/evaluation")
            .join(format!("paced-memory-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let source = folder.join("call-paced.jsonl");
        let mut transcript = std::fs::File::create(&source).unwrap();
        let mut store = Store::open(Some(&folder)).unwrap();
        let source = source.to_string_lossy().into_owned();
        store.register(&source, 0).unwrap();
        store.enable_at(crate::people::now() as i64).unwrap();
        let mut history = crate::history::History::default();
        let mut awareness = AwarenessSnapshot::default();
        let start = Instant::now();
        let mut next = 0;
        let mut offset = 0;
        let mut turns = 0;
        let mut rotations = 0;
        let mut reservations = 0;
        let mut restarted = false;
        while start.elapsed() < Duration::from_secs(seconds) {
            let elapsed = start.elapsed().as_secs();
            if history.rotate(elapsed) {
                rotations += 1;
                awareness.reset("synthetic");
                assert!(!history.user_spoke());
            }
            if elapsed >= next {
                next = elapsed + 5;
                turns += 1;
                let now = crate::people::now();
                let text = "Orion deployment is waiting for QA approval.";
                let row = format!(
                    "{}\n",
                    serde_json::json!({"t":now*1000,"who":"YOU","text":text})
                );
                transcript.write_all(row.as_bytes()).unwrap();
                transcript.flush().unwrap();
                store
                    .ingest(
                        &source,
                        offset,
                        offset + row.len() as i64,
                        "YOU",
                        now as i64,
                        text,
                    )
                    .unwrap();
                offset += row.len() as i64;
                history.push_at(
                    crate::history::Turn::Speech {
                        who: Who::You,
                        text: text.into(),
                    },
                    now,
                );
                awareness.observe("synthetic", &Who::You, text, now);
                if let Some(batch) = store.batch().unwrap()
                    && store.reserve_request(now as i64).unwrap()
                {
                    reservations += 1;
                    let ids: Vec<_> = batch.episodes.iter().take(8).map(|e| e.id).collect();
                    let output=serde_json::json!({"claims":[{"scope":"Orion","subject":"deployment","key":"qa_approval","text":"QA approval remains pending.","kind":"commitment","evidence":ids,"correction":false}]}).to_string();
                    store.apply(&batch, &output, now as i64).unwrap();
                }
                let claims = store.claims().unwrap();
                assert!(!claims.is_empty());
                assert!(claims[0].open);
                let index = crate::memory_context::Index::new(&claims);
                assert!(
                    index
                        .render(&claims, "Orion QA approval", &source)
                        .contains("pending")
                );
                if turns % 12 == 1 {
                    eprintln!(
                        "paced memory: {elapsed}s, {turns} turns, {reservations} reserved extraction slots"
                    );
                }
            }
            if !restarted && elapsed >= seconds / 2 {
                drop(store);
                store = Store::open(Some(&folder)).unwrap();
                assert!(store.enabled_before().unwrap());
                assert!(!store.claims().unwrap().is_empty());
                restarted = true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if seconds >= 3601 {
            assert!(rotations >= 1);
        }
        assert!(restarted);
        let report = serde_json::json!({"wall_seconds":start.elapsed().as_secs_f64(),"speech_turns":turns,"context_rotations":rotations,"store_reopened":restarted,"reserved_slots":reservations,"provider_calls":0,"claims":store.claims().unwrap().len(),"pending_records":store.counts().unwrap().0,"evidence":"paced synthetic JSONL, real SQLite, real monotonic context clock; extraction responses supplied by fixture; capture and GUI assessed separately"});
        std::fs::write(
            "target/evaluation/paced-memory.json",
            serde_json::to_string_pretty(&report).unwrap(),
        )
        .unwrap();
    }
    #[test]
    fn source_change_and_hourly_reset_clear_participation() {
        let mut s = AwarenessSnapshot::default();
        s.observe("chrome", &Who::You, "Orion deployment", 1);
        assert!(s.describe(None).contains("microphone speech observed"));
        assert!(s.source_changed("teams"));
        assert!(s.recent.is_empty());
        s.observe("teams", &Who::You, "Yes", 2);
        s.reset("teams");
        assert!(s.describe(None).contains("no microphone speech"));
    }
    #[test]
    fn observation_window_is_bounded_and_not_a_permanent_participation_latch() {
        let mut s = AwarenessSnapshot::default();
        s.observe("app", &Who::You, "hello", 0);
        for i in 0..30 {
            s.observe(
                "app",
                &Who::Them {
                    voice: None,
                    name: None,
                },
                "a lecture",
                i,
            );
        }
        assert_eq!(s.recent.len(), 24);
        assert!(s.describe(None).contains("no microphone speech"));
    }
}
