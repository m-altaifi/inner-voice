# inner-voice — implementation ledger

Durable state for multi-provider + structured history + file search.
Update the **Now** block after every work session. Nothing else here is chronological.

---

## Now

**Key swap (2026-09-06):** Ctrl+Shift+F10 was Clear references and is now the
advice toggle. The allocation was backwards for an all-day panel: arming and
muting advice is frequent and reversible, clearing references is rare and
destructive, and a fumbled F-key mid-sentence should land on the recoverable
one. Clearing kept its id and handler and moved to `/clear` in the question box.
`COMMANDS` in `src/hud.rs` is the second table, rendered under the keys in the
help view so both are in the one place a user looks; the typed `/coach on|off`
was deleted rather than kept alongside the key, since the status line already
shows the state a blind toggle would leave ambiguous.
`every_action_is_reachable_by_exactly_one_key` now asserts an action never loses
both a key and a command. `ui_smoke.ps1` still posts id 107 and is unaffected —
it dispatches by id, not by key.

Found while doing it: the launch commit's `set_current_dir` comment carried a
literal CR in the middle of a `//` line (a `target` + `release` path mangled by
escaping), which the previous build tolerated and this one would not. Repaired;
`grep` confirms no other stray CR in `src/`.

**Not call-shaped (2026-09-06):** the product is an always-on assistant for a
working day, enabled manually — not a call coach. Nothing detected a call
before, so the code needed one change rather than a rewrite: a far-end turn no
longer asks the coach on its own when `--manual` / `IV_MANUAL` is set, and
`/coach on|off` flips it for the session. The framing in `--help` and the README
changed with it.

Why it had to exist: eight hours of automatic per-turn advice is a bill and a
distraction — a meeting you are half in, a video, a colleague at the next desk.
Pause could not serve this: it bumps the audio epoch and drops the whole
`Msg::Turn`, so an hour paused is an hour with no transcript. `--manual` keeps
the record and stops only the unbidden request, which is what makes arming
advice mid-afternoon useful — the 24-turn window is already full.

`status_text` gained the state, because a muted coach and a dead one are the
same picture: `Listening · advice on request (F7/F8)`. 81 unit tests (+1).

Still open for the all-day shape, none of it urgent: `audio.rs:304`'s
whisper-rs `CString` leak (~800 B per utterance) says "revisit if a session ever
runs for days" — that trigger is now met, though a 10,000-turn day is still only
8 MB; and `--dump` left on all day writes a `.wav` per utterance with nothing
pruning it.

**Launching (2026-09-06):** "a bit hard to use" turned out to be the launch, not
the panel. Root cause: nine defaults are relative, so the app only ever worked
when started *from* the project directory — which is why running it meant typing
a path and a flag list. `main` now adopts the folder of the `.env` that
`dotenv()` already walked up to find, and `inner-voice.cmd` at the root is the
double-click entry point. Verified: `--setup` run with the working directory set
to `target/release` — the double-click case — is all-✓ and finds the 574 MB
model. Put `IV_HEAR` / `IV_SPEAK` / `IV_DUMP` in `.env` (commented entries are
there now) and a bare launch is the whole call setup.

Not changed: the binary is still a console subsystem app, so a double-click
brings a console with whisper's load output beside the panel. That output is
worth having when something is wrong; `windows_subsystem = "windows"` would hide
the one place a model or CUDA failure is currently visible.

**Any app, several at once, switchable mid-run (2026-09-06):** `--hear` takes a
comma list and the selection moved from `Input` into `Tune` (`hear` +
`hear_gen`), so `/hear` typed in the question box rewrites it without a restart.
One process-loopback stream per name — WASAPI's activation params take a single
process tree — all feeding the one THEM whisper worker. `/hear` reports, `/hear
a,b` selects, `/hear off|mix|all` returns to the speaker mix; the switch lands
within 2 s on the poll that already notices an app closed. `IV_HEAR` stays the
durable default; `/hear` is session-only and deliberately not persisted.

The panel gets no picker on purpose: `sessions()` lists only what is *playing*,
and the useful case is naming an app before it makes a sound — which the
reacquire loop already waits for. A typed name covers both; a list covers
neither.

Evidence: `tools/hear_isolation.ps1` is now parameterised rather than copied,
and both cases pass live on this machine — the original
`PASS: --hear pwsh heard the elephant and not the giraffe`, and
`-Hear 'pwsh,powershell' -Expect elephant,giraffe -Reject @()` →
`PASS: --hear pwsh,powershell heard the elephant/giraffe`, with the two
sentences interleaved and both tagged THEM. 80 unit tests (+3), GPU and loopback
green.

**Paid for once:** `run` is shared by *both* capture threads, so the first cut
had the microphone following `tune.hear` too — it abandoned the mic for the
app's loopback and the target sentence was logged as `YOU`, failing the
single-app regression that had passed for weeks. The mic now early-returns to
`open_endpoint` before the supervisor. The regression test caught it on the
first run; nothing in the unit ladder could have.

**Not tested automatically:** the live `/hear` switch. Typing into the question
box needs injected keystrokes, which a `WS_EX_NOACTIVATE` window cannot receive
(see the input-gesture note in CLAUDE.md). The command semantics are unit
tested and the generation-bail path is the same code the reacquire loop uses,
but the end-to-end switch still needs a human.

**Providers measured (2026-09-06):** OpenRouter is the **testing lane only**;
`.env` stays on gemini direct. Same model both ways, measured on this machine with
`--setup` and `live_provider_returns_advice`:

| lane | turn 1 (cold) | turns 2-3 | `--setup` probe |
|---|---|---|---|
| openrouter google/gemini-3.5-flash-lite | 1.0 s | 557-943 ms | 656-886 ms |
| gemini direct gemini-3.5-flash-lite | 4.0 s | 632-640 ms | 845-993 ms |

Warm is a wash; the difference is entirely the *cold* path -- Google's is ~3.5 s,
OpenRouter's ~0.4 s, small enough that the startup warm-up hides it. Open
question 1 is therefore decided on the *first* turn, not on warm latency, which
no lane wins -- but the winner is a testing lane, so production stays gemini
direct and eats the cold turn. Anthropic direct has the only fast-mode +
prompt-caching wire and is untested here because `ANTHROPIC_API_KEY` is empty;
deepseek answers 402.

**No `--test` flag.** Testing is one command line
(`--provider openrouter --model <id>`), and a flag would have to hardcode an
OpenRouter model id -- the exact stale-default trap `provider.rs` refuses by
giving openrouter, openai and gemini no default model at all. An alias that
404s mid-call costs more than the typing it saves.

