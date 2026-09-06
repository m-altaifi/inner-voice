# inner-voice

A Windows always-on realtime assistant. Whisper transcribes your microphone as
`YOU` and system playback as `THEM`. Suggestions stream into an always-on-top
panel. You choose what to say. Optional research starts only when you press a
hotkey.

It is not call-shaped: nothing detects a call, and nothing ends with one. You
start it when your working day starts and leave it running — pointing it at
whichever apps matter (`--hear`, changeable mid-run) and arming advice only when
you want it (`--manual`, `/coach on`).

## Setup

Requires Windows x64, an NVIDIA GPU/driver, CUDA, Rust, and Visual Studio C++
build tools with CMake and Ninja. The panel is egui on OpenGL and shares the GPU
with transcription; both run together on one card. Run from the project directory:

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
with the time to first token — the same quantity the panel shows as `first word`
— a live turn carries the pinned corpus and the last 24 turns, so expect it
higher), Windows OCR, and the `knowledge/` and `references/` folders, printing ✓
or ✗ with the fix next to each. Run it again after changing `.env`; compare
providers by the number it prints. Exit code 1 means something is ✗.

Edit `.env` to select a provider and supply its key before enabling coaching.
Flags override environment variables and `.env`. Use `--help` for all options.

### Running it

Double-click **`inner-voice.cmd`** in the project folder, or pin it to the
taskbar. Nothing else is needed: any flags you pass it are forwarded, so
`inner-voice.cmd --setup` works too.

You do not have to launch it *from* the project directory. `.env` is found by
walking up from wherever the exe starts, and its folder becomes the working
directory — so the model, `prompt.md`, `knowledge/`, `references/`, `logs/` and
`es.exe` resolve the same whether you double-click the exe in `target\release`,
use a shortcut, or run it from a subfolder. With no `.env` anywhere, the working
directory is left alone and every relative path is yours to supply.

Put the settings you use every time in `.env` rather than on the command line —
`IV_HEAR`, `IV_SPEAK`, `IV_DUMP` and the rest all have entries there — and a
bare launch does the right thing. Every flag has an `IV_*` equivalent.

`build.ps1` discovers Visual Studio using `vswhere`; `IV_VS_PATH` overrides it.
It respects `CUDA_PATH`, `CMAKE_CUDA_ARCHITECTURES`, and `CMAKE_CUDA_FLAGS`.
Architecture defaults to `native`; set an explicit list to target other GPUs.
The script retains the project's `-allow-unsupported-compiler` workaround.
Prefer a supported CUDA/MSVC combination and re-run the GPU test after changes.

## Coaching

Providers: `anthropic`, `openai`, `deepseek`, `gemini`, `openrouter`, or `none`.
OpenAI, Gemini, and OpenRouter require `IV_MODEL`/`--model`. Model availability,
limits, and billing depend on the provider. The Anthropic wire retains fast-mode
settings and needs a compatible model/account. See `.env.example` for keys.

Edit `prompt.md` for the persona. Suggestions use `ASK`, `SAY`, `NOTE`, and `FIX`.
Put call briefs in `knowledge/`: Markdown, text, CSV, spreadsheets
(XLSX/XLSM/XLS/ODS), PDF, Word (.docx) and images (read by Windows OCR) are
supported. A file in `knowledge/` that cannot be read — corrupt, a scanned PDF
with no text layer, an image with no recognisable text — stops startup with its
name; fix or remove it. An empty file is skipped. Replace sample company and
attendee facts with your own. Knowledge is read at startup and re-read before the
next advice whenever a file in the folder changes, so an edit mid-call reaches
the coach; sorted, capped at 400 KB with a truncation marker.
Its glossary also primes Whisper. The latest 24 speech/research turns form the
coaching context; JSONL retains speech, completed advice, and research results.

## Controls and research

The borderless panel is one page: a status line, the advice, the conversation,
and a one-line question box, all visible at once. Advice answers the last turn,
so you need both in front of you; pressing a key to see one of them mid-sentence
is the same interruption as reaching for the mouse. It has no buttons — every
action is a system-wide hotkey, so the panel needs neither the mouse nor focus
while your call app keeps the keyboard. The conversation sits under the advice
and just above the question box, so the newest speech is next to where you
answer it; it grows to at least the last eight turns and scrolls for more.

