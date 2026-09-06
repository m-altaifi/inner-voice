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
pub const FORMATS: &str = "TXT, Markdown, CSV, an Excel/OpenDocument spreadsheet, PDF, DOCX, or an image (PNG/JPG/BMP/TIFF/GIF, read by Windows OCR)";

fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// Whether `text` would try to read this file at all. `knowledge/` skips the
/// rest silently — a stray `.bak` in the folder is not an error.
pub fn supported(path: &Path) -> bool {
    // Office's lock file for an open document (`~$brief.docx`, hidden, ~160
    // bytes) passes on extension alone and then fails to parse — which aborts
    // startup, or makes `Corpus::refresh` return Err on every save for as long
    // as Word holds the file. Guarded here because every folder scan
    // (`knowledge::load`, `knowledge::count`, `references::import_folder`)
    // routes through this predicate; `text` stays permissive for an explicit drop.
    if path
        .file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with("~$"))
    {
        return false;
    }
    matches!(
        extension(path).as_str(),
        "txt"
            | "md"
            | "csv"
            | "xlsx"
            | "xlsm"
            | "xls"
            | "ods"
            | "pdf"
            | "docx"
            | "png"
            | "jpg"
            | "jpeg"
            | "bmp"
            | "tif"
            | "tiff"
            | "gif"
    )
}

