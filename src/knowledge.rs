//! The source of truth you teach it before a call.
//!
//! Everything in `knowledge/` is loaded once at startup and pinned into the
//! system prompt. No retrieval, no chunking, no embeddings — a curated corpus
//! that is already in context cannot fail to be recalled, and retrieval that
//! never runs cannot miss. That is the whole point when the goal is recall.
//!
//! ponytail: in-context corpus, capped at MAX_CHARS. Move to real retrieval
//! only when a real corpus exceeds it — chunking a 20-page brief is theatre.

use anyhow::{Context, Result};
use std::fmt::Write as _;
use std::path::Path;

/// Refuse to pin more than this. Well past any hand-curated brief, well inside
/// a 1M window. A corpus this big means the design assumption broke, not that
/// the limit should be raised.
const MAX_CHARS: usize = 400_000;

/// Load `knowledge/` into one block for the system prompt.
/// Missing folder is not an error — the coach just runs without a corpus.
pub fn load(dir: &Path) -> Result<String> {
    if !dir.is_dir() {
        return Ok(String::new());
    }
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .filter(|p| p.is_file())
        .collect();
    files.sort(); // stable order keeps the cached prompt prefix stable

    let mut out = String::new();
    for path in &files {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if !crate::extract::supported(path) {
            continue;
        }
        let body =
            crate::extract::text(path).with_context(|| format!("reading {}", path.display()))?;
        if body.trim().is_empty() {
            continue;
        }
        let _ = write!(out, "\n## {name}\n\n{}\n", body.trim());
        if out.len() > MAX_CHARS {
            let mut end = MAX_CHARS;
            while !out.is_char_boundary(end) {
                end -= 1;
            }
            out.truncate(end);
            out.push_str("\n\n[knowledge truncated]\n");
            break;
        }
    }
    Ok(out)
}

/// The folder as a value that knows when it last read itself.
///
/// On-disk is the source of truth (LEDGER Decision 9): editing a brief
/// mid-call must reach the next advice. No watcher — `route()` sees every
/// turn single-threaded and asks once per coach request, which is exactly
/// the granularity that matters, and `newest` is a stat of a few dozen files.
pub struct Corpus {
    dir: std::path::PathBuf,
    newest: Option<std::time::SystemTime>,
    pub text: String,
    files: usize,
}

impl Corpus {
    pub fn load(dir: &Path) -> Result<Self> {
        let text = load(dir)?;
        Ok(Self {
            dir: dir.to_path_buf(),
            newest: newest(dir),
            files: count(dir),
            text,
        })
    }
    /// Re-read if anything in the folder changed since the last read.
    pub fn refresh(&mut self) -> Result<bool> {
        let now = newest(&self.dir);
        if now == self.newest {
            return Ok(false);
        }
        // Recorded before the read, so a file that stays locked or half-written
        // is reported once per change and not once per turn; finishing the save
        // moves the mtime again and the next turn re-reads.
        self.newest = now;
        self.text = load(&self.dir)?;
        self.files = count(&self.dir);
        Ok(true)
    }
    pub fn files(&self) -> usize {
        self.files
    }
}

/// Latest mtime of the folder itself or any file in it. The folder's own
/// mtime moves when a file is added or removed, which a per-file max misses.
pub fn newest(dir: &Path) -> Option<std::time::SystemTime> {
    let own = dir.metadata().and_then(|m| m.modified()).ok();
    let files = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok()?.metadata().ok()?.modified().ok())
        .max();
    own.max(files)
}

fn count(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|d| {
            d.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_file() && crate::extract::supported(p))
                .count()
        })
        .unwrap_or(0)
}

/// ~200 tokens. Whisper's prompt window is `n_text_ctx / 2`, about 224 tokens,
/// and overrunning it drops the front of the glossary rather than erroring.
const PROMPT_CHARS: usize = 800;

/// Capitalised by position, not because they are names.
const SKIP: &[&str] = &[
    "The", "A", "An", "This", "That", "These", "Those", "It", "Its", "We", "Our", "They", "Their",
    "You", "Your", "If", "In", "On", "At", "For", "And", "But", "Or", "No", "Not", "Do", "Does",
    "Never", "Prefer", "Treat", "Source", "Ship", "Product", "Stack", "Learn", "Once", "Track",
    "Use", "Every", "Each", "When", "Where", "What", "Who", "Why", "How", "Nothing", "Highest",
];

