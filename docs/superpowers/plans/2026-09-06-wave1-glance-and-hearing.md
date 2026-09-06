# Wave 1 — Glance & Hearing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the panel meet the two-second-glance rule and hear only the call, so the user can run the acceptance call: Decision 7 restored in egui, per-app audio capture, `--speak` unmuted, research working over HTTP without Codex.

**Architecture:** Three independent tracks on disjoint files — `hud.rs` (A), `coach.rs`/`provider.rs`/`references.rs` (C), `audio.rs`/`speak.rs` (B) — followed by one integration track (I1) that wires `main.rs`, updates docs, runs the live checks and commits. Tasks run **sequentially in the main tree, one fresh subagent each**, in the order A → C → B → I1. (The spec calls the tracks "parallel" because they are independent; running them in separate worktrees would mean three full CUDA whisper builds and cargo serialising them on its lock anyway, so sequential is faster in wall-clock and has no merges.)

**Tech Stack:** Rust edition 2024 (let-chains are used throughout), `eframe`/`egui` 0.36 (`egui::Panel`, `App::ui`), `wasapi` 0.24 (`AudioClient::new_application_loopback_client`, session enumeration), `windows` 0.62.2, `ureq` 3 SSE streaming, `crossbeam-channel`, whisper-rs + CUDA.

**Spec:** `docs/superpowers/specs/2026-09-06-complete-inner-voice-design.md` — §4 is this wave. Read it first; the plan argues from it.

## Global Constraints

- **Build only through `.\build.ps1`** (PowerShell, from the repo root), never bare `cargo`. It sets the CUDA/MSVC environment. Everything after the script name is forwarded to cargo: `.\build.ps1 test --release`, `.\build.ps1 clippy --release --all-targets`, `.\build.ps1 fmt -- --check`, `.\build.ps1 build --release`.
- **Kill a running preview before building**: `Get-Process inner-voice -ErrorAction SilentlyContinue | Stop-Process -Force`. A live `inner-voice.exe` holds the file and the build fails with "Access is denied".
- **Verification ladder on every task, no exceptions:** `.\build.ps1 test --release` all green, `.\build.ps1 clippy --release --all-targets` with zero warnings, `.\build.ps1 fmt -- --check` empty. Tasks touching `hud.rs`, `audio.rs` or `main.rs` additionally run `tools/ui_smoke.ps1` against a live `--preview` (recipe in Task 2 step 8). A green `cargo test` alone has twice meant nothing in this project.
- **`fmt --check`'s exit code does not propagate through `build.ps1`.** Check its *output*: `$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { "DRIFT: $out" } else { "fmt clean" }`. If it drifts, run `.\build.ps1 fmt` and re-check.
- **No buttons, no view-switcher, no control that must be focused to be used** (LEDGER Decision 8). A new action is a row in `KEYS`, and this wave adds none.
- **Never make the panel take focus** except inside `borrow_keyboard` for the question box. `keep_unfocusable` re-asserts `WS_EX_NOACTIVATE` every frame — leave it.
- **Windows-only by design.** `wasapi`, `windows`, SAPI. No cross-platform shims.
- **Comments explain why a simpler thing was rejected**, not what the code does. Match the surrounding density.
- **Commit at the end of every task** on `main`, message in the imperative, ending with the trailer line `Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>`. Use `git commit -F -` with a here-string; never `--amend`, never `--no-verify`.
- File ownership: Tasks 1–2 touch only `src/hud.rs` and `tools/ui_smoke.ps1`. Tasks 3–5 only `src/coach.rs`, `src/provider.rs`, `src/references.rs`. Tasks 6–8 only `src/audio.rs`, `src/speak.rs`. Tasks 9–13 may touch anything. Stay inside your task's files, except where a step explicitly instructs a one-line compile fix elsewhere (Task 7 step 2, Task 8 step 2); if you believe you must touch another file beyond that, stop and say so instead.

---

## File Structure

| File | Responsibility after this wave |
| --- | --- |
| `src/hud.rs` | The egui panel. Gains `Mode`/`Wait`/`status_text` (status-line grammar as a pure function), `Tag` (tag → heading + colours, the single source `display_advice` reads), TTFT timing fields, the greyed-while-thinking rule, `THEM`-brighter contrast, the 10 s key hint. |
| `src/coach.rs` | Fast lane unchanged in behaviour; `consume` becomes sink-based and returns whether it completed; `body` takes `max_tokens`; new research lane (`Coach::research`, `Coach::cancel_research`, `RESEARCH_PROMPT`) on its own worker. |
| `src/provider.rs` | `Provider` derives `Clone` (the research worker needs its own copy). |
| `src/references.rs` | `retrieve_deep` (top 40 passages, 60 KB cap) beside `retrieve` (top 4). |
| `src/audio.rs` | `AppSession`, `sessions()`, `resolve()`; `Input.hear`; `open_endpoint`/`open_process`; the capture loop extracted to `pump` with a liveness hook; per-app reacquire loop. |
| `src/speak.rs` | `Speaker::new(mute: Option<Arc<AtomicBool>>)`. |
| `src/main.rs` | `--hear`, `--list-apps`, `--research-prompt`; wiring; research routing to HTTP when no `--agent-cmd`; notices. |
| `tools/ui_smoke.ps1` | Asserts the `wait` mirror field. |
| `tools/hear_isolation.ps1` | New live check: two processes play speech, `--hear` one, only its words become `THEM`. |
| `README.md`, `CLAUDE.md`, `LEDGER.md`, `.env.example` | Documented per spec §4 and §2. |

---

### Task 1: Status line as a pure function, with the wait shown as a number

**Files:**
- Modify: `src/hud.rs` — imports (line 9–14), `State` (line 238), `pump` (`AdviceStart`/`Advice` arms ~line 358–368), `command` `PAUSE` arm (~line 407), `status_line` (line 450–475), `mirror_state` (line 641–662), the status label in `ui()` (search for `self.status_line()`), the `State {` initialiser inside `run` (search for `pending: Vec::new(),`), tests module (line 950).
- Modify: `tools/ui_smoke.ps1:126` (after the `Preview seeded no advice` check).

**Interfaces:**
- Consumes: nothing new.
- Produces: `enum Mode { Preview, Paused, Listening { online: bool } }`, `enum Wait { Idle, Thinking(f32), FirstWord(f32) }`, `fn status_text(mode: Mode, wait: Wait, model: Option<&str>, researching: bool, pinned: bool, hint: bool) -> String`, `State::wait(&self) -> Wait`, `const HINT_SECS: u64 = 10`, mirror field `"wait"`. Sub-project 3 will pass `Some(model)`; this wave passes `None`.

- [ ] **Step 1: Write the failing test**

Append inside `mod tests` in `src/hud.rs`:

```rust
    #[test]
    fn status_line_counts_the_wait_and_only_hints_early() {
        assert_eq!(
            status_text(Mode::Listening { online: true }, Wait::Thinking(1.42), None, false, false, false),
            "Listening · coaching on · thinking 1.4s"
        );
        assert_eq!(
            status_text(Mode::Listening { online: true }, Wait::FirstWord(1.2), None, false, true, false),
            "Listening · coaching on · first word 1.2s · pinned"
        );
        // The hint is the last thing on the line and only while it is shown.
        assert_eq!(
            status_text(Mode::Preview, Wait::Idle, None, false, false, true),
            "Preview — microphone off · no online requests   ·   Ctrl+Shift+F11 for keys"
        );
        // A named model replaces the generic "coaching on" (sub-project 3 passes it).
        assert_eq!(
            status_text(Mode::Listening { online: true }, Wait::Idle, Some("gemini-3.5-flash-lite"), true, false, false),
            "Listening · gemini-3.5-flash-lite · researching"
        );
        assert_eq!(
            status_text(Mode::Listening { online: false }, Wait::Idle, None, false, false, false),
            "Listening · transcription only"
        );
        assert_eq!(
            status_text(Mode::Paused, Wait::Thinking(9.0), None, false, false, false),
            "Paused — audio is not being transcribed · thinking 9.0s"
        );
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `.\build.ps1 test --release status_line_counts_the_wait`
Expected: compile error — `Mode`, `Wait`, `status_text` not found.

- [ ] **Step 3: Add the types and the pure function**

Replace the `use std::{ collections::VecDeque, path::PathBuf, sync::{...} };` block's `std` import so it also brings in time (keep the existing items):

```rust
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
```

Immediately after `const VISIBLE_TURNS: f32 = 8.0;` add:

```rust
/// How long the status line carries `Ctrl+Shift+F11 for keys` after launch.
///
/// A permanent hint is permanent noise (LEDGER Decision 7); the empty advice
/// pane already names the key list, and the list is one key away.
const HINT_SECS: u64 = 10;

/// What the panel is doing, for the status line.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Preview,
    Paused,
    Listening { online: bool },
}

/// Where the current advice is in its wait, for the status line.
///
/// Doherty: the loop is ~1–2 s, well past the 400 ms "instant" line, so the
/// panel counts the wait instead of spinning — `thinking 1.4s` live, then
/// `first word 1.2s` frozen. The frozen number is the standing TTFT
/// instrument on every turn of a real call.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Wait {
    Idle,
    Thinking(f32),
    FirstWord(f32),
}

/// The status line, as a pure function so its grammar is testable.
///
/// Order: mode · model · wait · researching · pinned · hint. A named model
/// replaces "coaching on" rather than joining it; the hint is always last.
fn status_text(
    mode: Mode,
    wait: Wait,
    model: Option<&str>,
    researching: bool,
    pinned: bool,
    hint: bool,
) -> String {
    let mut s = match (mode, model) {
        (Mode::Preview, _) => "Preview — microphone off · no online requests".to_string(),
        (Mode::Paused, _) => "Paused — audio is not being transcribed".to_string(),
        (Mode::Listening { online: true }, Some(model)) => format!("Listening · {model}"),
        (Mode::Listening { online: true }, None) => "Listening · coaching on".to_string(),
        (Mode::Listening { online: false }, _) => "Listening · transcription only".to_string(),
    };
    match wait {
        Wait::Idle => {}
        Wait::Thinking(secs) => s.push_str(&format!(" · thinking {secs:.1}s")),
        Wait::FirstWord(secs) => s.push_str(&format!(" · first word {secs:.1}s")),
    }
    if researching {
        s.push_str(" · researching");
    }
    if pinned {
        s.push_str(" · pinned");
    }
    if hint {
        s.push_str("   ·   Ctrl+Shift+F11 for keys");
    }
    s
}
```

- [ ] **Step 4: Record the timing in `State` and wire `status_line` through it**

In `struct State`, after `drag: Option<Dragging>,` add:

```rust
    /// When the panel opened; the key hint leaves `HINT_SECS` later.
    launched: Instant,
    /// When the live generation's `AdviceStart` arrived.
    asked_at: Option<Instant>,
    /// When its first token landed. `asked_at → first_word_at` is the TTFT.
    first_word_at: Option<Instant>,
