# Wave 2 — Memory Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make on-disk the source of truth — the app never coaches from a stale copy — and read the formats the user actually has: one loader shared by `knowledge/` and references (adding PDF, DOCX and Windows OCR), references that persist in a folder and reload when edited, `knowledge/` that reloads mid-session, and a CLI research lane on `claude -p` that reads disk live.

**Architecture:** A new `src/extract.rs` owns path→text for every format; `knowledge.rs` and `references.rs` both call it. `references.rs` gains a folder, a manifest of dropped paths, and `(path, mtime)` identity. `knowledge.rs` gains a `Corpus` that re-reads on change; the router calls `refresh()` before each coach request and pushes the new prompt to the coach and the new glossary to whisper through `RwLock`s. `agent.rs` chooses its adapter by the executable's stem; the `claude` adapter's command line and parser are pinned to a real capture. Tasks run sequentially in the main tree, one fresh subagent each.

**Tech Stack:** Rust edition 2024; `pdf-extract` (new); `zip` 8.6.0 and `quick-xml` 0.41.0 (already transitive under calamine, promoted to direct); `windows` 0.62.2 WinRT (`Media_Ocr`, `Graphics_Imaging`, `Storage`, `Storage_Streams`, `Foundation_Collections`) + `windows-future` 0.3.2 (already in the lock); `claude.exe` from the Claude Code npm package.

**Spec:** `docs/superpowers/specs/2026-09-06-complete-inner-voice-design.md` — §5 is this wave, §2 the amended decisions. Read it first.

## Global Constraints

- **Build only through `.\build.ps1`** (PowerShell tool, repo root), never bare cargo. `.\build.ps1 test --release`, `.\build.ps1 clippy --release --all-targets`, `.\build.ps1 fmt -- --check` (check its *output*, not exit code: `$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { "DRIFT: $out" } else { "fmt clean" }`), `.\build.ps1 build --release`, `.\build.ps1 add <crate>` for dependencies.
- **Kill a running preview before building**: `Get-Process inner-voice -ErrorAction SilentlyContinue | Stop-Process -Force`.
- **Verification ladder on every task:** tests all green, clippy zero warnings, fmt clean; tasks touching `hud.rs`, `audio.rs` or `main.rs` also run `tools/ui_smoke.ps1` against a live `--preview` (recipe in Task 6 step 5).
- **On-disk is the source of truth (Decision 9).** Never copy a user's file; remember its path. Never hold a stale copy past the next turn.
- **Importing never executes a file or uses a network.** Per-file errors surface as import feedback and never abort a batch.
- **Research safety (LEDGER):** read-only tools only. The `claude` adapter passes `--allowedTools Read,Grep,Glob` and nothing that skips permissions.
- **No buttons, no view-switcher, never take focus.** Nothing in this wave adds a control.
- **Comments explain why a simpler thing was rejected**, not what the code does.
- **Commit at the end of every task** on `main`, trailer `Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>`, via `git commit -F -` with a PowerShell here-string. Never `--amend`, never `--no-verify`.
- **Edit files with the Edit tool.** Do not run perl/sed re-encoding one-liners over sources containing `—`, `·`, `…`; a previous task corrupted a file that way.
- File ownership per task is listed in each task; stay inside it unless a step explicitly says otherwise.

---

## File Structure

| File | Responsibility after this wave |
| --- | --- |
| `src/extract.rs` (new) | `text(path) -> Result<String>` and `supported(path) -> bool`; the format table; csv/sheet converters (moved from `knowledge.rs`); PDF, DOCX, OCR readers. |
| `src/knowledge.rs` | `load` via `extract`; `Corpus { dir, newest, text }` with `refresh()`; `newest(dir)`; `glossary` unchanged. |
| `src/references.rs` | Folder + manifest persistence; `Document.modified`; `(path, mtime)` identity; `stale()`; `import_folder()`; `new(tx, folder: Option<PathBuf>)`. |
| `src/agent.rs` | Adapter by executable stem: `claude` (pinned to `tests/fixtures/claude-stream.jsonl`) or `codex` (unchanged, unverified). |
| `src/coach.rs` | `prompt: Arc<RwLock<String>>`; `set_prompt`. |
| `src/audio.rs` | `Tune.prompt: RwLock<String>`; `transcribe` reads it per inference. |
| `src/main.rs` | `--references`; startup folder import; `build_prompt`; `route()` refreshes the corpus before each coach request; `--agent-root` default `.`; notices. |
| `src/hud.rs` | Notice filter gains `knowledge`. |
| `tests/fixtures/claude-stream.jsonl`, `tests/fixtures/ocr.png` | Real capture; rendered-text image. |
| `Cargo.toml` | `pdf-extract`, `zip`, `quick-xml`, `windows-future`; windows features. |
| `README.md`, `CLAUDE.md`, `LEDGER.md`, `.env.example` | Per spec §5 and §2. |

---

### Task 1: `src/extract.rs` — one loader, existing formats moved

**Files:**
- Create: `src/extract.rs`
- Modify: `src/main.rs` (add `mod extract;` to the module list), `src/knowledge.rs` (`load`, remove `csv_to_text`/`sheet_to_text`/`cell_text` and the `calamine`/`fmt::Write` imports they used), `src/references.rs` (`load`)

**Interfaces:**
- Produces: `pub fn extract::text(path: &Path) -> anyhow::Result<String>`; `pub fn extract::supported(path: &Path) -> bool`; `pub const extract::FORMATS: &str` (the human-readable list used in errors).

- [ ] **Step 1: Write the failing tests**

Create `src/extract.rs` with only the test module for now:

```rust
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
        assert!(err.contains(FORMATS), "the error names every accepted format: {err}");
        assert!(supported(&txt) && supported(&csv) && !supported(&exe));
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Add `mod extract;` after `mod coach;` in `src/main.rs`. Run: `.\build.ps1 test --release extract::`
Expected: compile error — `text`, `supported`, `FORMATS` not found.

- [ ] **Step 3: Write the module**

Above the test module in `src/extract.rs`:

```rust
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
```

Then move `csv_to_text`, `sheet_to_text` and `cell_text` from `src/knowledge.rs` into `src/extract.rs` verbatim, including their doc comments, changing `pub(crate) fn` to `fn`. Delete them from `knowledge.rs` along with `use calamine::{Data, Reader, open_workbook_auto};` and `use std::fmt::Write as _;` if nothing else there uses them (`load` uses `write!` — keep `std::fmt::Write` if so).

- [ ] **Step 4: Point both consumers at it**

In `src/knowledge.rs`, replace the body of the `for path in &files { ... }` loop's format dispatch — from `let ext = path.extension()...` through the `let body = match ext.as_str() { ... };` — with:

```rust
        if !crate::extract::supported(path) {
            continue;
        }
        let body =
            crate::extract::text(path).with_context(|| format!("reading {}", path.display()))?;
