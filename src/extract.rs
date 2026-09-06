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
pub const FORMATS: &str = "TXT, Markdown, CSV, an Excel/OpenDocument spreadsheet, or PDF";

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
        "txt" | "md" | "csv" | "xlsx" | "xlsm" | "xls" | "ods" | "pdf"
    )
}

/// The document's text, or why it could not be read.
pub fn text(path: &Path) -> Result<String> {
    match extension(path).as_str() {
        "txt" | "md" => std::fs::read_to_string(path).context("expected UTF-8 text"),
        "csv" => csv_to_text(path),
        "xlsx" | "xlsm" | "xls" | "ods" => sheet_to_text(path),
        "pdf" => pdf(path),
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

/// The text layer only. A scanned PDF has pages and no text: say so, rather
/// than importing an empty document that then "matches nothing".
fn pdf(path: &Path) -> Result<String> {
    let text = pdf_extract::extract_text(path).context("reading PDF")?;
    if text.trim().chars().count() < 20 {
        bail!("no text layer; export pages as images for OCR");
    }
    Ok(text)
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
    /// A one-page PDF with a correct xref, built here so the test needs no
    /// external tool and no committed binary. Helvetica is a standard-14 font,
    /// so no font program has to be embedded for the text to be recoverable.
    fn tiny_pdf(text: &str) -> Vec<u8> {
        let content = format!("BT /F1 24 Tf 20 100 Td ({text}) Tj ET");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 144] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }
    #[test]
    fn pdf_text_layer_is_read_and_a_scan_is_refused() {
        let pdf = scratch("d.pdf");
        std::fs::write(&pdf, tiny_pdf("Rollback owner is Sarah")).unwrap();
        assert!(text(&pdf).unwrap().contains("Rollback owner is Sarah"));
        let scan = scratch("e.pdf");
        std::fs::write(&scan, tiny_pdf("")).unwrap();
        let err = text(&scan).unwrap_err().to_string();
        assert!(err.contains("no text layer"), "{err}");
        assert!(supported(&pdf));
    }
}