```

In the `State { ... }` initialiser inside `run` (the block containing `pending: Vec::new(),`), add alongside `drag: None,`:

```rust
                launched: Instant::now(),
                asked_at: None,
                first_word_at: None,
```

In `pump`, replace the `AdviceStart` and `Advice` arms with:

```rust
                Msg::AdviceStart(id) => {
                    self.seq = id;
                    self.thinking = true;
                    self.asked_at = Some(Instant::now());
                    self.first_word_at = None;
                }
                Msg::Advice(id, text) if id == self.seq => {
                    if self.thinking {
                        self.advice.clear();
                        self.thinking = false;
                        self.first_word_at = Some(Instant::now());
                    }
                    self.advice.push_str(&text);
                }
```

In `command`, the `PAUSE` arm currently sets `self.seq = u64::MAX; self.thinking = false;` — add after those two lines:

```rust
                self.asked_at = None;
                self.first_word_at = None;
```

Replace the whole `fn status_line(&self) -> String { ... }` with:

```rust
    fn wait(&self) -> Wait {
        match (self.asked_at, self.first_word_at) {
            (Some(asked), Some(first)) => Wait::FirstWord(first.duration_since(asked).as_secs_f32()),
            (Some(asked), None) if self.thinking => Wait::Thinking(asked.elapsed().as_secs_f32()),
            _ => Wait::Idle,
        }
    }
    fn status_line(&self) -> String {
        let mode = if self.session.preview {
            Mode::Preview
        } else if self.paused {
            Mode::Paused
        } else {
            Mode::Listening {
                online: self.session.online,
            }
        };
        status_text(
            mode,
            self.wait(),
            None,
            self.researching,
            self.pinned,
            self.launched.elapsed() < Duration::from_secs(HINT_SECS),
        )
    }
```

In `ui()`, find `ui.label(RichText::new(self.status_line()).color(MUTED).size(13.0));` and make it monospace so the counting number does not twitch:

```rust
                ui.label(RichText::new(self.status_line()).color(MUTED).size(13.0).monospace());
```

In `mirror_state`, add to the `json!` object after `"dragging": self.drag.is_some(),`:

```rust
            "wait": match self.wait() {
                Wait::Idle => "",
                Wait::Thinking(_) => "thinking",
                Wait::FirstWord(_) => "first word",
            },
```

- [ ] **Step 5: Run the unit test**

Run: `.\build.ps1 test --release status_line_counts_the_wait`
Expected: PASS. Then `.\build.ps1 test --release` — all green (the existing `every_action_is_reachable_by_exactly_one_key` etc. still pass).

- [ ] **Step 6: Make the smoke test assert the number is on screen**

In `tools/ui_smoke.ps1`, directly after line `if ($state.advice -notmatch 'ASK') { throw 'Preview seeded no advice' }` add:

```powershell
# --preview seeds AdviceStart -> Advice -> AdviceEnd before the panel exists, so
# the TTFT instrument reads `first word 0.0s`. That it reads at all is the check.
if ($state.wait -ne 'first word') { throw "Status line is not showing the TTFT number: wait='$($state.wait)'" }
```

- [ ] **Step 7: Build and run the smoke test**

```powershell
Get-Process inner-voice -ErrorAction SilentlyContinue | Stop-Process -Force
.\build.ps1 build --release
$env:IV_UI_DUMP = "$env:TEMP\iv-ui.json"; Remove-Item $env:IV_UI_DUMP -ErrorAction SilentlyContinue
$p = Start-Process -PassThru -FilePath .\target\release\inner-voice.exe -ArgumentList '--preview'
Start-Sleep -Seconds 5
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\ui_smoke.ps1 -PreviewProcessId $p.Id -CloseAfterCheck
```

Expected: a line starting `PASS:`. Then open `tools/preview-advice.png` and confirm the status line reads `Preview — microphone off · no online requests · first word 0.0s   ·   Ctrl+Shift+F11 for keys` in monospace.

- [ ] **Step 8: Ladder and commit**

```powershell
.\build.ps1 clippy --release --all-targets
$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { .\build.ps1 fmt } 
git add src/hud.rs tools/ui_smoke.ps1
git commit -F - @'
hud: show the advice wait as a number, status line as a pure function

Decision 7 (Doherty): `thinking 1.4s` live, `first word 1.2s` frozen — the
standing TTFT instrument. Replaces the "preparing advice…" spinner text.
The key hint now leaves after 10 s.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
'@
```

---

### Task 2: Tag colours, greyed-while-thinking, THEM brighter than YOU

**Files:**
- Modify: `src/hud.rs` — colour consts (line 41–44), `display_advice` (line 205–226), `conversation` (line 478–501), `main_pane` `TRANSCRIPT` arm (line 512–518) and the advice arm (line 562–570), tests `advice_labels_are_readable_without_losing_content` (line 1009).

**Interfaces:**
- Consumes: nothing new.
- Produces: `enum Tag { Ask, Say, Note, Fix }` with `Tag::TABLE`, `Tag::heading(self) -> &'static str`, `Tag::colours(self) -> (Color32, Color32)`; `display_advice(&str) -> Vec<(Option<Tag>, String)>`; consts `AMBER`, `SOFT_GREEN`.

- [ ] **Step 1: Write the failing tests**

Replace the existing `advice_labels_are_readable_without_losing_content` test with these two:

```rust
    #[test]
    fn advice_labels_are_readable_without_losing_content() {
        let shown =
            display_advice("ASK What is the rollback plan?\nASKING is not a tag\nNOTE Owner unclear");
        assert_eq!(shown[0], (Some(Tag::Ask), "What is the rollback plan?".into()));
        // Only the exact tag followed by whitespace becomes a heading.
        assert_eq!(shown[1], (None, "ASKING is not a tag".into()));
        assert_eq!(shown[2], (Some(Tag::Note), "Owner unclear".into()));
        assert_eq!(Tag::Ask.heading(), "ASK NEXT");
        assert_eq!(Tag::Fix.heading(), "CLARIFY");
    }
    #[test]
    fn only_ask_and_fix_are_saturated() {
        // Von Restorff: colour every line and nothing stands out.
        assert_eq!(Tag::Ask.colours().0, ACCENT);
        assert_eq!(Tag::Fix.colours().0, AMBER);
        assert_eq!(Tag::Say.colours().0, SOFT_GREEN);
        assert_eq!(Tag::Note.colours(), (MUTED, MUTED));
        assert_ne!(Tag::Say.colours().0, ACCENT);
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `.\build.ps1 test --release only_ask_and_fix`
Expected: compile error — `Tag`, `AMBER`, `SOFT_GREEN` not found.

- [ ] **Step 3: Add the tag table and colours**

After `const ACCENT: Color32 = ...;` add:

```rust
const AMBER: Color32 = Color32::from_rgb(0xff, 0xb4, 0x54);
const SOFT_GREEN: Color32 = Color32::from_rgb(0x9f, 0xd8, 0xa0);
```

Replace `fn display_advice` (and its doc comment) with:

```rust
/// The four line tags `prompt.md` emits, and how the panel shows each.
///
/// This table is the only place the tag vocabulary lives on the panel side:
/// change it in one place and it silently renders as body text in the other.
/// The literals are matched exactly, case-sensitively and only when followed
/// by whitespace, so a sentence beginning "ASKING" is left alone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tag {
    Ask,
    Say,
    Note,
    Fix,
}

impl Tag {
    const TABLE: [(Tag, &'static str, &'static str); 4] = [
        (Tag::Ask, "ASK", "ASK NEXT"),
        (Tag::Say, "SAY", "SUGGESTED WORDING"),
        (Tag::Note, "NOTE", "KEEP IN MIND"),
        (Tag::Fix, "FIX", "CLARIFY"),
    ];
    fn heading(self) -> &'static str {
        Self::TABLE
            .iter()
            .find(|(tag, ..)| *tag == self)
            .map_or("", |(_, _, heading)| *heading)
    }
    /// `(heading, body)` colours. Von Restorff: only `ASK` and `FIX` are
    /// saturated, because colouring every line makes nothing stand out.
    /// `NOTE` recedes entirely — it is the thing to hold, not the thing to do.
    fn colours(self) -> (Color32, Color32) {
        match self {
            Tag::Ask => (ACCENT, FG),
            Tag::Fix => (AMBER, FG),
            Tag::Say => (SOFT_GREEN, FG),
            Tag::Note => (MUTED, MUTED),
        }
    }
}

/// Split advice into tagged sections the panel can style.
fn display_advice(text: &str) -> Vec<(Option<Tag>, String)> {
    text.lines()
        .map(|line| {
            let line = line.trim();
            for (tag, literal, _) in Tag::TABLE {
                if let Some(rest) = line
                    .strip_prefix(literal)
                    .filter(|s| s.starts_with(char::is_whitespace))
                {
                    return (Some(tag), rest.trim().to_string());
                }
            }
            (None, line.to_string())
        })
        .filter(|(tag, body)| tag.is_some() || !body.is_empty())
        .collect()
}
```

- [ ] **Step 4: Paint with the table, grey while thinking, brighten THEM**

Replace the advice arm at the end of `main_pane`'s `match` (the `_ => { for (heading, body) in display_advice(&self.advice) { ... } }` block) with:

```rust
                _ => {
                    // Never blank: the last turn's advice stays up while the
                    // next one is prepared, greyed so the eye knows it is old.
                    let stale = self.thinking;
                    for (tag, body) in display_advice(&self.advice) {
                        let (head, text) = match (tag, stale) {
                            (_, true) => (MUTED, MUTED),
                            (Some(tag), false) => tag.colours(),
                            (None, false) => (FG, FG),
                        };
                        if let Some(tag) = tag {
                            ui.add_space(6.0);
                            ui.label(RichText::new(tag.heading()).color(head).strong().size(13.0));
                        }
                        ui.label(RichText::new(body).color(text).size(19.0));
                    }
                }
```

In `conversation`, replace the `for (who, text) in &self.transcript { ... }` loop with:

```rust
                for (who, text) in &self.transcript {
                    // Miller: THEM brighter than YOU, because THEM is what you
                    // react to. Your own words are context, not a prompt.
                    let (label, body) = if who == "YOU" { (MUTED, MUTED) } else { (ACCENT, FG) };
                    ui.horizontal_top(|ui| {
                        ui.add_sized(
                            [72.0, ui.text_style_height(&TextStyle::Body)],
                            egui::Label::new(RichText::new(who).color(label).monospace()),
                        );
                        ui.label(RichText::new(text).color(body));
                    });
                }
