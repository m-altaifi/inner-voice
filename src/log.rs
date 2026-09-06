//! Append-only JSON Lines transcript, one file per call.
//!
//! Two readers, both real: the user reading the call back afterwards, and a
//! later pass tuning the speaker-ID threshold against the audio that produced
//! these lines. JSONL because a transcript only ever grows — an array would
//! have to be rewritten whole on every turn, and a half-written array is not
//! parseable while the call is still running.

use anyhow::{Context, Result};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

static SESSION: AtomicU64 = AtomicU64::new(0);

pub struct Log {
    path: PathBuf,
    file: Mutex<std::fs::File>,
}

fn millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl Log {
    pub fn new(dir: &Path) -> Result<Log> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        // Unix millis, not an ISO timestamp: `2026-09-05T14:32:11` cannot be a
        // Windows filename at all, and every readable alternative wants a date
        // crate for what is only ever a unique, sortable stem.
        let path = dir.join(format!(
            "call-{}-{}-{}.jsonl",
            millis(),
            std::process::id(),
            SESSION.fetch_add(1, Ordering::Relaxed)
        ));
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("creating {}", path.display()))?;
        Ok(Log {
            path,
            file: Mutex::new(file),
        })
    }

    /// Append one turn. `who` is whatever label the caller displays — this
    /// knows nothing about speakers.
    ///
    /// Failure is surfaced in the HUD without ending the call.
    pub fn append(&self, who: &str, text: &str) -> Result<()> {
        // Formatted before the lock, and written with one `write_all`: two
        // capture threads log concurrently, and a line split across writes is
        // how a transcript ends up with interleaved halves.
        let line = format!(
            "{}\n",
            serde_json::json!({ "t": millis(), "who": who, "text": text })
        );
        let mut f = self
            .file
            .lock()
            .map_err(|_| anyhow::anyhow!("transcript log lock poisoned"))?;
        f.write_all(line.as_bytes())
            .context("writing transcript log")
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn lines(log: &Log) -> Vec<serde_json::Value> {
        std::fs::read_to_string(log.path())
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("bad line {l:?}: {e}")))
            .collect()
    }

    #[test]
    fn one_line_per_turn() {
        let log = Log::new(&dir("iv_log_turns")).unwrap();
        log.append("YOU", "hello").unwrap();
        log.append("THEM", "hi there").unwrap();
        log.append("Sarah Chen", "ship by Q3").unwrap();

        let ls = lines(&log);
        assert_eq!(ls.len(), 3);
        assert_eq!(ls[1]["who"], "THEM");
        assert_eq!(ls[2]["text"], "ship by Q3");
        assert!(ls[0]["t"].as_u64().unwrap() > 1_700_000_000_000);
        assert!(log.path().extension().unwrap() == "jsonl");
    }

    #[test]
    fn newlines_and_quotes_stay_on_one_line() {
        let log = Log::new(&dir("iv_log_escapes")).unwrap();
        let nasty = "line one\nline \"two\"\ttab — café\r\n";
        log.append("YOU", nasty).unwrap();

        let ls = lines(&log);
        assert_eq!(
            ls.len(),
            1,
            "a newline in the text must not split the record"
        );
        assert_eq!(ls[0]["text"], nasty);
    }

    #[test]
    fn concurrent_appends_do_not_interleave() {
        let log = Log::new(&dir("iv_log_threads")).unwrap();
        let log = &log; // shared by reference; scope() is why there is no Arc
        std::thread::scope(|s| {
            for t in 0..8 {
                s.spawn(move || {
                    for i in 0..25 {
                        log.append("THEM", &format!("thread {t} turn {i}\nsecond \"line\""))
                            .unwrap();
                    }
                });
            }
        });
        assert_eq!(lines(log).len(), 200);
    }
}