/// Terms to prime whisper's decoder with, drawn from the taught corpus.
///
/// Whisper is a language model with an ear attached: a decoder that has already
/// seen "Sarah Chen" is far likelier to return it than "Sarah Chan", and one
/// that has seen nothing is free to emit a confident non-word. The corpus
/// itself is far too big for the prompt window — it goes to the coach's system
/// prompt instead — so this is a glossary of the proper nouns and jargon in it,
/// which is the part transcription actually gets wrong.
///
/// Runs of capitalised words are kept together so "Sarah Chen" primes as a name
/// rather than as two unrelated tokens.
pub fn glossary(corpus: &str) -> String {
    let is_term = |w: &str| {
        let mut c = w.chars();
        let first = c.next().is_some_and(char::is_uppercase);
        // Interior capitals catch acronyms and CamelCase (EMEA, WASAPI, CUDA).
        (first || c.any(char::is_uppercase)) && w.len() >= 2 && !SKIP.contains(&w)
    };

    let mut terms: Vec<String> = Vec::new();
    for line in corpus.lines() {
        let mut run: Vec<&str> = Vec::new();
        let flush = |run: &mut Vec<&str>, terms: &mut Vec<String>| {
            if !run.is_empty() {
                let term = run.join(" ");
                if !terms.contains(&term) {
                    terms.push(term);
                }
                run.clear();
            }
        };
        for word in line.split(|c: char| c.is_whitespace() || ",|()\"“”:;".contains(c)) {
            let w = word.trim_matches(|c: char| !c.is_alphanumeric());
            if is_term(w) {
                run.push(w);
            } else {
                flush(&mut run, &mut terms);
            }
        }
        flush(&mut run, &mut terms);
    }

    let mut out = String::new();
    for t in terms {
        if out.len() + t.len() + 2 > PROMPT_CHARS {
            break;
        }
        if !out.is_empty() {
            out.push_str(", ");
        }
        out.push_str(&t);
    }
    // A null byte would panic whisper-rs's CString conversion mid-call, and the
    // corpus is whatever the user dropped in the folder.
    out.retain(|c| c != '\0');
    if out.is_empty() {
        out
    } else {
        format!("Glossary: {out}.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_becomes_key_value_lines() {
        let dir = std::env::temp_dir().join("iv_knowledge_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("facts.csv"),
            "headcount,42\nregion,EMEA\nsla,99.9,uptime\nowner,\"Smith, John\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("brief.md"), "# Brief\nShip by Q3.").unwrap();

        let text = load(&dir).unwrap();
        assert!(text.contains("headcount: 42"), "key/value pair: {text}");
        assert!(
            text.contains("sla: 99.9 | uptime"),
            "wide row joins: {text}"
        );
        assert!(
            text.contains("owner: Smith, John"),
            "quoted comma stays one field: {text}"
        );
        assert!(text.contains("Ship by Q3"), "markdown included: {text}");
        assert!(
            text.find("brief.md") < text.find("facts.csv"),
            "sorted order keeps the cache prefix stable"
        );
    }

    #[test]
    fn glossary_keeps_names_and_jargon_and_drops_sentence_openers() {
        let g = glossary(
            "## attendees.csv\n\nSarah Chen: Platform Lead | owns the migration\n\
             Marcus Webb: CTO | ex-Stripe\n\
             The region is EMEA and the stack is Rust on WASAPI.\n",
        );
        assert!(g.starts_with("Glossary: "), "{g}");
        // Adjacent capitals stay one name — the thing that actually gets mangled.
        assert!(g.contains("Sarah Chen"), "{g}");
        assert!(g.contains("Marcus Webb"), "{g}");
        // Acronyms and CamelCase survive without a leading capital rule.
        for term in ["CTO", "Stripe", "EMEA", "Rust", "WASAPI"] {
            assert!(g.contains(term), "missing {term}: {g}");
        }
        // "The" opens a sentence; it is not a name.
        assert!(!g.contains("The"), "{g}");
    }

    #[test]
    fn glossary_stays_inside_whispers_prompt_window() {
        // 500 distinct names is well past the window; it must truncate, not
        // overrun, because whisper drops the front of an oversized prompt.
        let corpus: String = (0..500).map(|i| format!("Name{i} Surname{i}\n")).collect();
        let g = glossary(&corpus);
        assert!(g.len() <= PROMPT_CHARS + 12, "got {} chars", g.len());
        assert!(g.contains("Name0 Surname0"), "keeps the earliest terms");
    }

    #[test]
    fn empty_corpus_primes_nothing() {
        assert_eq!(glossary(""), "");
        assert_eq!(glossary("all lowercase, no names here\n"), "");
    }

    #[test]
    fn missing_folder_is_not_an_error() {
        assert_eq!(load(Path::new("does-not-exist")).unwrap(), "");
    }

    #[test]
    fn corpus_reloads_only_when_the_folder_changed() {
        let dir = std::env::temp_dir().join(format!("iv_corpus_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("facts.md"), "Owner is Sarah").unwrap();
        let mut corpus = Corpus::load(&dir).unwrap();
        assert!(corpus.text.contains("Sarah"));
        assert!(!corpus.refresh().unwrap(), "nothing changed");
        std::thread::sleep(std::time::Duration::from_millis(1_100));
        std::fs::write(dir.join("facts.md"), "Owner is Marcus").unwrap();
        assert!(corpus.refresh().unwrap(), "an edit is noticed");
        assert!(corpus.text.contains("Marcus") && !corpus.text.contains("Sarah"));
        // Adding a file changes the directory's own mtime.
        std::thread::sleep(std::time::Duration::from_millis(1_100));
        std::fs::write(dir.join("more.md"), "Region is EMEA").unwrap();
        assert!(corpus.refresh().unwrap());
        assert_eq!(corpus.files(), 2);
        assert!(corpus.text.contains("EMEA"));
    }

    #[test]
    fn corpus_limit_preserves_utf8_boundaries() {
        let dir = std::env::temp_dir().join(format!("iv_unicode_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("brief.md"), "界".repeat(140_000)).unwrap();
        let text = load(&dir).unwrap();
        assert!(text.ends_with("[knowledge truncated]\n"));
        assert!(text.len() <= MAX_CHARS + 30);
        std::fs::remove_file(dir.join("brief.md")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