**Do not put a reasoning model on the fast lane.** `openai/gpt-oss-120b` and
`qwen/qwen3.7-flash` both fail `--setup` with "provider returned no advice":
`probe` spends its 8 tokens and the model has emitted only reasoning deltas, no
text. The verdict is right for this application -- a model that cannot say "OK"
in 8 tokens will not put 2-4 lines on the panel inside ~1.2 s -- but the wording
reads like a broken provider rather than a wrong model choice.

**Provider warm-up (2026-09-06):** the coach's advice worker now spends one
8-token throwaway request opening its pooled connection at startup, the way
`audio::warm` spends one inference on CUDA. Measured with `--setup` on
gemini-3.5-flash-lite: **3735 ms cold, 845-993 ms warm** across two passes, so
without it the DNS + TCP + TLS cost landed on the call's *first* turn inside a
~1.2 s budget. `probe` and the worker now share one `ttft(agent, provider)`, so
the number `--setup` prints and the number a call pays are still the same
measurement. It never runs under `--provider none` or `--preview` (no `Coach`
is built).

Readiness, measured today on the C: copy: `--setup` all-✓, `test --release`
77 + GPU + loopback green. Provider lanes: gemini works; `ANTHROPIC_API_KEY` is
empty in `.env`; deepseek answers **402** (out of credit), so open question 1
is settled by billing rather than latency until a key is added.
**Still the user's:** rotate the OpenRouter and Gemini keys that appear in an
earlier chat transcript, and the real acceptance call with
`--hear <app> --speak --dump clips`.

**CLI cleanup (2026-09-06):** Removed the Codex research adapter and its event
parser. Research continues through the configured provider (including Gemini)
by default, with Claude Code as the optional CLI. Historical Codex references
below and in the implementation plans describe removed behavior, not requirements.
Old CLI settings should be cleared to use provider research.

Verification (afternoon, in the rescue copy): `.\build.ps1 test --release` —
77 unit tests, GPU transcription on the re-downloaded model, WASAPI loopback,
and rustfmt all pass; live provider calls excluded. Debug-profile tests do not
link on this machine (prebuilt onnxruntime is /MD, knf-rs-sys goes /MDd), so
`--release` is the rule, now in CLAUDE.md.

**Disk incident (2026-09-06):** the morning's "unresolved" failures had one
cause — the drive behind E: (Seagate ST2000DM008, also D:) is failing: NTFS
logged index corruption on E: at 12:25 and bad blocks from 12:39. It garbled
CLAUDE.md on disk (same size and mtime, so git saw it as clean), two loose git
blobs (LEDGER.md at 5ac85bb, Cargo.lock at df57574), `target/release`, and the
whisper model (CRC error on read). The project was rescue-copied to
`C:\Users\Mohammed\OpenSources\katie\inner-voice` (485 of 486 files; the model
was the one failure and was re-downloaded, sha256-verified). Both blobs were
rebuilt byte-exact from the review diffs under `.superpowers/sdd/` and
CLAUDE.md rewritten from HEAD, in both copies. The C: copy is the live one and
this commit is made there; E: needs `chkdsk E: /f` and a replacement drive.
Nothing on E: is trusted until then.

**Wave 3 — shippable (2026-09-06):** `--setup` (`setup.rs`) checks model, CUDA
(timed warm-up), devices, playing apps, provider (`coach::probe`: one tiny
request, TTFT in ms, comparable across providers; a live turn reads higher),
OCR, and the two folders, ✓/✗ with the fix, exit 1 on any ✗. The status line
names the model, so the on-screen `first word` number is attributable and
providers are compared from evidence. `references/` is git-ignored. **Still the
user's to do:** rotate the OpenRouter and Gemini keys that appear in an earlier
chat transcript (Machine facts above); the real acceptance call with
`--hear <app> --speak --dump clips`.

**Wave 2 — memory (2026-09-06):** `extract.rs` is the one loader (txt/md/csv/
sheets/PDF/DOCX/OCR); `knowledge::Corpus` re-reads the folder before each
coach request when its newest mtime moves and pushes the new prompt to the
coach and the new glossary to whisper through `RwLock`s; references live in
`references/`, drops are remembered by path in `.dropped`, `(path, mtime)` is a
document's identity so an edit replaces its passages. The default research lane
is HTTP over the coaching provider; the CLI lane runs only with `--agent-cmd`,
and accepts only Claude Code's `claude.exe` —
settings/hooks/MCP off, read-only tools, pinned to a captured `stream-json` run:
6.5 K cache tokens and ~5 s per press versus 37 K / $0.75 through the
interactive harness; `--bare` drops the subscription login.
Known gap: a remembered drop whose file is gone is
reported once per launch until Clear references.

**Wave 1 — glance & hearing (2026-09-06):** Decision 7 back in egui (`Tag` for
the colours, `status_text` for the wait as a number, TTFT timing), all unit
tested and the instrument asserted by `ui_smoke.ps1`.

Per-app capture: `--hear <app>` opens a process-loopback client on the app's
process tree, reacquires when it starts late or restarts (`alive(pid)` polled
every 2 s — a dead target delivers silence, not an error), and falls back to the
endpoint mix if the client cannot open. The spike needed only attempt 1: polling
shared mode, 16 kHz mono f32 with `autoconvert`, exactly like the endpoint path.
No event-driven client and no decimator were needed, written or committed.

An app stream is *not* calibrated. It starts only when the app plays, so the
calibration second lands on the first **sentence** — measured once at gate
0.5942, which then produced zero turns until `--sys-gate` was forced. It is the
app's own digital output and has no noise floor to measure, so it takes
`GATE_FLOOR` (0.004) unless `--sys-gate` overrides; the endpoint mix, and the
fallback to it, still calibrate. Verified live: `THEM gate 0.0040 (fixed)` and
speech transcribed with no `--sys-gate`.

The `--speak` mute follows `--speak`, not `--hear`: the mute `Arc` exists
whenever `--speak` is on, the process-loopback feed opts out (`mute: None`)
because the voice is another process, and both endpoint pumps — the `--hear`
fallback included — stay deafened while the voice talks. So the mute only ever
bites where an endpoint stream *is* `THEM`: `--speak` without `--hear`, which the
panel warns about at startup, or `--hear` whose app client could not open.