```

In `main_pane`'s `TRANSCRIPT` arm, replace the loop body with the same contrast rule:

```rust
                TRANSCRIPT => {
                    for (who, text) in &self.transcript {
                        let (label, body) = if who == "YOU" { (MUTED, MUTED) } else { (ACCENT, FG) };
                        ui.label(RichText::new(who).color(label).monospace());
                        ui.label(RichText::new(text).color(body));
                        ui.add_space(8.0);
                    }
                }
```

- [ ] **Step 5: Run the tests**

Run: `.\build.ps1 test --release`
Expected: all green, including the two tests from step 1.

- [ ] **Step 6: Build, run the smoke test, and look at the screenshot**

Same recipe as Task 1 step 7. Then open `tools/preview-advice.png`: `ASK NEXT` heading in blue accent, `KEEP IN MIND` heading *and* its body both muted grey, the `THEM` label bright with white text. If the seeded advice has no `FIX`/`SAY`, that is expected — the unit test covers their colours.

- [ ] **Step 7: Ladder and commit**

```powershell
.\build.ps1 clippy --release --all-targets
$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { .\build.ps1 fmt }
git add src/hud.rs
git commit -F - @'
hud: restore Decision 7 — tag colours, greyed-while-thinking, THEM brighter

Only ASK and FIX are saturated (Von Restorff); the last advice stays up
greyed until the next one lands (never blank); THEM outshines YOU (Miller).
The tag table is now the single source for headings and colours.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
'@
```

---

### Task 3: Sink-based `consume`, `body` with `max_tokens`

**Files:**
- Modify: `src/coach.rs` — `body` (line 115–141), `stream` (line 159–179), `consume` (line 181–255), tests `read_sse` (line 261–271), `superseded_stream_does_not_emit_advice` (line 333–345), `openai_body_omits_anthropic_only_knobs` (line 422–431).
- Modify: `src/provider.rs:18` — derive `Clone`.

**Interfaces:**
- Produces: `fn body(p: &Provider, prompt: &str, transcript: &str, max_tokens: u32) -> Value`; `fn request(agent: &ureq::Agent, p: &Provider, system: &str, user: &str, max_tokens: u32, seq: u64, live: &AtomicU64, sink: &mut dyn FnMut(String) -> Result<()>) -> Result<bool>` (true = completed, false = superseded); `fn consume(reader: impl BufRead, wire: Wire, seq: u64, live: &AtomicU64, sink: &mut dyn FnMut(String) -> Result<()>) -> Result<bool>`. `Provider: Clone`.

- [ ] **Step 1: Update the tests first**

Replace `fn read_sse` with:

```rust
    /// Drive `consume` the way the advice lane does: deltas become `Advice`,
    /// completion becomes `AdviceEnd`, supersession becomes nothing.
    fn read_sse(data: &str) -> (Result<()>, Vec<Msg>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        let live = AtomicU64::new(1);
        let result = consume(
            std::io::Cursor::new(data),
            Wire::OpenAi,
            1,
            &live,
            &mut |t| Ok(tx.send(Msg::Advice(1, t))?),
        )
        .and_then(|done| {
            if done {
                tx.send(Msg::AdviceEnd(1))?;
            }
            Ok(())
        });
        (result, rx.try_iter().collect())
    }
```

Replace `superseded_stream_does_not_emit_advice` with:

```rust
    #[test]
    fn superseded_stream_does_not_emit_advice() {
        let mut seen = Vec::new();
        let done = consume(
            std::io::Cursor::new("data: [DONE]\n\n"),
            Wire::OpenAi,
            1,
            &AtomicU64::new(2),
            &mut |t| {
                seen.push(t);
                Ok(())
            },
        )
        .unwrap();
        assert!(!done, "a superseded stream reports it did not complete");
        assert!(seen.is_empty());
    }
```

In `openai_body_omits_anthropic_only_knobs`, change both `body(&p(...), "sys", "hello")` calls to `body(&p(...), "sys", "hello", MAX_TOKENS)`, and add at the end of that test:

```rust
        assert_eq!(body(&p(Wire::OpenAi), "s", "u", 2_000)["max_tokens"], 2_000);
```

- [ ] **Step 2: Run to verify they fail**

Run: `.\build.ps1 test --release coach::`
Expected: compile errors about `consume`/`body` signatures.

- [ ] **Step 3: Refactor `body`, `consume`, and split `stream` into `request` + the advice wrapper**

`body` signature and its two `MAX_TOKENS` uses become the parameter:

```rust
fn body(p: &Provider, prompt: &str, transcript: &str, max_tokens: u32) -> Value {
    match p.wire {
        Wire::Anthropic => json!({
            "model": p.model,
            "max_tokens": max_tokens,
            "stream": true,
            "speed": "fast",
            "fallbacks": "default",
            "output_config": {"effort": "low"},
            "system": [{"type": "text", "text": prompt,
                        "cache_control": {"type": "ephemeral"}}],
            "messages": [{"role": "user", "content": transcript}],
        }),
        Wire::OpenAi => json!({
            "model": p.model,
            "max_tokens": max_tokens,
            "stream": true,
            "messages": [
                {"role": "system", "content": prompt},
                {"role": "user", "content": transcript},
            ],
        }),
    }
}
```

(Keep the existing doc comment above `body`.) Replace `fn stream` with two functions:

```rust
/// One streamed request. Deltas go to `sink`; the return says whether the
/// stream ran to its completion marker (`false` means a newer generation
/// superseded it and the socket was abandoned).
///
/// Shared by both lanes: the advice lane turns deltas into `Msg::Advice`, the
/// research lane accumulates them and reports once at the end.
#[allow(clippy::too_many_arguments)]
fn request(
    agent: &ureq::Agent,
    p: &Provider,
    system: &str,
    user: &str,
    max_tokens: u32,
    seq: u64,
    live: &AtomicU64,
    sink: &mut dyn FnMut(String) -> Result<()>,
) -> Result<bool> {
    let req = agent.post(p.url).header("content-type", "application/json");
    let req = match p.wire {
        Wire::Anthropic => req
            .header("x-api-key", &p.key)
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", BETAS),
        Wire::OpenAi => req.header("authorization", &format!("Bearer {}", p.key)),
    };
    let resp = req.send_json(body(p, system, user, max_tokens))?;
    let reader = BufReader::new(resp.into_body().into_reader());
    consume(reader, p.wire, seq, live, sink)
}

