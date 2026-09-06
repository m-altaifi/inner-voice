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
use calamine::{Data, Reader, open_workbook_auto};
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
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        let body = match ext.as_str() {
            "md" | "txt" => std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?,
            "xlsx" | "xlsm" | "xls" | "ods" => {
                sheet_to_text(path).with_context(|| format!("reading {}", path.display()))?
            }
            "csv" => csv_to_text(path).with_context(|| format!("reading {}", path.display()))?,
            _ => continue,
        };
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

/// Same flattening for CSV. Not an Excel format, so calamine cannot read it —
/// and a real parser matters here because a facts sheet will contain quoted
/// values with commas in them.
pub(crate) fn csv_to_text(path: &Path) -> Result<String> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_path(path)?;
    let mut out = String::new();
    for rec in rdr.records() {
        let cells: Vec<String> = rec?
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        match cells.len() {
            0 => {}
            1 => {
                let _ = writeln!(out, "{}", cells[0]);
            }
            _ => {
                let _ = writeln!(out, "{}: {}", cells[0], cells[1..].join(" | "));
            }
        }
    }
    Ok(out)
}

/// Flatten a spreadsheet to `key: value` lines.
///
/// Two columns is the common shape for a facts sheet. Wider rows keep column 1
/// as the key and join the rest, so a table with notes still reads sensibly.
pub(crate) fn sheet_to_text(path: &Path) -> Result<String> {
    let mut wb = open_workbook_auto(path)?;
    let mut out = String::new();
    for name in wb.sheet_names().to_vec() {
        let range = wb
            .worksheet_range(&name)
            .with_context(|| format!("reading worksheet {name:?}"))?;
        let mut rows = String::new();
        for row in range.rows() {
            let cells: Vec<String> = row
                .iter()
                .map(cell_text)
                .filter(|s| !s.is_empty())
                .collect();
            match cells.len() {
                0 => {}
                1 => {
                    let _ = writeln!(rows, "{}", cells[0]);
                }
                _ => {
                    let _ = writeln!(rows, "{}: {}", cells[0], cells[1..].join(" | "));
                }
            }
        }
        if !rows.trim().is_empty() {
            let _ = write!(out, "### {name}\n{rows}\n");
        }
    }
    Ok(out)
}

fn cell_text(c: &Data) -> String {
    match c {
        Data::Empty => String::new(),
        Data::String(s) => s.trim().to_string(),
        Data::Float(f) => {
            // Whole numbers as integers: "12" reads better than "12.0" as a fact.
            if f.fract() == 0.0 && f.abs() < 1e15 {
                format!("{}", *f as i64)
            } else {
                f.to_string()
            }
        }
        other => other.to_string().trim().to_string(),
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