```

(`name` is still derived above it; keep that line.)

In `src/references.rs` `fn load`, replace everything from `let extension = path.extension()` through the closing `};` of `let text = match extension.as_str() { ... };` with:

```rust
    let text = crate::extract::text(&path)?;
```

Keep the 10 MB check above it and the `ensure!(text.len() <= MAX_TEXT, "{OVERSIZE}")` / `no readable text` checks below it. The "size before decode" comment and the per-extension pre-check go: the whole file (≤ 10 MB) is read and then measured, which is what `oversized_text_blames_its_size_not_its_encoding` asserts.

Update the unsupported-format message in `references.rs` `preview()` empty-state text from `Supported: TXT, Markdown, CSV, XLSX, XLSM, XLS, ODS.` to `Supported: {}` formatted with `crate::extract::FORMATS` (use `format!` — that string is currently a literal; make it `format!("Drop reference files into this window.\r\n\r\nSupported: {}.\r\n\r\nDocuments stay in memory for this session. …", crate::extract::FORMATS)` keeping the rest verbatim, and delete the trailing sentence `PDF, Word documents and images are not supported yet.` — Tasks 2–4 make it false).

- [ ] **Step 5: Run everything**

Run: `.\build.ps1 test --release`
Expected: all green — the `knowledge::` tests (`csv_becomes_key_value_lines` etc.) and `references::` tests unchanged and passing through the new path, plus the new `extract::` test.

- [ ] **Step 6: Ladder and commit**

```powershell
.\build.ps1 clippy --release --all-targets
$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { .\build.ps1 fmt }
git add src/extract.rs src/knowledge.rs src/references.rs src/main.rs
git commit -F - @'
extract: one loader for knowledge/ and references

csv and spreadsheet readers move out of knowledge.rs; both consumers call
extract::text, so a format added once is read in both places.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
'@
```

---

### Task 2: PDF

**Files:**
- Modify: `Cargo.toml` (via `.\build.ps1 add pdf-extract`), `src/extract.rs`

**Interfaces:**
- Produces: `.pdf` in `supported`/`text`; `FORMATS` gains `PDF`.

- [ ] **Step 1: Write the failing test**

Add to `extract::tests`:

```rust
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
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes(),
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
```

- [ ] **Step 2: Run to verify it fails**

Run: `.\build.ps1 test --release pdf_text_layer`
Expected: FAIL — `text` returns the unsupported-format error for `.pdf`.

- [ ] **Step 3: Add the crate and the reader**

`.\build.ps1 add pdf-extract` (record the resolved version in the commit body). Check the crate's public API in the vendored source (`~/.cargo/registry/src/*/pdf-extract-*/src/lib.rs`): the function to use is the one that takes a path and returns the whole document's text as a `String` (in recent versions `pdf_extract::extract_text(path)`, returning `Result<String, OutputError>`). Then in `extract.rs`:

- `FORMATS` becomes `"TXT, Markdown, CSV, an Excel/OpenDocument spreadsheet, or PDF"`.
- `supported` gains `| "pdf"`.
- `text` gains the arm `"pdf" => pdf(path),`.
- Add:

```rust
/// The text layer only. A scanned PDF has pages and no text: say so, rather
/// than importing an empty document that then "matches nothing".
fn pdf(path: &Path) -> Result<String> {
    let text = pdf_extract::extract_text(path).context("reading PDF")?;
    if text.trim().chars().count() < 20 {
        bail!("no text layer; export pages as images for OCR");
    }
    Ok(text)
}
```

`pdf-extract` logs warnings through the `log` facade for odd files; nothing in this binary installs a logger, so they are silent. If the function name or error type differs in the resolved version, adapt to the real signature and say so in the report.

- [ ] **Step 4: Run the tests**

Run: `.\build.ps1 test --release extract::` → all green.

- [ ] **Step 5: Ladder and commit**

```powershell
.\build.ps1 clippy --release --all-targets
$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { .\build.ps1 fmt }
git add Cargo.toml Cargo.lock src/extract.rs
git commit -F - @'
extract: PDF text layer via pdf-extract <version>

