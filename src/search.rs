//! Explicit, bounded Everything filename search; never executes result paths.
use anyhow::{Context, Result, bail};
use std::io::Read;
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::AtomicBool,
    time::Duration,
};

pub fn files(executable: &Path, query: &str, cancel: &AtomicBool) -> Result<Vec<PathBuf>> {
    let query = query.trim();
    if query.is_empty() || query.len() > 512 || query.chars().any(char::is_control) {
        bail!("search query must contain 1–512 bytes and no control characters");
    }
    // A quoted literal prevents a filename from being interpreted as ES flags
    // or an Everything query operator. Quotes themselves are not filename chars.
    if query.contains('"') {
        bail!("search query cannot contain quotes");
    }
    // ES console output uses the console code page. Its export format is UTF-8:
    // https://www.voidtools.com/support/everything/command_line_interface/
    let export = Export::new()?;
    let mut command = Command::new(executable);
    command
        .args(["-n", "20", "-match-path", "-timeout", "1000", "-export-txt"])
        .arg(export.0.join("results.txt"))
        .arg(format!("\"{query}\""));
    crate::process::run(command, String::new(), Duration::from_secs(5), cancel)?;
    let mut out = String::new();
    std::fs::File::open(export.0.join("results.txt"))?
        .take(1_048_577)
        .read_to_string(&mut out)
        .context("reading UTF-8 search results")?;
    if out.len() > 1_048_576 {
        bail!("search results exceed 1 MB");
    }
    Ok(out
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .take(20)
        .map(PathBuf::from)
        .collect())
}

struct Export(PathBuf);
impl Export {
    fn new() -> Result<Self> {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("inner-voice-search-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for Export {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.0.join("results.txt"));
        let _ = std::fs::remove_dir(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_queries_before_spawning() {
        for query in ["", "\n", "a\"b", "a\tb"] {
            assert!(files(Path::new("missing.exe"), query, &AtomicBool::new(false)).is_err());
        }
    }
}
