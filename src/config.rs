//! The settings the Config pane shows and edits.
//!
//! One table, `FIELDS`, drives the pane the same way `KEYS`/`COMMANDS` drive the
//! hotkeys and `preset()` drives the providers: a new setting is a row, not a
//! block of render code. Each row says where the value lives (`env`, an `IV_*`
//! var), what shape it is (`Kind`), and whether editing it changes anything
//! without a restart (`Apply`).
//!
//! Persistence is `.env` itself — already the source of truth, already
//! git-ignored — through `upsert_env`. Almost nothing can change live (provider,
//! model, key, devices and gates are all read once and baked into threads at
//! startup), so "editable + persistent" means: write every edit to `.env` now,
//! apply the three that *can* change live immediately, and mark the rest
//! "restart to apply". The split is the `Apply` column, not scattered `if`s.

use std::io;
use std::path::{Path, PathBuf};

/// The API key row. Its real `IV_*`-equivalent (`ANTHROPIC_API_KEY`, …) depends
/// on the selected provider, so it carries a sentinel instead of a fixed var and
/// the pane resolves the target through `provider::key_var`. Not a real env name.
pub const KEY_SENTINEL: &str = "IV_API_KEY";

/// The provider names the enum offers: the `preset()` vendors plus `none`.
pub const PROVIDERS: &[&str] = &["anthropic", "openai", "deepseek", "gemini", "openrouter", "none"];

#[derive(Clone, Copy, PartialEq)]
pub enum Apply {
    /// Takes effect immediately, through an existing live path.
    Live,
    /// Persisted now, read at the next launch.
    Restart,
}

#[derive(Clone, Copy)]
pub enum Kind {
    Text,
    /// Masked, never mirrored to the state dump, never logged.
    Secret,
    Bool,
    Enum(&'static [&'static str]),
    /// Inclusive integer range. Bounds mirror the clap attrs in `main.rs`.
    Number { min: i64, max: i64 },
    /// A VAD threshold, validated by `crate::parse_gate` (0 < x < 1).
    Gate,
}

pub struct Field {
    pub label: &'static str,
    pub env: &'static str,
    pub section: &'static str,
    pub kind: Kind,
    pub apply: Apply,
}

use Apply::{Live, Restart};

/// Every editable setting, grouped by section (the pane prints a heading when
/// the section changes, so order matters). Excludes the run-mode flags
/// (`--preview`, `--setup`, `--list-*`, `--search-files`) — they are actions,
/// not configuration.
pub const FIELDS: &[Field] = &[
    // Provider
    Field { label: "Provider", env: "IV_PROVIDER", section: "Provider", kind: Kind::Enum(PROVIDERS), apply: Restart },
    Field { label: "Model", env: "IV_MODEL", section: "Provider", kind: Kind::Text, apply: Restart },
    Field { label: "API key", env: KEY_SENTINEL, section: "Provider", kind: Kind::Secret, apply: Restart },
    Field { label: "Coaching persona (path)", env: "IV_PROMPT", section: "Provider", kind: Kind::Text, apply: Restart },

    // Audio
    Field { label: "Microphone", env: "IV_MIC", section: "Audio", kind: Kind::Text, apply: Restart },
    Field { label: "Loopback device", env: "IV_LOOPBACK", section: "Audio", kind: Kind::Text, apply: Restart },
    Field { label: "Hear only these apps", env: "IV_HEAR", section: "Audio", kind: Kind::Text, apply: Live },
    Field { label: "Mic gate", env: "IV_MIC_GATE", section: "Audio", kind: Kind::Gate, apply: Restart },
    Field { label: "System gate", env: "IV_SYS_GATE", section: "Audio", kind: Kind::Gate, apply: Restart },
    Field { label: "Read advice aloud", env: "IV_SPEAK", section: "Audio", kind: Kind::Bool, apply: Restart },
    Field { label: "Voice", env: "IV_VOICE", section: "Audio", kind: Kind::Text, apply: Restart },

    // Behaviour
    Field { label: "Coaching armed", env: "IV_MANUAL", section: "Behaviour", kind: Kind::Bool, apply: Live },
    Field { label: "Opacity (0-255)", env: "IV_ALPHA", section: "Behaviour", kind: Kind::Number { min: 0, max: 255 }, apply: Live },
    Field { label: "Minimum words", env: "IV_MIN_WORDS", section: "Behaviour", kind: Kind::Number { min: 0, max: 100 }, apply: Restart },
    Field { label: "Settle (ms)", env: "IV_SETTLE", section: "Behaviour", kind: Kind::Number { min: 0, max: 10_000 }, apply: Restart },

    // Naming
    Field { label: "People book (path)", env: "IV_PEOPLE", section: "Naming", kind: Kind::Text, apply: Restart },
    Field { label: "Keep days", env: "IV_KEEP_DAYS", section: "Naming", kind: Kind::Number { min: 0, max: 100_000 }, apply: Restart },
    Field { label: "Keep turns", env: "IV_KEEP_TURNS", section: "Naming", kind: Kind::Number { min: 0, max: 100_000 }, apply: Restart },
    Field { label: "Voice model (path)", env: "IV_VOICES", section: "Naming", kind: Kind::Text, apply: Restart },

    // Knowledge
    Field { label: "Knowledge folder (path)", env: "IV_KNOWLEDGE", section: "Knowledge", kind: Kind::Text, apply: Restart },
    Field { label: "References folder (path)", env: "IV_REFERENCES", section: "Knowledge", kind: Kind::Text, apply: Restart },

    // Research
    Field { label: "Research persona (path)", env: "IV_RESEARCH_PROMPT", section: "Research", kind: Kind::Text, apply: Restart },
    Field { label: "Agent command (path)", env: "IV_AGENT_CMD", section: "Research", kind: Kind::Text, apply: Restart },
    Field { label: "Agent root (path)", env: "IV_AGENT_ROOT", section: "Research", kind: Kind::Text, apply: Restart },
    Field { label: "Agent timeout (s)", env: "IV_AGENT_TIMEOUT", section: "Research", kind: Kind::Number { min: 5, max: 300 }, apply: Restart },
    Field { label: "Everything es.exe (path)", env: "IV_ES", section: "Research", kind: Kind::Text, apply: Restart },
    Field { label: "Filename search", env: "IV_SEARCH_QUERY", section: "Research", kind: Kind::Text, apply: Restart },

    // Logging
    Field { label: "Whisper model (path)", env: "IV_WHISPER", section: "Logging", kind: Kind::Text, apply: Restart },
    Field { label: "Transcript folder (path)", env: "IV_LOG", section: "Logging", kind: Kind::Text, apply: Restart },
    Field { label: "Dump folder (path)", env: "IV_DUMP", section: "Logging", kind: Kind::Text, apply: Restart },
];