A scanned PDF is refused with a message that says what to do instead.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
'@
```

---

### Task 3: DOCX

**Files:**
- Modify: `Cargo.toml` (`.\build.ps1 add zip@8.6.0 quick-xml@0.41.0` — the versions already in `Cargo.lock` under calamine, so nothing new is downloaded), `src/extract.rs`

**Interfaces:**
- Produces: `.docx` in `supported`/`text`; `FORMATS` gains `DOCX`.

- [ ] **Step 1: Write the failing test**

Add to `extract::tests`:

```rust
    /// The smallest thing Word would call a document: a zip holding
    /// `word/document.xml`. Built here with the same `zip` crate the reader
    /// uses, so the test needs no fixture file.
    fn tiny_docx(document_xml: &str) -> std::path::PathBuf {
        use std::io::Write as _;
        let path = scratch("f.docx");
        let file = std::fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("word/document.xml", zip::write::SimpleFileOptions::default()).unwrap();
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
```

- [ ] **Step 2: Run to verify it fails**

Run: `.\build.ps1 test --release docx_paragraphs`
Expected: compile error — `zip` not a direct dependency.

- [ ] **Step 3: Promote the crates and write the reader**

`.\build.ps1 add zip@8.6.0 quick-xml@0.41.0`. Both API surfaces are in the vendored sources under `~/.cargo/registry/src/*/zip-8.6.0/` and `quick-xml-0.41.0/`; verify the names used below against them (`ZipArchive::new`, `by_name`, `ZipWriter::start_file` with `SimpleFileOptions`; `Reader::from_str`, `read_event`, `Event::{Start,End,Empty,Text,Eof}`, `BytesText::unescape` or its current spelling, `BytesStart::name`).

In `extract.rs`: `FORMATS` gains `, DOCX` before the PDF item (keep it a readable sentence, e.g. `"TXT, Markdown, CSV, an Excel/OpenDocument spreadsheet, PDF, or DOCX"`); `supported` gains `| "docx"`; `text` gains `"docx" => docx(path),`; add:

```rust
/// Word's own XML, not a `docx` crate: it is a zip holding
/// `word/document.xml`, and the two crates that read those are already in the
/// tree under calamine. Paragraphs become lines; inside a table, cells become
/// tab-separated columns and rows become lines.
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
        match reader.read_event()? {
            Event::Eof => break,
            Event::Text(t) => out.push_str(&t.unescape()?),
            Event::Empty(e) if e.name().as_ref() == b"w:tab" => out.push('\t'),
            Event::Empty(e) if e.name().as_ref() == b"w:br" => out.push('\n'),
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
```

- [ ] **Step 4: Run the tests**

Run: `.\build.ps1 test --release extract::` → all green.

- [ ] **Step 5: Ladder and commit**

```powershell
.\build.ps1 clippy --release --all-targets
$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { .\build.ps1 fmt }
git add Cargo.toml Cargo.lock src/extract.rs
git commit -F - @'
extract: DOCX through zip + quick-xml, no new crate

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
'@
```

---

### Task 4: Images via Windows OCR

**Files:**
- Modify: `Cargo.toml` (windows features; `.\build.ps1 add windows-future@0.3.2`), `src/extract.rs`, `src/references.rs` (COM init on the import thread)
- Create: `tests/fixtures/ocr.png`

**Interfaces:**
- Produces: `.png .jpg .jpeg .bmp .tif .tiff .gif` in `supported`/`text`; `FORMATS` gains `images (OCR)`.

- [ ] **Step 1: Make the fixture**

From the PowerShell tool, once (commit the result):

```powershell
New-Item -ItemType Directory -Force tests\fixtures | Out-Null
Add-Type -AssemblyName System.Drawing
$bmp = New-Object System.Drawing.Bitmap 520, 90
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.Clear([System.Drawing.Color]::White)
$g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAlias
$font = New-Object System.Drawing.Font('Arial', 28)
$g.DrawString('Rollback owner is Sarah', $font, [System.Drawing.Brushes]::Black, 12, 22)
$bmp.Save((Join-Path $PWD 'tests\fixtures\ocr.png'), [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
(Get-Item tests\fixtures\ocr.png).Length
```

Expected: a file of a few KB. Open it with the Read tool to confirm it shows the sentence clearly.

- [ ] **Step 2: Write the failing test**

Add to `extract::tests`:

```rust
    #[test]
    fn images_are_read_by_windows_ocr_when_a_language_is_installed() {
        // Same rule as the GPU test: the machine decides whether this runs.
        if !ocr_available() {
            eprintln!("skipping: no Windows OCR language installed");
            return;
        }
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ocr.png");
        let out = text(&path).unwrap();
        assert!(out.contains("Sarah"), "{out:?}");
        assert!(supported(&path));
    }
```

- [ ] **Step 3: Run to verify it fails**

Run: `.\build.ps1 test --release images_are_read`
Expected: compile error — `ocr_available` not found.

- [ ] **Step 4: Features, crate, reader**

In `Cargo.toml`, add to the `windows` features list: `"Media_Ocr", "Graphics_Imaging", "Storage", "Storage_Streams", "Foundation", "Foundation_Collections"`. Run `.\build.ps1 add windows-future@0.3.2`.

In `extract.rs`: `FORMATS` gains `, or an image (PNG/JPG/BMP/TIFF/GIF, read by Windows OCR)`; `supported` gains `| "png" | "jpg" | "jpeg" | "bmp" | "tif" | "tiff" | "gif"`; `text` gains `"png" | "jpg" | "jpeg" | "bmp" | "tif" | "tiff" | "gif" => ocr(path),`; add:

```rust
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
    // `canonicalize` would yield a `\\?\` path, which StorageFile refuses.
    let absolute = std::path::absolute(path)?;
    let engine = OcrEngine::TryCreateFromUserProfileLanguages().context(
        "no OCR language installed: Settings > Time & language > Language & region > \
         add a language, then its optional Optical character recognition feature",
    )?;
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(absolute.as_os_str()))?.get()?;
    let stream = file.OpenAsync(FileAccessMode::Read)?.get()?;
    let bitmap = BitmapDecoder::CreateAsync(&stream)?
        .get()?
        .GetSoftwareBitmapAsync()?
        .get()?;
    let result = engine.RecognizeAsync(&bitmap)?.get()?;
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

/// Whether an OCR engine can be created on this machine.
pub fn ocr_available() -> bool {
    windows::Media::Ocr::OcrEngine::TryCreateFromUserProfileLanguages().is_ok()
}
```

WinRT activation needs COM initialised on the calling thread. `main` already runs `initialize_mta()`; the references import thread does not — in `src/references.rs`, at the top of the `std::thread::spawn(move || { ... })` closure in `References::new`, add:

```rust
            // OCR is WinRT and needs an apartment on this thread; the capture
            // threads do the same for WASAPI.
            let _ = unsafe {
                windows::Win32::System::Com::CoInitializeEx(
                    None,
                    windows::Win32::System::Com::COINIT_MULTITHREADED,
                )
            };
```

and in the test add `let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED) };` as the first line of `images_are_read_by_windows_ocr_when_a_language_is_installed` (before the `ocr_available()` check). `Win32_System_Com` is already a feature.

If `TryCreateFromUserProfileLanguages` returns `Ok` holding a null interface on a machine without a language (the WinRT method returns null rather than failing), `ocr_available` will misreport; check the vendored binding's return type — if it is `Result<OcrEngine>`, a null comes back as `Err`, and this is fine. Say in the report which it was on this machine.

- [ ] **Step 5: Run the tests**

Run: `.\build.ps1 test --release extract::` → green (or the skip line if no language is installed — then also run `.\build.ps1 build --release` to prove it compiles, and say so).

- [ ] **Step 6: Ladder and commit**

```powershell
.\build.ps1 clippy --release --all-targets
$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { .\build.ps1 fmt }
git add Cargo.toml Cargo.lock src/extract.rs src/references.rs tests/fixtures/ocr.png
git commit -F - @'
extract: images through Windows OCR, no new engine

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
'@
```

---

### Task 5: The `claude -p` research adapter, pinned to a real capture

**Files:**
- Create: `tests/fixtures/claude-stream.jsonl` (copy of `E:\OpenSources\katie\inner-voice\.superpowers\sdd\2026-09-06-wave2-memory\claude-stream-trimmed.jsonl`)
- Modify: `src/agent.rs` (`research`, new `response_claude`, tests), `src/main.rs` (`--agent-root` default and the `--agent-cmd` error text), `.env.example`

**Interfaces:**
- Consumes: `process::run(Command, String, Duration, &AtomicBool) -> Result<String>` (exists).
- Produces: adapter selection by `config.executable.file_stem()`; `fn response_claude(output: &str) -> Result<String>`.

**Facts established by capture on this machine (2026-09-06)**, which this task must not re-derive:
- `claude` on PATH is an npm shim; the binary is `%APPDATA%\npm\node_modules\@anthropic-ai\claude-code\bin\claude.exe`. It is an `.exe`, so the existing `--agent-cmd` rule holds.
- Command line that keeps the subscription login but drops hooks, plugins and MCP servers: `-p --output-format stream-json --verbose --strict-mcp-config --setting-sources "" --allowedTools Read,Grep,Glob --max-turns 6`, prompt on stdin, cwd = agent root. Measured: 5 s wall with one file read, 6.5 K cache tokens, ≈ $0.09 — versus 37 K tokens / $0.75 through the interactive harness. `--bare` drops the login and is unusable.
- Output is one JSON object per line. The answer is `result.result` on the `{"type":"result"}` line; an error is `result.is_error == true` (the `subtype` stays `"success"` even then). Assistant prose also appears as `{"type":"assistant","message":{"content":[{"type":"text","text":…}]}}`; tool calls as `assistant` content of type `tool_use` and `user` content of type `tool_result`.

- [ ] **Step 1: Copy the fixture and write the failing tests**

`Copy-Item '.superpowers\sdd\2026-09-06-wave2-memory\claude-stream-trimmed.jsonl' 'tests\fixtures\claude-stream.jsonl'`. Open it with the Read tool and confirm it contains an `init`, an `assistant` `tool_use`, a `user` `tool_result`, and a `result` line whose `result` mentions `inner-voice` — that is the shape the parser is pinned to.

Add to `agent::tests`:

```rust
    #[test]
    fn claude_answer_is_the_result_line_and_is_error_is_the_failure_signal() {
        let capture = include_str!("../tests/fixtures/claude-stream.jsonl");
        let answer = response_claude(capture).unwrap();
        assert!(answer.contains("inner-voice"), "{answer}");
        assert!(answer.contains("company.md"), "the tool path was exercised: {answer}");
        // subtype stays "success" on failure; is_error is the signal.
        let err = response_claude(
            r#"{"type":"result","subtype":"success","is_error":true,"result":"Not logged in · Please run /login"}"#,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("Not logged in"), "{err}");
        // No result line at all: quote the output so a shape change is diagnosable.
        let err = response_claude("some banner\n{\"type\":\"system\",\"subtype\":\"init\"}\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("some banner"), "{err}");
    }
    #[test]
    fn adapter_is_chosen_by_the_executable_name() {
        assert_eq!(adapter(std::path::Path::new(r"C:\x\claude.exe")), Adapter::Claude);
        assert_eq!(adapter(std::path::Path::new(r"C:\x\CLAUDE.EXE")), Adapter::Claude);
        assert_eq!(adapter(std::path::Path::new(r"C:\x\codex.exe")), Adapter::Codex);
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `.\build.ps1 test --release agent::`
Expected: compile errors — `response_claude`, `adapter`, `Adapter` not found.

- [ ] **Step 3: Implement**

In `src/agent.rs`, add after `pub struct Config`:

```rust
/// Which CLI's dialect to speak, decided by the executable's own name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Adapter {
    Claude,
    Codex,
}

fn adapter(executable: &std::path::Path) -> Adapter {
    match executable
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .as_deref()
    {
        Some("claude") => Adapter::Claude,
        _ => Adapter::Codex,
    }
}
```

In `fn research`, replace the block from `let mut command = Command::new(&config.executable);` to the end of the function with:

```rust
    let mut command = Command::new(&config.executable);
    command.current_dir(&config.root);
    match adapter(&config.executable) {
        // Keeps the subscription login, drops hooks/plugins/MCP: measured at
        // 6.5 K cache tokens and ~5 s against 37 K and ~$0.75 through the
        // interactive harness. `--bare` would drop the login too. Read-only
        // tools only — in `-p` mode anything else is denied, never prompted.
        Adapter::Claude => {
            command.args([
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--strict-mcp-config",
                "--setting-sources",
                "",
                "--allowedTools",
                "Read,Grep,Glob",
                "--max-turns",
                "6",
            ]);
            let output = process::run(command, prompt, config.timeout, cancel)?;
            response_claude(&output)
        }
        Adapter::Codex => {
            command.args([
                "exec",
                "--ignore-user-config",
                "--ignore-rules",
                "--ephemeral",
                "--sandbox",
                "read-only",
                "--skip-git-repo-check",
                "--json",
                "--color",
                "never",
                "-c",
                "approval_policy=\"never\"",
                "-",
            ]);
            let output = process::run(command, prompt, config.timeout, cancel)?;
            response(&output)
        }
    }
}

/// Claude Code's `stream-json`: one object per line, the answer on the
/// `result` line. `subtype` stays "success" even on failure — `is_error` is
/// the signal, learned from a captured "Not logged in" run.
fn response_claude(output: &str) -> Result<String> {
    for line in output.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if event["type"] != "result" {
            continue;
        }
        let text = event["result"].as_str().unwrap_or("").trim();
        if event["is_error"].as_bool().unwrap_or(false) {
            bail!("research failed: {}", if text.is_empty() { "CLI error" } else { text });
        }
        if text.is_empty() {
            bail!("research returned an empty answer");
        }
        return Ok(text.chars().take(16_000).collect());
    }
    bail!(
        "research returned no answer; first output line: {}",
        output
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("<empty>")
            .chars()
            .take(200)
            .collect::<String>()
    );
}
```

Also change the prompt's second-to-last sentence so the CLI knows where to look. In the `let prompt = format!(...)` block, replace `Use read-only research within the working directory.` with `Use read-only research within the working directory; the user's briefing is in knowledge/ and their reference files in references/.`

In `src/main.rs`: the `--agent-root` flag's `default_value = "knowledge"` becomes `default_value = "."` and its doc comment becomes `/// Working directory for read-only research; knowledge/ and references/ live under it`; the `--agent-cmd` ensure message `"--agent-cmd must be a Codex .exe, not a shell script"` becomes `"--agent-cmd must be an .exe (codex.exe, or Claude Code's bin\\claude.exe), not a shell script"`; the flag's doc comment `/// Route F8 research through this Codex .exe instead of the provider (no shell command strings)` becomes `/// Route F8 research through this CLI .exe instead of the provider (codex.exe or Claude Code's claude.exe; no shell command strings)`.

In `.env.example`, replace the `# IV_AGENT_CMD=C:\path\to\codex.exe` / `# IV_AGENT_ROOT=knowledge` lines with:

```
# Optional: route F8 research through a CLI instead of the provider. Claude
# Code's binary is under its npm package; codex.exe also works (unverified).
# IV_AGENT_CMD=C:\Users\<you>\AppData\Roaming\npm\node_modules\@anthropic-ai\claude-code\bin\claude.exe
# IV_AGENT_ROOT=.
```

- [ ] **Step 4: Run the tests, then one live run**

Run: `.\build.ps1 test --release agent::` → green. Then a live check from the PowerShell tool:

```powershell
Get-Process inner-voice -ErrorAction SilentlyContinue | Stop-Process -Force
.\build.ps1 build --release
$exe = "$env:APPDATA\npm\node_modules\@anthropic-ai\claude-code\bin\claude.exe"
$env:IV_UI_DUMP = "$env:TEMP\iv-agent.json"
$p = Start-Process -PassThru -FilePath .\target\release\inner-voice.exe -ArgumentList '--preview','--agent-cmd',$exe
```

`--preview` does not register research keys, so this only proves the flag validates (`$p.HasExited` is false after 3 s and the notice is not an `--agent-cmd` error). Stop it. A full live research run needs a provider and a real turn; it is covered by the gate. Report what you saw.

- [ ] **Step 5: Ladder and commit**

```powershell
.\build.ps1 clippy --release --all-targets
$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { .\build.ps1 fmt }
git add src/agent.rs src/main.rs .env.example tests/fixtures/claude-stream.jsonl
git commit -F - @'
agent: claude -p adapter, pinned to a captured stream-json run

Chosen by the executable name; read-only tools; settings, hooks and MCP off
so a research press costs ~6.5 K cache tokens instead of 37 K. Codex stays
as it was, unverified.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
'@
```

---

### Task 6: References that live in a folder and follow edits

**Files:**
- Modify: `src/references.rs`, `src/main.rs` (`--references` flag, construction, startup import; the preview passes `None`)

**Interfaces:**
- Produces: `References::new(tx: Sender<Msg>, folder: Option<PathBuf>) -> Self`; `References::import_folder(&self)`; `References::stale(&self) -> Vec<PathBuf>`; `Document.modified: SystemTime`; manifest `<folder>/.dropped`.

- [ ] **Step 1: Write the failing tests**

Add to `references::tests` (the module already has `use super::*;`):

```rust
    fn wait_until(refs: &References, f: impl Fn(&str) -> bool) -> String {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        loop {
            let p = refs.preview();
            if f(&p) || std::time::Instant::now() > deadline {
                return p;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("iv_refs_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
    #[test]
    fn folder_files_and_remembered_drops_return_at_startup() {
        let dir = folder("startup");
        std::fs::write(dir.join("brief.txt"), "Rollback owner is Sarah").unwrap();
        let outside = std::env::temp_dir().join(format!("iv_outside_{}.txt", std::process::id()));
        std::fs::write(&outside, "Recovery target is 20 minutes").unwrap();
        let (tx, _) = crossbeam_channel::unbounded();
        let refs = References::new(tx.clone(), Some(dir.clone()));
        refs.import_folder();
        wait_until(&refs, |p| p.contains("brief.txt"));
        // A drop from outside the folder is remembered by path, never copied.
        refs.import(vec![outside.clone()]);
        wait_until(&refs, |p| p.contains("iv_outside"));
        let manifest = std::fs::read_to_string(dir.join(".dropped")).unwrap();
        assert!(manifest.contains("iv_outside"), "{manifest}");
        assert!(!dir.join(outside.file_name().unwrap()).exists(), "copied instead of remembered");
        // A fresh session sees both again.
        let again = References::new(tx.clone(), Some(dir.clone()));
        again.import_folder();
        let p = wait_until(&again, |p| p.contains("brief.txt") && p.contains("iv_outside"));
        assert!(p.starts_with("References folder: "), "{p}");
        assert!(again.retrieve("rollback owner").contains("Sarah"));
        // Clear empties the index and the manifest; folder files come back next launch.
        again.clear();
        assert_eq!(std::fs::read_to_string(dir.join(".dropped")).unwrap().trim(), "");
        assert!(again.retrieve("rollback").is_empty());
    }
    #[test]
    fn an_edited_file_is_reimported_by_mtime_not_skipped_as_a_duplicate() {
        let dir = folder("edit");
        let file = dir.join("plan.txt");
        std::fs::write(&file, "owner is Sarah").unwrap();
        let (tx, _) = crossbeam_channel::unbounded();
        let refs = References::new(tx, Some(dir.clone()));
        refs.import_folder();
        wait_until(&refs, |p| p.contains("plan.txt"));
        assert!(refs.retrieve("owner").contains("Sarah"));
        // NTFS keeps 100 ns mtimes, but a same-tick rewrite is possible: wait.
        std::thread::sleep(std::time::Duration::from_millis(1_100));
        std::fs::write(&file, "owner is Marcus").unwrap();
        let stale = refs.stale();
        assert_eq!(stale, vec![file.canonicalize().unwrap()]);
        refs.import(stale);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        while !refs.retrieve("owner").contains("Marcus") && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(refs.retrieve("owner").contains("Marcus"));
        assert!(!refs.retrieve("owner").contains("Sarah"), "the old text was replaced, not appended");
        assert!(refs.stale().is_empty());
    }
```

Also update the existing `retrieval_selects_evidence_and_cites_source` and `deep_retrieval_returns_more_passages_but_stays_bounded` tests: `References::new(tx)` becomes `References::new(tx, None)`, and their `Document::new(PathBuf::from(...), chunks)` calls become `Document::new(PathBuf::from(...), chunks, std::time::SystemTime::UNIX_EPOCH)`.

- [ ] **Step 2: Run to verify they fail**

Run: `.\build.ps1 test --release references::`
Expected: compile errors — `new` arity, `import_folder`, `stale`.

- [ ] **Step 3: Implement**

In `src/references.rs`:

`Document` gains `modified: std::time::SystemTime`; `Document::new(path, chunks, modified)` stores it. `Library` gains `folder: Option<PathBuf>`.

`References::new(tx, folder: Option<PathBuf>)`: create the folder if `Some` and missing (`std::fs::create_dir_all`, ignore the error — `import_folder` reports it), store it in `Library`. The import worker's dedup arm changes from `if library.documents.iter().any(|d| d.path == document.path)` to:

```rust
                        Ok(document)
                            if library
                                .documents
                                .iter()
                                .any(|d| d.path == document.path && d.modified == document.modified) =>
                        {
                            format!("Already added: {name}")
                        }
                        // Same path, newer mtime: the file was edited. Replace it,
                        // or the panel keeps citing text that is no longer there.
                        Ok(document) if library.documents.iter().any(|d| d.path == document.path) => {
                            library.documents.retain(|d| d.path != document.path);
                            let count = document.chunks.len();
                            library.documents.push(document);
                            format!("Updated: {name} — {count} passages")
                        }
```

`fn load(path)` passes `metadata.modified()?` into `Document::new`.

New methods:

```rust
    /// Everything in the folder, plus every path the manifest remembers.
    pub fn import_folder(&self) {
        let Some(folder) = self.folder() else {
            return;
        };
        let mut paths: Vec<PathBuf> = match std::fs::read_dir(&folder) {
            Ok(entries) => entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_file() && crate::extract::supported(p))
                .collect(),
            Err(e) => {
                let _ = self.tx.send(Msg::ReferenceStatus(format!(
                    "Couldn't read {}: {e}",
                    folder.display()
                )));
                return;
            }
        };
        paths.sort();
        paths.extend(self.manifest().into_iter().map(PathBuf::from));
        self.import(paths);
    }
    /// Imported files whose file on disk has changed since.
    pub fn stale(&self) -> Vec<PathBuf> {
        let library = self.library.read().unwrap_or_else(|e| e.into_inner());
        library
            .documents
            .iter()
            .filter(|d| {
                d.path
                    .metadata()
                    .and_then(|m| m.modified())
                    .map_or(true, |m| m != d.modified)
            })
            .map(|d| d.path.clone())
            .collect()
    }
    fn folder(&self) -> Option<PathBuf> {
        self.library
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .folder
            .clone()
    }
    fn manifest(&self) -> Vec<String> {
        self.folder()
            .and_then(|f| std::fs::read_to_string(f.join(".dropped")).ok())
            .map(|s| s.lines().filter(|l| !l.trim().is_empty()).map(str::to_string).collect())
            .unwrap_or_default()
    }
```

`import(&self, paths)`: after queuing each path, remember drops from outside the folder — before the loop compute `let folder = self.folder().and_then(|f| f.canonicalize().ok());` and inside, after a successful `try_send`:

```rust
            // Remembered by path, never copied: a copy is stale the moment the
            // original is edited, and on-disk is the source of truth.
            if let Some(folder) = &folder
                && let Ok(canonical) = path.canonicalize()
                && !canonical.starts_with(folder)
            {
                let line = canonical.to_string_lossy().into_owned();
                if !self.manifest().contains(&line) {
                    let _ = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(folder.join(".dropped"))
                        .and_then(|mut f| {
                            use std::io::Write as _;
                            writeln!(f, "{line}")
                        });
                }
            }
```

(`folder.join(".dropped")` on the canonical folder path is fine: `\\?\`-prefixed paths open normally.)

`clear()`: after clearing documents, truncate the manifest: `if let Some(f) = self.folder() { let _ = std::fs::write(f.join(".dropped"), ""); }`, and change the status text to `References cleared. Original files were not changed; files in the references folder return next launch.`

`preview()`: when the folder is `Some`, prefix the output with `format!("References folder: {}\r\n\r\n", folder.display())` — for both the empty-state text and the document list.

`retrieve`, `retrieve_deep`, `local_answer`: at the top of each, `self.import(self.stale());` — the re-read happens on the import thread, so *this* retrieval may still use the old text and the next one will not; say so in a comment there.

In `src/main.rs`: add the flag

```rust
    /// Folder of reference files that persist across sessions and reload when edited
    #[arg(long, env = "IV_REFERENCES", default_value = "references")]
    references: String,
```

change `let references = references::References::new(ui_tx.clone());` to `let references = references::References::new(ui_tx.clone(), Some(std::path::PathBuf::from(&args.references)));` and add `references.import_folder();` right after it. In the `--preview` block, `references::References::new(tx.clone())` becomes `references::References::new(tx.clone(), None)` — the preview stays free of the user's files so the smoke test's assumptions hold.

- [ ] **Step 4: Run the tests**

Run: `.\build.ps1 test --release references::` → green (the two new tests take a few seconds: one waits 1.1 s deliberately).

- [ ] **Step 5: Build and smoke**

```powershell
Get-Process inner-voice -ErrorAction SilentlyContinue | Stop-Process -Force
.\build.ps1 build --release
$env:IV_UI_DUMP = "$env:TEMP\iv-ui.json"; Remove-Item $env:IV_UI_DUMP -ErrorAction SilentlyContinue
$p = Start-Process -PassThru -FilePath .\target\release\inner-voice.exe -ArgumentList '--preview'
Start-Sleep -Seconds 5
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\ui_smoke.ps1 -PreviewProcessId $p.Id -CloseAfterCheck
```

Expected: `PASS:`.

- [ ] **Step 6: Ladder and commit**

```powershell
.\build.ps1 clippy --release --all-targets
$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { .\build.ps1 fmt }
git add src/references.rs src/main.rs
git commit -F - @'
references: a folder that persists, drops remembered by path, edits followed

(path, mtime) is a document's identity now, so an edited file replaces its
old passages instead of being skipped as a duplicate. Nothing is copied.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
'@
```

---

### Task 7: `knowledge/` reloads mid-session (integration)

**Files:**
- Modify: `src/knowledge.rs` (`Corpus`, `newest`), `src/coach.rs` (`prompt` behind `RwLock`, `set_prompt`), `src/audio.rs` (`Tune.prompt: RwLock<String>`, `transcribe`), `src/main.rs` (`build_prompt`, `route()` refresh, wiring), `src/hud.rs` (notice filter)

**Interfaces:**
- Produces: `knowledge::Corpus { pub text: String, .. }` with `Corpus::load(dir: &Path) -> Result<Corpus>`, `Corpus::refresh(&mut self) -> Result<bool>`, `Corpus::files(&self) -> usize`; `knowledge::newest(dir: &Path) -> Option<SystemTime>`; `Coach::set_prompt(&self, prompt: String)`; `audio::Tune.prompt: std::sync::RwLock<String>`; `main::build_prompt(persona: &str, corpus: &str) -> String`.

- [ ] **Step 1: Write the failing tests**

In `knowledge::tests`:

```rust
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
```

In `coach::tests`:

```rust
    #[test]
    fn a_new_prompt_is_what_the_next_request_carries() {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let coach = Coach::new(refused(), "old".into(), tx);
        coach.set_prompt("new".into());
        assert_eq!(coach.prompt(), "new");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `.\build.ps1 test --release corpus_reloads` and `.\build.ps1 test --release a_new_prompt`
Expected: compile errors.

- [ ] **Step 3: knowledge.rs**

Add:

```rust
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
        self.text = load(&self.dir)?;
        self.files = count(&self.dir);
        self.newest = now;
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
```

- [ ] **Step 4: coach.rs**

`Coach` gains `prompt: Arc<std::sync::RwLock<String>>`. In `new`, `let prompt = Arc::new(std::sync::RwLock::new(prompt));` before the advice worker spawn; the worker captures a clone and, per job, reads it: replace `&prompt` in the `stream(...)` call with `&prompt.read().unwrap_or_else(|e| e.into_inner()).clone()` (bind it to a local first: `let current = prompt.read().unwrap_or_else(|e| e.into_inner()).clone();` then `&current`). Store the `Arc` in `Self`. Add:

```rust
    /// Swap the system prompt for every request after this one. The corpus
    /// changed on disk; a changed prompt busts the provider's cache prefix
    /// once, which is the point.
    pub fn set_prompt(&self, prompt: String) {
        *self.prompt.write().unwrap_or_else(|e| e.into_inner()) = prompt;
    }
    #[cfg(test)]
    fn prompt(&self) -> String {
        self.prompt.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
```

- [ ] **Step 5: audio.rs**

`Tune.prompt` becomes `pub prompt: std::sync::RwLock<String>` (doc comment: `/// Glossary primed into whisper's decoder — see knowledge::glossary. Behind a lock because the corpus can be re-read mid-call.`). `transcribe` becomes:

```rust
fn transcribe(state: &mut WhisperState, audio: &[f32], tune: &Tune) -> Result<String> {
    let prompt = tune.prompt.read().unwrap_or_else(|e| e.into_inner()).clone();
    state.full(params(&prompt), audio)?;
```

(rest unchanged). Every other `tune.prompt` read in the crate (`main.rs` prints it at startup: `tune.prompt.is_empty()`, `.matches(", ")`, `.trim_start_matches(...)`) reads through the lock the same way — bind `let glossary = tune.prompt.read().unwrap_or_else(|e| e.into_inner()).clone();` once in `main` and use `glossary` there.

- [ ] **Step 6: main.rs**

Add:

```rust
/// Persona plus the taught corpus, in the shape the coach pins behind its
/// cache breakpoint. One place, because the same text is built at startup and
/// again on every reload.
fn build_prompt(persona: &str, corpus: &str) -> String {
    if corpus.is_empty() {
        return persona.to_string();
    }
    format!(
        "{persona}\n\n# Source of truth\n\nThe following is what the user taught you \
         before this call. Treat it as authoritative and prefer it over your own \
         assumptions. If a fact is not here and not in the transcript, say so instead \
         of inventing one.\n{corpus}"
    )
}
```

Replace the startup block `let corpus = knowledge::load(...)?; let corpus_note = ...; prompt.push_str(...)` with:

```rust
    // Taught corpus is pinned into the system prompt, never retrieved — and
    // re-read on change, so on-disk stays the source of truth mid-call.
    let corpus = knowledge::Corpus::load(std::path::Path::new(&args.knowledge))?;
    let corpus_note = if corpus.text.is_empty() {
        "no knowledge/ corpus".to_string()
    } else {
        format!("knowledge: {} KB pinned", corpus.text.len() / 1024)
    };
    let persona = prompt;
    let prompt = build_prompt(&persona, &corpus.text);
```

(`let mut prompt = ...` above it becomes `let prompt = ...`.) `Tune { prompt: knowledge::glossary(&corpus), ... }` becomes `prompt: std::sync::RwLock::new(knowledge::glossary(&corpus.text)),`.

`ContextServices` gains `corpus: knowledge::Corpus, persona: String, tune: Arc<audio::Tune>`; pass `corpus`, `persona`, `tune.clone()` when constructing it. In `route`, destructure them as `let ContextServices { agent, references, research_prompt, mut corpus, persona, tune } = services;` (only the router mutates the corpus — a `let mut` in `main` would be an `unused_mut` warning), and add this free function:

```rust
/// Before anything goes to the coach: if the folder changed, the coach and
/// whisper both learn it now, and the panel says so.
fn refresh_corpus(
    corpus: &mut knowledge::Corpus,
    persona: &str,
    coach: &Option<coach::Coach>,
    tune: &audio::Tune,
    tx: &Sender<Msg>,
) {
    match corpus.refresh() {
        Ok(true) => {
            if let Some(coach) = coach {
                coach.set_prompt(build_prompt(persona, &corpus.text));
            }
            *tune.prompt.write().unwrap_or_else(|e| e.into_inner()) =
                knowledge::glossary(&corpus.text);
            let _ = tx.send(Msg::Sys(format!(
                "knowledge reloaded: {} files, {} KB",
                corpus.files(),
                corpus.text.len() / 1024
            )));
        }
        Ok(false) => {}
        Err(e) => {
            let _ = tx.send(Msg::Sys(format!("knowledge reload failed: {e:#}")));
        }
    }
}
```

Call `refresh_corpus(&mut corpus, &persona, &coach, &tune, &tx);` at the top of the `Msg::Question` arm, the `Msg::Research` arm, and in the `Msg::Turn` branch immediately before `if let Some(coach) = &coach && who.is_them() ...`.

In `src/hud.rs` `pump`'s `Msg::Sys` filter, add `|| text.starts_with("knowledge")`.

- [ ] **Step 7: Tests, build, smoke, a real run**

`.\build.ps1 test --release` → green. Smoke recipe (Task 6 step 5) → `PASS`. Then a real run proves the reload end to end:

```powershell
$p = Start-Process -PassThru -FilePath .\target\release\inner-voice.exe -ArgumentList '--provider','none','--log',"$env:TEMP\iv-reload"
Start-Sleep -Seconds 40
Add-Content knowledge\company.md "`nReload check line $(Get-Date -Format s)"
# speak or play any sentence so a THEM turn arrives, wait 15 s, then:
Get-ChildItem "$env:TEMP\iv-reload" -Filter *.jsonl | Get-Content | Select-String 'knowledge reloaded' | Select-Object -First 1
Stop-Process -Id $p.Id -Force
git checkout -- knowledge\company.md
```

Expected: a `knowledge reloaded: 3 files, N KB` line in the Diagnostics (the JSONL only carries turns; check the state mirror's `notice` or the panel's Diagnostics pane instead if the log does not show it — say which you used).

- [ ] **Step 8: Ladder and commit**

```powershell
.\build.ps1 clippy --release --all-targets
$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { .\build.ps1 fmt }
git add src/knowledge.rs src/coach.rs src/audio.rs src/main.rs src/hud.rs
git commit -F - @'
knowledge: re-read the folder before each coach request

On-disk is the source of truth: an edited brief reaches the next advice and
the next whisper prompt, and the panel says so.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
'@
```

---

### Task 8: Docs, LEDGER, final ladder

**Files:**
- Modify: `README.md`, `CLAUDE.md`, `LEDGER.md`, `.env.example`

- [ ] **Step 1: README**

In "Coaching", replace `Put call briefs in knowledge/: Markdown, text, CSV, XLSX, XLSM, XLS, and ODS are supported.` with `Put call briefs in knowledge/: Markdown, text, CSV, spreadsheets (XLSX/XLSM/XLS/ODS), PDF, Word (.docx) and images (read by Windows OCR) are supported.` and `Knowledge loads once at startup, sorted, capped at 400 KB with a truncation marker.` with `Knowledge is read at startup and re-read before the next advice whenever a file in the folder changes, so an edit mid-call reaches the coach; sorted, capped at 400 KB with a truncation marker.`

In "Controls and research", replace the paragraph starting `Drag TXT, Markdown, CSV, XLSX, XLSM, XLS, or ODS files into the window.` with:

```markdown
Drag any supported file into the window — TXT, Markdown, CSV, spreadsheets,
PDF (text layer; scanned pages need OCR first), Word (.docx), or images, which
Windows OCR reads if a language with OCR is installed. Files in `references/`
(`--references` / `IV_REFERENCES`) are imported at every start; a file dropped
from elsewhere is remembered by its path in `references/.dropped` and re-read
next start — never copied, so editing the original is enough. An edited file is
re-read before the next retrieval. Limits: 24 files, 10 MB per file, 400 KB
extracted text per file. Clear references empties the index and the remembered
paths; files in the folder return next launch.
```

In the research section, after the sentence about `--agent-cmd` switching research to that CLI, add:

```markdown
Claude Code's own binary works as that CLI on a subscription — no API key —
at `%APPDATA%\npm\node_modules\@anthropic-ai\claude-code\bin\claude.exe`
(the `claude` on PATH is a shim). It runs with settings, hooks and MCP servers
off and read-only tools only, and reads `knowledge/` and `references/` live
from disk. Its output format is pinned to a captured run
(`tests/fixtures/claude-stream.jsonl`). Codex remains supported but unverified.
```

- [ ] **Step 2: CLAUDE.md**

In the Architecture paragraph about the HUD, after `references.rs owns session-only imports and local passage retrieval.` change to `references.rs owns the references folder, the manifest of dropped paths, and local passage retrieval; extract.rs is the one path→text loader both it and knowledge.rs use.` Replace the "Knowledge corpus" section's first sentence with: `knowledge/ (.md .txt .csv .xlsx .xlsm .xls .ods .pdf .docx and images) is loaded at startup, pinned into the system prompt behind an Anthropic cache breakpoint, and re-read by route() before each coach request whenever the folder's newest mtime moves (knowledge::Corpus).` Add a trap bullet:

```markdown
- **`claude -p` costs depend entirely on what it loads.** The interactive
  harness (hooks, plugins, MCP, CLAUDE.md dirs) is 37 K cache tokens per call;
  `--strict-mcp-config --setting-sources ""` is 6.5 K and keeps the login.
  `--bare` drops the login. `result.subtype` is "success" even on failure —
  `is_error` is the signal. All measured; the fixture is the proof.
```

- [ ] **Step 3: LEDGER**

Add to the **Now** block, above the Wave 1 entry:

```markdown
**Wave 2 — memory (2026-09-06):** `extract.rs` is the one loader (txt/md/csv/
sheets/PDF/DOCX/OCR); `knowledge::Corpus` re-reads the folder before each
coach request when its newest mtime moves and pushes the new prompt to the
coach and the new glossary to whisper through `RwLock`s; references live in
`references/`, drops are remembered by path in `.dropped`, `(path, mtime)` is a
document's identity so an edit replaces its passages. The CLI research lane's
default adapter is Claude Code's `claude.exe` with settings/hooks/MCP off and
read-only tools, pinned to a captured `stream-json` run: 6.5 K cache tokens
and ~5 s per press versus 37 K / $0.75 through the interactive harness;
`--bare` drops the subscription login. Codex stays unverified.
```

Confirm Decisions 9 and 10 and the Decision 4/7 amendments from Wave 1 are present (Wave 1 Task 11 added them); if not, add them from spec §2.

- [ ] **Step 4: Final ladder**

```powershell
Get-Process inner-voice -ErrorAction SilentlyContinue | Stop-Process -Force
.\build.ps1 test --release
.\build.ps1 clippy --release --all-targets
$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { "FMT DRIFT: $out" } else { "fmt clean" }
# smoke (Task 6 step 5) -> PASS
git add README.md CLAUDE.md LEDGER.md .env.example
git commit -F - @'
docs: memory wave — one loader, persistent references, live corpus, claude lane

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
'@
git status --short   # empty
```
