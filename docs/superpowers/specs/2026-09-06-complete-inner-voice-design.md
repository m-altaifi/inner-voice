# inner-voice — completing the product

Design spec, 2026-09-06. Approved in conversation section by section; this is the
written form the implementation plan derives from. Read `LEDGER.md` "Decisions
locked" first; this document amends it in §2 and otherwise obeys it.

## 1. Goal

`prompt.md` states it: *"They read you off a heads-up panel in under two seconds
and then decide whether to speak."*

inner-voice is not an application you use. It is a heads-up display for a live
conversation with a real person, happening now. The panel must never become the
thing you are doing. Every constraint in this document is derived from that:
anything that costs the user attention, hands, keyboard or focus is a defect,
even when it is a feature.

"Complete" means three things, in this order:

1. **Glance & hearing** — the panel meets the two-second requirement on a real
   call, and hears only the call.
2. **Memory** — on-disk is the source of truth; the app never coaches from a
   stale copy; it reads the formats the user actually has.
3. **Shippable** — a fresh machine goes from clone to a working call, and the
   user can tell what the app is using and that it works before a call starts.

Sub-project 1 ends in a human gate (a real call). Everything after that gate in
sub-project 1 is planned from the evidence the call produces, not designed here.

## 2. Amendments to locked decisions

These change `LEDGER.md` "Decisions locked". Everything not listed stands.

- **Decision 4 (provider) relaxes.** *Providers are choices; Gemini free is the
  default.* No production credential is required to run. The panel names the
  provider and model in its status line, and `--setup` measures TTFT for
  whatever is configured, so switching is a decision made from evidence. Claude
  direct remains the only wire with prompt caching and stays available.
- **Decision 7 (a glance, not a screen) is restored in egui.** The rewrite
  dropped three of its rules; they return (§4.1). The "6 transcript lines" cap is
  superseded by the user's explicit 8-turn floor.
- **New Decision 9 — on-disk is the source of truth.** The app never coaches
  from a stale copy. `knowledge/` reloads when a file changes; references live
  in a folder and reload; the CLI research lane reads disk live. Dropped files
  are remembered by path, never copied — a copy is stale the moment the original
  is edited.
- **New Decision 10 — capture is per-app when an app is named.** `--hear <app>`
  captures only that process tree as `THEM`; the endpoint mix is the fallback.
  TTS from `--speak` is therefore never heard, and the loopback mute survives in
  exactly one combination (`--speak` without `--hear`).
- **Research is no longer welded to Codex.** HTTP research through the provider
  table is the default; the CLI lane switches its default adapter to `claude -p`
  (installed, authenticated, read-only tools). Codex stays as an unverified
  adapter.

## 3. Scope and definition of done

| # | Sub-project | Done when |
| --- | --- | --- |
| 1 | Glance & hearing | On a real call the panel is readable in two seconds, it heard only the call, and the TTFT number is on screen. Judged by the user. |
| 2 | Memory | Edit a file mid-session and the next advice reflects it. Drop a PDF and its passages are cited. Restart and the references are still there. |
| 3 | Shippable | Fresh clone → `--setup` all green → a call works. Leaked keys rotated. |

**Step 0, before any of it: commit the baseline.** The tree has 4,851 lines and
no commits; parallel subagents need something to diff against.

**Out of scope** (deliberately): an installer or release zip; rasterising scanned
PDFs for OCR; verifying the Codex adapter; a filesystem watcher; a vector store
or embeddings (Decision 6 stands); a local Ollama model (open question stays
open); hotkey remapping; any button or view-switcher (Decision 8).

## 4. Sub-project 1 — Glance & hearing

Everything that must be true *before* the acceptance call.

### 4.1 Decision 7 in egui (`src/hud.rs`)

**Wait as a number (Doherty).** `State` gains `asked_at: Option<Instant>` (set on
`Msg::AdviceStart` for the live generation) and `first_word_at: Option<Instant>`
(set on the first `Msg::Advice` token of that generation). The status line reads
`thinking 1.4s`, live, while thinking; once the first token lands it freezes to
`first word 1.2s` and stays until the next `AdviceStart`. Pause clears both.
Digits are monospace so the number does not twitch. This is the standing TTFT
instrument on every turn of the acceptance call, which is why it ships before the
call rather than after.

**Never blank.** Old advice already stays until the first new token replaces it.
The missing half: while `thinking && !advice.is_empty()`, the old advice renders
in `MUTED`. One conditional.