fn stream(
    agent: &ureq::Agent,
    p: &Provider,
    prompt: &str,
    transcript: &str,
    seq: u64,
    live: &AtomicU64,
    tx: &Sender<Msg>,
) -> Result<()> {
    let done = request(agent, p, prompt, transcript, MAX_TOKENS, seq, live, &mut |text| {
        Ok(tx.send(Msg::Advice(seq, text))?)
    })?;
    if done {
        tx.send(Msg::AdviceEnd(seq))?;
    }
    Ok(())
}
```

In `consume`: change the signature to

```rust
fn consume(
    mut reader: impl BufRead,
    wire: Wire,
    seq: u64,
    live: &AtomicU64,
    sink: &mut dyn FnMut(String) -> Result<()>,
) -> Result<bool> {
```

then: the early `return Ok(());` for supersession becomes `return Ok(false);`; `tx.send(Msg::Advice(seq, text))?;` becomes `sink(text)?;`; and the final two lines `tx.send(Msg::AdviceEnd(seq))?; Ok(())` become `Ok(true)`.

In `src/provider.rs`, change `pub struct Provider {` to:

```rust
#[derive(Clone)]
pub struct Provider {
```

- [ ] **Step 4: Run the tests**

Run: `.\build.ps1 test --release`
Expected: all green — the SSE tests, the cancel test, the body test.

- [ ] **Step 5: Ladder and commit**

```powershell
.\build.ps1 clippy --release --all-targets
$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { .\build.ps1 fmt }
git add src/coach.rs src/provider.rs
git commit -F - @'
coach: sink-based consume and a max_tokens parameter, ahead of the research lane

`consume` reports whether it completed instead of sending the end message
itself, so a second lane can accumulate the same stream. No behaviour change.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
'@
```

---

### Task 4: The research lane over HTTP

**Files:**
- Modify: `src/coach.rs` — consts (after `MAX_TOKENS`), `struct Coach` (line 38–43), `Coach::new` (line 59–90), new methods after `ask`, `impl Drop` (line 106–110), tests.

**Interfaces:**
- Consumes: `request` from Task 3; `Msg::ToolStart(u64)`, `Msg::ToolEnd(u64, Result<String, String>)` from `main.rs` (exist).
- Produces: `pub const RESEARCH_PROMPT: &str`; `Coach::research(&self, system: String, context: String) -> u64`; `Coach::cancel_research(&self)`.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests`:

```rust
    /// A provider that refuses the connection at once, so the worker's fate
    /// is deterministic without any network.
    fn refused() -> Provider {
        Provider {
            url: "http://127.0.0.1:1",
            model: "test".into(),
            key: "test".into(),
            wire: Wire::OpenAi,
        }
    }

    /// Every `ToolEnd` that arrives within `secs`.
    fn tool_ends(rx: &Receiver<Msg>, secs: u64) -> Vec<(u64, std::result::Result<String, String>)> {
        let deadline = std::time::Instant::now() + Duration::from_secs(secs);
        let mut ends = Vec::new();
        while std::time::Instant::now() < deadline {
            while let Ok(m) = rx.try_recv() {
                if let Msg::ToolEnd(id, r) = m {
                    ends.push((id, r));
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        ends
    }

    #[test]
    fn research_announces_its_job_and_closes_it_exactly_once() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let coach = Coach::new(refused(), String::new(), tx);
        let id = coach.research("sys".into(), "ctx".into());
        assert!(matches!(rx.recv().unwrap(), Msg::ToolStart(i) if i == id));
        let ends = tool_ends(&rx, 3);
        assert_eq!(ends.len(), 1, "one job, one end: {ends:?}");
        assert_eq!(ends[0].0, id);
        assert!(ends[0].1.is_err(), "a refused connection is reported, not swallowed");
    }

    #[test]
    fn cancelling_research_closes_it_once_and_a_stray_cancel_says_nothing() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let coach = Coach::new(refused(), String::new(), tx);
        coach.cancel_research();
        assert!(tool_ends(&rx, 1).is_empty(), "nothing was running");
        let id = coach.research("sys".into(), "ctx".into());
        coach.cancel_research();
        // Whichever of the cancel and the worker gets there first is the only
        // closer; a second `ToolEnd` would overwrite a finished result later.
        let ends = tool_ends(&rx, 3);
        assert_eq!(ends.len(), 1, "{ends:?}");
        assert_eq!(ends[0].0, id);
        assert!(ends[0].1.is_err());
        // And a finished job cannot be "cancelled" after the fact.
        coach.cancel_research();
        assert!(tool_ends(&rx, 1).is_empty());
    }

    #[test]
    fn research_body_has_no_line_limit_and_room_to_answer() {
        assert!(!RESEARCH_PROMPT.contains("14 words"));
        // Tied to the wire rather than asserted on the constant, which clippy
        // rejects as an assertion on a constant.
        assert_eq!(body(&refused(), RESEARCH_PROMPT, "u", RESEARCH_MAX_TOKENS)["max_tokens"], 2_000);
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `.\build.ps1 test --release research_`
Expected: compile errors — `research`, `cancel_research`, `RESEARCH_PROMPT`, `RESEARCH_MAX_TOKENS` not found.

- [ ] **Step 3: Add the lane**

After `const MAX_TOKENS: u32 = 400;` add:

```rust
/// The slow lane's system prompt. `research.md` beside `prompt.md` overrides
/// it (read in `main`); this is what runs when that file is absent.
pub const RESEARCH_PROMPT: &str = "You are researching one moment of a live call for the user, who \
reads a heads-up panel and decides what to say. Answer the question implied by the newest THEM \
line, in depth: what is established, what is uncertain, and the one question that would settle \
it. Use the reference excerpts and cite them as [filename, passage N]. Never invent facts about \
the user's company, product or numbers — say what is missing instead. Plain text, short \
paragraphs, no markdown headers.";

/// Research answers a question, not a glance: room to reason, not 2–4 lines.
const RESEARCH_MAX_TOKENS: u32 = 2_000;
```

Replace `pub struct Coach { ... }` with:

```rust
pub struct Coach {
    tx: Sender<Msg>,
    seq: Arc<AtomicU64>,
    jobs: Sender<(u64, String)>,
    pending: Receiver<(u64, String)>,
    research: Lane,
}

/// The slow lane: its own worker, its own generation counter, one job at a
/// time. It exists so that a 30 s research answer never queues behind or in
/// front of the ~1 s advice the fast lane is for.
struct Lane {
    seq: Arc<AtomicU64>,
    /// The id whose `ToolStart` was announced and not yet closed, else 0.
    /// Whoever swaps it to 0 first — the worker finishing or a cancel — is the
    /// one that sends `ToolEnd`; a second end for the same id would replace a
    /// finished result on the panel with "cancelled".
    open: Arc<AtomicU64>,
    jobs: Sender<(u64, String, String)>,
    pending: Receiver<(u64, String, String)>,
}
```

In `Coach::new`, after the existing `std::thread::spawn(move || { ... });` for the advice worker and before `Self { ... }`, add the research worker, and add `research` to the returned struct:

```rust
        let research = {
            let (jobs, pending) = bounded::<(u64, String, String)>(1);
            let seq = Arc::new(AtomicU64::new(0));
            let open = Arc::new(AtomicU64::new(0));
            let (rx, live, owner, output, provider) =
                (pending.clone(), seq.clone(), open.clone(), tx.clone(), provider.clone());
            std::thread::spawn(move || {
                let agent = pooled_agent();
                while let Ok((generation, system, context)) = rx.recv() {
                    if live.load(Ordering::SeqCst) != generation {
                        continue;
                    }
                    let mut text = String::new();
                    let outcome = request(
                        &agent,
                        &provider,
                        &system,
                        &context,
                        RESEARCH_MAX_TOKENS,
                        generation,
                        &live,
                        &mut |t| {
                            text.push_str(&t);
                            Ok(())
                        },
                    );
                    let result = match outcome {
                        Ok(true) => Ok(text),
                        Ok(false) => continue, // superseded; its successor announced itself
                        Err(e) => Err(e.to_string()),
                    };
                    if owner
                        .compare_exchange(generation, 0, Ordering::SeqCst, Ordering::SeqCst)
                        .is_ok()
                    {
                        let _ = output.send(Msg::ToolEnd(generation, result));
                    }
                }
            });
            Lane {
                seq,
                open,
                jobs,
                pending,
            }
        };
        Self {
            tx,
            seq,
            jobs,
            pending,
            research,
        }
```

`provider` is moved into the advice worker's `move` closure today, which would leave nothing for the research block to clone. Directly above the advice worker's `std::thread::spawn(move || {` add `let advice_provider = provider.clone();`, and inside that closure change `stream(&agent, &provider, ...)` to `stream(&agent, &advice_provider, ...)`. The research block above then clones the original `provider`.

After `pub fn ask`, add:

```rust
    /// Start a research job. Announces `ToolStart(id)` at once — the panel
    /// counts the wait — and returns the id so a caller can correlate.
    pub fn research(&self, system: String, context: String) -> u64 {
        let lane = &self.research;
        let id = lane.seq.fetch_add(1, Ordering::SeqCst) + 1;
        lane.open.store(id, Ordering::SeqCst);
        let _ = self.tx.send(Msg::ToolStart(id));
        let _ = lane.pending.try_recv();
        let _ = lane.jobs.try_send((id, system, context));
        id
    }

    /// Retire the running research job, if there is one.
    ///
    /// Same rule as `cancel`: a cancel has no successor, so it must close the
    /// generation it retires or the panel says "researching" forever. It only
    /// speaks when it wins the `open` token — a job the worker already
    /// finished, or nothing running at all, gets no second "cancelled" end.
    pub fn cancel_research(&self) {
        let lane = &self.research;
        let retired = lane.seq.fetch_add(1, Ordering::SeqCst);
        let _ = lane.pending.try_recv();
        if retired != 0
            && lane
                .open
                .compare_exchange(retired, 0, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        {
            let _ = self.tx.send(Msg::ToolEnd(retired, Err("cancelled".into())));
        }
    }
```

In `impl Drop for Coach`, add `self.research.seq.fetch_add(1, Ordering::SeqCst);` after the existing line.

- [ ] **Step 4: Run the tests**

Run: `.\build.ps1 test --release coach::`
Expected: all green. `research_announces_its_job_and_closes_it_exactly_once` takes ~3 s (it waits to prove no second end arrives).

- [ ] **Step 5: Ladder and commit**

```powershell
.\build.ps1 clippy --release --all-targets
$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { .\build.ps1 fmt }
git add src/coach.rs
git commit -F - @'
coach: research lane over HTTP on its own worker

F8 no longer needs a CLI: the same wire as advice, a research prompt,
2000 tokens, 60 s, one job at a time. `ToolStart`/`ToolEnd` unchanged so the
panel and router need nothing new. Exactly one closer per job, by CAS.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
'@
```

---

### Task 5: `retrieve_deep` — the passages research gets

**Files:**
- Modify: `src/references.rs` — `retrieve` (line 146), `local_answer` (line 149), `excerpts` (line 157–198), tests (line 276).

**Interfaces:**
- Produces: `References::retrieve_deep(&self, query: &str) -> String` — same citation format as `retrieve`, top 40 passages, output capped at 60 000 bytes.

- [ ] **Step 1: Write the failing test**

Add to `mod tests`:

```rust
    #[test]
    fn deep_retrieval_returns_more_passages_but_stays_bounded() {
        let (tx, _) = crossbeam_channel::unbounded();
        let refs = References::new(tx);
        // 50 passages that all match, each ~1.4 KB: enough to prove both the
        // wider window and the byte cap.
        let chunks: Vec<String> = (0..50)
            .map(|i| format!("rollback plan variant {i} {}", "x".repeat(1_350)))
            .collect();
        refs.library
            .write()
            .unwrap()
            .documents
            .push(Document::new(PathBuf::from("plan.txt"), chunks));
        let shallow = refs.retrieve("rollback plan");
        let deep = refs.retrieve_deep("rollback plan");
        assert_eq!(shallow.matches("passage ").count(), 4);
        assert!(deep.matches("passage ").count() > 4);
        assert!(deep.len() <= 60_000 + 1_500, "cap is per whole passage: {}", deep.len());
        assert!(deep.matches("passage ").count() <= 40);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `.\build.ps1 test --release deep_retrieval`
Expected: compile error — `retrieve_deep` not found.

- [ ] **Step 3: Parametrise `excerpts`**

Replace the three functions:

```rust
    pub fn retrieve(&self, query: &str) -> String {
        self.excerpts(query, true, 4, usize::MAX)
    }
    /// What research gets: wider than a glance, still bounded.
    ///
    /// Not "everything indexed" — 24 files × 400 KB does not fit a request —
    /// but the ranker `retrieve` already uses, given room. Cut on whole
    /// passages so a citation is never half a passage.
    pub fn retrieve_deep(&self, query: &str) -> String {
        self.excerpts(query, true, 40, 60_000)
    }
    pub fn local_answer(&self, query: &str) -> String {
        let excerpts = self.excerpts(query, false, 4, usize::MAX);
        if excerpts.is_empty() {
            "No matching passages found.\r\n\r\nTry a name, product, date, or phrase used in your reference files. This is a local search; no AI service was contacted.".into()
        } else {
            format!("LOCAL REFERENCE MATCHES\r\n\r\n{excerpts}")
        }
    }
    fn excerpts(&self, query: &str, for_model: bool, limit: usize, cap: usize) -> String {
```

and inside `excerpts`, replace the `for (_, di, ci) in hits.into_iter().take(4) { ... out.push_str(&format!(...)); }` loop with:

```rust
        for (_, di, ci) in hits.into_iter().take(limit) {
            let d = &library.documents[di];
            let entry = format!(
                "\n[{}, passage {}]\n{}\n",
                // The body is JSON-escaped; a filename is user data in the same
                // header and must not close a citation the model reads as structure.
                d.path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .replace(['[', ']'], ""),
                ci + 1,
                if for_model {
                    serde_json::json!(d.chunks[ci]).to_string()
                } else {
                    d.chunks[ci].clone()
                }
            );
            if out.len() + entry.len() > cap {
                break;
            }
            out.push_str(&entry);
        }
```

- [ ] **Step 4: Run the tests**

Run: `.\build.ps1 test --release references::`
Expected: all green, including `retrieval_selects_evidence_and_cites_source`.

- [ ] **Step 5: Ladder and commit**

```powershell
.\build.ps1 clippy --release --all-targets
$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { .\build.ps1 fmt }
git add src/references.rs
git commit -F - @'
references: retrieve_deep for the research lane

Top 40 passages, 60 KB, cut on whole passages. Same ranker as retrieve.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
'@
```

---

### Task 6: Audio sessions — list the apps, choose one

**Files:**
- Modify: `src/audio.rs` — imports (line 15), new items after `pub const FRAME` (line 20), tests.

**Interfaces:**
- Produces: `pub struct AppSession { pub pid: u32, pub active: bool, pub name: String }`; `pub fn sessions(dev: &wasapi::Device) -> Result<Vec<AppSession>>`; `pub fn resolve(sessions: &[AppSession], wanted: &str) -> Option<u32>`; `pub fn alive(pid: u32) -> bool`.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/audio.rs`:

```rust
    #[test]
    fn hear_picks_the_active_session_by_name() {
        let s = |pid, active, name: &str| AppSession {
            pid,
            active,
            name: name.into(),
        };
        let list = [s(10, false, "Discord"), s(11, true, "Discord"), s(20, true, "chrome")];
        // Case-insensitive substring; the one actually playing wins.
        assert_eq!(resolve(&list, "discord"), Some(11));
        assert_eq!(resolve(&list, "CHROME"), Some(20));
        assert_eq!(resolve(&list, "zoom"), None);
        // Nothing active yet: the first match, so a quiet app still gets hooked
        // and picked up the moment it speaks.
        assert_eq!(resolve(&list[..1], "disc"), Some(10));
        assert_eq!(resolve(&[], "discord"), None);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `.\build.ps1 test --release hear_picks`
Expected: compile error — `AppSession`, `resolve` not found.

- [ ] **Step 3: Implement listing, choosing, and liveness**

Change the `wasapi` import line to:

```rust
use wasapi::{
    Device, DeviceEnumerator, Direction, SampleType, SessionState, StreamMode, WaveFormat,
    initialize_mta,
};
```

After `pub const FRAME: usize = ...;` add:

```rust
/// One app with an audio session on the loopback endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppSession {
    pub pid: u32,
    /// At least one of its streams is running right now.
    pub active: bool,
    /// The session's display name, else the executable's stem. Most apps set
    /// no display name, so it is usually the stem — `Discord`, `chrome`.
    pub name: String,
}

/// Every app with a session on `dev`, system sounds excluded.
pub fn sessions(dev: &Device) -> Result<Vec<AppSession>> {
    let list = dev.get_iaudiosessionmanager()?.get_audiosessionenumerator()?;
    let mut out = Vec::new();
    for i in 0..list.get_count()? {
        let session = list.get_session(i)?;
        let pid = session.get_process_id()?;
        if pid == 0 {
            continue; // the system-sounds session has no process
        }
        let mut name = session.get_display_name().unwrap_or_default();
        if name.is_empty() {
            name = image_stem(pid).unwrap_or_default();
        }
        if name.is_empty() {
            continue;
        }
        let active = matches!(session.get_state()?, SessionState::Active);
        out.push(AppSession { pid, active, name });
    }
    Ok(out)
}

/// Which session `--hear <name>` means: case-insensitive substring on the
/// name, an active session over a silent one, else the first listed.
pub fn resolve(sessions: &[AppSession], wanted: &str) -> Option<u32> {
    let wanted = wanted.to_lowercase();
    let matching: Vec<&AppSession> = sessions
        .iter()
        .filter(|s| s.name.to_lowercase().contains(&wanted))
        .collect();
    matching
        .iter()
        .find(|s| s.active)
        .or(matching.first())
        .map(|s| s.pid)
}

/// Whether `pid` is still running. A process-loopback stream whose target
/// exits delivers silence, not an error, so this is how a closed app is
/// noticed and re-acquired under its new pid.
pub fn alive(pid: u32) -> bool {
    use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut code = 0u32;
        let running = GetExitCodeProcess(handle, &mut code).is_ok() && code == STILL_ACTIVE.0 as u32;
        let _ = CloseHandle(handle);
        running
    }
}

fn image_stem(pid: u32) -> Option<String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };
    use windows::core::PWSTR;
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = vec![0u16; 1024];
        let mut len = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(handle);
        if !ok {
            return None;
        }
        let path = String::from_utf16_lossy(&buffer[..len as usize]);
        std::path::Path::new(&path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
    }
}
```

(Task 7 adds `AudioCaptureClient` and `AudioClient` to that import when it needs them.)

- [ ] **Step 4: Run the tests and clippy**

Run: `.\build.ps1 test --release hear_picks` → PASS. `.\build.ps1 clippy --release --all-targets` → zero warnings.

- [ ] **Step 5: Ladder and commit**

```powershell
$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { .\build.ps1 fmt }
git add src/audio.rs
git commit -F - @'
audio: list the apps playing on the loopback endpoint, and choose one by name

`sessions`/`resolve`/`alive`: the pieces `--hear` and `--list-apps` stand on.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
'@
```

---

### Task 7: Per-app capture with reacquire — and the autoconvert spike

**Files:**
- Modify: `src/audio.rs` — `Input` (line 214–220), `run` (line 222–424: the stream setup at 235–251 and the loop at 341–423 move into functions).

**Interfaces:**
- Consumes: `sessions`, `resolve`, `alive` (Task 6).
- Produces: `Input.hear: Option<String>`; private `open_endpoint(dev: &Device) -> Result<Stream>`, `open_process(pid: u32) -> Result<Stream>`, `struct Stream { client: AudioClient, capture: AudioCaptureClient, event: Option<wasapi::Handle> }`, `fn pump(stream: &Stream, feed: &Feed, alive: &mut dyn FnMut() -> bool) -> Result<()>`. Behaviour: with `hear = None` identical to today; with `Some(name)` the reacquire loop from spec §4.2.

- [ ] **Step 1: Extract the stream setup and the loop, unchanged in behaviour**

Change the `wasapi` import line to:

```rust
use wasapi::{
    AudioCaptureClient, AudioClient, Device, DeviceEnumerator, Direction, SampleType, SessionState,
    StreamMode, WaveFormat, initialize_mta,
};
```

Add `hear` to `Input`:

```rust
pub struct Input {
    pub who: Who,
    pub dir: Direction,
    pub name: String,
    pub gate_override: Option<f32>,
    pub mute: Option<Arc<AtomicBool>>,
    /// Hear only this app's process tree as `THEM` (`--hear`). `None` is the
    /// endpoint mix, as before.
    pub hear: Option<String>,
}
```

Add these items above `pub fn run`:

```rust
/// An open capture stream. `event` is set only for event-driven streams.
struct Stream {
    client: AudioClient,
    capture: AudioCaptureClient,
    event: Option<wasapi::Handle>,
}

impl Stream {
    /// Stop before drop, so the next client — the reacquired app — opens
    /// against a released endpoint.
    fn close(self) {
        let _ = self.client.stop_stream();
    }
}

/// Loopback on the whole endpoint: everything the speakers play.
fn open_endpoint(dev: &Device) -> Result<Stream> {
    let mut client = dev.get_iaudioclient()?;
    let fmt = WaveFormat::new(32, 32, &SampleType::Float, RATE, 1, None);
    let (_default_period, min_period) = client.get_device_period()?;
    client.initialize_client(
        &fmt,
        &Direction::Capture,
        &StreamMode::PollingShared {
            autoconvert: true,
            // >=500 ms of slack against scheduler jitter. Transcription no
            // longer runs on this thread (see the worker below), so this is now
            // headroom rather than the only thing preventing dropped audio —
            // which it was, badly, at the previous 100 ms. 32 KB either way.
            buffer_duration_hns: min_period.max(5_000_000),
        },
    )?;
    let capture = client.get_audiocaptureclient()?;
    client.start_stream()?;
    Ok(Stream {
        client,
        capture,
        event: None,
    })
}

/// Loopback on one process tree: only that app, whatever else is playing.
///
/// `get_device_period` is unsupported on this client (crate docs), so the
/// slack is fixed rather than derived. The same 16 kHz mono f32 is requested
/// as for the endpoint, so nothing downstream learns which client fed it.
fn open_process(pid: u32) -> Result<Stream> {
    let fmt = WaveFormat::new(32, 32, &SampleType::Float, RATE, 1, None);
    let mut client = AudioClient::new_application_loopback_client(pid, true)?;
    client.initialize_client(
        &fmt,
        &Direction::Capture,
        &StreamMode::PollingShared {
            autoconvert: true,
            buffer_duration_hns: 5_000_000,
        },
    )?;
    let capture = client.get_audiocaptureclient()?;
    client.start_stream()?;
    Ok(Stream {
        client,
        capture,
        event: None,
    })
}

/// Everything the capture loop needs that is not the stream itself.
struct Feed<'a> {
    label: &'a str,
    tx: &'a Sender<Msg>,
    tune: &'a Arc<Tune>,
    utt_tx: &'a Sender<(u64, Vec<f32>)>,
    gate_override: Option<f32>,
    mute: Option<&'a Arc<AtomicBool>>,
}
```

Now restructure `run`. Keep everything from `let Input { .. } = input;` through the end of the worker spawn block (the `{ let (ctx, tx, tune) = ...; std::thread::spawn(move || { ... }); }`), but add `hear` to the destructuring. Delete the old stream setup (`let mut client = dev.get_iaudioclient()?;` … `client.start_stream()?;`) from its old position — it is now `open_endpoint`. Replace everything after the worker spawn block (from `let mut raw: VecDeque<u8> = VecDeque::new();` to the end of `run`) with:

```rust
    let feed = Feed {
        label: &label,
        tx: &tx,
        tune: &tune,
        utt_tx: &utt_tx,
        gate_override,
        mute: mute.as_ref(),
    };
    let Some(app) = hear else {
        // The endpoint mix, exactly as before: a stream error ends the thread
        // and `main` reports it.
        let stream = open_endpoint(&dev)?;
        return pump(&stream, &feed, &mut || true);
    };

    // The app may not be running yet — the call app usually starts second —
    // and may restart with a new pid mid-call. Neither is an error here.
    let mut waiting_said = false;
    loop {
        let pid = match sessions(&dev).map(|list| resolve(&list, &app)) {
            Ok(Some(pid)) => pid,
            Ok(None) => {
                if !waiting_said {
                    tx.send(Msg::Sys(format!("hearing: waiting for {app}…")))?;
                    waiting_said = true;
                }
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
            Err(e) => {
                tx.send(Msg::Sys(format!("hearing: {e}")))?;
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        waiting_said = false;
        let stream = match open_process(pid) {
            Ok(stream) => stream,
            Err(e) => {
                tx.send(Msg::Sys(format!(
                    "hearing: {app} failed: {e}; using the whole speaker mix"
                )))?;
                let stream = open_endpoint(&dev)?;
                return pump(&stream, &feed, &mut || true);
            }
        };
        tx.send(Msg::Sys(format!("hearing: {app} (pid {pid})")))?;
        if let Err(e) = pump(&stream, &feed, &mut || alive(pid)) {
            tx.send(Msg::Sys(format!("hearing: {app} stopped: {e}")))?;
        }
        stream.close();
    }
}

/// Read, gate, segment — all microseconds — until the stream fails or
/// `still_there` says the source is gone. Never transcribes: see the worker.
fn pump(stream: &Stream, feed: &Feed, still_there: &mut dyn FnMut() -> bool) -> Result<()> {
    let Feed {
        label,
        tx,
        tune,
        utt_tx,
        gate_override,
        mute,
    } = feed;
    let mut raw: VecDeque<u8> = VecDeque::new();
    let mut frame = vec![0f32; FRAME];
    let mut cal: Vec<f32> = Vec::new();
    let mut seg = gate_override.map(Segmenter::new);
    if let Some(g) = gate_override {
        tx.send(Msg::Sys(format!("{label} gate {g:.4} (fixed)")))?;
    }
    let mut last = Instant::now();
    let calibrating_since = Instant::now();
    let mut checked = Instant::now();
    let mut previous_epoch = tune.epoch.load(Ordering::SeqCst);

    loop {
        if checked.elapsed() >= Duration::from_secs(2) {
            checked = Instant::now();
            if !still_there() {
                anyhow::bail!("the app closed");
            }
        }
        // A silent render endpoint supplies no packets. Finish calibration by
        // wall clock so the first spoken second is not consumed as room noise.
        if seg.is_none() && calibrating_since.elapsed() >= Duration::from_secs(1) {
            let g = calibrated_gate(&mut cal);
            tx.send(Msg::Sys(format!("{label} gate {g:.4}")))?;
            seg = Some(Segmenter::new(g));
        }
        if let Some(event) = &stream.event {
            let _ = event.wait_for_event(100);
        }
        stream.capture.read_from_device_to_deque(&mut raw)?;
        let epoch = tune.epoch.load(Ordering::SeqCst);
        if epoch != previous_epoch {
            raw.clear();
            if let Some(seg) = seg.as_mut() {
                seg.reset();
            }
            previous_epoch = epoch;
        }
        if !epoch.is_multiple_of(2) {
            raw.clear();
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }

        if raw.len() < FRAME * 4 {
            // Loopback delivers nothing while the far end is silent, so the hang
            // timer would stall and the last turn would never flush. Feed it quiet.
            if last.elapsed() >= Duration::from_millis(FRAME_MS as u64)
                && let Some(seg) = seg.as_mut()
            {
                last = Instant::now();
                frame.fill(0.0);
                if let Some(utt) = seg.push(&frame) {
                    send(utt_tx, epoch, utt)?;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }

        while raw.len() >= FRAME * 4 {
            for s in frame.iter_mut() {
                let b = [
                    raw.pop_front().unwrap(),
                    raw.pop_front().unwrap(),
                    raw.pop_front().unwrap(),
                    raw.pop_front().unwrap(),
                ];
                *s = f32::from_le_bytes(b);
            }
            last = Instant::now();

            if seg.is_none() {
                // First second of audio sets the gate. Rooms, mics and headsets
                // all differ; --mic-gate / --sys-gate override it.
                cal.push(rms(&frame));
                if cal.len() >= 1000 / FRAME_MS {
                    let g = calibrated_gate(&mut cal);
                    tx.send(Msg::Sys(format!("{label} gate {g:.4}")))?;
                    seg = Some(Segmenter::new(g));
                }
                continue;
            }
            let seg = seg.as_mut().unwrap();

            if mute.is_some_and(|m| m.load(Ordering::Relaxed)) {
                frame.fill(0.0); // the AI is talking; don't hear ourselves
            }
            if let Some(utt) = seg.push(&frame) {
                send(utt_tx, epoch, utt)?;
            }
        }
    }
}
```

`stream.client` is read by `Stream::close`, so the field is not dead code; do not `#[allow]` anything here.

- [ ] **Step 2: Build and run the existing tests — behaviour with `hear = None` must be identical**

Run: `.\build.ps1 build --release` — main.rs does not set `hear` yet, so it will not compile until Task 9 adds the field at the two `audio::Input { .. }` sites. To test this task in isolation, temporarily add `hear: None,` to both `audio::Input { ... }` initialisers in `src/main.rs` (the loop at ~line 628) **and leave that edit in place** — Task 9 replaces it with the real value. Then `.\build.ps1 test --release` → all green, and `tests/wasapi_loopback.rs` still passes (it exercises the endpoint path).

- [ ] **Step 3: The spike — does the process-loopback client honour 16 kHz mono autoconvert?**

This step is discovery on this machine; keep only what turns out to be needed.

Play audio from some app (e.g. open a YouTube video in the browser, or run `powershell -Command "(New-Object -ComObject SAPI.SpVoice).Speak('testing one two three, testing one two three')"` in a loop). Temporarily hard-code `hear: Some("chrome".into())` (or `"powershell"`) on the THEM `Input` in `main.rs`, build, and run:

```powershell
Get-Process inner-voice -ErrorAction SilentlyContinue | Stop-Process -Force
.\build.ps1 build --release
$env:IV_UI_DUMP = "$env:TEMP\iv-hear.json"
$p = Start-Process -PassThru -FilePath .\target\release\inner-voice.exe -ArgumentList '--provider','none','--log',"$env:TEMP\iv-hear-logs"
Start-Sleep -Seconds 40
Get-Content -Raw $env:IV_UI_DUMP
Get-ChildItem "$env:TEMP\iv-hear-logs" -Filter *.jsonl | Get-Content | Select-String '"who":"THEM"' | Select-Object -Last 5
Stop-Process -Id $p.Id -Force
```

Outcomes, in order — **stop at the first that works and delete the code for the others**:

1. `THEM` lines appear with the spoken words → attempt 1 works. Done; `open_process` stays as written.
2. `open_process` errors at `initialize_client` (the panel's Diagnostics show `hearing: … failed: …`) → switch `open_process` to event-driven:

   ```rust
   client.initialize_client(
       &fmt,
       &Direction::Capture,
       &StreamMode::EventsShared {
           autoconvert: true,
           buffer_duration_hns: 5_000_000,
       },
   )?;
   let event = client.set_get_eventhandle()?;
   let capture = client.get_audiocaptureclient()?;
   client.start_stream()?;
   Ok(Stream { client, capture, event: Some(event) })
   ```

   (`pump` already waits on `stream.event` when present.) Rebuild, rerun.
3. Still refused, or frames arrive but transcripts are garbage (wrong rate) → request 48 kHz stereo and convert. Add to `Stream` a field `convert: bool`, set it true in `open_process` with `WaveFormat::new(32, 32, &SampleType::Float, 48_000, 2, None)`, and in `pump` read `FRAME * 4 * 6` bytes per frame when `convert` (960 stereo f32 pairs → 320 mono samples) through this decimator, defined above `pump`:

   ```rust
   /// 48 kHz stereo → 16 kHz mono, only for a process-loopback client that
   /// refused the 16 kHz request. Average the pair, low-pass with a 9-tap
   /// Hann-windowed sinc at 8 kHz, keep every third sample. Integer ratio, no
   /// dependency; speech has little energy above 8 kHz so the short filter is
   /// enough for intelligibility.
   struct Decimate {
       taps: [f32; 9],
       history: VecDeque<f32>,
   }
   impl Decimate {
       fn new() -> Self {
           let fc = 8_000.0 / 48_000.0;
           let mut taps = [0f32; 9];
           let mut sum = 0.0;
           for (i, t) in taps.iter_mut().enumerate() {
               let n = i as f32 - 4.0;
               let sinc = if n == 0.0 { 2.0 * fc } else { (2.0 * std::f32::consts::PI * fc * n).sin() / (std::f32::consts::PI * n) };
               let hann = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / 8.0).cos();
               *t = sinc * hann;
               sum += *t;
           }
           for t in taps.iter_mut() {
               *t /= sum;
           }
           Self { taps, history: VecDeque::from(vec![0.0; 9]) }
       }
       /// `stereo` is interleaved L/R at 48 kHz; returns 16 kHz mono.
       fn push(&mut self, stereo: &[f32], out: &mut Vec<f32>) {
           for (i, pair) in stereo.chunks_exact(2).enumerate() {
               self.history.pop_front();
               self.history.push_back(0.5 * (pair[0] + pair[1]));
               if i % 3 == 2 {
                   out.push(self.history.iter().zip(self.taps.iter()).map(|(x, t)| x * t).sum());
               }
           }
       }
   }
   ```

   In `pump`, hold `let mut decimate = stream.convert.then(Decimate::new);` and, in the `while raw.len() >= bytes_per_frame` loop, when `decimate` is `Some`, decode 960×2 f32 from `raw` into a scratch `Vec<f32>`, call `push` into a `Vec<f32>` that must come out as exactly `FRAME` samples, and copy into `frame`. Rebuild, rerun.
4. None of it produces speech → leave `open_process` as attempt 1 so the failure surfaces as `hearing: … failed: … using the whole speaker mix`, and **report back before continuing** — that is a spec-level finding.

Record which attempt won in the commit message. Remove the temporary hard-coded `hear` from `main.rs` (back to `hear: None,`).

- [ ] **Step 4: Reacquire check — the app closes and reopens**

With `hear` temporarily hard-coded to `Some("powershell".into())`: start inner-voice, then run `powershell -Command "(New-Object -ComObject SAPI.SpVoice).Speak('first sentence about a purple elephant')"`; wait 10 s; run it again (a new pid). Diagnostics (the `IV_UI_DUMP` file's `notice`, or the JSONL) must show `hearing: powershell (pid A)`, then `hearing: powershell stopped: the app closed`, then `hearing: powershell (pid B)`, and a `THEM` line for each sentence. Revert the hard-code.

- [ ] **Step 5: Ladder and commit**

```powershell
Get-Process inner-voice -ErrorAction SilentlyContinue | Stop-Process -Force
.\build.ps1 test --release
.\build.ps1 clippy --release --all-targets
$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { .\build.ps1 fmt }
git add src/audio.rs src/main.rs
git commit -F - @'
audio: hear one app's process tree instead of the whole endpoint mix

`Input.hear` opens a process-loopback client on the named app, reacquires
when it starts late or restarts, and falls back to the endpoint if the
client cannot be opened. Endpoint capture is unchanged. Spike result: <state
which attempt from the plan worked: polling / events / 48k+decimate>.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
'@
```

---

### Task 8: `--speak` keeps its mute only when nothing else prevents echo

**Files:**
- Modify: `src/speak.rs:18-40`.

**Interfaces:**
- Produces: `Speaker::new(mute: Option<Arc<AtomicBool>>) -> Self`.

- [ ] **Step 1: Change the signature and make the flag optional inside the thread**

Replace `pub fn new(mute: Arc<AtomicBool>) -> Self {` and the two `mute.store(...)` lines:

```rust
    /// `mute` is `Some` only when the loopback is the whole endpoint mix and
    /// would otherwise transcribe this voice as the other party. With `--hear`
    /// the app is a different process and is simply never heard, so nothing
    /// needs deafening and no speech is lost.
    pub fn new(mute: Option<Arc<AtomicBool>>) -> Self {
```

and

```rust
                if let Some(m) = &mute {
                    m.store(true, Ordering::Relaxed);
                }
                // Synchronous on purpose: `mute` must stay set for the whole utterance.
                let _ = voice.Speak(PCWSTR(w.as_ptr()), SPF_DEFAULT.0 as u32, None);
                if rx.is_empty()
                    && let Some(m) = &mute
                {
                    m.store(false, Ordering::Relaxed);
                }
```

Update the module doc's first paragraph to: `//! It holds \`mute\` while talking — when given one — so an endpoint loopback doesn't transcribe the AI's own voice straight back in as the other person.`

- [ ] **Step 2: Build**

`main.rs` still passes `Arc<AtomicBool>` — wrap that call site for now: change `speak::Speaker::new(mute.clone())` to `speak::Speaker::new(Some(mute.clone()))` in `src/main.rs` (Task 9 rewrites this line). `.\build.ps1 test --release` → green.

- [ ] **Step 3: Ladder and commit**

```powershell
.\build.ps1 clippy --release --all-targets
$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { .\build.ps1 fmt }
git add src/speak.rs src/main.rs
git commit -F - @'
speak: the loopback mute becomes optional

Only an endpoint-mix loopback needs deafening while the voice talks.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
'@
```

---

### Task 9: Integration — `--hear`, `--list-apps`, speak wiring, notices

**Files:**
- Modify: `src/main.rs` — `Args` (line 136–138 and after 186), the `list_devices` check (line 450–452), the mute/speaker lines (line 564–565), the capture loop (line 604–639), the startup notices (line 657–658).

**Interfaces:**
- Consumes: `audio::sessions`, `audio::Input.hear` (Tasks 6–7), `Speaker::new(Option<..>)` (Task 8).

- [ ] **Step 1: Flags**

Replace the `speak` flag's doc and add two flags after `es`:

```rust
    /// Read advice aloud. With --hear the voice is never captured; without it
    /// the call is deafened while the voice talks
    #[arg(long, env = "IV_SPEAK")]
    speak: bool,
```

```rust
    /// Hear only this app as THEM (any part of its name, see --list-apps).
    /// Without it, THEM is everything the speakers play.
    #[arg(long, env = "IV_HEAR")]
    hear: Option<String>,

    /// List the apps with audio on the loopback device and exit
    #[arg(long)]
    list_apps: bool,
```

- [ ] **Step 2: `--list-apps`**

After the `if args.list_devices { return list(&enumerator); }` block add:

```rust
    if args.list_apps {
        let dev = pick(&enumerator, Direction::Render, &args.loopback)?;
        println!("\nApps with audio on {}:", dev.get_friendlyname()?);
        for s in audio::sessions(&dev)? {
            println!(
                "  {:>6}  {:<7}  {}",
                s.pid,
                if s.active { "active" } else { "silent" },
                s.name
            );
        }
        println!("\n--hear takes any part of a name; an active one wins.");
        return Ok(());
    }
```

- [ ] **Step 3: Mute only without `--hear`; wire `hear`; say what THEM is**

Replace the two lines `let mute = Arc::new(AtomicBool::new(false)); let speaker = args.speak.then(|| speak::Speaker::new(mute.clone()));` with:

```rust
    // The voice is deafened out of the loopback only when the loopback is the
    // whole endpoint mix. With --hear the app is another process and the voice
    // is never captured, so nothing is muted and no speech is lost.
    let mute = (args.speak && args.hear.is_none()).then(|| Arc::new(AtomicBool::new(false)));
    let speaker = args.speak.then(|| speak::Speaker::new(mute.clone()));
```

In the capture loop, the tuple gains `hear`: YOU gets `None`, THEM gets `args.hear.clone()`; the THEM `mute` entry becomes `mute.clone()` (it is already an `Option`), YOU stays `None`. The destructuring becomes `for (who, dir, name, gate, mute, hear) in [...]` and the `audio::Input { ... }` initialiser gains `hear,`. Replace the comment above the loop with:

```rust
    // YOU hears itself. THEM is the endpoint mix, or one app's process tree
    // with --hear; only the endpoint mix is deafened while the voice talks.
```

Replace `let _ = ui_tx.send(Msg::Sys(format!("THEM <- {sys_name} (loopback)")));` with:

```rust
    let _ = ui_tx.send(Msg::Sys(match &args.hear {
        Some(app) => format!("THEM <- {app} (app loopback on {sys_name})"),
        None => format!("THEM <- {sys_name} (loopback)"),
    }));
    if args.speak && args.hear.is_none() {
        let _ = ui_tx.send(Msg::Sys(
            "speak: reading advice mutes the call; add --hear <app> so it doesn't".into(),
        ));
    }
```

- [ ] **Step 4: Build, run `--list-apps` against a playing app, run the smoke test**

```powershell
Get-Process inner-voice -ErrorAction SilentlyContinue | Stop-Process -Force
.\build.ps1 build --release
Start-Process powershell -ArgumentList '-Command', "(New-Object -ComObject SAPI.SpVoice).Speak('word word word word word word word word word word word word word word word word word word word word')\"
Start-Sleep -Seconds 2
.\target\release\inner-voice.exe --list-apps
```

Expected: a `powershell` row marked `active`. Then the smoke recipe from Task 1 step 7 → `PASS`.

- [ ] **Step 5: Ladder and commit**

```powershell
.\build.ps1 test --release
.\build.ps1 clippy --release --all-targets
$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { .\build.ps1 fmt }
git add src/main.rs
git commit -F - @'
main: --hear and --list-apps; speak only mutes the endpoint mix

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
'@
```

---

### Task 10: Integration — research routes to the HTTP lane when there is no CLI

**Files:**
- Modify: `src/main.rs` — `Args` (add a flag), `ContextServices` (line 243–246), `route` (line 257–303 and the Turn branch at 334–369), `main` (line 584–601).

**Interfaces:**
- Consumes: `coach::RESEARCH_PROMPT`, `Coach::research`, `Coach::cancel_research` (Task 4), `References::retrieve_deep` (Task 5).

- [ ] **Step 1: Flag and services**

Add after `research_prompt`-less flags (next to `prompt`):

```rust
    /// Research persona (F8). Falls back to a built-in prompt if missing.
    #[arg(long, env = "IV_RESEARCH_PROMPT", default_value = "research.md")]
    research_prompt: String,
```

Change `ContextServices` to:

```rust
struct ContextServices {
    agent: Option<agent::Agent>,
    references: references::References,
    research_prompt: String,
}
```

and its destructuring in `route` to `let ContextServices { agent, references, research_prompt } = services;`. Add `let mut last_them = String::new();` next to `let mut called: Option<String> = None;`.

- [ ] **Step 2: Route research**

Replace the `Msg::Research` and `Msg::CancelResearch` arms with:

```rust
            Msg::Research => {
                let transcript = history.render();
                if let Some(agent) = &agent {
                    agent.ask(format!("{transcript}{}", references.retrieve(&transcript)));
                } else if let Some(coach) = &coach {
                    // Research the newest THEM line — that is the question the
                    // user pressed F8 about — with the wide passage window.
                    let query = if last_them.is_empty() { &transcript } else { &last_them };
                    coach.research(
                        research_prompt.clone(),
                        format!("{transcript}{}", references.retrieve_deep(query)),
                    );
                } else {
                    let _ = tx.send(Msg::Sys(
                        "research off: needs a provider, or --agent-cmd".into(),
                    ));
                }
                continue;
            }
            Msg::CancelResearch => {
                if let Some(agent) = &agent {
                    agent.cancel();
                } else if let Some(coach) = &coach {
                    coach.cancel_research();
                }
                continue;
            }
```

In the `Msg::Turn` branch, after `called = roster.addressed(text).map(str::to_string);` add:

```rust
            if who.is_them() {
                last_them = text.clone();
            }
```

- [ ] **Step 3: Enable the keys and pass the prompt**

Replace `let research_enabled = agent_config.is_some();` with:

```rust
    // F8 works with a CLI *or* a provider now; only `--provider none` with no
    // CLI leaves it off, and then the keys are not registered at all.
    let research_enabled = agent_config.is_some() || provider.is_some();
    let research_prompt = std::fs::read_to_string(&args.research_prompt)
        .unwrap_or_else(|_| coach::RESEARCH_PROMPT.to_string());
```

and pass `research_prompt` into `ContextServices { agent, references, research_prompt }` in the `route` spawn.

- [ ] **Step 4: Build, then a live research check if a provider key is configured**

`.\build.ps1 test --release` → green. If `.env` has a working provider (Gemini free is the default), run the app with `--provider gemini`, speak a sentence into the mic or play one, press Ctrl+Shift+F8: the status line shows `· researching`, then the research pane fills within ~10 s. Ctrl+Shift+F9 mid-run must clear `· researching` and show `Research couldn't finish` + `cancelled`. If no key is available, say so in the commit message and rely on the unit tests from Task 4.

- [ ] **Step 5: Ladder and commit**

```powershell
.\build.ps1 clippy --release --all-targets
$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { .\build.ps1 fmt }
git add src/main.rs
git commit -F - @'
main: F8 research over HTTP whenever a provider is online

The CLI lane still wins when --agent-cmd is set. research.md overrides the
built-in research prompt.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
'@
```

---

### Task 11: Integration — HUD notice filter, docs, `.env.example`, LEDGER

**Files:**
- Modify: `src/hud.rs` `pump` notice filter (line 342–350); `README.md` (Controls, Audio and privacy); `CLAUDE.md` (Architecture traps); `LEDGER.md` (Now block, Decisions locked); `.env.example`.

- [ ] **Step 1: Notice filter**

In `pump`, the `Msg::Sys(text)` arm's `if` gains two prefixes:

```rust
                    if text.contains("failed")
                        || text.contains("stopped")
                        || text.starts_with("coach:")
                        || text.starts_with("research off")
                        || text.starts_with("Preview")
                        || text.starts_with("hearing:")
                        || text.starts_with("speak:")
                    {
```

- [ ] **Step 2: `.env.example`**

Replace the `# IV_SPEAK=false` line with:

```
# Read advice aloud. With IV_HEAR the voice is never captured; without it the
# call is deafened while the voice talks.
IV_SPEAK=1
# Hear only one app as THEM (any part of its name; --list-apps shows them).
# Watch a video during a call without it being transcribed as the far end.
# IV_HEAR=discord
```

and after the `IV_AGENT_TIMEOUT` line add:

```
# Research persona for F8 (falls back to a built-in prompt). F8 works with a
# provider alone now; a CLI is optional.
# IV_RESEARCH_PROMPT=research.md
```

- [ ] **Step 3: README**

In "Controls and research", after the paragraph that starts `Both panes follow new content`, add:

```markdown
The status line counts the wait for advice — `thinking 1.4s` while the model
works, then `first word 1.2s`, which is the time to first token on that turn
and stays up until the next one. `ASK` and `FIX` headings are the only saturated
colour; `SAY` is soft green and `NOTE` recedes. `THEM` is brighter than `YOU`
because `THEM` is what you react to. The last turn's advice stays on screen,
greyed, until the next replaces it. The `Ctrl+Shift+F11` reminder leaves the
status line after ten seconds.
```

Replace the sentence `Ctrl+Shift+F8 opens the research view.` paragraph's first two sentences with:

```markdown
Ctrl+Shift+F8 opens the research view. From any other view it just brings the
last result back up; pressing it again while that result is already on screen
starts a fresh job. With a provider configured, research runs over the same
connection as advice — no CLI needed — with the whole 24-turn window, up to
forty reference passages, and room to answer; `research.md` beside `prompt.md`
replaces the built-in research prompt. Setting `--agent-cmd` switches research
to that CLI instead:
```

In "Audio and privacy", after `Use a headset: ...` add a paragraph:

```markdown
To hear only your call app — and not the video you are watching, the music, or
the voice this app reads advice in — name it: `--hear discord` (any part of the
name; `--list-apps` shows what is playing). `THEM` is then that app's process
tree alone. If the app is not running yet the panel says `hearing: waiting for
discord…` and hooks it when it starts; if it restarts, the panel follows it to
its new process. Without `--hear`, `THEM` is everything the speakers play.
`--speak` reads advice aloud; with `--hear` the voice is never captured, without
it the call is deafened while the voice talks (the panel says so at startup).
```

- [ ] **Step 4: CLAUDE.md**

In the "Architecture" UX paragraph, after the sentence ending `the hand-written Win32 version could not.` add: `Decision 7's glance rules are in \`Tag\` (colours) and \`status_text\` (the wait as a number); both are tested. \`THEM\` capture is per-app when \`--hear\` names one (\`audio::open_process\`), with reacquire on restart; the endpoint mix is the fallback.`

Add a trap bullet after the `**No input gesture can be tested with injected input.**` bullet:

```markdown
- **A process-loopback stream whose app exits delivers silence, not an error.**
  `audio::pump` therefore polls `alive(pid)` every 2 s and bails, which is what
  lets `run`'s reacquire loop hook the app again under its new pid. Do not
  "simplify" that poll away; the stream will look healthy forever.
```

- [ ] **Step 5: LEDGER**

Under `## Decisions locked`, append after item 8:

```markdown
9. **On-disk is the source of truth (2026-09-06).** The app never coaches from
   a stale copy. `knowledge/` reloads on change, references live in a folder
   and reload, dropped files are remembered by path and never copied. (Wave 2
   implements this; recorded here because Wave 1's research design assumes it.)
10. **Capture is per-app when an app is named (2026-09-06).** `--hear <app>`
    captures only that process tree as `THEM`; the endpoint mix is the
    fallback. The `--speak` voice is another process and is therefore never
    heard, so the loopback mute survives in exactly one combination: `--speak`
    without `--hear`.
```

Amend item 4 by appending: `*Amended 2026-09-06: providers are choices; Gemini free is the default; no production credential is required to run. Claude direct remains the only wire with prompt caching.*`

Amend item 7 by replacing its italic closing note with: `*Restored in egui 2026-09-06 (Wave 1): the wait as a number, ASK/FIX-only saturation, THEM brighter than YOU, greyed-while-thinking, hint that leaves. The 6-line transcript cap is superseded by the user's 8-turn floor.*`

Add a new **Now** entry at the top of the Now block:

```markdown
**Wave 1 — glance & hearing (2026-09-06):** Decision 7 back in egui (`Tag`,
`status_text`, TTFT timing). Per-app capture: `--hear <app>` opens a
process-loopback client on the app's process tree, reacquires when it starts
late or restarts (`alive(pid)` poll — a dead target delivers silence, not an
error), falls back to the endpoint if the client cannot open. Spike result on
the 16 kHz mono autoconvert: <attempt that worked>. `--speak` keeps its mute
only without `--hear`. Research runs over HTTP on its own worker whenever a
provider is online (`Coach::research`, exactly one `ToolEnd` per job by CAS);
the CLI lane still wins when `--agent-cmd` is set. Verified: unit ladder,
`ui_smoke.ps1` (now asserts the TTFT instrument), and `tools/hear_isolation.ps1`
(two processes speak, only the named one becomes THEM).
```

- [ ] **Step 6: Build, smoke, commit**

```powershell
Get-Process inner-voice -ErrorAction SilentlyContinue | Stop-Process -Force
.\build.ps1 test --release
.\build.ps1 clippy --release --all-targets
$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { .\build.ps1 fmt }
# smoke recipe from Task 1 step 7 → PASS
git add src/hud.rs README.md CLAUDE.md LEDGER.md .env.example
git commit -F - @'
docs: per-app hearing, the TTFT instrument, research without a CLI

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
'@
```

---

### Task 12: Integration — the capture-isolation live check

**Files:**
- Create: `tools/hear_isolation.ps1`.

**Interfaces:**
- Consumes: the built `target/release/inner-voice.exe` with `--hear`, `--log`, `--provider none`; the JSONL log lines `{"t":…,"who":"THEM","text":"…"}`.

- [ ] **Step 1: Write the script**

```powershell
# Proves --hear isolates one app: two processes speak different sentences at
# the same time through the speakers, the panel hears only one of them, and
# only that sentence's words reach THEM. The microphone will pick up both from
# the speakers as YOU — that is expected and is why only THEM lines are judged.
#
# Speech comes from SAPI in each process directly, so the audio session belongs
# to that process. The two shells have different image names — `pwsh` and
# `powershell` — which is what --hear keys on. Run this from powershell.exe.
param([string]$Exe = '.\target\release\inner-voice.exe')
$ErrorActionPreference = 'Stop'
if (-not (Get-Command pwsh.exe -ErrorAction SilentlyContinue)) { throw 'pwsh.exe (PowerShell 7) is needed as the second voice' }
$log = Join-Path $env:TEMP 'iv-hear-isolation'
Remove-Item -Recurse -Force $log -ErrorAction SilentlyContinue
Get-Process inner-voice -ErrorAction SilentlyContinue | Stop-Process -Force

$app = Start-Process -PassThru -FilePath $Exe -ArgumentList '--provider','none','--hear','pwsh','--log',$log
try {
    Start-Sleep -Seconds 30   # model load and warm-up; the panel says "hearing: waiting for pwsh…"
    $heard = 'The purple elephant is dancing in the kitchen tonight.'
    $decoy = 'An orange giraffe is reading a newspaper by the river.'
    $say = { param($shell, $text, $times) Start-Process -PassThru -FilePath $shell -ArgumentList '-NoProfile','-Command',"`$v = New-Object -ComObject SAPI.SpVoice; 1..$times | ForEach-Object { `$v.Speak('$text') | Out-Null; Start-Sleep 2 }" }
    # The decoy talks first and longest; the target repeats so the second
    # reading lands after --hear has hooked its (new) process.
    $d = & $say 'powershell.exe' $decoy 3
    $t = & $say 'pwsh.exe' $heard 3
    $t.WaitForExit(); $d.WaitForExit()
    Start-Sleep -Seconds 8   # hang + transcription of the last utterance
} finally {
    Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue
}
$lines = Get-ChildItem $log -Filter *.jsonl | Get-Content
$them = ($lines | Where-Object { $_ -match '"who":"THEM"' }) -join ' '
"THEM heard: $them"
if ($them -notmatch 'elephant') { throw 'The named app was not transcribed as THEM' }
if ($them -match 'giraffe') { throw 'The decoy app leaked into THEM: --hear is not isolating' }
'PASS: --hear pwsh heard the elephant and not the giraffe'
```

- [ ] **Step 2: Run it**

```powershell
Get-Process inner-voice -ErrorAction SilentlyContinue | Stop-Process -Force
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\hear_isolation.ps1
```

Expected: `PASS: --hear pwsh heard the elephant and not the giraffe`. If `THEM heard:` is empty, the `--hear` hook never caught the target: raise the target's repeat count to 5 and re-run once; if still empty, this is a Task 7 finding — report it.

- [ ] **Step 3: Commit**

```powershell
git add tools/hear_isolation.ps1
git commit -F - @'
tools: hear_isolation.ps1 — two apps speak, only the named one becomes THEM

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
'@
```

---

### Task 13: Integration — full ladder, real run, wave commit

**Files:** none new.

- [ ] **Step 1: The whole ladder**

```powershell
Get-Process inner-voice -ErrorAction SilentlyContinue | Stop-Process -Force
.\build.ps1 test --release          # expect 60+ unit, 1 gpu_transcribes, 1 wasapi_loopback, 0 failed
.\build.ps1 clippy --release --all-targets
$out = .\build.ps1 fmt -- --check 2>&1 | Out-String; if ($out.Trim()) { "FMT DRIFT: $out" } else { "fmt clean" }
```

- [ ] **Step 2: Smoke test**

Task 1 step 7 recipe → `PASS`, and the process exits (`Get-Process inner-voice` finds nothing after `-CloseAfterCheck`).

- [ ] **Step 3: Real run with CUDA and the panel — the check that caught the OLE crash**

```powershell
$p = Start-Process -PassThru -FilePath .\target\release\inner-voice.exe -ArgumentList '--provider','none','--hear','powershell'
Start-Sleep -Seconds 45
if ($p.HasExited) { throw "exited early: $($p.ExitCode)" } else { 'still running with CUDA whisper + OpenGL panel + process loopback' }
Stop-Process -Id $p.Id -Force
```

- [ ] **Step 4: Isolation check**

`powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\hear_isolation.ps1` → `PASS`.

- [ ] **Step 5: Confirm the tree is clean and committed**

`git status --short` → empty. `git log --oneline -12` shows the task commits from Tasks 1–12 on top of `9c0ce1a`. Nothing else to commit. Report: the list of commits, the spike result from Task 7, and the isolation check output line.

**Then the gate:** the user runs a real call with `--hear <app> --speak --dump clips` and reports. Wave 2's plan is written against the tree as it stands after this.