/// The document's text, or why it could not be read.
pub fn text(path: &Path) -> Result<String> {
    match extension(path).as_str() {
        "txt" | "md" => std::fs::read_to_string(path).context("expected UTF-8 text"),
        "csv" => csv_to_text(path),
        "xlsx" | "xlsm" | "xls" | "ods" => sheet_to_text(path),
        "pdf" => pdf(path),
        "docx" => docx(path),
        "png" | "jpg" | "jpeg" | "bmp" | "tif" | "tiff" | "gif" => ocr(path),
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

/// Word's own XML, not a `docx` crate: it is a zip holding
/// `word/document.xml`, and the two crates that read those are already in the
/// tree under calamine. Paragraphs become lines; inside a table, cells become
/// tab-separated columns and rows become lines.
///
/// ponytail: text is taken from every element rather than gated on `w:t`, so a
/// field code or a tracked deletion leaks into the output; gate on `w:t` (and
/// skip `w:delText`) if a briefing ever reads wrong because of it.
/// ponytail: only `word/document.xml` is read — headers, footers and footnotes
/// are not; add their parts if a brief ever keeps facts there.
fn docx(path: &Path) -> Result<String> {
    use quick_xml::events::Event;
    use std::io::Read as _;
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file).context("not a DOCX (zip) file")?;
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")
        .context("not a DOCX: no word/document.xml inside")?
        .read_to_string(&mut xml)?;
    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut out = String::new();
    let mut in_cell = 0usize;
    loop {
        match reader.read_event().context("reading DOCX XML")? {
            Event::Eof => break,
            Event::Text(t) => out.push_str(&t.decode()?),
            // 0.41 delivers `&amp;` and `&#8217;` as their own event, not inside
            // the surrounding text.
            Event::GeneralRef(r) => {
                out.push_str(&quick_xml::escape::unescape(&format!("&{};", r.decode()?))?)
            }
            Event::Empty(e) if e.name().as_ref() == b"w:tab" => out.push('\t'),
            // A line break inside a cell is still one cell: splitting there
            // would break the row across lines and lose the column pairing.
            Event::Empty(e) if e.name().as_ref() == b"w:br" => {
                out.push(if in_cell > 0 { ' ' } else { '\n' })
            }
            Event::Start(e) if e.name().as_ref() == b"w:tc" => in_cell += 1,
            Event::End(e) => match e.name().as_ref() {
                // A paragraph inside a cell is still one cell: keep the row on one line.
                b"w:p" if in_cell > 0 => out.push(' '),
                b"w:p" => out.push('\n'),
                b"w:tc" => {
                    in_cell = in_cell.saturating_sub(1);
                    // Trim the paragraph's trailing space before the column break.
                    while out.ends_with(' ') {
                        out.pop();
                    }
                    out.push('\t');
                }
                b"w:tr" => out.push('\n'),
                _ => {}
            },
            _ => {}
        }
    }
    Ok(out)
}

/// Windows' own OCR (WinRT `Windows.Media.Ocr`), which is already in the
/// `windows` crate this app links — no tesseract, no model download. It needs
/// a language pack with OCR installed, and an apartment on the calling thread.
fn ocr(path: &Path) -> Result<String> {
    use windows::{
        Graphics::Imaging::BitmapDecoder,
        Media::Ocr::OcrEngine,
        Storage::{FileAccessMode, StorageFile},
        core::HSTRING,
    };
    // References hands over canonical paths (`load` canonicalises before
    // reading) and `absolute` leaves a verbatim prefix untouched, so strip it
    // here: StorageFile wants the Win32 form and answers `\\?\…` with
    // ERROR_BAD_PATHNAME, which made every imported image fail.
    let absolute = std::path::absolute(path)?.to_string_lossy().into_owned();
    let absolute = match absolute.strip_prefix(r"\\?\UNC\") {
        Some(share) => format!(r"\\{share}"),
        None => absolute
            .strip_prefix(r"\\?\")
            .unwrap_or(&absolute)
            .to_owned(),
    };
    let engine = OcrEngine::TryCreateFromUserProfileLanguages().context(
        "no OCR language installed: Settings > Time & language > Language & region > \
         add a language, then its optional Optical character recognition feature",
    )?;
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(absolute.as_str()))?.join()?;
    let stream = file.OpenAsync(FileAccessMode::Read)?.join()?;
    let bitmap = BitmapDecoder::CreateAsync(&stream)?
        .join()?
        .GetSoftwareBitmapAsync()?
        .join()?;
    let result = engine.RecognizeAsync(&bitmap)?.join()?;
    let mut out = String::new();
    for line in result.Lines()? {
        out.push_str(&line.Text()?.to_string_lossy());
        out.push('\n');
    }
    if out.trim().is_empty() {
        bail!("no text recognised in the image");
    }
    Ok(out)
}

/// Whether an OCR engine can be created on this machine. Activates WinRT, so
/// like `ocr` it needs COM initialised on the calling thread — Wave 3's
/// `--setup` must call it after `initialize_mta()`, or it reports "no OCR"
/// on a machine that has one.
pub fn ocr_available() -> bool {
    windows::Media::Ocr::OcrEngine::TryCreateFromUserProfileLanguages().is_ok()
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
        // Word's lock file for an open document: a `.docx` that is not one.
        assert!(!supported(Path::new("~$brief.docx")));
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
    /// The smallest thing Word would call a document: a zip holding
    /// `word/document.xml`. Built here with the same `zip` crate the reader
    /// uses, so the test needs no fixture file.
    fn tiny_docx(document_xml: &str) -> std::path::PathBuf {
        use std::io::Write as _;
        let path = scratch("f.docx");
        let file = std::fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(
            "word/document.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(document_xml.as_bytes()).unwrap();
        zip.finish().unwrap();
        path
    }
    #[test]
    fn docx_paragraphs_become_lines_and_cells_become_columns() {
        let path = tiny_docx(
            r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Rollback owner</w:t></w:r><w:r><w:t xml:space="preserve"> is Sarah &amp; team</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>Target</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>20 minutes</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body></w:document>"#,
        );
        let out = text(&path).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "Rollback owner is Sarah & team", "{out:?}");
        assert!(lines[1].starts_with("Target\t20 minutes"), "{out:?}");
        assert!(supported(&path));
        let junk = scratch("g.docx");
        std::fs::write(&junk, b"not a zip").unwrap();
        assert!(text(&junk).unwrap_err().to_string().contains("DOCX"));
    }
    #[test]
    fn images_are_read_by_windows_ocr_when_a_language_is_installed() {
        let _ = unsafe {
            windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            )
        };
        // Same rule as the GPU test: the machine decides whether this runs.
        if !ocr_available() {
            eprintln!("skipping: no Windows OCR language installed");
            return;
        }
        // Canonicalised on purpose: `references::load` hands `text` a `\\?\`
        // path, and that is the shape OCR has to survive.
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/ocr.png")
            .canonicalize()
            .unwrap();
        let out = text(&path).unwrap();
        assert!(out.contains("Sarah"), "{out:?}");
        assert!(supported(&path));
    }
}