| Key | Action |
| --- | --- |
| Ctrl+Shift+F1 | Advice |
| Ctrl+Shift+F2 | Whole conversation |
| Ctrl+Shift+F3 | References |
| Ctrl+Shift+F4 | Diagnostics |
| Ctrl+Shift+F5 | Pause / resume transcription |
| Ctrl+Shift+F6 | Hide / show the panel |
| Ctrl+Shift+F7 | Type a question (Enter sends, Esc cancels) |
| Ctrl+Shift+F8 | Research the last turn |
| Ctrl+Shift+F9 | Cancel research |
| Ctrl+Shift+F10 | Clear references |
| Ctrl+Shift+F11 | This list |
| Ctrl+Shift+F12 | Pin the panel where it is |

Ctrl+Shift+F11 shows the same list inside the panel. The research keys register
whenever a coaching provider is online or `--agent-cmd` is set; `--provider none`
with no CLI leaves them unregistered. Ctrl+Shift+F7 puts the caret in the
question box; Enter sends, Escape clears it. Escape does not close anything.
Quit has no hotkey on purpose: Alt+F4 closes the window.

Hotkeys are process-wide and first come, first served. If another app already
owns one of these combinations, registration fails; the panel then names the lost
keys in the notice line and in Diagnostics. Free the key in the other app.

Both panes follow new content to the newest line and stop following as soon as
you scroll up to re-read; scroll back to the end and they resume.

The status line counts the wait for advice — `thinking 1.4s` while the model
works, then `first word 1.2s`, which is the time to first token on that turn
and stays up until the next one. It also names the model in use, so the number
is attributable. `ASK` and `FIX` headings are the only saturated colour; `SAY`
is soft green and `NOTE` recedes. `THEM` is brighter than `YOU` because `THEM`
is what you react to. The last turn's advice stays on screen, greyed, until the
next replaces it. The `Ctrl+Shift+F11` reminder leaves the status line after ten
seconds.

Drag anywhere on the panel to move it — there is no title strip to aim for, and
the panel is meant to sit wherever the call is not. An edge or corner resizes.
Neither needs the panel focused first.
Ctrl+Shift+F12 pins it: while pinned nothing moves or resizes it, and the status
line says so. Panel text is not selectable, which is what leaves the whole
surface free to drag; nothing is lost, because the panel never holds keyboard
focus and so could never have answered Ctrl+C.

The panel never takes the keyboard from your call, not even when you click it —
Ctrl+Shift+F7 is the one exception, and Enter or Escape hands the keyboard
straight back. Pause discards buffered audio and invalidates in-flight
transcription. Diagnostics are separate from the conversation.

Try the interface without a model, microphone, or provider connection:

```powershell
.\target\release\inner-voice.exe --preview
```

Drag any supported file into the window — TXT, Markdown, CSV, spreadsheets,
PDF (text layer; scanned pages need OCR first), Word (.docx), or images, which
Windows OCR reads if a language with OCR is installed. Files in `references/`
(`--references` / `IV_REFERENCES`) are imported at every start; a file dropped
from elsewhere is remembered by its path in `references/.dropped` and re-read
next start — never copied, so editing the original is enough. An edited file is
queued for re-reading at the next retrieval, so the one after it sees the new
text. Limits: 24 files, 10 MB per file, 400 KB extracted text per file. Clear
references empties the index and the remembered paths; files in the folder
return next launch.

Relevant passages are selected through local word matching, with filename and
passage citations. Questions work locally in preview/transcription mode; online
coaching also retrieves passages for the latest remote turn. Before adding a
dropped file in online mode, the interface explains that selected excerpts may
go to the provider and asks you to confirm; files already in `references/` and
the drops `.dropped` remembers are imported at start without that dialog.
Unrelated passages are not automatically sent.

Guided setup is still planned; unsupported files receive an explicit error.
This is not semantic search.

Ctrl+Shift+F8 opens the research view. From any other view it just brings the
last result back up; pressing it again while that result is already on screen
starts a fresh job.