**Von Restorff.** The tag table in `display_advice` gains a colour and becomes
the single source for heading text *and* colour:

| Tag | Heading | Heading colour | Body colour |
| --- | --- | --- | --- |
| `ASK` | ASK NEXT | `ACCENT` (0x7dc4ff) | `FG` |
| `FIX` | CLARIFY | `AMBER` (0xffb454) | `FG` |
| `SAY` | SUGGESTED WORDING | `SOFT_GREEN` (0x9fd8a0) | `FG` |
| `NOTE` | KEEP IN MIND | `MUTED` | `MUTED` |

Only `ASK` and `FIX` are saturated. Body size stays 19 px, headings 13 px.
`display_advice` returns `(Option<Tag>, body)`; the colour lookup lives beside
the table, not in the paint code.

**Contrast (Miller).** In both the conversation pane and the whole-conversation
view: `THEM` (any non-`YOU` label) label `ACCENT`, text `FG`; `YOU` label and
text `MUTED`. You react to them, not to yourself.

**Chrome.** `Ctrl+Shift+F11 for keys` shows for the first `HINT_SECS = 10`
seconds after launch and then leaves — a permanent hint is permanent noise, the
empty advice pane already carries it, and the list is one key away. `·
researching` and `· pinned` stay: they are state, not hints.

**Status line grammar** becomes a pure function so it can be tested:

```
fn status_text(mode: Mode, wait: Wait, model: Option<&str>,
               researching: bool, pinned: bool, hint: bool) -> String
Mode = Preview | Paused | Listening { online: bool }
Wait = Idle | Thinking(secs) | FirstWord(secs)
```

Renders, in order: mode text · model (sub-project 3 passes it; `None` until
then) · wait · `researching` · `pinned` · hint. Examples:
`Listening · coaching on · thinking 1.4s`,
`Listening · first word 1.2s · pinned`,
`Preview — microphone off · no online requests · first word 0.0s`.

**State mirror** (`IV_UI_DUMP`) gains `"wait": "" | "thinking" | "first word"`.
`--preview` seeds `AdviceStart → Advice → AdviceEnd` before the panel exists, so
it reports `first word` at ~0.0 s; the smoke test asserts that string.

### 4.2 Per-app capture (`src/audio.rs`, `src/main.rs`)

**Flags.** `--hear <name>` / `IV_HEAR`: which app to hear as `THEM`.
`--list-apps`: print the audio sessions on the loopback endpoint (the `--loopback`
device, else the default render device) as `pid  active|inactive|expired  name`.
Name is the session display name; when that is empty (common), the process image
name from `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` +
`QueryFullProcessImageNameW`, file stem. The system-sounds session (pid 0) is
skipped.

**Resolution** is a pure, tested function over `(pid, active, name)` triples:
case-insensitive substring on `name`; several matches → the *active* session
wins, then the first. `include_tree = true` on the chosen pid so child processes
(Discord's audio process, a browser's renderer) ride along.

**Capture.** `audio::Input` gains `hear: Option<String>` — the *name*, because
the app may restart with a new pid. When set, the `THEM` thread runs:

```
loop {
    match resolve(endpoint, name) {
        Some(pid) => { notice "hearing: {name}"; capture(pid) until error; notice "hearing: {name} stopped" }
        None      => { notice "hearing: waiting for {name}…" (once); sleep 2 s }
    }
}
```

`capture(pid)` opens `AudioClient::new_application_loopback_client(pid, true)`
and initialises it with the same 16 kHz mono f32 `WaveFormat` the pipeline
assumes. Per the crate docs, `get_device_period` is unsupported on that client:
skip it and pass a fixed 20 ms period. Everything downstream — gate, VAD,
segmenter, epoch, pause — is unchanged and never learns which client it is fed
by. Startup must not fail when the app is not running yet; the call app often
starts second.

**The spike inside the step.** Whether the process-loopback client honours
autoconvert to 16 kHz mono f32 is unproven. Order of attempts, each kept only if
the previous one fails on this machine:

1. `PollingShared { autoconvert: true }` with the 16 kHz mono format (minimal
   change).