/// Validate a typed value for `kind`, returning the string to persist. Bool and
/// Enum never reach here — the pane commits those from a click, not typed text.
pub fn validate(kind: &Kind, input: &str) -> Result<String, String> {
    let v = input.trim();
    match kind {
        Kind::Gate => crate::parse_gate(v).map(|g| g.to_string()),
        Kind::Number { min, max } => {
            let n: i64 = v.parse().map_err(|_| "must be a whole number".to_string())?;
            if n < *min || n > *max {
                return Err(format!("must be between {min} and {max}"));
            }
            Ok(n.to_string())
        }
        // Text/Secret persist verbatim (trimmed); a path is not checked for
        // existence here, since it may be created before the next launch.
        _ => Ok(v.to_string()),
    }
}

/// Write `KEY=value` into `path`, preserving every other line.
///
/// Replaces the first line whose body is `KEY=…`, **commented or not** — the
/// `.env`/`.env.example` ship the keys as `# ANTHROPIC_API_KEY=`, and a naive
/// `^KEY=` match would skip the commented form and append a duplicate that the
/// commented one then shadows on read. If no such line exists, appends one.
///
/// Written through a temp file and `rename` (atomic on the same volume) so a
/// crash mid-write never truncates the file that holds the API keys.
pub fn upsert_env(path: &Path, key: &str, value: &str) -> io::Result<()> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let prefix = format!("{key}=");
    let mut lines: Vec<String> = existing.lines().map(str::to_string).collect();
    let mut done = false;
    for line in &mut lines {
        // Strip one optional leading `#` before matching, so a commented key is
        // uncommented in place rather than duplicated.
        let body = line.trim_start();
        let body = body.strip_prefix('#').map(str::trim_start).unwrap_or(body);
        if body.starts_with(&prefix) {
            *line = format!("{key}={value}");
            done = true;
            break;
        }
    }
    if !done {
        lines.push(format!("{key}={value}"));
    }
    let mut out = lines.join("\n");
    // Keep a trailing newline (POSIX text file, and dotenvy is happier with it).
    out.push('\n');

    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, out)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_env_uncomments_appends_and_preserves() {
        // A unique scratch file; no tempfile dependency for one round-trip.
        let mut path = std::env::temp_dir();
        path.push(format!(
            "iv-config-test-{}-{}.env",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(
            &path,
            "# a header comment\n# ANTHROPIC_API_KEY=old\nIV_MODEL=claude-opus-5\n",
        )
        .unwrap();

        // Commented key: uncommented in place, not appended.
        upsert_env(&path, "ANTHROPIC_API_KEY", "sk-new").unwrap();
        // Existing uncommented key: replaced in place.
        upsert_env(&path, "IV_MODEL", "gpt-4o").unwrap();
        // Brand-new key: appended.
        upsert_env(&path, "IV_ALPHA", "200").unwrap();

        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("# a header comment"), "lost the comment");
        assert!(out.contains("ANTHROPIC_API_KEY=sk-new"), "key not written");
        assert!(!out.contains("# ANTHROPIC_API_KEY="), "commented form left behind");
        assert_eq!(out.matches("ANTHROPIC_API_KEY=").count(), 1, "duplicated key");
        assert!(out.contains("IV_MODEL=gpt-4o") && !out.contains("claude-opus-5"));
        assert!(out.contains("IV_ALPHA=200"), "new key not appended");
        // Atomic write leaves no temp behind.
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        assert!(!PathBuf::from(tmp).exists(), "temp file not renamed away");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn validate_bounds_and_gate() {
        assert!(validate(&Kind::Number { min: 0, max: 255 }, "256").is_err());
        assert_eq!(validate(&Kind::Number { min: 0, max: 255 }, " 200 ").unwrap(), "200");
        assert!(validate(&Kind::Gate, "0").is_err());
        assert!(validate(&Kind::Gate, "0.5").is_ok());
    }
}