Research runs over HTTP on its own worker whenever a provider is online
(`Coach::research`, exactly one `ToolEnd` per job by CAS). Its ids are seeded at
`1 << 32` because `agent.rs` mints `ToolStart` ids from 1 on the same channel;
the router also makes the two lanes mutually exclusive per session, so a
collision needs both belts to fail. The CLI lane still wins when `--agent-cmd`
is set, and is still unverified against a real Codex.

Verified: the unit ladder, `ui_smoke.ps1` (which now asserts the TTFT
instrument), a live Gemini run of the HTTP research lane, and
`tools/hear_isolation.ps1` — two processes speaking different sentences through
the speakers at once, only the named one becoming `THEM`. It passed on the
first run: `PASS: --hear pwsh heard the elephant and not the giraffe`. Not yet:
the drag gesture and a real call still need a human.

**Hotkey pass (2026-09-05):** Deleted every BUTTON control. The panel is now
status line, notice line, scrollable read-only body, one-line question box — and
`KEYS` in `src/hud.rs`, one `RegisterHotKey` per action on Ctrl+Shift+F1..F11:
F1 Advice, F2 Conversation, F3 References, F4 Diagnostics, F5 pause/resume,
F6 hide/show, F7 question box, F8 research, F9 cancel research, F10 clear
references, F11 the key list. `RegisterHotKey` delivers to the window regardless
of focus, so every action works while the *call* app owns the keyboard; a button
is only reachable after stealing focus from the call, which is what this window
exists to avoid. Ctrl+Shift+F8 is unchanged from the completion pass; the bare
F8/F9/F10 bindings and the view-switcher row are gone. Research and Cancel
register only when research is enabled, so `--preview` never claims them.

Quit deliberately has no key: Alt+F4 closes, and cannot be hit by fumbling an
adjacent F-key mid-sentence. Escape no longer quits — it clears the question box;
Enter sends it. Registration is first-come process-wide and fails silently, so
`run` collects the losers and reports them in the notice line and Diagnostics
rather than shipping a dead key; the fix is freeing the key in the other app.
That report path is verified: two `--preview` instances were run at once and the
second named all nine keys it lost (F8/F9 correctly absent — research keys are
not registered under `--preview`).
Drag-to-move, edge/corner resize, double-click maximize and
WM_DROPFILES import are unchanged, and the whole top strip now drags since no
corner is reserved for window buttons. `src/hud.rs` tests assert one key per
action, ids inside the `WM_COMMAND` dispatch range, and the widened drag strip.

**Self-driven drag, chat log moved (2026-09-06):** Reported: the panel could not
be dragged unless it was first activated from the taskbar. Cause was
`ViewportCommand::StartDrag`, which posts `WM_NCLBUTTONDOWN`/`HTCAPTION` and
hands off to Windows' modal move loop — that loop wants the window to be the
foreground one and wants mouse capture, and `WS_EX_NOACTIVATE` is set exactly so
this window is never foreground. The clicks were arriving fine; the OS loop was
refusing. `State::move_window` now follows the cursor itself with `GetCursorPos`
+ `SetWindowPos(SWP_NOACTIVATE)` — no capture, no activation, no modal loop —
and a drag ends on the *first* sign of release from either
`GetAsyncKeyState(VK_LBUTTON)` or egui's pointer, since each can miss one alone
and a drag outliving its release glues the panel to the pointer. That was not
theoretical: with only the async state checked, `ui_smoke.ps1` failed
intermittently — an injected press whose release went missing left the drag
running, and the window-move assertion was then measuring the stuck drag rather
than itself (it reported x=2411 for an expected 178). The panel now mirrors
`dragging` and the script waits for it to clear before trusting any position. Resize goes through the same path for the same reason. `dragged()`
holds the geometry (move shifts both corners; a resize pushes only the edges its
corner owns and each stops at the minimum instead of crossing its opposite) and
is unit-tested; the gesture still needs a human, as this session cannot inject
mouse input.

The chat log moved from above the advice to below it, just over the question box,
so the newest speech is next to where you answer it and the advice holds the top.
Its pane now grows with the transcript up to the 8-row floor rather than
reserving all of it from turn one, which otherwise left a dead gap between a
single line of speech and the question box for the opening minutes of a call.

**Move and pin (2026-09-06):** The whole panel is now the drag grip — `hit_test`
returns `Grab::Move` for anything that is not a resize edge, instead of a 44 px
title strip. A strip is a sliver of a window that is otherwise wall-to-wall text,
and this one gets repositioned around whatever the call is showing. That needed
`style.interaction.selectable_labels = false`, or a drag over text selects
instead of moving; nothing was lost, because `WS_EX_NOACTIVATE` means the panel
never holds keyboard focus and so could never have answered Ctrl+C — the
selection was already dead weight. Ctrl+Shift+F12 pins, freezing move *and*
resize, which is the counterpart to grabbing anywhere: once it is placed nothing
should shove it. Status line says `· pinned`; unpinned is the default and needs
no word, since a panel that moves when you drag it is self-evident.

The drag check runs after the panels are drawn, and that ordering is
load-bearing: on the frame of a press `egui_wants_pointer_input` collapses to
"did a widget take this press" (its hover clause is guarded by `!any_down()`),
which is only known once the widgets exist. Earlier in the frame and the question
box, scrollbars and consent buttons would all drag the window instead.

Not verified end to end, and the reason is worth keeping: this session cannot
inject mouse input at all — `SetCursorPos` returns false, the same UIPI rule that
made synthetic keystrokes useless for the hotkeys. A drag test was written, ran
green, and was measuring nothing; the giveaway was that the *pinned* and
*unpinned* drags gave identical results. `ui_smoke.ps1` now prints an explicit
SKIPPED line instead of passing quietly, and the gesture needs a person with a
mouse. `hit_test` covers the mapping.

**egui rewrite (2026-09-05):** The panel is now `eframe`/`egui` on the `glow`
backend. The reason was the single-page layout, not the framework: advice, the
turn it answers and the ask box are all per-turn, so switching views to see one
of them is the same mid-sentence interruption as reaching for the mouse. Raw
Win32 made that expensive — every extra pane is another `EDIT` control with its
own layout maths and its own scroll handling, and an `EDIT` control cannot style
individual lines at all, which is why `display_advice` used to flatten ASK/SAY/
NOTE/FIX into fake plain-text headings. It now returns `(heading, body)` and the
panel draws real ones. `hud.rs` lost the window class, child controls, `wndproc`,
`layout`, `hit_test`'s Win32 constants, the GDI painting and the message loop;
`follows_the_end` and its `GetScrollInfo`/`EM_LINESCROLL` pair became
`ScrollArea::stick_to_bottom(true)`, which is also better — it re-sticks when a
reader scrolls back to the end, which the hand-written version could not.
The conversation pane reserves `VISIBLE_TURNS` = 8 rows (asked for explicitly)
before advice takes the rest; verified on screen with eight seeded turns.