2. `EventsShared` (the crate's documented example) with an event handle.
3. Request 48 kHz stereo f32 and add the downmix + decimate-by-3 the CLAUDE.md
   latency section warned would one day be needed — ~40 lines, integer ratio,
   no dependency. Only on this path.
4. `Msg::Sys("hearing: {name} failed: {e}; using the whole speaker mix")` and
   fall back to endpoint loopback.

The implementer deletes the branches that turn out unnecessary; the code keeps
only what is proven to be needed.

**Notices** are `Msg::Sys` strings prefixed `hearing:`; the HUD notice filter
gains that prefix.

**No `--hear`** → today's endpoint loopback, untouched.

### 4.3 `--speak` (`src/speak.rs`, `src/main.rs`)

Kept. `Speaker::new(mute: Option<Arc<AtomicBool>>)`; `main` passes `Some` only
when `speak && hear.is_none()`, with the startup notice
`speak: reading advice mutes the call; add --hear <app> so it doesn't`. With
`--hear`, TTS is another process and is never captured: no mute, no lost speech.
The compiled default stays off; `.env.example` ships `IV_SPEAK=1` for the user's
machine ("activated" is read as: on, on this machine, by configuration).

### 4.4 Research over HTTP (`src/coach.rs`, `src/main.rs`)

`Coach::research(context: String, references: String)` runs on its **own worker
thread** with its own generation counter, so the fast lane is never blocked.
Request: the research prompt (built-in default; `research.md` beside `prompt.md`
overrides it if present) as system text, then the 24-turn `history.render()` and
the reference passages as the user message. No 14-word limit. 60 s deadline.
The reply is accumulated from the existing streaming reader and delivered as the
existing `Msg::ToolStart(id)` / `Msg::ToolEnd(id, Ok(text) | Err(e))`, so the
research pane, the never-enters-history-on-error rule and the logging path are
unchanged.

**References for research** are the top-40 passages by the existing word-match
scorer against the last `THEM` turn (or the typed question), capped at 60 KB —
not "all indexed text": 24 files × 400 KB does not fit a request, and the ranker
already exists.

**Cancel (F9).** `Msg::CancelResearch` → `coach.cancel_research()`: bump the
counter and emit `ToolEnd(old_id, Err("cancelled"))` so the panel and router
close the generation — the same lesson as `AdviceStart`/`AdviceEnd`.

**Selection.** `research_enabled = agent_cmd.is_some() || provider != none`. F8
/ F9 register when enabled. `Msg::Research` routes to the CLI lane when
`--agent-cmd` is set, else to `coach.research`.

### 4.5 The gate

The user runs a real call with `--hear <app> --speak --dump clips` and brings
back: the clips, the JSONL log, and what the panel felt like in the moment. From
that, the tail is planned: `voiceid` thresholds from the clips (method in the
LEDGER speaker-embedding spike), and whatever the glance failed on. Not designed
here.

### 4.6 Verification for sub-project 1

- Unit: `status_text` grammar; the tag→colour table; `resolve` over a fake
  session list; research prompt assembly; existing `dragged`/`hit_test`.
- Smoke (`tools/ui_smoke.ps1`): assert `wait == "first word"` on the seeded
  preview; existing assertions unchanged.
- Live, scripted on this machine: **capture isolation** — audio playing in two
  processes, `--hear` one of them, assert only its speech becomes `THEM`. This is
  the claim that matters and it is scriptable.
- Live: `--list-apps` lists a playing app; `--hear` reacquires after the app is
  closed and reopened; a real run with CUDA (the OLE crash only ever appeared
  there).

## 5. Sub-project 2 — Memory

### 5.1 `knowledge/` reloads (`src/knowledge.rs`, `src/main.rs`, `src/coach.rs`, `src/audio.rs`)

`knowledge::Corpus { dir, newest: SystemTime, text, glossary }` with
`fn refresh(&mut self) -> bool`. `newest(dir)` is the max of every file's mtime
and the directory's own mtime (adds/removes change it on NTFS); it is a scan of a
few dozen files, microseconds. `route()` calls `refresh()` before each coach
request. When it reloads: `coach.set_prompt(build_prompt(persona, &text))` (the
worker reads an `Arc<RwLock<String>>` per request), the whisper workers' glossary
`Arc<RwLock<String>>` is replaced (they read it before each inference instead of
capturing a string at spawn), and `Msg::Sys("knowledge reloaded: {n} files, {kb}
KB")` is sent. The HUD notice filter gains `knowledge`. A changed corpus busts
the Anthropic cache prefix once — intended. Sorted order, 400 KB cap and marker
stay. No file watcher: per-turn is exactly the granularity that matters.

### 5.2 Persistent references (`src/references.rs`, `src/main.rs`)

- `--references <dir>` / `IV_REFERENCES`, default `references/`. At startup every
  supported file in it is imported through the same pipeline as a drop, with the
  same reading/ready/error feedback. `preview()` names the folder at the top.
- Dropped files are **not copied**. A manifest `<dir>/.dropped` (one absolute
  path per line) records drops whose path is outside `dir`; startup re-imports
  the originals. A missing original is listed with an error, not dropped
  silently.
- Dedup key becomes `(path, mtime)`. `stale()` lists imported files whose mtime
  changed; `retrieve` and `local_answer` re-import them first.
- `clear()` (F10) empties the index and truncates the manifest; folder files
  return on the next launch, because that is what the folder is for. README says
  so.

### 5.3 One loader — `src/extract.rs`

`pub fn text(path: &Path) -> Result<String>`, called by both `knowledge::load`
and `references::import`. `csv_to_text` and `sheet_to_text` move here from
`knowledge.rs`. Consequence: `knowledge/` gains the new formats too.

| Extension | Method |
| --- | --- |
| txt, md | read |
| csv | existing |
| xlsx, xlsm, xls, ods | existing (calamine) |
| pdf | `pdf-extract`, text layer only. Near-empty text → error `no text layer; export pages as images for OCR`. |
| docx | `zip` + `quick-xml`, both already transitive under calamine and promoted to direct deps at the same versions: `word/document.xml`, paragraphs → lines, table cells tab-separated. No `docx` crate. |
| png, jpg, jpeg, bmp, tif, tiff, gif | WinRT `Windows.Media.Ocr` through the `windows` crate already in the tree: file stream → `BitmapDecoder` → `SoftwareBitmap` → `OcrEngine::TryCreateFromUserProfileLanguages()` → `RecognizeAsync` → lines. Features `Media_Ocr`, `Graphics_Imaging`, `Storage`, `Storage_Streams`; async results via `windows-future`. Needs a COM apartment on the calling thread (the import thread; `main` is already MTA). No engine → error naming the Windows language-pack setting. |

Existing limits (10 MB per file, 400 KB text) unchanged. Per-file errors surface
as import feedback and never abort the batch.

### 5.4 CLI research → `claude -p` (`src/agent.rs`; `src/process.rs` unchanged)

Adapter chosen by the executable's file stem. `claude` →
`claude -p --output-format stream-json --allowedTools Read,Grep,Glob` with cwd =
`--agent-root` (default: the launch directory, so `knowledge/` and `references/`
are reachable) and the prompt on stdin (never a shell). Tools not in
`--allowedTools` are denied in `-p` mode, so side effects are impossible — the
LEDGER safety rule applied even though F8 is already a keypress. Tools are kept,
so the LEDGER's 3.1 s "stripped" floor does not apply; this is the slow lane.

**The schema is settled, not guessed:** one real `stream-json` capture on this
machine is a plan step that precedes the parser, and the parser test pins to the
captured shape. The `codex` adapter stays as-is, documented as unverified. The
research prompt names `knowledge/` and `references/` as the places to read —
live from disk, which is the on-disk access the user asked for.

### 5.5 ES

A checkbox in Everything (tick E:), no code. `--search-query` keeps feeding the
CLI lane.

### 5.6 Verification for sub-project 2

- `extract::text` — one fixture per format under `tests/fixtures/` (a one-page
  PDF with known text, a `.docx`, a `.png` of rendered text). OCR skips when no
  engine, the way the GPU test skips without its model.
- Reload — the offline-provider harness from `coach.rs`: two turns with an edit
  to a temp `knowledge/` between them, assert the prompt changed.
- Persistence — temp folder + manifest round-trip; `(path, mtime)` dedup
  re-imports an edited file.
- `claude -p` — the live capture, then the pinned parser test.

## 6. Sub-project 3 — Shippable

- **`--setup`** (`src/setup.rs`): each prerequisite prints ✓ or ✗ with the fix
  next to it, exit code non-zero on any ✗: model file (with the download
  command); CUDA (build the context and run the existing warm-up — "does it
  run", not accuracy — with the time); mic + speakers + `--list-apps`; the
  configured provider (key present, one tiny request, **TTFT printed in ms**);
  OCR engine present; `knowledge/` and `references/` found with counts and
  sizes, `references/` created if missing.
- **Provider label.** `hud::Session` gains `model: Option<String>`; the status
  line shows it (§4.1 grammar). When a model is named, `coaching on` is dropped
  as redundant.
- **Checklist (user, not code):** rotate the OpenRouter and Gemini keys the
  LEDGER records as present in a chat transcript; record it in the LEDGER when
  done.
- `.env.example` gains `IV_HEAR`, `IV_REFERENCES`, `IV_SPEAK=1`; `.gitignore`
  gains `references/`. README "Setup" leads with `--setup`; "Controls" documents
  `--hear`, `--list-apps`, `--references`, the speak behaviour, persistence
  semantics, and the new formats.
- No installer, no release zip: build-from-source is the story; this makes it
  true.

## 7. Verification ladder — every task, no exceptions

`.\build.ps1 test --release`, `clippy --release --all-targets` clean, `fmt --check`
clean; and for anything touching the panel or audio, `tools/ui_smoke.ps1` against
a live `--preview` **or** a real run. This session established twice that a green
`cargo test` can mean nothing: the OLE `RPC_E_CHANGED_MODE` crash only appeared on
a real run, and a drag test passed while measuring nothing. No task is done on
unit tests alone. The planner re-runs the live checks at every wave boundary.

## 8. Implementation model — waves of Opus 5 subagents

The hazard is two agents editing `src/main.rs`: it is the hub (flags, `route()`,
wiring). Each wave's agents therefore own **disjoint files**, and each wave ends
with **one integration task** that wires `main.rs`, updates README / CLAUDE.md /
LEDGER, runs the full ladder and commits. Commits land on `main`, one per wave.

| Wave | Parallel tasks (file ownership) | Then |
| --- | --- | --- |
| 0 | — | Baseline commit of the tree as it stands. |
| 1 | **A** `hud.rs` — §4.1 + tests + smoke `wait` · **B** `audio.rs`, `speak.rs` — §4.2 spike + §4.3 · **C** `coach.rs` — §4.4 + tests | **I1** `main.rs`: `--hear`, `--list-apps`, `resolve`, speak notice, research routing; docs; capture-isolation live check; ladder; commit. |
| — | **Gate: the user's real call.** | Tail planned from evidence. |
| 2 | **D** `extract.rs` + fixtures + tests, `knowledge.rs` switched to it · **F** `agent.rs` — §5.4 with the live capture first | **E** `references.rs` — §5.2 (after D lands). **I2** `main.rs`, `coach.rs`, `audio.rs`: §5.1 reload, `--references`; docs; ladder; commit. |
| 3 | **H** `setup.rs`, `Session.model` + status label, `.env.example`, `.gitignore`, README | **I3** final ladder; commit. |

Parallel tasks touch only the files they own. The integration task runs alone
after they land and may touch any file — that is where cross-file wiring (the
HUD notice filter for `hearing:` / `knowledge`, `main.rs` flags, the glossary
handle) belongs. The planner (this model) reviews each wave's diff against this
spec before the next wave starts; Opus 5 agents implement.

## 9. Risks, stated plainly

| Risk | Mitigation |
| --- | --- |
| Process-loopback client refuses 16 kHz mono autoconvert | The four-step attempt order in §4.2; worst case ~40 lines of downmix + decimate. |
| `pdf-extract` fails to build under this toolchain or chokes on a file | Alternative: `lopdf` with manual text extraction. Per-file errors never abort an import. |
| No OCR language pack on the machine | Clear error naming the setting; test skips; `--setup` reports it. |
| `claude -p` stream-json shape unknown | Capture before parse; parser pinned to the capture. |
| Free-tier provider slow or throttled for research | 60 s deadline; shown as an error; never enters history. |
| The real call reveals something structural (e.g. per-app capture misses audio) | That is what the gate is for; it becomes a new brainstorm, not an improvisation. |
| Subagent edit conflicts | Waves with disjoint file ownership; single integrator per wave. |
| Something private in the baseline commit | Checked: `knowledge/` is the preview's sample data; no key-shaped strings; `.env`, logs, models, clips ignored. |

## 10. Needs from the user

- The real call, with `--hear <app> --speak --dump clips`, and a report.
- Rotate the two leaked keys.
- Optional: tick E: in Everything's index.
- `--setup` will say whether an OCR language pack is installed.