With a coaching provider online and no `--agent-cmd`, research runs over the same
connection as advice — no CLI needed — on its own lane: a second worker with its
own generation counter, so a long answer never queues in front of the ~1 s
advice. It gets the whole 24-turn window, the newest THEM line matched against up
to forty reference passages instead of the four a glance gets, and five times the
token budget an advice turn is allowed. It answers under `research.md` if that
file is in the launch directory, otherwise under a built-in prompt
(`--research-prompt`, `IV_RESEARCH_PROMPT`; `--research-prompt` resolves on its
own, not relative to `--prompt`).

Setting `--agent-cmd` switches research to Claude Code instead;
the CLI wins wherever it is configured, so exactly one lane is live per session.
To use it, install/authenticate the CLI, then configure its actual executable:

```powershell
.\target\release\inner-voice.exe --agent-cmd "$env:APPDATA\npm\node_modules\@anthropic-ai\claude-code\bin\claude.exe"
```

Claude Code's own binary works as that CLI on a subscription — no API key —
at `%APPDATA%\npm\node_modules\@anthropic-ai\claude-code\bin\claude.exe`
(the `claude` on PATH is a shim). It runs with settings, hooks and MCP servers
off and read-only tools only, and reads `knowledge/` and `references/` live
from disk. Its output format is pinned to a captured run
(`tests/fixtures/claude-stream.jsonl`).
The default root is the app folder, which holds `.env`: the tools are read-only
and the prompt points the CLI at `knowledge/` and `references/`, but nothing
stops it opening other files there — point `--agent-root` elsewhere if that
matters to you.

Transcript text goes through stdin, never a shell. Hidden processes belong to a
Windows Job Object; deadlines
(90 seconds by default, applied to the CLI child), cancellation, and app exit
terminate the process tree. Each output stream is truncated at 1 MB rather than
failing the job. Only one research job runs at a time.

`--agent-root` is a working directory, not a filesystem read boundary. The prompt
requests research within it; the CLI is configured with read-only tools. Research
sends context to the CLI's service and may incur charges. A result survives newer
speech and stays available to later coaching turns, truncated to 2,000 characters
so it cannot crowd real speech out of the 24-turn window. A failed job is shown
and logged but never enters that window.

The HTTP lane has been run live against Gemini. No CLI is required for provider
research. To return to it from an old CLI configuration, remove `--agent-cmd`
and unset `IV_AGENT_CMD` in your environment and `.env`.

Research requires an enabled provider; `--provider none` disables it and rejects
a configured research CLI. Speech never triggers it; the fast coach has no
tools. Claude Code is the only supported optional research CLI.

## File search

Install Everything and `es.exe`, start the indexer, and ensure your drive is
indexed. The app does not change indexing settings.

```powershell
.\target\release\inner-voice.exe --search-files 'coach.rs' --es .\es.exe
# Include matching filenames in manually triggered research:
.\target\release\inner-voice.exe --agent-cmd 'C:\path\to\claude.exe' --search-query 'migration'
```

Literal filename/path search is capped at 20 results and five seconds. It does
not read file contents or execute paths. The standalone command needs no model,
key, or audio device. An unavailable indexer produces an error; research records
it as missing evidence and continues.

## Audio and privacy

Use a headset: acoustic echo can put remote speech on the microphone. Calls
outside this PC cannot be captured. Select the actual call playback device with
`--loopback 'Headphones name'`; select a mic with `--mic 'Microphone name'`.
Keep quiet during the first calibration second. Override with `--mic-gate 0.02`
or `--sys-gate 0.02` if needed (values strictly between 0 and 1).
Whisper transcribes English with beam search and a full encoder window.

### Choosing what it listens to

This is not only a call coach: `THEM` is whatever you point it at, so the same
panel transcribes a meeting, a lecture, a video or a colleague's screen share.