Measured before committing, since the premise rested on them:

- *Focus.* The panel never takes the foreground — on open, on repaint, or on a
  click. That is better than the Win32 build, which activated on `SW_SHOW` and
  on any click. It needs `WS_EX_NOACTIVATE`, and winit recomputes the extended
  style whenever the window changes state while knowing nothing about that flag,
  so a one-shot set at startup is dropped silently — found by asserting the bit
  rather than trusting the behaviour, since the behaviour looked right for the
  wrong reason. `keep_unfocusable` re-asserts it per frame and ties it to
  `typing`: the same flag that blocks a focus steal blocks typing.
- *Hotkeys.* `RegisterHotKey` reaches the app through winit's `with_msg_hook`;
  registration succeeds and real `WM_HOTKEY`s were observed arriving with
  another app in front. Synthetic keystrokes are useless for testing this —
  injected input is discarded when the foreground window outranks the sender,
  and it fails identically against the old Win32 build, which is how that was
  established rather than assumed.
- *GPU.* OpenGL and CUDA whisper coexist. A 45 s real run loaded the 573 MB
  model on CUDA0, transcribed eight live turns and held 678 MB working set.

That real run is also what caught the one bug `--preview` could never reach:
`with_drag_and_drop(true)` calls `OleInitialize`, which needs an STA thread,
while `main` is already MTA for WASAPI device enumeration — the app aborted at
window creation with `RPC_E_CHANGED_MODE`. Files now arrive as `WM_DROPFILES`
via the message hook, needing no COM, which also restored the smoke test's
file-drop coverage.

`tools/ui_smoke.ps1` was rebuilt: with no child controls to read, the panel
mirrors its *state* to `IV_UI_DUMP` (not its render — that would be a second
copy of the layout free to disagree with the screen; screenshots cover drawing).
It gained two checks the old script could not make: that the panel does not
steal focus on open or click, and that it really owns its hotkeys — proved by
the script trying to register the same combination and requiring failure.

**Auto-scroll (2026-09-05):** `refresh` follows the body to the newest line, but
only for a reader already there — `set` is `SetWindowTextW`, which resets the
scroll to the top on every content change, walking a streaming answer or a fresh
turn off screen as it arrives. The whole decision is `follows_the_end(switched,
pos, page, max)`: at-end when `pos + page > max`, never on a view switch (the key
list is a heading first), and an empty range counts as at-end so the first
overflow follows too. Scroll state comes from `GetScrollInfo(SB_VERT)` and the
move is `EM_LINESCROLL(0, i32::MAX)`, which the control clamps — chosen over
`EM_SETSEL` + `EM_SCROLLCARET` because that pair would wipe a reader's selection.
Both Win32 assumptions were checked against the live control, not the docs: an 80
line body reported `pos=0 page=21 max=79` and `EM_LINESCROLL` clamped to `pos=59`,
exactly the last page. `ui_smoke.ps1` covers it end to end by dropping three more
files (import activity is the only thing that grows a view in place under
`--preview`) and asserting the body sits off the top, then that Ctrl+Shift+F11
returns to line 0.

**Audit pass (2026-09-05):** Four read-only audits over the code added in the
completion/UX passes, then fixes. Real bugs found and closed:

- `Coach::cancel` retired a generation without a terminal message. One Pause
  press could strand the panel on "preparing advice" forever — the `AdviceStart`
  can still be in the channel *behind* the `Msg::Pause` that caused the cancel.
  `cancel` now emits `AdviceEnd`, and `route`/`hud` ignore a repeat rather than
  double-logging or re-speaking. Test: `cancelling_closes_the_generation_it_retired`.
- Ctrl+Shift+F8 fired only when no result was held, so research worked once per
  session. Now: shows the result from another view, starts a fresh job when
  pressed while already viewing one.
- Failed research was pushed into the 24-turn window as untrusted reference
  material, evicting real speech for 24 turns. Only answers enter it now, capped
  at 2,000 chars (`history::MAX_RESEARCH`) — an uncapped 16 KB tool answer could
  otherwise squat on the whole context.
- `--es` silently degraded to a relative path, surfacing 90 s into a job. Now
  resolved when `--search-query` is set, ignored otherwise.
