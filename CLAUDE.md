# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build and test

Always go through `build.ps1` — never bare `cargo`. It sets the CUDA/MSVC
environment whisper-rs needs on this machine (Ninja generator, explicit
`-ccbin`, `-allow-unsupported-compiler`, and a deliberately minimal `PATH`
because `vcvars64.bat` overruns cmd's 8191-char limit otherwise). Everything
after the script name is forwarded to cargo.

```powershell
.\build.ps1 build --release
.\build.ps1 test --release                 # debug links fail: prebuilt onnxruntime is /MD, knf-rs-sys goes /MDd (LNK2038)
.\build.ps1 test --release emits_one_utterance   # single test by name substring
.\build.ps1 run --release -- --list-devices
```

- `tests/gpu_transcribes.rs` skips itself unless `models/ggml-large-v3-turbo-q5_0.bin`
  exists (see README for the download). It is the only evidence the unsupported-compiler
  override did not corrupt runtime — re-run it after any toolchain change.
- `tests/wasapi_loopback.rs` needs real audio hardware; 0 captured bytes is a pass.
- The live provider test in `src/coach.rs` costs money and is opt-in:
  `$env:IV_LIVE_TEST=1; .\build.ps1 test --release live_provider`.
- `CMAKE_CUDA_ARCHITECTURES` in `build.ps1` is hardcoded to `86` (RTX 3080 Ti).
- `.\target\release\inner-voice.exe --setup` is the first thing to run on a new
  machine or after editing `.env`: it loads the model, warms CUDA, lists devices
  and playing apps, times the provider's first token, and checks OCR and the
  folders.

## Architecture

UX update: the HUD is an **eframe/egui** overlay (`eframe` with the `glow`
backend; no Win32 window class or child control is left). It is one page, not a
view-switcher: status line, advice, conversation and question box are on screen
together, because all three are per-turn and needing a keypress to see one of
them is the same mid-sentence interruption as reaching for the mouse. The
conversation sits *below* the advice and above the question box. Only
deliberately-consulted things (references, diagnostics, research, the key list)
take over the main pane. `KEYS` in `src/hud.rs` is still the entire action
table: one `RegisterHotKey` per row on Ctrl+Shift+F1..F12, dispatched by
`State::command`, and the id in each row is both the hotkey id and the id a test
posts as `WM_HOTKEY`. The conversation pane grows with the transcript up to
`VISIBLE_TURNS` (8) rows and advice takes the rest. Auto-scroll is `ScrollArea::stick_to_bottom`,
which also re-sticks when a reader scrolls back to the end — the hand-written
Win32 version could not. Decision 7's glance rules are in `Tag` (colours) and
`status_text` (the wait as a number, which doubles as the standing TTFT
instrument); both are tested. `THEM` capture is per-app when `--hear` names any (`audio::open_process` on each
app's process tree, one stream per name because WASAPI's activation params take
a single tree), with reacquire when one starts late or restarts; the endpoint
mix is the fallback, and only when a *single* app was named — falling back with
several would transcribe the others twice, since the mix already contains them.
The selection lives in `Tune` (`hear` + `hear_gen`), not in `Input`, because
`/hear` in the question box rewrites it mid-call: every stream polls the
generation on the same two-second tick that notices an app closed, and bails.
**`run` is shared by both capture threads, so a selectable source must be gated
to `THEM`** — the mic early-returns to `open_endpoint` before the supervisor.
Letting it through cost one red `hear_isolation.ps1`: the mic thread opened the
same process loopback and the target app's sentence was logged as `YOU`. The app is looked for only in
the `--loopback` device's own sessions, so an app playing elsewhere is never
found. The *whole* window is the drag grip (`hit_test` returns
`Grab::Move` for anything that is not a resize edge), which is why
`style.interaction.selectable_labels` is off: text selection would swallow the
gesture, and it was dead weight anyway since a `WS_EX_NOACTIVATE` window never
has focus to answer Ctrl+C. Ctrl+Shift+F12 pins, freezing move and resize both.
- **The panel moves itself; do not go back to `ViewportCommand::StartDrag`.**
  That posts `WM_NCLBUTTONDOWN`/`HTCAPTION` and leaves the rest to Windows'
  modal move loop, which needs the window to be the foreground one and needs
  mouse capture — and `WS_EX_NOACTIVATE` exists precisely so this window never
  becomes foreground. The symptom was a panel that could only be dragged after
  being activated from the taskbar. `State::move_window` follows the cursor with
  `GetCursorPos` + `SetWindowPos(SWP_NOACTIVATE)` instead: no capture, no
  activation, no modal loop. A drag ends on the *first* sign of
  release from either `GetAsyncKeyState(VK_LBUTTON)` or egui's pointer, because
  each can miss one alone — without capture a release outside the window never
  reaches egui, and the async key state can be left stale by injected input —
  and a drag that outlives its release glues the panel to the pointer. That is
  not hypothetical: it made `ui_smoke.ps1` fail intermittently, with a later
  position assertion silently measuring the stuck drag instead of itself. The
  panel therefore mirrors `dragging`, and the script waits for it to clear
  before trusting any window position. `dragged()` has the geometry and tests.
`references.rs` owns the references folder, the manifest of dropped paths, and
local passage retrieval; `extract.rs` is the one path→text loader both it and
`knowledge.rs` use.
`--preview` opens the real interface with sample conversation and no
audio/network (its research keys are not registered). `tools/ui_smoke.ps1` checks
this preview only, and needs `IV_UI_DUMP` set to a path before launch: no child
controls exist to read with `GetDlgItem` any more, so the panel mirrors its
*state* to that file — deliberately not its render, which would be a second copy
of the layout free to disagree with the screen. Screenshots cover the drawing.
`Msg::Sys` lines are panel-only: they land in Diagnostics (Ctrl+Shift+F4) and are
never written to the JSONL log — `route` records `COACH`, `RESEARCH` and turns
and drops `Sys`. Of the `Sys` diagnostics only the prefixes `pump`'s filter
admits reach the notice line (`hearing:`, `speak:`, `coach:`, `research off`,
`Preview`, `knowledge`, and anything saying "failed" or "stopped"); reference status,
research start/end, cleared references and the hotkey-clash note set `notice`
directly and pass through no filter at all. Pause changes an audio epoch, and `audio.rs` discards queued
and in-flight work from earlier epochs — but only up to `Msg::Turn`. A `Turn`
carries no epoch, and `Msg::Pause` travels the same FIFO behind it, so one turn
captured just before the pause still lands. Do not describe pause as a hard
barrier past the audio worker.

Current additions (2026-09-05): `history.rs` owns the typed 24-turn context.
`agent.rs` runs optional hotkey-triggered research through Claude Code's
`claude.exe` (`--agent-cmd`; anything else is rejected at startup); `process.rs` contains
hidden subprocesses with Job Objects, cancellation, deadlines, and output caps.
`search.rs` uses bounded Everything UTF-8 exports. `setup.rs` runs the `--setup`
checks and renders them; it calls `audio::warm`, `audio::sessions`,
`coach::probe` and `extract::ocr_available` rather than duplicating any of them.
Research completions and coach messages go through the router for logging. The
coach now owns one HTTP worker and a single newest pending request, rather than
spawning per turn. See README for the current controls and configuration; older
thread counts below are history.

Threads plus crossbeam channels; one `Msg` enum (`src/main.rs`) is everything
the panel can render.

```
mic    (Capture) ─→ audio::run ─┐                    ┌─→ route() ─→ ui_tx ─┐
                   (read/VAD)   ├─→ utt_tx ─→ worker ─┤  (name + log)       ├─→ hud::run
speakers (Render) ─→ audio::run ┘   per stream        └─→ coach::ask ───────┘  (main thread)
                   (read/VAD)      (whisper ∥ voiceid)    (SSE, per turn)
```

Six threads: two capture, two whisper workers, one router, one HUD (plus one per
in-flight coach request, one scoped embedding thread per far-end utterance, and
SAPI's if `--speak`).

- `Msg::Turn` carries a `Who`: `You`, or `Them { voice, name }` where `voice` is
  an audio cluster and `name` a roster binding. They are separate because they
  come from different evidence — audio says *which* voice, transcript says what
  it is *called*.
- **Naming lives in `route()`, not in the THEM worker**, and must stay there:
  "Sarah, what do you think?" is normally spoken by YOU, on the other capture
  thread. `route()` is the only place that sees both streams, in order, single
  threaded — which is also why the JSONL log is written there.

- **YOU vs THEM is the two sockets, not a model.** Mic = `YOU`, render endpoint
  opened for capture (loopback) = `THEM`. Exact by construction. Telling apart
  people *within* `THEM` is a separate mechanism (`src/voiceid.rs`) and is
  best-effort; YOU/THEM never is.
- **No resampling or downmix code exists** because WASAPI's `autoconvert` gives
  16 kHz mono f32 directly. `tests/wasapi_loopback.rs` guards that assumption; if
  it fails, `src/audio.rs` needs a resampler.
- **COM interfaces are not `Send`.** `main` resolves devices to *names* (so a bad
  `--mic` fails before the 570 MB model loads) and each capture thread reopens by
  name after its own `initialize_mta`.
- **Capture threads never call whisper.** They read, gate and segment (all
  microseconds) and hand finished utterances to a per-stream worker over a
  channel. Inference on the capture thread meant WASAPI went unread for the
  duration and the VAD's silence clock stopped with it — capture has a realtime
  deadline, inference does not. Don't put anything slow back on that thread; the
  `--dump` writes live in the worker for the same reason.
- One shared `WhisperContext`, one `WhisperState` per worker — `WhisperState` is
  not `Send`, so each worker builds its own from the shared (`Sync`) context, and
  both speakers decode in parallel with no lock.
- Each worker runs one throwaway inference at startup (`warm`), because the first
  CUDA call costs 100 ms+ and would otherwise land on the call's first turn.
- `route()` keeps a 24-turn window and only asks the coach on a finished `THEM`
  turn of `--min-words` or more.
- **Coach cancellation is a generation counter.** `ask()` bumps `seq`; the SSE
  reader abandons the stream when `live != seq`, and the HUD drops any
  `Msg::Advice` tagged with a superseded generation.
- `hud::run` must own the main thread: `eframe::run_native` runs winit's event
  loop there. State lives in the `eframe::App` itself now, not a `thread_local`
  `RefCell` — which also retires the whole re-entrancy hazard the `wndproc`
  version had, since nothing re-enters a borrow.

### Latency budget (~1.2 s to first word)

`HANG_MS` (500 ms, `src/audio.rs`) + whisper (~100 ms for a 4 s turn) + provider
TTFT (~600 ms warm). Any change that adds a hop — a router, a subprocess, a
retrieval step — is measured against this. `HANG_MS` now dominates.

The budget is deliberately not spent down: whisper runs beam search and a full
`audio_ctx` because accuracy is scarcer here than milliseconds. Don't "optimise"
either back without real-call evidence.

Two sweeps now run on the router thread ahead of every `coach.ask`, both
microseconds in the normal case and both with a named tail.
`references::retrieve` calls `stale()`, one stat per indexed file (24 at most) —
but a *remembered* drop on a mapped drive that has gone away stats a dead
network path, and the turn waits out the SMB timeout; move the sweep into the
import worker and read its last result if that ever bites. `Corpus::refresh`
stats `knowledge/` and every entry in it, and on the turn *after* an edit
re-reads the whole folder synchronously — PDF parsing and Windows OCR included,
so an image brief is seconds, not microseconds; a per-file cache keyed by
(path, mtime) is the upgrade there. Both notes are also `ponytail:` comments at
the code.

Traps here, each already paid for once (see LEDGER):

- **ASR accuracy cannot be benchmarked on `jfk.wav` or on TTS.** The model has
  that clip memorised and completes it from language prior even with the audio
  truncated; synthetic speech scores identically at every setting. Both were
  tried, both reported no difference, and neither could have. Use `--dump` to
  collect real utterances instead.
- The coach must read the SSE body **to its end**, not `break` on the terminator.
  ureq only pools a connection once the body is exhausted, so breaking early
  throws the socket away and re-handshakes TLS on every turn (~290 ms).
- Whisper's `initial_prompt` window is ~224 tokens. `knowledge::glossary` caps at
  800 chars for that reason; an overrun drops the front silently.
- `ort` is on ndarray 0.17 and `knf-rs` on 0.16, so their array types are
  unrelated to the compiler. Bridge tensors with `(shape, Vec<f32>)`.
- **`voiceid`'s thresholds are unvalidated** (0.70 / 0.50). Do not "fix" the fact
  that the far end often stays `THEM` by lowering them on intuition — that trades
  an unnamed turn for a misattributed one, which `prompt.md` explicitly forbids.
  Tune from `--dump` audio of a real call, or leave them.
- **egui, not Win32 — and the two traps that cost the most to find.** (1) winit
  recomputes the window's extended style whenever anything about the window
  changes and knows nothing about `WS_EX_NOACTIVATE`, so setting it once at
  startup is silently dropped. `State::keep_unfocusable` re-asserts it every
  frame and ties it to `typing`, because the same flag that stops a click
  stealing the call's keyboard also stops a keystroke reaching the question box.
  (2) `ViewportBuilder::with_drag_and_drop(true)` calls `OleInitialize`, which
  demands an STA thread, and `main` has already put this one in MTA to enumerate
  WASAPI devices — so the app aborts at window creation with `RPC_E_CHANGED_MODE`.
  Files therefore arrive as `WM_DROPFILES` through the winit message hook, which
  needs no COM. Neither trap is reachable under `--preview`, and the second only
  appears on a real run — which is the argument for doing one.
- **The panel is driven by global hotkeys. Do not add buttons or a view-switcher
  back.** It is an always-on-top overlay used while the *call* app owns the
  keyboard, so any control that must be focused to be pressed costs a mouse grab
  mid-sentence — which is the thing the window exists to avoid. `RegisterHotKey`
  delivers `WM_HOTKEY` to the window whatever has focus; an accelerator table only
  works once you have stolen focus, which is why it is not the mechanism. A new
  action is a row in `KEYS`, not a control. Quit has no key on purpose (Alt+F4),
  and Escape only clears the question box. Registration is first-come
  process-wide and fails *silently*, which used to leave an action unreachable
  with no sign; `run` now collects the losers into the notice line and
  Diagnostics. `RegisterHotKey` still reaches the app under winit, but only via
  `with_msg_hook` — winit owns the message loop and would drop `WM_HOTKEY`.

- **No input gesture can be tested with injected input.** Keystrokes
  (`keybd_event`/`SendInput`) *and* mouse events are discarded when the
  foreground window outranks the sending process — `SetCursorPos` itself returns
  false — so a press never arrives and the assertion passes having tested
  nothing. It fails the same way against the old Win32 build, so a green run
  proves nothing either way. `ui_smoke.ps1` therefore posts `WM_HOTKEY` for
  dispatch, proves hotkey *ownership* by trying to `RegisterHotKey` the same
  combination itself and requiring failure, and prints an explicit SKIPPED line
  when injection is unavailable rather than passing quietly. The drag gesture
  has no substitute: `hit_test` is unit-tested, and the gesture needs a human.
- **`--manual` is not Pause, and the two must not be merged.** Pause bumps the
  audio epoch and `route` drops the whole `Msg::Turn` — no log, no history, no
  name binding — because "stop listening" is what it means. `--manual` and
  Ctrl+Shift+F10 only stop the *unbidden* `coach.ask`: the turn is still
  transcribed, named, logged and pushed to history, so arming advice later
  starts from a full 24-turn window rather than a blank one. This exists
  because the panel is meant to run for a working day, where advice on every
  overheard sentence is a bill and a distraction. `status_text` shows which
  state it is in — a muted coach and a dead one are otherwise the same picture,
  and `a_muted_coach_says_so_rather_than_looking_idle` guards that.
- **A key or a command, never neither, and the split is not arbitrary.** F10
  went from Clear references to the advice toggle because the toggle is
  frequent and reversible while a clear is rare and destructive — a fumbled
  F-key mid-sentence should reach the recoverable one. Clearing kept its `id`
  and handler and moved to `/clear`. `COMMANDS` in `src/hud.rs` is that second
  table, rendered under `KEYS` in the help view, and
  `every_action_is_reachable_by_exactly_one_key` asserts an action never loses
  both. A command is handled by whoever owns the state it changes: `/hear` in
  `route` (it writes `Tune`), `/clear` in the panel (it also empties the
  panel's import list, which `route` cannot reach).
- **The app selector is a typed command, not a control.** `/hear` is parsed in
  `route()`'s `Msg::Question` arm because F7's box is the only place in a
  no-controls panel where a name can be typed — and unlike a picker of what is
  playing, a typed name can register an app that has not started yet, which is
  what `sessions()` can never list. Bare `/hear` reports rather than clears: a
  user checking the selection should not risk dropping the far end.
- **A process-loopback stream whose app exits delivers silence, not an error.**
  `audio::pump` therefore polls `alive(pid)` every 2 s and bails, which is what
  lets `run`'s reacquire loop hook the app again under its new pid. Do not
  "simplify" that poll away; the stream will look healthy forever.
- **A process-loopback stream must not be calibrated.** It starts only when the
  app plays, so the calibration second lands on the first *sentence* — measured
  once at gate 0.5942, which then gated every later turn out and produced zero
  turns until `--sys-gate` was forced. It is the app's own digital output and has
  no noise floor to measure, so `app_feed` pins `GATE_FLOOR` (0.004) unless
  `--sys-gate` overrides. The endpoint mix, *and the fallback to it*, still
  calibrate. Verified live: `THEM gate 0.0040 (fixed)` and speech transcribed
  with no `--sys-gate`.
- **The `--speak` mute follows `--speak`, not `--hear`.** The mute `Arc` exists
  whenever `--speak` is on; the process-loopback feed opts out (`mute: None`)
  because the voice is another process and deafening that stream would only lose
  the far end. Both endpoint pumps — including the fallback taken when the app
  client cannot open — stay deafened while the voice talks, or the panel coaches
  on its own advice.
- **The CLI research lane is Claude Code only.** `agent.rs` accepts nothing but
  `claude.exe` (the Codex adapter was removed on 2026-09-06; the user has no
  Codex access) and parses its `stream-json` pinned to a captured run
  (`tests/fixtures/claude-stream.jsonl`). Still unverified offline: the
  `--disallowedTools Read(./.env)` flag — a rejected flag surfaces as `unknown
  option` inside the no-answer error on the first real F8 press. F8 runs over the
  coaching provider by default (`Coach::research`), which *has* been run live
  against Gemini; `--agent-cmd` switches to the CLI, and the router keeps exactly
  one lane live per session.
- **Every `AdviceStart` must get an `AdviceEnd`.** `ask` supersedes by announcing
  a *newer* generation, so the panel moves on by itself; `cancel` has no
  successor and must close the generation it retires, or the panel sits on
  "preparing advice" forever — reachable with one Pause press, because the
  `AdviceStart` can still be in the channel behind the `Msg::Pause` that caused
  the cancel. `Coach::cancel` therefore emits the terminal message, and both
  consumers (`route`, `hud`) ignore a repeat rather than logging or speaking it
  twice. `coach.rs`'s `cancelling_closes_the_generation_it_retired` guards this.
- **`claude -p` costs depend entirely on what it loads.** The interactive
  harness (hooks, plugins, MCP, CLAUDE.md dirs) is 37 K cache tokens per call;
  `--strict-mcp-config --setting-sources ""` is 6.5 K and keeps the login.
  `--bare` drops the login. `result.subtype` is "success" even on failure —
  `is_error` is the signal. All measured; the fixture is the proof.

### Providers

`src/provider.rs` is a preset table, not a trait: two wire formats
(`Wire::Anthropic`, `Wire::OpenAi`) cover five vendors, since OpenAI, DeepSeek,
Gemini and OpenRouter all speak `/chat/completions`. Anthropic keeps its own path
only for `speed:"fast"` + `effort:"low"` + prompt caching. Adding a vendor that
speaks the OpenAI shape is one row in `preset()`.

Providers whose model ids churn (openai, gemini, openrouter) deliberately carry
**no default model** — a stale default 404s mid-call; a startup error does not.

### Knowledge corpus

`knowledge/` (`.md .txt .csv .xlsx .xlsm .xls .ods .pdf .docx` and images) is
loaded at startup, pinned into the system prompt behind an Anthropic cache
breakpoint, and re-read by `route()` before each coach request whenever the
folder's newest mtime moves (`knowledge::Corpus`). No retrieval,
no chunking, no embeddings — deliberate, and capped at 400 KB. Files are sorted so
the cached prompt prefix stays stable; do not change that ordering casually.

### The prompt is coupled to the HUD

`prompt.md` emits lines tagged `ASK` / `SAY` / `NOTE` / `FIX`. `display_advice()`
in `src/hud.rs` matches those four literals exactly (case-sensitive, and only
when followed by whitespace) and returns `(heading, body)` sections the panel
styles — an accent heading over large body text, which an `EDIT` control could
never do and which is why the headings used to be faked as plain text. Change the
tag vocabulary in one place and it silently renders as body text in the other.

## Conventions

- Every CLI flag has an `IV_*` env fallback via clap `env`, and `dotenvy::dotenv()`
  runs before `Args::parse`, so the app runs bare with a `.env`. Add both when you
  add a flag, and document it in `.env.example`.
- **The `.env`'s folder becomes the working directory.** Nine defaults are
  relative (model, `prompt.md`, `research.md`, `knowledge/`, `references/`,
  `logs/`, `es.exe`), which used to mean the app only ran when launched *from*
  the project directory — the actual reason starting it meant typing a path.
  `dotenv()` already walks up to find the file, so `main` adopts its parent with
  `set_current_dir`. A relative default is therefore safe to add; do not "fix" it
  into an exe-relative path. No `.env` anywhere leaves the directory untouched.
  `inner-voice.cmd` at the root is the double-click entry point and forwards its
  arguments.
- `LEDGER.md` is the durable project state — phases, measured numbers, and locked
  decisions that are not to be re-litigated. Update its **Now** block after a work
  session; read the decisions before proposing an architecture change.
- Comments here explain *why a simpler thing was rejected*, not what the code does.
  Match that; `ponytail:` marks a deliberate shortcut with its ceiling.
- Windows-only by design (WASAPI, SAPI, global hotkeys, `WM_DROPFILES`). egui
  is portable; everything it sits on here is not, so there is no cross-platform
  path.