To hear only your call app — and not the video you are watching, the music, or
the voice this app reads advice in — name it: `--hear discord` (any part of the
name, case-insensitive; `--list-apps` shows what is playing). `THEM` is then that
app's process tree alone. Name several with a comma — `--hear discord,chrome` —
and each gets its own stream; both arrive as `THEM`. If an app is not running yet
the panel says `hearing: waiting for discord…` and hooks it when it starts; if it
restarts, the panel follows it to its new process. **You do not have to pick from
what is playing**: a name is registered and waited for, which is why there is no
list to choose from — an app that has not made a sound yet has no audio session
to appear in one. `--hear` looks for apps on the `--loopback` device —
`--list-apps` shows which device it searched — so if your call app plays through
a different device, name that device with `--loopback`. Without `--hear`, `THEM`
is everything the speakers play.

### Running it all day

Left running through a working day, advice on every overheard sentence is both
expensive and wrong — a meeting you are only half in, a video, someone at the
next desk. `--manual` (or `IV_MANUAL=true`) starts it listening and *not*
advising: turns are still transcribed, named, logged and kept in the 24-turn
history, so the moment you arm it the coach already knows what has been said.
Advice comes from F7 (ask a question) and F8 (research) until then.

Arm and mute it from the question box with `/coach on` and `/coach off`. The
status line always says which state it is in — `Listening · advice on request
(F7/F8)` versus `Listening · <model>` — because otherwise a quiet coach and a
broken one look identical.

This is not Pause. **F5 stops transcription altogether** and writes nothing
down; `/coach off` keeps the record and only stops the unbidden advice.

### Changing what it hears, mid-run

Press Ctrl+Shift+F7 for the question box and type a `/hear` command.

| Typed | Effect |
| --- | --- |
| `/hear` | reports what is selected; changes nothing |
| `/hear zoom` | hear only Zoom |
| `/hear zoom, chrome` | hear both, each on its own stream |
| `/hear off` (or `mix`, `all`) | back to the whole speaker mix |

The switch takes up to two seconds — the streams check for it on the same
two-second poll that notices an app has closed. `IV_HEAR` in `.env` is the
durable version of the same list; `/hear` is for the session and is not saved.

For transcription with no coach and no network at all, add `--provider none`:
turns still reach the panel, `logs/*.jsonl` and `--dump` still write, F7 still
searches your reference files locally.

An app stream is that app's own digital output, so there is no room noise to
measure and no calibration second: it takes the floor gate (0.004) and reports
`THEM gate 0.0040 (fixed)` in Diagnostics. `--sys-gate` still overrides it. The
speaker-mix path — including the fallback — calibrates as before.

`--speak` reads advice aloud, and is off unless set (`.env.example` ships it on).
With `--hear` the voice is another process and is never captured — unless that
app's stream cannot be opened and `THEM` falls back to the speaker mix, which is
deafened like any other. Without `--hear` the call is deafened while the voice
talks and remote speech can be lost; the panel says so at startup, and adding
`--hear` is the fix.

Optional far-end speaker identification:

```powershell
curl.exe -fL -o models/campplus_sv_en_voxceleb_16k.onnx https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_campplus_sv_en_voxceleb_16k.onnx
```

Names come from `knowledge/attendees.csv` and introductions/direct address.
Uncertain matches stay `THEM`. Thresholds (0.70/0.50) remain unvalidated on real
calls; collect `--dump clips` audio before tuning them.

Coaching sends transcript and knowledge to the provider. Audio stays local.
Logs default to `logs/`; `IV_LOG=` disables them. Audio dumps are opt-in.
`.env`, logs, models, `clips/` and `references/` are gitignored; custom dump
directories need their own ignore rule. Keep call material private and obtain
consent as required.

## Verification

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\build.ps1 test --release
powershell -NoProfile -ExecutionPolicy Bypass -File .\build.ps1 fmt -- --check
```

Tests cover VAD, roster/voice matching, Unicode knowledge, typed history, logging,
SSE errors, child deadlines, research parsing, GPU transcription, and WASAPI.
The GPU test skips without its model; WASAPI needs audio hardware.
Provider testing is opt-in and sends a synthetic transcript:

```powershell
$env:IV_LIVE_TEST = '1'
powershell -NoProfile -ExecutionPolicy Bypass -File .\build.ps1 test --release live_provider
Remove-Item Env:\IV_LIVE_TEST
```

A real call is needed to assess recognition and speaker accuracy. The prerecorded
GPU check proves runtime operation, not conversational accuracy.