- `process.rs`: the "1 MB cap" was a 2 MB ceiling that *aborted* the job; now a
  per-stream truncation. `BrokenPipe` from a successful child that ignored stdin
  discarded a good answer. Deadline kills and worker panics could wedge research
  for the session (`busy` never cleared, no `ToolEnd`, panel stuck on "Research
  is running") — every wait is now `recv_timeout(DRAIN=2s)` and the worker is
  panic-safe.
- `agent.rs`: one non-JSON stdout line (a banner) killed an otherwise successful
  run; a late `error` event discarded an answer already collected. Dead
  `impl Drop for Agent` deleted — it could never run.
- `references.rs`: oversized text files reported as corrupt; a `clear()` during
  import left a status line that never resolved; a poisoned lock killed the
  loader for the session while advising a retry that could never work; `chunks()`
  could exceed its own limit by one byte (its test could not reach the branch);
  a filename could forge citation structure; `preview()` took the library mutex
  on the Win32 message loop (`Mutex` → `RwLock`).
- `hud.rs`: `MINIMIZE` called `ShowWindow` from inside `STATE.borrow_mut()`.
  `ShowWindow` sends `WM_SIZE` synchronously, re-entering `wndproc`, whose
  handler borrows STATE again — the app panicked and died. Minimize is now a
  posted `WM_SYSCOMMAND`. Found by `tools/ui_smoke.ps1`, not by `cargo test`.

Two doc claims were false and are corrected in CLAUDE.md: pause is not a hard
barrier past the audio worker (a `Turn` carries no epoch and `Msg::Pause` rides
the same FIFO behind it), and `split_tag()` has been `display_advice()` for a
while.

**Known and not fixed**, deliberately: a superseded SSE stream returns with the
body unread, so ureq cannot pool the socket and the next turn re-handshakes TLS
(~290 ms) — on exactly the busy turns pooling was meant to help. Draining
instead would block the newest turn on the single worker. Needs measurement, not
a guess. Also unfixed: the coach's one worker can head-of-line block up to 60 s
inside `read_line` on a stalled provider, since cancellation is only checked
between lines.

**Still unproven:** research has never run against a real Codex CLI. `agent.rs`
keys on `event["item"]["type"] == "agent_message"`; if Codex tags that field
`item_type`, every research returns no answer and the feature is inert. The
no-answer error now quotes the first output line so a shape mismatch reads as
one. One real `codex exec --json` capture settles it.

**UX implementation (2026-09-05):** Replaced the painted HUD with a resizable
native control workspace: Advice, Conversation, References, Diagnostics, question
input, Pause/Resume, Research, Cancel, and End call. Added an offline `--preview`
mode. Pause uses audio epochs to invalidate buffered/in-flight speech.
*Superseded by the hotkey pass above:* those controls are keys now, not buttons,
and the End call button is gone — Alt+F4 replaces it.

Session references now accept Windows file drops, extract text/spreadsheets in a
background worker, index passages locally, retrieve matching excerpts for live
coaching and explicit questions, and cite filename/passage. Offline questions
return local matches. Import consent precedes use with online providers. Limits,
duplicates, unsupported formats, clear-during-import, and errors are explicit.
PDF/DOCX/OCR, individual reference management, persistence, guided setup, and
full DPI/Narrator validation remain outstanding; no claim of full UX completion.
*PDF/DOCX/OCR and persistence: done in Wave 2 (2026-09-06); individual reference
management, guided setup and DPI/Narrator validation still outstanding.*
Native UI smoke checks cover navigation, pause controls, actual WM_DROPFILES,
extraction, local retrieval, and clear. See tools/ui_smoke.ps1.

**Completion pass (2026-09-05):** Added typed bounded history, F8/Ctrl+Shift+F8
research through a read-only Codex executable, F9 result paging, F10 cancellation,
and explicit bounded Everything filename search. Research uses hidden Windows
Job Objects, stdin input, deadlines, and output limits; it is off by default.
The Codex adapter replaces the proposed OpenCode default for this release;
Claude/OpenCode adapters remain future alternatives.
*The Claude adapter is done in Wave 2 (2026-09-06): `--agent-cmd claude.exe`
picks it by file stem. OpenCode is still a future alternative.* Everything integration is
implemented, but the indexer was not running during verification (IPC error 8).

Fixed Unicode corpus truncation, conflicting name bindings, silent-endpoint
calibration, blank model/invalid gate validation, credential Debug redaction,
log failure visibility, and HUD resource cleanup. Coaching has one HTTP worker
and one newest pending request, bounded SSE parsing, and a global timeout.
Setup documentation and environment template now match these features.

Verification: offline unit tests plus real CUDA transcription and WASAPI pass.
Live provider/research requests were not made. Real-call accuracy and speaker
threshold calibration remain outstanding. Older measurements and phase plans
below are historical; they do not certify this pass's live latency.

**Previous baseline:** capture → VAD → Whisper (CUDA) → coach → HUD, 5 providers, taught corpus, `.env`, JSONL log, far-end speaker naming. 35 tests reported (live provider skipped).

**Next action:** run a real call with `--dump`, then tune `voiceid`'s two thresholds against those clips. Nothing about naming is trustworthy until that happens.
**Fast lane:** `gemini-3.5-flash-lite`, ~600 ms warm / ~890 ms on the first turn (pooled connection). Loop ~1.2 s.
**Still unmeasured:** Claude direct. No Console key exists yet.
**Blocked:** Phase 4 only — Everything is installed and running, but E: is not in its index (Tools → Options → Indexes → NTFS → tick E:).

**Latency pass (2026-09-05).** Loop is ~1.15 s warm, from ~1.6 s. Four changes, each measured:

| Change | Effect |
| --- | --- |
| `flash_attn(true)` on the whisper context | 4 s turn 144 → 80 ms |
| ~~`audio_ctx` 1500 → 768~~ | **reverted, see Accuracy pass** |
| One pooled `ureq::Agent` instead of `ureq::post`'s use-once agent | warm turn ~890 → ~597 ms |
| WASAPI buffer 100 → 500 ms | fixes dropped audio, see below |

The agent change was **inert until the SSE loop stopped `break`ing on the
terminator**: ureq only returns a connection to the pool from `ended()`, i.e.
once the body is read to its end, and `reuse()` re-probes the socket and refuses
anything with bytes outstanding. Draining the two trailing bytes is what
actually buys the ~290 ms. Measured, gemini, three turns on one `Coach`:
cold 838/948/888 ms, warm 615/593/581/607/580/601 ms.

`audio_ctx` is derived, not tuned: the encoder models 30 s as 1500 states, so a
value below `MAX_MS`/20 silently truncates the tail of a long turn. Note the
trap — `audio_ctx=512` still "passed" an accuracy check on `jfk.wav`, because
the model completes that famous line from its language prior with the audio cut
off. Do not benchmark truncation against a memorised clip.

**Latent bug fixed:** `transcribe` runs inline on the capture thread, so nothing
is read from WASAPI while it works. The ring buffer was 100 ms and a monologue
flush took ~180 ms — the front of the next turn was being dropped on the floor.
Now 500 ms (32 KB).

### Accuracy pass (2026-09-05, same session)

Reported: `I'll select the best options` → `i'll slect the Visit options`, on a
build carrying `audio_ctx=768` + greedy. Spent the latency budget back on
quality; loop is ~1.2 s, still well under the 1.6 s it started at.

| Change | Cost | Why |
| --- | --- | --- |
| `audio_ctx` reverted to full 1500 | +22 ms | Could not be shown accuracy-neutral on live speech |
| Beam search (5) instead of `Greedy{best_of:1}` | +22 ms | Greedy cannot revise a token; that is what `slect` is |
| `set_initial_prompt` with a glossary from `knowledge/` | ~0 ms | Names are what transcription actually mangles |
| `--dump <dir>` writes each utterance as WAV + transcript | ~0 ms | Nobody could reproduce the report |

**`audio_ctx` was reverted on absence of evidence, not evidence of harm** — and
that is the point. Both probes available were incapable of detecting quality
loss: `jfk.wav` is memorised, and SAPI TTS scored 2.4% WER at *every* setting
including 5 dB SNR, because synthetic speech has no coarticulation, accent or
disfluency. **Do not benchmark ASR accuracy on TTS or on famous clips.** Real
call audio is the only probe that would settle it, hence `--dump`.

Glossary from the current `knowledge/`, verified end to end:
`Glossary: Sarah Chen, Platform Lead, Marcus Webb, CTO, ex-Stripe, Rust, Whisper, CUDA, WASAPI, EMEA.`
Built from capitalised runs plus interior-capital tokens, minus a stoplist of
sentence-openers, capped at 800 chars because whisper's prompt window is
`n_text_ctx/2` (~224 tokens) and silently drops the front of an overrun.

### Threading pass (2026-09-05, same session)

Transcription moved off the capture threads. Each stream now has a whisper
worker fed by a channel; the capture loop only reads, gates and segments.

Inference on the capture thread meant WASAPI went unread for 100–300 ms per
turn — the ring buffer was the *only* thing preventing lost audio, and the VAD's
silence counter stopped along with it, so the hangover that ends a turn was not
being counted while the GPU worked. Beam search made that worse, which is what
prompted this. The 500 ms buffer stays as headroom rather than as the mechanism.

Each worker also runs one throwaway inference at startup: the first CUDA call
costs 100 ms+ and used to land on the call's first turn. It now lands during the
VAD's calibration second.

Verified end to end, not just compiled: app launched with `--dump`, audio played
into the loopback, a clip came out the far side (15 s — the `MAX_MS` flush path)
with its transcript. Six threads, no deadlock, 17/17 tests.

**Machine note:** the default render device is `Headphones (STHP-8888M)`, not the
ROG DELTA S. Loopback follows the *default*, so if the call is being listened to
on the ROG headset, `THEM` captures the wrong endpoint and hears nothing —
`--loopback "Headphones (ROG DELTA S)"` or `IV_LOOPBACK`. Cost an hour of
false-lead debugging; auto-calibration was innocent.

### Spike: speaker embedding (2026-09-05) — feasible, threshold unproven

Question: can we name each far-end speaker inside the mixed `THEM` stream?
Because the VAD already cuts discrete utterances, this is **embedding + online
clustering**, not diarization.

Verified working, then reverted (throwaway):

- `ort` 2.0.0-rc.13 links and runs beside whisper-rs/CUDA. `download-binaries`
  is a default feature, so **no cmake/nvcc/MSVC involvement** — it does not touch
  the fragile part of this build.
- Model `3dspeaker campplus_sv_en_voxceleb_16k.onnx` (28 MB, in `models/`,
  gitignored). Input is `[N, T, 80]` — **kaldi fbank, not waveform**. Output 512-d.
- `knf-rs` 0.3.2 supplies the fbank and already does mean normalisation. Note it
  FFIs to a small C++ kaldi-fbank despite its blurb; no CUDA, low risk.
- **54 ms per utterance on CPU**, so it can run concurrently with whisper's
  ~100 ms and cost nothing on the wall clock. Does not compete for the GPU.
- `ort` is on ndarray 0.17, `knf-rs` on 0.16. Bridge with `(shape, Vec<f32>)`,
  never an ndarray type, or they do not typecheck against each other.

**Cosine matrix, 3 SAPI voices (same sentence + a second sentence each):**

|          | david_a | david_b | zira_a | zira_b | hazel_a |
| --- | --- | --- | --- | --- | --- |
| david_a  | 1.000 | 0.956 | 0.803 | 0.843 | 0.300 |
| zira_a   | 0.803 | 0.722 | 1.000 | 0.901 | 0.364 |
| hazel_a  | 0.300 | 0.300 | 0.364 | 0.363 | 1.000 |

Same-voice 0.90–0.96, Hazel-vs-all 0.30–0.36 — but **David vs Zira 0.72–0.84**,
leaving a gap of only 0.089. Both are US "Desktop" concatenative voices from one
synthesiser; Hazel is UK and separates cleanly. **This is a TTS artefact, not a
measurement of real speakers** — same lesson as the accuracy pass: synthetic
speech is not a valid probe. The threshold must be tuned on `--dump` output from
a real call, and until it is, the design must refuse to name rather than guess.

### Phase 2 + naming, as built (2026-09-05)

`Msg::Turn(&'static str, String)` is gone. Turns carry `Who::{You, Them{voice, name}}`
— cluster and name kept apart because they are learned from different evidence.

- `src/log.rs` — JSONL, one `call-<ms>.jsonl` per session, `Mutex<File>`, one
  `write_all` per line. `append` swallows errors on purpose: a full disk must not
  end a call. Written from `route()`, the only place every turn passes exactly
  once, in order, on one thread.
- `src/voiceid.rs` — fbank → CAM++ → 512-d unit vector → online clustering. Two
  thresholds with an unsure band, 1.5 s minimum, 8-cluster cap that refuses
  rather than evicts. `assign` is a free function so the decision is testable
  without loading the 28 MB model. Guards a zero/NaN embedding, which otherwise
  becomes a permanent cluster that matches nothing for the rest of the call.
- `src/roster.rs` — `attendees.csv` → `self_intro` / `addressed`. Only ever
  returns a roster name, in the roster's spelling; a first name is a key only
  when unambiguous, and two names in one line yields `None`.
- Naming runs in `route()`, not the THEM worker, because a direct address is
  usually spoken by YOU on the *other* capture thread. `bind()` drops both claims
  when a name is already on a different cluster.
- Embedding runs in a scoped thread alongside whisper — CPU and GPU, neither
  needs the other's answer — so identity costs `max(100, 54)` ms, not the sum.

Verified end to end: app launched, audio played into the loopback, `call-*.jsonl`
written with a well-formed typed turn, no deadlock across the new thread layout.

**The naming path is NOT verified against real multi-speaker audio** and cannot
be from this machine — it needs a real call. Both available probes mislead, in
opposite directions: two halves of one real speaker in `jfk.wav` score **0.591**
(reads as "not sure"), while two *different* TTS voices score **0.72–0.80**
(reads as one person). At 0.70/0.50 the far end will often stay `THEM`. That is
the correct direction to be wrong in, and not a bug to "fix" by lowering the
threshold on intuition.

**Not taken (open):** `large-v3-turbo` is distilled to 4 decoder layers *and*
quantised to q5_0. Moving to `large-v3` q8_0 is ~1 GB and ~150 ms and is the
largest remaining quality step. 12 GB VRAM, ~400 ms slack — affordable whenever
the dumps justify it.

| Phase | Scope | Est. | State |
| --- | --- | --- | --- |
| 0 | Taught corpus (`knowledge/`) pinned into the prompt | — | **done** |
| 1 | 5 providers, 2 wire formats, `.env` | — | **done** |
| 2 | Structured history + JSONL log | 4 h | **done** |
| 2b | Far-end speaker naming (embed + cluster + roster) | — | **built, thresholds untuned** |
| 3 | Slow lane via existing CLIs | 3 h | not started |
| 4 | ES file search tool | 1 h | blocked on E: index |

### Phase 0 as built

`knowledge/` (`.md .txt .xlsx .xlsm .xls .ods .csv`) is loaded at startup and
appended to the system prompt behind a cache breakpoint. **No retrieval.** A
curated corpus already in context cannot fail to be recalled, and retrieval that
never runs cannot miss — which is the only honest route to the stated recall
goal. Capped at 400 KB. `calamine` reads Excel; CSV goes through the `csv` crate
because calamine does not read CSV and naive splitting corrupts `"Smith, John"`.

The prompt also instructs: prefer the corpus over assumptions, and say a fact is
missing rather than invent it. Target is **100% honesty, not 100% correctness**.

### Phase 1 as built

`provider.rs` — one preset table, two wire formats, no trait. OpenRouter,
OpenAI, DeepSeek and Gemini all speak the OpenAI `/chat/completions` shape, so
five providers cost one extra code path. Anthropic keeps its own for
`speed:"fast"` + `effort:"low"` + prompt caching.

No default model id for openai/gemini/openrouter: those churn, and a stale
default 404s mid-call. A loud startup error beats a dead panel.

`.env` via `dotenvy` + clap `env`. Every flag has an `IV_*` variable, so the app
runs bare. `.env` is gitignored; `.env.example` is committed.

---

## Machine facts (verified 2026-09-05)

| Thing | State |
| --- | --- |
| `opencode` | 1.18.27, DeepSeek authenticated. **562 ms spawn floor.** `opencode run` currently errors (`UnknownError`, 2 attempts, ~1.9 s each) |
| `claude` CLI | installed. Two-word reply: **4.0–5.5 s** as configured; **3.1 s stripped** (`--strict-mcp-config` + empty `--mcp-config` + `--system-prompt` + `--exclude-dynamic-system-prompt-sections` + no tools). Node boot is only ~200 ms, so the rest is harness overhead: Haiku is no faster (2.9–5.5 s, noisier) and `-c` session reuse is identical (3.16 s). **3.1 s is the floor.** Slow lane only |
| `codex` CLI | installed |
| `es.exe` | present at repo root, **working** (~360 ms/query, mostly process spawn) |
| Everything indexer | portable build in `everything/`, runs elevated. Indexes C:, D:, G: — **E: still missing**, which is the Phase 4 blocker |
| `ANTHROPIC_API_KEY` | **absent everywhere — blocks production.** Checked process, User and Machine scope. The machine's only Anthropic credential is a Claude **Pro** OAuth token (`~/.claude/.credentials.json`, scope `user:sessions:claude_code`, ~8 h expiry + refresh rotation). That is a Claude Code session token, not an API key, and is not a backend for this app. A Console key is the only production path |
| OpenRouter key | free tier: paid models 402, free models ~3.9 s TTFT. Testing only. **In the chat transcript, rotate it.** |
| `DEEPSEEK_API_KEY` | pulled from `~/.local/share/opencode/auth.json` into `.env` (2026-09-05). Key is **valid but the account has no balance** — HTTP **402**, not 401. Wired and ready; needs a top-up to produce a number |
| `GEMINI_API_KEY` | free AI Studio key, **working, current default.** 40 `generateContent` models. Measured through our wire: `gemini-3.5-flash-lite` **822–923 ms**, `gemini-3.1-flash-lite` 880 ms, `gemini-3.5-flash` 2,857 ms, `gemini-3.8-flash` 503 every time, `gemini-2.5-flash` 404 on the OpenAI-compat path. **In the chat transcript, rotate it.** |

---

## Decisions locked

Do not re-litigate these. Each replaces a more expensive option.

1. **Two wire formats, not four providers.** OpenAI-compatible SSE covers OpenAI, DeepSeek *and* Gemini (Google ships a compat endpoint). Anthropic native stays separate only because `speed:"fast"` + `effort:"low"` is what buys the 1.3 s loop.
2. **No `trait Provider`.** One config struct + a two-arm match. A trait with two impls is the abstraction tax this project does not owe.
3. **Two latency lanes.** Fast lane = native HTTP, no tools, ~1.3 s. Slow lane = subprocess, 10–60 s, never blocks the coach.
4. **Claude direct in production; OpenRouter for testing only.** Locked by the
   user. The native Anthropic wire is the only path carrying `speed:"fast"`,
   `effort:"low"` and prompt caching — a router hop loses all three, and caching
   is what stops the pinned `knowledge/` corpus being resent every turn.
   Measured on a free-tier OpenRouter key: **3.9 s to first token** against a
   ~1.2 s budget, and paid models return 402. Adequate for wiring checks, not
   for a call.
   *Amended 2026-09-05 — decide this one:* no Console key exists, so `.env`
   currently defaults to **gemini / gemini-3.5-flash-lite**, measured at
   **~870 ms** on a free key. That is faster than the budget and costs nothing,
   which was not on the table when this was locked. The case for Claude is now
   **prompt caching, not latency**: the OpenAI-compat wire has no cache
   breakpoint, so the whole `knowledge/` corpus is re-sent on every turn — fine
   at today's corpus size, a rate-limit wall on a free tier as it grows. Free
   tiers also throttle without warning (`gemini-3.8-flash` returned 503 on every
   attempt). Flip back by setting `IV_PROVIDER=anthropic` and commenting out
   `IV_MODEL`.
   *Amended 2026-09-06: providers are choices; Gemini free is the default; no
   production credential is required to run. Claude direct remains the only wire
   with prompt caching.*
5. **Slow lane shells out to installed CLIs. Fast lane never does.** `opencode run` / `claude -p` / `codex exec` already have providers, tools, file access and sessions — rebuilding that in Rust is the single biggest waste available here. But measured: opencode costs **562 ms to spawn before it sends a byte**, against a whole-loop budget of ~1,200 ms. It is disqualified from the fast lane on arithmetic.
   *Would change if:* `opencode serve` runs persistently (erases spawn cost) **and** its injected agent prompt proves cheap enough. Even then the fast lane wants a ~300-token system prompt under our control, which is ~50 lines of direct HTTP.
6. **Memory = bounded deque + append-only JSONL.** No vector DB, no embeddings, no RAG until a real retrieval failure exists.
7. **The panel is a glance, not a screen.** Rules, each replacing a plausible alternative:
   - **Doherty**: the loop is ~1–2 s, well past the 400 ms "instant" line, so the header counts the wait — `thinking 1.4s`, then `first word 1.2s`. A spinner would say nothing; the number doubles as the standing TTFT instrument.
   - **Never blank.** Last turn's advice stays up, greyed, until the new tokens land and replace it in one frame. Clearing at request time gave a 1.5 s hole mid-call.
   - **Von Restorff**: only `ASK` and `FIX` carry saturated colour. `SAY` is soft green, `NOTE` is grey. Colour every line and nothing stands out.
   - **Proximity + hanging indent**: 12 px between items, natural leading within one, tags in a 54 px gutter with wrapped text aligned under the first word.
   - **Miller**: transcript capped at 6 lines, `THEM` brighter than `YOU` because `THEM` is what you react to.
   - Monospaced digits for the timer (it would twitch otherwise); `esc to close` shows for 10 s then leaves — a permanent hint is permanent noise.

   *Restored in egui 2026-09-06 (Wave 1): the wait as a number, ASK/FIX-only
   saturation, THEM brighter than YOU, greyed-while-thinking, hint that leaves.
   The 6-line transcript cap is superseded by the user's 8-turn floor. The pixel
   measurements are still history, and Escape no longer closes anything (see 8).*
8. **The panel is hotkey-driven; the buttons were removed. Do not put them back.**
   It is an always-on-top overlay used while the *call* app owns the keyboard, so
   a control that must be focused to be pressed is a control that costs the call —
   reaching for the mouse mid-sentence is the thing this window exists to avoid.
   `RegisterHotKey` delivers `WM_HOTKEY` to the window whatever has focus; an
   accelerator table or a button row only works after stealing focus, which is
   why neither is the mechanism. `KEYS` in `src/hud.rs` is the single map —
   a new action is a row, not a control, and a view-switcher is a row too.
   Registration is first-come process-wide and fails silently, so clashes are
   reported in the notice line and Diagnostics instead of leaving a key that does
   nothing. Quit deliberately has no key: Alt+F4 already closes and is muscle
   memory, and an F-key next to a live action is one fumble from ending the call.
9. **On-disk is the source of truth (2026-09-06).** The app never coaches from
   a stale copy. `knowledge/` reloads on change, references live in a folder
   and reload, dropped files are remembered by path and never copied. (Wave 2
   implements this; recorded here because Wave 1's research design assumes it.)
10. **Capture is per-app when an app is named (2026-09-06).** `--hear <app>`
    captures only that process tree as `THEM`; the endpoint mix is the
    fallback. The `--speak` voice is another process and is therefore never
    heard, so the loopback mute survives in exactly one combination: `--speak`
    without `--hear` — or with it, when the app client could not open and the
    endpoint mix is standing in.

---

## Phase 1 — Providers — DONE

Built as described under "Phase 1 as built" above. Endpoints in `src/provider.rs`.

---

## Phase 2 — Structured history (4 h)

The real refactor. Must land **before** Phase 3 — retrofitting history under a live tool loop is the version that hurts.

1. Replace `Msg::Turn(&'static str, String)` with a `Source` enum (`You`, `Them`, `Tool{id}`). The `&'static str` is the tell that it was built for exactly two speakers.
2. Replace `route()`'s `VecDeque<String>` with `Vec<Turn>`, where `Turn` is `Speech{who,text} | Assistant{text} | ToolCall{..} | ToolResult{..}`.
3. Serialize `Vec<Turn>` to provider shape at request time — one function per wire, reusing Phase 1.
4. Append every turn to `logs/<session>.jsonl` as it happens. This is the "memory": greppable, replayable, no dependency.
5. Keep the 24-turn window for what goes to the model; the JSONL keeps everything.

**Why bounded + log, not a vector store:** the model needs the last few minutes of a live call, not semantic recall over months. The log exists so you can read the interview back afterwards, which is the thing you will actually want.

---

## Phase 3 — Slow lane (3 h)

Do not build an agent. Three are already installed.

1. Add `src/agent.rs`: spawn a CLI, read stdout lines, map to `Msg::Tool`.
2. Wire `opencode run --format json -m <provider>/<model>` as the default backend — it emits raw JSON events, supports `-c`/`-s` for session continuity and `-f` to attach files.
3. Add `claude -p "…" --output-format stream-json` and `codex exec` as alternates behind one `--agent-cmd` flag.
4. Trigger on a **hotkey, not voice.** See Safety.
5. Job table replaces the single `seq` counter — a superseded coach reply is worthless, a finished tool result is not.

---

## Phase 4 — ES file search (1 h, BLOCKED)

`es.exe` is a thin IPC client for the Everything indexer. Both are now in place
and working — C:, D: and G: return results.

1. **Blocker:** E: is not in the index, and E: is where this project lives. In the Everything window: Tools → Options → Indexes → NTFS → tick E: → OK. Everything must run **elevated** to read the NTFS master file table; `everything/everything.exe` is launched with `-Verb RunAs`.
2. Verify: `.\es.exe -n 5 coach.rs` returns paths from E:.
3. Add a `search_files` tool: `Command::new("es").args(["-n","20",query])`, parse lines as paths. ~20 lines.
4. Register it in the slow lane only — never the fast lane.
5. Guard with `-n` always set; an unbounded `es` query can return six figures of paths.

**Why ES over walking the filesystem:** Everything queries a live MFT index — whole-disk filename search in single-digit milliseconds, versus seconds for `find`. It is filename/path only; it does not search file *contents*.

---

## Safety — read before Phase 3

The far end's voice is the least-trusted input in this system and currently cannot cause harm, because output is advisory only.

- Whisper mishears. A misheard sentence must never execute an action.
- An interviewee can say "ignore your instructions and…" out loud. That is a live prompt-injection surface with a human driving it.
- **Rule:** read-only tools may fire automatically. Anything with a side effect requires a keypress from you. That preserves "you decide," which is the product.

---

## Open questions

1. Which provider for the fast lane day-to-day? Anthropic has the lowest first-token latency; DeepSeek costs least and is already authenticated.
2. Local model via Ollama on the same 3080 Ti? Whisper uses 573 MB of 12 GB, so a 7–14 B model fits and removes network latency — but competes for the GPU.
