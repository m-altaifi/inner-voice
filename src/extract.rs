//! One reader for every document format the app accepts.
//!
//! `knowledge/` and dropped references both come through here, so a format
//! added once is read in both places. Path in, text out; nothing here executes
//! a file or touches the network.
use anyhow::{Context, Result, bail};
use calamine::{Data, Reader, open_workbook_auto};
use std::fmt::Write as _;
use std::path::Path;

/// What `text` accepts, for error messages and `--setup`.
pub const FORMATS: &str = "TXT, Markdown, CSV, or an Excel/OpenDocument spreadsheet";

fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// Whether `text` would try to read this file at all. `knowledge/` skips the
/// rest silently — a stray `.bak` in the folder is not an error.
pub fn supported(path: &Path) -> bool {
    matches!(
        extension(path).as_str(),
        "txt" | "md" | "csv" | "xlsx" | "xlsm" | "xls" | "ods"
    )
}

/// The document's text, or why it could not be read.
pub fn text(path: &Path) -> Result<String> {
    match extension(path).as_str() {
        "txt" | "md" => std::fs::read_to_string(path).context("expected UTF-8 text"),
        "csv" => csv_to_text(path),
        "xlsx" | "xlsm" | "xls" | "ods" => sheet_to_text(path),
        _ => bail!("unsupported format; use {FORMATS}"),
    }
}

/// Same flattening for CSV. Not an Excel format, so calamine cannot read it —
/// and a real parser matters here because a facts sheet will contain quoted
/// values with commas in them.
fn csv_to_text(path: &Path) -> Result<String> {
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
fn sheet_to_text(path: &Path) -> Result<String> {
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
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("iv_extract_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }
    #[test]
    fn reads_text_and_csv_and_names_the_formats_it_does_not() {
        let txt = scratch("a.txt");
        std::fs::write(&txt, "plain words").unwrap();
        assert_eq!(text(&txt).unwrap(), "plain words");
        let csv = scratch("b.csv");
        std::fs::write(&csv, "owner,\"Smith, John\"\n").unwrap();
        assert!(text(&csv).unwrap().contains("owner: Smith, John"));
        let exe = scratch("c.exe");
        std::fs::write(&exe, b"MZ").unwrap();
        let err = text(&exe).unwrap_err().to_string();
        assert!(err.contains("unsupported format"), "{err}");
        assert!(
            err.contains(FORMATS),
            "the error names every accepted format: {err}"
        );
        assert!(supported(&txt) && supported(&csv) && !supported(&exe));
    }
}
