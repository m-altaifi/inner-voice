# Wave 3 — Shippable Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A fresh machine goes from clone to a working call, and before a call the user knows what the app is using and that it works: `--setup` checks every prerequisite with the fix next to each ✗ and *measures* the configured provider's TTFT; the status line names the model so the on-screen `first word` number is attributable; the repo ignores user data and the docs lead with the check.

**Architecture:** One new module, `src/setup.rs`, runs the checks and renders them; it calls into what already exists (`audio::warm`, `audio::sessions`, `coach::probe`, `extract::ocr_available`, `provider::resolve`) rather than duplicating any of it. `hud::Session` gains `model: Option<String>`, which `status_text` already accepts. Tasks run sequentially, one fresh subagent each.

**Tech Stack:** Rust edition 2024; nothing new.

**Spec:** `docs/superpowers/specs/2026-09-06-complete-inner-voice-design.md` — §6 is this wave. Read it first.

## Global Constraints

- **Build only through `.\build.ps1`** (PowerShell tool, repo root), never bare cargo; kill a running preview before building (`Get-Process inner-voice -ErrorAction SilentlyContinue | Stop-Process -Force`).
- **Verification ladder on every task:** `.\build.ps1 test --release` green, `.\build.ps1 clippy --release --all-targets` zero warnings, `fmt --check` output empty (`$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { "DRIFT: $out" } else { "fmt clean" }`); tasks touching `hud.rs` or `main.rs` also run `tools/ui_smoke.ps1` (recipe in Task 3 step 4).
- **`--setup` sends exactly one tiny request to the configured provider** and nothing else leaves the machine; with `--provider none` it sends nothing.
- **No buttons, no view-switcher, never take focus.** The status label change is text only.
- **Comments explain why a simpler thing was rejected.**
- **Commit at the end of every task** on `main`, trailer `Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>`, `git commit -F -` with a here-string; never `--amend`, never `--no-verify`. Edit with the Edit tool; no re-encoding one-liners over sources containing `—`/`·`/`…`.

---

## File Structure

| File | Responsibility after this wave |
| --- | --- |
| `src/setup.rs` (new) | `Check`, `Inputs`, `run(inputs, enumerator) -> Vec<Check>`, `render(&[Check]) -> String`. |
| `src/coach.rs` | `pub fn probe(provider: &Provider) -> Result<Duration>` — one tiny request, time to first token. |
| `src/audio.rs` | `warm` becomes `pub`. |
| `src/main.rs` | `--setup` flag and dispatch; `pick` becomes `pub(crate)`; `Session.model` wired. |
| `src/hud.rs` | `Session.model: Option<String>`; `status_line` passes it. |
| `.gitignore`, `README.md`, `CLAUDE.md`, `LEDGER.md` | `references/` ignored; Setup leads with `--setup`; key-rotation checklist; Now block. |

---

### Task 1: `coach::probe` — one request, the time to first token

**Files:**
- Modify: `src/coach.rs` (new fn after `RESEARCH_MAX_TOKENS`; test)

**Interfaces:**
- Consumes: `request(agent, p, system, user, max_tokens, seq, live, sink) -> Result<bool>` (Wave 1).
- Produces: `pub fn probe(provider: &Provider) -> Result<Duration>`.

- [ ] **Step 1: Write the failing test**

Add to `coach::tests`:

```rust
    #[test]
    fn probe_reports_a_refused_provider_as_an_error_not_a_hang() {
        let started = std::time::Instant::now();
        let err = probe(&refused()).unwrap_err().to_string();
        assert!(!err.is_empty());
        assert!(started.elapsed() < Duration::from_secs(30), "a dead endpoint must fail fast");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `.\build.ps1 test --release probe_reports`
Expected: compile error — `probe` not found.

- [ ] **Step 3: Implement**

After `const RESEARCH_MAX_TOKENS` add:

```rust
/// One tiny request, timed to the first token. This is `--setup`'s evidence
/// that the configured provider answers and how fast — the same number the
/// status line shows as `first word` on a call, so a user choosing between
/// providers compares like with like.
pub fn probe(provider: &Provider) -> Result<Duration> {
    let agent = pooled_agent();
    let started = std::time::Instant::now();
    let mut first: Option<Duration> = None;
    let done = request(
        &agent,
        provider,
        "Reply with the single word OK.",
        "OK?",
        8,
        1,
        &AtomicU64::new(1),
        &mut |_| {
            first.get_or_insert_with(|| started.elapsed());
            Ok(())
        },
    )?;
    match (done, first) {
        (true, Some(ttft)) => Ok(ttft),
        _ => bail!("the provider answered without any text"),
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `.\build.ps1 test --release coach::` → green.

- [ ] **Step 5: Ladder and commit**

```powershell
.\build.ps1 clippy --release --all-targets
$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { .\build.ps1 fmt }
git add src/coach.rs
git commit -F - @'
coach: probe — one request, timed to the first token, for --setup

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
'@
```

---

### Task 2: `src/setup.rs` and `--setup`

**Files:**
- Create: `src/setup.rs`
- Modify: `src/audio.rs` (`fn warm` → `pub fn warm`), `src/main.rs` (`mod setup;`, `pick` → `pub(crate) fn pick`, the `--setup` flag, dispatch after `--list-apps`)

**Interfaces:**
- Consumes: `audio::warm(&mut WhisperState)`, `audio::sessions(&Device)`, `coach::probe(&Provider)`, `extract::ocr_available()`, `extract::supported(&Path)`, `provider::resolve(name, model)`, `main::pick(&DeviceEnumerator, Direction, &Option<String>) -> Result<Device>`.
- Produces: `setup::Check { name: &'static str, ok: bool, detail: String }`, `setup::Inputs { whisper, provider, model, mic, loopback, knowledge, references, agent_cmd }` (all `String`/`Option<String>`), `setup::run(inputs: &Inputs, enumerator: &DeviceEnumerator) -> Vec<Check>`, `setup::render(checks: &[Check]) -> String`.

- [ ] **Step 1: Write the failing test**

Create `src/setup.rs` with the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn render_marks_each_check_and_aligns_the_details() {
        let checks = [
            Check { name: "model", ok: true, detail: "models/x.bin (547 MB)".into() },
            Check { name: "ocr", ok: false, detail: "no Windows OCR language installed — add one".into() },
            Check { name: "research", ok: true, detail: "optional: not configured".into() },
        ];
        let text = render(&checks);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "  ✓ model       models/x.bin (547 MB)");
        assert_eq!(lines[1], "  ✗ ocr         no Windows OCR language installed — add one");
        assert!(text.ends_with("1 of 3 checks failed; fix the ✗ lines above.\n"), "{text:?}");
        assert!(!all_ok(&checks));
        assert!(all_ok(&checks[..1]));
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Add `mod setup;` after `mod search;` in `src/main.rs`. Run: `.\build.ps1 test --release setup::`
Expected: compile error.

- [ ] **Step 3: Write the module**

Above the tests in `src/setup.rs`:

```rust
//! `--setup`: every prerequisite, ✓ or ✗, with the fix printed next to it.
//!
//! Before a call is the only time a failing prerequisite is cheap. The provider
//! line *measures* time to first token rather than merely checking the key, so
//! choosing between providers is a decision made from evidence — the same
//! number the panel shows as `first word` on a live turn.
use crate::{audio, coach, extract, provider};
use std::fmt::Write as _;
use std::path::Path;
use wasapi::{DeviceEnumerator, Direction};
use whisper_rs::{WhisperContext, WhisperContextParameters};

pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

/// What `--setup` needs from the command line, copied out of `Args` so this
/// module does not depend on clap.
pub struct Inputs {
    pub whisper: String,
    pub provider: String,
    pub model: Option<String>,
    pub mic: Option<String>,
    pub loopback: Option<String>,
    pub knowledge: String,
    pub references: String,
    pub agent_cmd: Option<String>,
}

pub fn run(inputs: &Inputs, enumerator: &DeviceEnumerator) -> Vec<Check> {
    let mut checks = Vec::new();

    let model = Path::new(&inputs.whisper);
    checks.push(match model.metadata() {
        Ok(m) => Check {
            name: "model",
            ok: true,
            detail: format!("{} ({} MB)", inputs.whisper, m.len() / 1_000_000),
        },
        Err(_) => Check {
            name: "model",
            ok: false,
            detail: format!(
                "{} missing — curl.exe -fL -o {} https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
                inputs.whisper, inputs.whisper
            ),
        },
    });

    // "Does it run", not accuracy: one warm-up inference on the GPU, timed.
    checks.push(if model.exists() {
        let started = std::time::Instant::now();
        let mut params = WhisperContextParameters::default();
        params.flash_attn(true);
        match WhisperContext::new_with_params(&inputs.whisper, params)
            .and_then(|ctx| ctx.create_state())
        {
            Ok(mut state) => {
                audio::warm(&mut state);
                Check {
                    name: "cuda",
                    ok: true,
                    detail: format!("model loaded and warmed in {} ms", started.elapsed().as_millis()),
                }
            }
            Err(e) => Check {
                name: "cuda",
                ok: false,
                detail: format!("{e} — check the NVIDIA driver and CUDA install; see README"),
            },
        }
    } else {
        Check {
            name: "cuda",
            ok: false,
            detail: "skipped: no model to load".into(),
        }
    });

    for (name, dir, wanted) in [
        ("microphone", Direction::Capture, &inputs.mic),
        ("speakers", Direction::Render, &inputs.loopback),
    ] {
        checks.push(match crate::pick(enumerator, dir, wanted).and_then(|d| Ok(d.get_friendlyname()?)) {
            Ok(friendly) => Check { name, ok: true, detail: friendly },
            Err(e) => Check {
                name,
                ok: false,
                detail: format!("{e:#} — run --list-devices and set --mic / --loopback"),
            },
        });
    }

    checks.push(
        match crate::pick(enumerator, Direction::Render, &inputs.loopback)
            .and_then(|d| audio::sessions(&d))
        {
            Ok(apps) if apps.is_empty() => Check {
                name: "apps",
                ok: true,
                detail: "nothing is playing right now; --list-apps shows apps as they play".into(),
            },
            Ok(mut apps) => {
                apps.sort_by_key(|a| !a.active);
                let names: Vec<String> = apps
                    .iter()
                    .map(|a| if a.active { format!("{} (active)", a.name) } else { a.name.clone() })
                    .collect();
                Check {
                    name: "apps",
                    ok: true,
                    detail: format!("{} — --hear takes any part of a name", names.join(", ")),
                }
            }
            Err(e) => Check { name: "apps", ok: false, detail: format!("{e:#}") },
        },
    );

    checks.push(if inputs.provider == "none" {
        Check {
            name: "provider",
            ok: true,
            detail: "none — transcription only, nothing leaves the machine".into(),
        }
    } else {
        match provider::resolve(&inputs.provider, inputs.model.as_deref()) {
            Ok(p) => match coach::probe(&p) {
                Ok(ttft) => Check {
                    name: "provider",
                    ok: true,
                    detail: format!("{} {} — first token {} ms", inputs.provider, p.model, ttft.as_millis()),
                },
                Err(e) => Check {
                    name: "provider",
                    ok: false,
                    detail: format!("{} {} — {e:#}", inputs.provider, p.model),
                },
            },
            Err(e) => Check { name: "provider", ok: false, detail: format!("{e:#}") },
        }
    });

    checks.push(if extract::ocr_available() {
        Check { name: "ocr", ok: true, detail: "Windows OCR available for image references".into() }
    } else {
        Check {
            name: "ocr",
            ok: false,
            detail: "no Windows OCR language installed — Settings > Time & language > Language & region > add a language, then its Optical character recognition feature (only needed for image files)".into(),
        }
    });

    checks.push(folder("knowledge", &inputs.knowledge, false));
    checks.push(folder("references", &inputs.references, true));

    checks.push(match &inputs.agent_cmd {
        Some(exe) if Path::new(exe).is_file() => Check {
            name: "research",
            ok: true,
            detail: format!("F8 routes through {exe}"),
        },
        Some(exe) => Check {
            name: "research",
            ok: false,
            detail: format!("--agent-cmd {exe} not found"),
        },
        None => Check {
            name: "research",
            ok: true,
            detail: "optional: F8 uses the provider; set IV_AGENT_CMD to route it through a CLI".into(),
        },
    });

    checks
}

fn folder(name: &'static str, dir: &str, create: bool) -> Check {
    let path = Path::new(dir);
    if !path.is_dir() {
        if create && std::fs::create_dir_all(path).is_ok() {
            return Check { name, ok: true, detail: format!("{dir} — created, empty") };
        }
        return Check {
            name,
            ok: !create,
            detail: format!("{dir} not found{}", if create { "" } else { " — optional; briefs go here" }),
        };
    }
    let (mut files, mut bytes) = (0usize, 0u64);
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() && extract::supported(&p) {
                files += 1;
                bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    Check { name, ok: true, detail: format!("{dir} — {files} files, {} KB", bytes / 1024) }
}

pub fn all_ok(checks: &[Check]) -> bool {
    checks.iter().all(|c| c.ok)
}

pub fn render(checks: &[Check]) -> String {
    let mut out = String::new();
    for c in checks {
        let _ = writeln!(out, "  {} {:<11} {}", if c.ok { '✓' } else { '✗' }, c.name, c.detail);
    }
    let failed = checks.iter().filter(|c| !c.ok).count();
    if failed == 0 {
        out.push_str("all checks passed.\n");
    } else {
        let _ = writeln!(out, "{failed} of {} checks failed; fix the ✗ lines above.", checks.len());
    }
    out
}
```

`crate::pick` — in `src/main.rs` change `fn pick(` to `pub(crate) fn pick(`. In `src/audio.rs` change `fn warm(` to `pub fn warm(`.

- [ ] **Step 4: The flag and dispatch**

In `Args`, next to `list_apps`:

```rust
    /// Check every prerequisite — model, CUDA, devices, provider (one tiny
    /// request, timed), OCR, folders — print ✓/✗ with the fix, and exit
    #[arg(long)]
    setup: bool,
```

After the `if args.list_apps { ... }` block:

```rust
    if args.setup {
        let checks = setup::run(
            &setup::Inputs {
                whisper: args.whisper.clone(),
                provider: args.provider.clone(),
                model: args.model.clone(),
                mic: args.mic.clone(),
                loopback: args.loopback.clone(),
                knowledge: args.knowledge.clone(),
                references: args.references.clone(),
                agent_cmd: args.agent_cmd.as_ref().map(|p| p.display().to_string()),
            },
            &enumerator,
        );
        print!("\ninner-voice --setup\n\n{}", setup::render(&checks));
        std::process::exit(if setup::all_ok(&checks) { 0 } else { 1 });
    }
```

(`Args` must derive nothing new; `provider` is a `String` already; `references` exists from Wave 2.)

- [ ] **Step 5: Tests, then run it for real**

`.\build.ps1 test --release setup::` → green. Then:

```powershell
Get-Process inner-voice -ErrorAction SilentlyContinue | Stop-Process -Force
.\build.ps1 build --release
.\target\release\inner-voice.exe --setup; "exit=$LASTEXITCODE"
.\target\release\inner-voice.exe --setup --provider none; "exit=$LASTEXITCODE"
```

Expected: ten lines with ✓/✗, the provider line showing `first token N ms` (with the `.env` default provider) or a clear failure, `cuda` with a warm-up time, exit 0 when all pass. Paste the output into the report.

- [ ] **Step 6: Ladder and commit**

```powershell
.\build.ps1 clippy --release --all-targets
$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { .\build.ps1 fmt }
git add src/setup.rs src/main.rs src/audio.rs
git commit -F - @'
setup: --setup checks every prerequisite and times the provider

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
'@
```

---

### Task 3: The status line names the model

**Files:**
- Modify: `src/hud.rs` (`Session`, `status_line`), `src/main.rs` (both `hud::Session { .. }` constructions)

**Interfaces:**
- Consumes: `status_text(mode, wait, model: Option<&str>, …)` (Wave 1).
- Produces: `hud::Session.model: Option<String>`.

- [ ] **Step 1: Implement**

In `src/hud.rs`, `pub struct Session` gains `/// Provider model in the status line, so the on-screen TTFT is attributable.` `pub model: Option<String>,`. In `status_line`, the `None` argument to `status_text` becomes `self.session.model.as_deref()`.

In `src/main.rs`: before `let coach = provider.map(|p| coach::Coach::new(p, prompt, turn_tx.clone()));` add `let model = provider.as_ref().map(|p| p.model.clone());`, and in the `hud::Session { .. }` at the end add `model,`. In the `--preview` block's `hud::Session { .. }` add `model: None,`.

- [ ] **Step 2: Build, test, smoke**

`.\build.ps1 test --release` → green (the `status_text` test already covers `Some(model)`). Smoke:

```powershell
Get-Process inner-voice -ErrorAction SilentlyContinue | Stop-Process -Force
.\build.ps1 build --release
$env:IV_UI_DUMP = "$env:TEMP\iv-ui.json"; Remove-Item $env:IV_UI_DUMP -ErrorAction SilentlyContinue
$p = Start-Process -PassThru -FilePath .\target\release\inner-voice.exe -ArgumentList '--preview'
Start-Sleep -Seconds 5
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\ui_smoke.ps1 -PreviewProcessId $p.Id -CloseAfterCheck
```

→ `PASS`. Then one real run with the `.env` provider for 20 s and a screenshot of the top strip showing `Listening · <model>`:

```powershell
$p = Start-Process -PassThru -FilePath .\target\release\inner-voice.exe
Start-Sleep -Seconds 30
Add-Type -AssemblyName System.Drawing
Add-Type 'using System;using System.Runtime.InteropServices;public class R{[StructLayout(LayoutKind.Sequential)]public struct T{public int L,Tp,Rr,B;}[DllImport("user32.dll")]public static extern bool GetWindowRect(IntPtr h,out T r);}'
$w = (Get-Process -Id $p.Id).MainWindowHandle; $r = New-Object R+T; [void][R]::GetWindowRect($w,[ref]$r)
$bmp = New-Object System.Drawing.Bitmap ($r.Rr-$r.L), 60; $g=[System.Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($r.L,$r.Tp,0,0,$bmp.Size); $bmp.Save("$env:TEMP\status.png"); $g.Dispose(); $bmp.Dispose()
Stop-Process -Id $p.Id -Force
```

Open `$env:TEMP\status.png` with the Read tool; the status line must read `Listening · <model name>` (no `coaching on`).

- [ ] **Step 3: Ladder and commit**

```powershell
.\build.ps1 clippy --release --all-targets
$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { .\build.ps1 fmt }
git add src/hud.rs src/main.rs
git commit -F - @'
hud: the status line names the model

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
'@
```

---

### Task 4: `.gitignore`, README, CLAUDE.md, LEDGER — and the final ladder

**Files:**
- Modify: `.gitignore`, `README.md`, `CLAUDE.md`, `LEDGER.md`

- [ ] **Step 1: `.gitignore`**

Append `references/` (user data, never the repo's) and `tests/fixtures/*.tmp`.

- [ ] **Step 2: README**

Replace the "Setup" section's command block so it reads:

```markdown
```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\build.ps1 build --release
# First setup only; preserve an existing .env:
Copy-Item .env.example .env
New-Item -ItemType Directory -Force models
curl.exe -fL -o models/ggml-large-v3-turbo-q5_0.bin https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin
.\target\release\inner-voice.exe --setup
```

`--setup` checks the model, CUDA (one warm-up inference, timed), your microphone
and speakers, which apps are playing, the configured provider (one tiny request,
with the time to first token — the same number the panel shows as `first word`),
Windows OCR, and the `knowledge/` and `references/` folders, printing ✓ or ✗ with
the fix next to each. Run it again after changing `.env`; compare providers by
the number it prints. Exit code 1 means something is ✗.
```

(Keep the paragraph about `build.ps1` discovering Visual Studio that follows.) In "Controls and research", after `The status line counts the wait for advice …` sentence add: `It also names the model in use, so the number is attributable.`

- [ ] **Step 3: CLAUDE.md**

Under "Build and test" add a line: `- `.\target\release\inner-voice.exe --setup` is the first thing to run on a new machine or after editing `.env`: it loads the model, warms CUDA, lists devices and playing apps, times the provider's first token, and checks OCR and the folders.` Under Architecture add: `setup.rs runs the --setup checks and renders them; it calls audio::warm, audio::sessions, coach::probe and extract::ocr_available rather than duplicating any of them.`

- [ ] **Step 4: LEDGER**

Add to the **Now** block, above the Wave 2 entry:

```markdown
**Wave 3 — shippable (2026-09-06):** `--setup` (`setup.rs`) checks model, CUDA
(timed warm-up), devices, playing apps, provider (`coach::probe`: one tiny
request, TTFT in ms), OCR, and the two folders, ✓/✗ with the fix, exit 1 on any
✗. The status line names the model, so the on-screen `first word` number is
attributable and providers are compared from evidence. `references/` is
git-ignored. **Still the user's to do:** rotate the OpenRouter and Gemini keys
that appear in an earlier chat transcript (Machine facts above); the real
acceptance call with `--hear <app> --speak --dump clips`.
```

- [ ] **Step 5: Final ladder**

```powershell
Get-Process inner-voice -ErrorAction SilentlyContinue | Stop-Process -Force
.\build.ps1 test --release
.\build.ps1 clippy --release --all-targets
$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { "FMT DRIFT: $out" } else { "fmt clean" }
# smoke (Task 3 step 2) -> PASS
.\target\release\inner-voice.exe --setup; "exit=$LASTEXITCODE"
git add .gitignore README.md CLAUDE.md LEDGER.md
git commit -F - @'
docs: shippable — --setup first, model in the status line, references/ ignored

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
'@
git status --short   # empty
```
