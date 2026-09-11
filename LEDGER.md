# inner-voice — implementation ledger

Durable state for multi-provider + structured history + file search.
Update the **Now** block after every work session. Nothing else here is chronological.

---

## Now

**Merge preparation (2026-09-11):** the user authorized merging all eight PRs
into `main` after fixes. Final review also fixed hidden memory edit results and
learning errors in the notice line. The new regression failed before the fix;
189 unit tests, both integration tests, Clippy, formatting, whitespace, release
build and the full memory GUI smoke now pass. Injected-click focus behavior
passed; the physical drag gesture remains a manual check. The final evidence
is in the [review validation](docs/validation/2026-09-11-memory-review.md).

**PR review follow-up (2026-09-11):** PR #8 now requires complete project names
and correction phrases in source evidence. An explicit correction returning to
a superseded value creates a new revision; ordinary repetition stays suppressed,
and user confirmations and dismissals persist. Three new regressions failed
before the fixes; all four added tests now pass. The release suite passes 188
unit tests and both integration tests, plus Clippy, formatting, whitespace and
the offline release build. All eight PRs remain open for review. See the
[review validation](docs/validation/2026-09-11-memory-review.md).

**Memory architecture (2026-09-10):** implementation is complete across the
[task ledger](docs/plans/brain-inspired-memory-ledger.md), with dependent PRs
in the private `m-altaifi/inner-voice` repository. Review precedes merging.
SQLite knowledge includes attributed evidence, stable IDs, atomic extraction,
corrections, retention and restart-persistent request budgets. Background
learning observes the accepted five-minute/twelve-per-hour limit and mute gate.
Working context and open items feed scoped retrieval; local panel commands
inspect, confirm, correct, dismiss and complete items.

[Validation](docs/validation/2026-09-10-memory.md): 184 unit tests and two
integration tests pass, as do Clippy, formatting, release build and GUI smoke.
The 100,000-turn synthetic replay retained 20,000 claims and answered 1,000/1,000
scoped queries at p95 4.56 ms. A 3,602-second M06 paced test crossed hourly context
rotation and reopened the store. No paid-provider or full-meeting evaluation was
performed; mouse injection and physical dragging remain outside the GUI evidence.

This extends Decision 6: JSONL remains episode evidence; a derived SQLite store
supports transactional knowledge revisions. It follows the transcript logging
opt-out and does not import historical logs automatically.

**Reliability follow-up (2026-09-07):** typed questions have explicit answer
intent and priority over later speech. Silent model responses become visible
failures on this path. Advice uses an eight-second HTTP limit and expires jobs
that already waited eight seconds; research retains its longer timeout.
Interrupted streams clear partial text before completion/logging/TTS. New
meaningful speech retires automatic advice, including already completed text
on the HUD, and late messages cannot bring it back. Audio older than eight
seconds after enqueue is rejected with a notice before/after inference (and
preserved by `--dump`); dropped turns are absent from transcript and recall.
Rare-name recall now scores all matching query terms, preserving old specific
facts despite thousands of generic newer records.

Evidence: `docs/validation/2026-09-07-reliability.md`. Local HTTP fault injection
covers stalled bodies and recovery, truncated streams, explicit silence,
expired jobs and cancellation. The 100,000-turn replay again passed 999/999
recalls, p95 903 microseconds. One configured Gemini request answered the
explicit audit question in 1.255 seconds and rejected false approval, but
assumed an unstated review report. This validates response behavior, not
factual accuracy; spontaneous spoken questions still use automatic judgment.

**Hourly context, recall, interruption control and stress (2026-09-07):**
`history` now rotates the live window every monotonic hour. `route` cancels old
generations and the panel clears stale advice/research. `memory` indexes bounded,
attributed speech from existing transcript logs (no second private store), with
selective local recall before advice/research and `/recall <topic>` for a silent
local pane. Generated advice/research never becomes remembered speech.

`wisdom` preserves short consequential answers, suppresses routine chatter, and
filters streamed `SILENT` markers before display/TTS. The router suppresses exact
repeat triggers for 30 seconds, clears unspent requests on pause/mute/question,
and measures settlement from speech rather than unrelated channel messages.
Capture queues are bounded to four utterances with visible oldest-item loss.

Evidence and unresolved model failures:
`docs/validation/2026-09-07-stress.md`. Final GPU pressure: 1,009 decodes in two
minutes; synthetic normalized WER 0% clean / 0.99% noisy. Accelerated memory:
100,000 turns, 138 hourly resets, 999/999 exact-topic recalls. Seventeen synthetic
Gemini requests exposed unnecessary interruption, a bad base-rate calculation,
and over-silence on a wording request. The quantitative guidance improved the
targeted scientific retest; over-silence remains. These are not human-call or
multi-hour wall-clock results. The older test counts and latency budget below
are historical, and the 800 ms settlement window must be included in end-to-end
latency claims.

**Vigilance and mindfulness (2026-09-07):** the panel advised on what it heard
and never on what the user said. `FIX` — the one tag written for the user's own
vague or oversold line — could only ride along with the far end's *next* turn,
which is an exchange after the sentence was said and long past being walked
back. `route` now owes a request on a finished turn from **either side**,
through the same three gates as before (`--min-words`, `worth_asking`, the
settle window).

**The cost does not double, and the reason is structural rather than lucky.**
The ordinary rhythm of a conversation — they ask, the user answers — has no
real pause in the middle, so both turns share one settle window and buy one
request. The bill only rises where the room actually falls quiet after the user
speaks, which is exactly the turn worth advising on. Guarded three ways: the
user's own claim buys advice, an answer following straight on does not buy a
second, and "yeah, okay, sure" from the user costs nothing just as it already
cost nothing from them.

`situation()` carries a third fact. "React to the newest line" is ambiguous
precisely where it matters — a far-end line wants an answer, the user's own
wants reading back — and it cannot be inferred from the transcript, whose newest
line after a settle window is not necessarily the turn that bought the request.
It rides on the user turn beside the other two facts, never the cached system
prompt, for the same reason they do. `prompt.md` names the posture the tag
vocabulary was already half-built for: **vigilant** outward (the dodged
question, the number that quietly changed between turns, the commitment made on
the user's behalf) and **mindful** inward (what they just conceded, whether they
answered the question asked or the one they wanted, whether they are agreeing
faster than the evidence warrants).

**124 tests, up from 120.** Both behavioural claims mutation-checked: restoring
`who.is_them()` to the trigger fails exactly one test, and dropping the third
fact from `situation` fails exactly one other. A fourth test fell out of it —
`roster()` named its temp folder after the roster it held, so the two tests that
both want `["Sara Osman"]` created, wrote and deleted the same path
concurrently; the loser failed with NotFound. A naming test going red for
reasons unrelated to naming, visible only when thread timing moved.

**The open trade:** advice about the user's own turn arrives while they are
still in the conversation they just spoke into, which is the point, but it is
also a second thing on the panel during their own thinking time. Worth a real
call before deciding whether it needs its own switch. `--manual` and Ctrl+Shift+F3
already mute the whole lane if it proves noisy.

**Provider-cost pass (2026-09-07):** four agents fanned out over the code to
measure where paid requests are actually spent. Three layers now stand between
audio and a request, each catching something the other two structurally cannot.

**1. Whisper scores its own hallucinations and nobody was reading it.** Near
silence makes it invent fluent boilerplate — "Thanks for watching", subtitle
credits — and as *text* that is indistinguishable from speech, so every gate
downstream passed it: three words clears `--min-words`, it becomes a turn,
enters the history, is written to the log, and buys a request. Unbounded by
whether anyone is talking to the user, because background audio crossing the
calibrated gate is enough. `no_speech_probability()` is per segment and was
never read. Filtering it inside `transcribe` costs nothing extra: the utterance
leaves empty and the existing `!text.is_empty()` check already declines to send
a turn — no new message, no new field.

**2. One question was costing eight requests.** The VAD ends a turn after 500 ms
of quiet, which is shorter than the pause between two sentences of one thought,
so a spoken question arrives as many turns and each bought a request — seven of
the answers discarded the moment the next fragment landed. `Coach::ask`'s
`bounded(1)` only collapses what arrives *while* a request is in flight, and
natural speech gaps are longer than the round-trip. Worse than the count:
`cache_control` sits on the system block only, so `messages` is never cached and
each redundant request re-billed the whole 24-turn window and every reference
excerpt with it. `--settle` (default 800 ms) waits for the far end to stop, and
builds the prompt at fire time so the seven prompts we used to assemble and
throw away are never assembled. Measured by driving the real `route` against a
refused provider and counting `AdviceStart`: **4 requests became 1**.

**3. `--min-words 3` let "yeah okay sure" through.** `worth_asking` skips a turn
whose every word is a backchannel. Deliberately timid: a question mark or any
digit asks unconditionally, and "no" is off the list because a bare negative is
usually a decisive answer rather than an acknowledgement.

**Two naming defects, both found by writing the test rather than reading the
code.** The three evidence tiers are documented as *falling*, but `name_voice`
only knew "named" from "blank" — so the weakest tier permanently blocked the
strongest. "Marcus, can you confirm?" names whoever speaks next, which on a
shared line is usually not the person addressed, and "Actually, I'm Priya" could
not take it back for the rest of the call. `guessed` is the missing distinction.
Writing that test then exposed an older bug beneath it: `roster::introduced`
returned **"Priya Marcus"**, because `name_at` takes a second capitalised word as
a surname without checking whether the first ended the clause, and `bare` strips
the comma before it can be seen. "I'm Drew, Sarah's colleague" enrolled one
person under two people's names. Existing coverage missed it because "I'm Ada,
the platform lead" only passed on "the" being in `NOT_A_NAME` — no test ever put
a plausible name after the comma.

**A heard name with no voice to attach it to is now said out loud.** Under
`voiceid`'s 1.5 s floor there is no cluster at all, so the naming block was
skipped entirely and the name vanished with no sign. With pre-roll and hang
around it, "Hey, I'm Drew" *straddles* that floor rather than sitting under it,
so it failed intermittently — worse than failing always, because nobody
notices. Binding it anyway was considered and rejected: nothing says the next
turn is the same person, and the sentence after a self-introduction is usually
somebody else replying to it, so the guess would reach `people.json` and repeat
on every later call. The panel reports what it heard and points at `/who`.

**120 tests, up from 109**, and every behavioural fix mutation-checked against
the code it replaced. Two of those checks earned their keep on the spot:
inverting the no-speech threshold left `gpu_transcribes`' transcript byte for
byte identical, proving that test was no evidence about this filter at all and
forcing the `speech()` seam that can actually be checked; and the notice test
caught Rust line continuations flattening into runs of literal spaces — in its
own expected string, which is the third time that trap has been paid for here.

**Still unvalidated, and now more consequential:** `NO_SPEECH` (0.9) and
`voiceid`'s 0.70/0.50 both need real `--dump` audio. `--settle 800` is a
deliberate trade rather than a free win — advice on the turn that matters
arrives 800 ms later, and `--settle 0` restores the old behaviour.

**Portability pass (2026-09-07):** audited for whether a competent developer on
another machine could clone this and run it without asking a question. The
premise had to be narrowed first — Windows-only is a locked decision, so the
target is *any reasonable Windows x64 machine*, not any machine.

**1. CUDA was a hard requirement with no way round it.** `whisper-rs` carried
`features = ["cuda"]` unconditionally, so a machine without an NVIDIA GPU and
the toolkit failed at build. It is now `default = ["cuda"]` and
`--no-default-features` builds whisper on CPU, with `build.ps1` no longer
demanding `nvcc` on that path. Verified rather than assumed: the CPU build took
2m41s, produced `ggml-cpu.lib` and no `ggml-cuda.lib`, and its exe runs
`--setup` at `use gpu = 0`. **That run is also the honest ceiling — the warm-up
inference measured 25.6 s on CPU against a few hundred ms on the GPU**, so
`large-v3-turbo` cannot meet the 1.2 s budget there and every latency number in
this ledger stays a CUDA number.

**2. The README documented an interface that no longer exists.** After twelve
keys became six it still told a new user to press Ctrl+Shift+F4 for the picker,
F7 to ask, F8 to research, F10 to arm advice and F12 to pin — none of them
registered — and named `/coach on` and `/coach off`, which are not in
`COMMANDS`. The same stale names survived in `--setup`'s own output, in clap's
`--help` on three flags, and twice in `.env.example`. Everything user-facing now
names the **command** instead: `/research` does not renumber when a key does.
`the_readme_documents_the_keys_and_commands_that_exist` guards both shapes of
the drift and caught one of these lines that the fixing edit itself had missed.

**3. Nothing declared a toolchain floor.** `rust-version = "1.95"` now does —
eframe/egui's own MSRV, read out of the dependency tree, because edition 2024's
1.85 would still have been ten releases short. The README also says at last that
debug builds cannot link (LNK2038: prebuilt ONNX Runtime is `/MD`, `knf-rs-sys`
goes `/MDd`) and that the build downloads its own ONNX Runtime, so it needs
network beyond crates.io. Both were known and recorded only in CLAUDE.md.

**4. `cargo test` needed an audio device.** `wasapi_loopback` expected a default
render endpoint and turned red without one, so a VM, a CI runner or an RDP
session failed for a reason unrelated to the code. It skips now — but only when
there is *no endpoint at all*; a device that is present and refuses 16 kHz mono
still fails, because that is the assumption `audio.rs` rests on. Checked against
a real missing device rather than a mock: this machine has no capture endpoint
under RDP.

**Proved from a clean state, not from memory.** A fresh `git clone` into
`C:\iv-clone` with no `.env`, no models and no inherited `IV_*` environment ran
the documented commands verbatim: build **7m44s**, both `curl.exe` model
downloads (28 MB and 547 MB), then `--setup` — **14 of 15 checks green**, CUDA
warmed in 797 ms. The one failure is this RDP session having no microphone,
which `--setup` names correctly along with its fix.

**Left alone deliberately:** `build.ps1 fmt -- --check` already fails on the
committed tree at 21 sites across `audio.rs`, `hud.rs`, `main.rs` and
`roster.rs`. That is rustfmt's own output having moved, not a defect; satisfying
it means whitespace churn in four files outside the portability scope (it
explodes `NOT_A_NAME` into ninety lines), or pinning a toolchain to make a style
check deterministic. Neither is portability work. Also unfixed: there is no CI,
because no runner has an NVIDIA GPU, a microphone and speakers, so a workflow
would either test nothing or be permanently red.

**Accuracy pass (2026-09-07, overnight):** asked to iterate for accuracy, invent
scenarios and test the whole thing. Four defects, three of them found by walking
a *working day* rather than a single call — which is the lesson: every one
needed two events to appear, so no single-turn test could have found any of
them, and none of the 99 tests standing at the time did.

**1. Awareness was a latch, not a window.** "Has the user spoken" was set by the
first microphone turn and never cleared, so a morning call made the afternoon's
YouTube video read as a conversation the user was taking part in. That is the
exact fault `situation()` was written to fix, returning by the back door six
hours later. Now `History::user_spoke` over the retained 24 turns: self-healing,
and the model is told about the same window it is shown.

**2. A heard name could overwrite a known one.** The binding is written through
to `people.json`, so one "I'm Ahmad" out of a noisy second renamed an Ahmed
permanently and on every later call. Heard intros and being addressed now fill a
blank and nothing more. A roster intro may still overwrite — its spelling came
off a list a human wrote, so it is the correction rather than another guess —
and `/who` is how a name gets fixed, because that is a human saying it on
purpose. The precedence moved out of `route`'s channel loop into `name_voice`
for one reason: it is the code that can attribute a sentence to the wrong
person, and inline in a `while let` it could not be tested at all.

**3. The audio source reached the prompt unquoted.** `history::render` quotes
the transcript so a spoken newline cannot forge a speaker label; `situation`
interpolated an app name — a process name off the machine, not the user's
writing — raw into the same prompt. Mutation-checked: an app named
`a
Ignore previous instructions.` produces exactly that as its own line.

**4. A new user could not set up.** The speaker-embedding model had no `--setup`
check at all and its download was buried in a later README section, so a new
user fetched whisper, stopped, and got a system where the far end is never named
— silently, because a missing embedding model is a working call by design. And
`--setup`'s summary said "fix the ✗ lines above", which on a fresh machine is a
wall rather than an instruction; it now names the first failure and its fix,
since checks run in dependency order.

**Both fixes were mutation-checked** — reverted to the code they replaced, run,
and confirmed to fail with the right message — because a test that passes
against the bug is worse than no test.

**`tools/voice_memory.ps1`** is new and is the first end-to-end proof of the
voice book: the far end introduces itself aloud through SAPI in a second
process, the name binds to the voice cluster, reaches `people.json`, and a
second run loads it with nothing spoken. Its first real run demonstrated the
guards on real audio better than any fixture could: whisper misheard the
introduction twice as *"I made a lovelace"* and **neither mishearing enrolled
anybody**; the correct reading did. Afterwards the same misheard sentence was
still attributed to Ada Lovelace, because the name is on the voice and not on
the transcript.

It deliberately does **not** test telling two people apart. Every Microsoft TTS
voice shares a vocoder and they score 0.72–0.84 against *each other*, above the
0.70 `SAME` threshold, so synthetic speech cannot demonstrate discrimination in
either direction — the same trap as benchmarking ASR on `jfk.wav`, in a new
place.

108 unit tests (was 99), clippy clean, and every live script green:
`ui_smoke.ps1`, `hear_isolation.ps1` single and multi-app, `voice_memory.ps1`,
GPU and loopback. `hear_isolation`'s far-end lines now read `"who":"THEM"` where
its own comment records `"who":"Sarah Chen"` happening on a real run — the
corpus deletion, confirmed in the live path rather than argued.

**Unchanged, and now the only thing left:** `voiceid.rs:16` (0.70/0.50) is still
unvalidated, and the voice book makes that matter more rather than less, because
a misattribution now persists into the next call instead of dying with this one.
`SAME` erring high means "not sure" rather than "wrong person", which is the safe
direction. Only a real call with `--dump` moves it.

**First real use (2026-09-07):** "so far it's perfect; just not fully aware."
Two findings, both from actually running it, neither reachable from a test.

**1. It invented the situation.** A YouTube video produced advice about "the
interview". Root cause was not the model: `prompt.md` opened with "You are the
user's inner voice during a live call" and carried a rule about interviews, and
nothing in the request ever said what was being listened to. So the model filled
the gap with the only shape it had been given. This program *knew* the answer —
the `--hear` selection — and threw it away every turn.

`situation()` now prefixes each coach request with the two facts available:
which app is selected (through `describe`, lifted out of `set_hearing` so the
notice line and the coach cannot disagree about what `THEM` is) and whether YOU
has ever spoken. The second is the stronger signal and costs one bool: a video
has speakers and no participants, and a user who never answers is not in a
conversation. On the **user** turn, not the system prompt — the system prompt is
behind the cache breakpoint and rebuilt only on corpus change, while `/hear` can
change the answer mid-session, and a stale situation line is worse than none.
`prompt.md` stops asserting a call, is told not to name a situation the
transcript has not established, and drops `SAY` when the user has not spoken.

**2. Long turns ran off the right edge.** egui gives children of a horizontal
layout unbounded width, and the conversation strip is `YOU` + body in a
`horizontal_top`. The body moved into a `ui.vertical` child, which gets the
width that is left.

Worth recording how badly the *diagnosis* went, because the fix was right for
ten minutes before I broke my own evidence. The preview seeds the screenshots,
and every seeded line was short enough to fit one row — so no screenshot could
ever have shown this, which is why it survived to a real session. Adding a long
one, I wrote it with Rust `\` line continuations that got flattened into runs of
literal spaces, read the resulting gaps as egui justifying the text, and wrote a
comment recording that false finding. The clean string wraps correctly. The
comment now says the true reason. **A screenshot is only evidence if the thing
being photographed is the real case.**

99 unit tests (+1), GPU, loopback and `ui_smoke.ps1` green, verified by eye
against a captured frame rather than by assertion — wrapping is drawing, and
`ui_smoke` deliberately mirrors state rather than render.

**The voice book (2026-09-06):** "the ai should identify speakers and store the
data. and they're different people everytime." The second half is the part that
made the first half wrong: `attendees.csv` is a list somebody writes *before* a
call, so it only ever worked for a standing meeting with the same three people.
Everything else stayed THEM forever.

`people.rs` is the store — a name and a CAM++ centroid per person, in
`people.json` (`--people`, `IV_PEOPLE`; empty disables it and writes nothing
about anyone's voice to disk). The design decision that kept it small: the
book's entries *are* `voiceid`'s clusters, seeded at startup instead of starting
empty, so recognising someone from last month and recognising them from earlier
in the call is one code path rather than a second lookup bolted alongside. Only
named people are written; an anonymous cluster cannot be recognised again
without a name to offer.

Shared `Arc<Mutex<Book>>` between the THEM whisper worker and `route`, because
naming is split across threads by a rule that is not up for revisiting: the
worker has the embeddings, `route` has the transcript of both sides. `remember`
only ever adds — `bind`'s conflict rule clears both voices when two claim one
name, which is right for the call in front of you and wrong for a record.

`MAX_VOICES` became `MAX_NEW`, counted per call. Eight strangers in one meeting
is a clustering failure; eight hundred people across a year is just a year, and
the old cap would have refused to hear the ninth person ever.

`roster::introduced` is the only place this program accepts a name nobody wrote
down, and it is the narrowest thing that could work: self-introduction only,
never being addressed — "Ahmed, can you take this?" is far commoner than "I'm
Ahmed" and is exactly the sentence that enrols someone who is not on the call.
`OPENERS` drops `it's` (the roster path can afford it; without a list, "it's
Tuesday" enrols a person). Two of its guards came from tests failing rather than
from foresight: possessives ("I'm Ahmed's manager" named the manager Ahmed) and
hyphenated names.

`/who <name>` names whoever spoke last, and is the path that matters most —
plenty of calls never say a name aloud at all. Bare `/who` reports. `--setup`
gained a `people` check listing the book, verified live loading a book written
by hand and reporting `off` when disabled.

98 unit tests (+13), GPU and loopback green, clippy clean. Two members went dead
in the process (`Book::known`, `Roster::is_empty`) and were deleted rather than
kept for symmetry.

**Unchanged and still the tail:** the thresholds this all now depends on
(`voiceid.rs:16`, 0.70/0.50) are still unvalidated, and a store makes them
matter more, not less — a misattribution now persists to the next call instead
of dying with this one. `SAME` erring high means "not sure" rather than "wrong
person", which is the safe direction, but the acceptance call with `--dump` is
the only way to tune it. That call remains the blocking item for this feature as
much as for the last three.

**"wtf is sarah chen" (2026-09-06):** this project shipped a sample corpus in
`knowledge/` — `attendees.csv` naming Sarah Chen (Platform Lead) and Marcus Webb
(CTO), a `facts.csv` with a headcount of 42, a `company.md`. Placeholder text by
intent. Every real run loaded it, and six log files across earlier sessions carry
"Sarah Chen" — nine occurrences — as a *speaker*.

It is not a cosmetic default, because the folder feeds three consumers and
invented people break all three:

1. the coach is told the corpus is source of truth, so it advises around a
   roster that does not exist;
2. `knowledge::glossary` puts the proper nouns into whisper's `initial_prompt`,
   which **primes the transcriber to hear them** — garbled audio comes back
   spelling "Sarah Chen";
3. `roster.rs` reads `attendees.csv` from the same folder and binds the heard
   name to the far-end voice cluster for the rest of the call, relabelling every
   later turn in the panel and the JSONL.

An invented fact that is only ever read is a curiosity. This one is fed back into
the transcriber that produces the evidence, so it manufactures its own
confirmation — which is why it looked random from the outside and why the
2026-09-06 wave-1 report found it as a `--hear` defect before tracing it here.

Fixed by deletion, not by better sample data: `knowledge/` is now git-ignored
apart from `.gitkeep`, and the folder documentation lives in README rather than
in the folder — a README in there *is* corpus, and the first draft of it named
Sarah Chen, so it primed whisper on the very run that was meant to prove the
fix. Caught by `--setup` reporting `1 files`.

Two things now make the mechanism visible instead of relying on nobody repeating
it. `Roster::names` puts the loaded roster in the startup notice — `naming: Ada
Lovelace, Grace Hopper` rather than the old `naming: on` — so a name arriving
mid-call is traceable to a file the user has seen; `naming:` was added to the
notice-line filter, since Diagnostics-only would have kept it invisible. And
`--setup` gained a `priming` check that prints the first six glossary terms:
`whisper is biased toward: Ada Lovelace, Platform Lead, Grace Hopper, CTO`, or
`nothing — whisper hears only what is said`. A file count cannot show bias; the
terms can. Verified both ways.

Not fixed, and still needing real-call evidence: a *correct* roster can still
bind a primed mishearing to the wrong voice. `prompt.md` holds that a wrong name
is worse than no name, and the wave-1 finding on this belongs to the acceptance
call along with `voiceid.rs:16`. Removing invented names removes the case that
was firing every run; it does not remove the mechanism.

Old logs still contain the fabricated speaker. Left alone — they are the user's
record, and rewriting a transcript to remove a name it really contained is a
worse habit than a footnote.

**Twelve keys to six (2026-09-06):** "the options feel crowded and too much."
Correct, and it was the accumulation pattern rather than any one decision: each
key was argued for on its own merits and the set was never re-read as a whole,
so a panel whose entire claim is *don't make me look away mid-sentence* ended
up with a key list you had to look away to read.

Keys kept: Back to advice, Ask, the advice toggle, Pause, Hide, Help — F1..F6.
The test that keeps this honest is not taste: a command has to be **typed**, and
typing calls `borrow_keyboard`, so anything that must never cost the call app
its keyboard cannot become one. That is what saves Pause and Hide, the two you
reach for when something private happens, however rarely they are pressed.

The other seven became `/` commands with their `id` and handler untouched:
`/sources` `/transcript` `/references` `/research` `/cancel` `/pin` `/clear`
`/diagnostics`. Nothing was removed and nothing became unreachable —
`every_action_is_reachable_by_exactly_one_key` now walks all fourteen actions
and asserts exactly one of the two, never neither and never both, which
replaces the case-by-case grandfathering it had grown into.

`COMMANDS` is now `(name, id, help)` and *is* the dispatcher. It was two
hardcoded `if text == ...` arms plus a help table that only happened to agree
with them. Six sentences on screen also spelled out key numbers by hand; `fkey`
derives them, so moving a row can no longer make a sentence lie.

**Found by doing it:** the two commands that already existed set `typing = false`
without `return_keyboard`, leaving the panel foreground with `prior`
unrestored — typing a command took the call app's keyboard and never gave it
back. Invisible while two rare commands existed; not once most actions reach the
user that way. Fixed at the single path all of them now walk.

**F7..F12 are given back.** `RegisterHotKey` is first-come process-wide, so six
rows we did not need were six combinations withheld from every other app on this
machine. `ui_smoke.ps1` asserts they are now free — the mirror image of the
ownership check beside it, and the only part of the cut a unit test cannot see.

First clippy run on the crate: one warning, `route`'s eight arguments, allowed
in place with the reason. 85 unit tests, GPU, loopback and `ui_smoke.ps1` green.

**Still not done, and unchanged by any of this:** the system has never met a
real call. `voiceid.rs:16` (0.70/0.50), `HANG_MS` and whisper's accuracy are all
still guesses that have not been contradicted. The Gemini and OpenRouter keys
want rotating.

**The app picker (2026-09-06):** asked for three times, and refused twice on a
misreading of the panel's own rule. The no-controls rule is about *focus* — a
control that must be focused to be pressed costs a mouse grab and steals the
keyboard from the app being listened to. A `WS_EX_NOACTIVATE` window still
receives mouse input, which is exactly how dragging has always worked, so a
clickable row costs neither. The rule never forbade this; I did.

Ctrl+Shift+F4 now opens Sources: the whole speaker mix as the first row, then
every app `audio::playing` finds on the loopback device. Click to hear one,
click more for several at once, click again to turn one off. A selected app
that has gone quiet stays listed as `— not playing`, so a click can always undo
itself. `Tune::hear_only` / `Tune::toggle` are the one write path, shared with
`/hear`, so the write-then-bump ordering is written down once.

`/hear` stays, and now for a stated reason rather than an excuse: a list can
only offer apps with a live audio session, and naming an app *before* it starts
is precisely what the reacquire loop exists for. Diagnostics gave up F4 and
moved to `/diagnostics` — it is a when-something-breaks view, which is what the
command surface is for.

Verified live rather than argued: launched with `--hear notrunning` while six
apps were playing, posted the hotkey, and read the panel's own mirror —
`view: 115`, all six offered with `playing=True`, and `notrunning` listed with
`playing=False`. `mirror_state` now carries `sources` and `hearing` so the
picker is scriptable at all. 85 unit tests (+2).

**The voice (2026-09-06):** "so robotic" had a one-line cause. `--speak` used
whatever `ISpVoice` picks with no `SetVoice`, which is the SAPI5 category — and
that category holds only *Desktop* voices, Microsoft's oldest concatenative set.
Six OneCore voices (George, Hazel, Susan, David, Mark, Zira) were installed the
whole time in a category SAPI does not enumerate unless asked for it by id.
`speak::voices` now asks, via `ISpObjectTokenCategory::SetId` on the
`Speech_OneCore\Voices` key.

Measured on the way: `SpObjectToken::SetId` straight to a OneCore token fails
with `SPERR_NOT_FOUND` (0x80045041); going through the category enumerates all
six and `SetVoice` takes them. Proved in PowerShell before any Rust was written.

`--voice` / `IV_VOICE` picks one by any part of its name, the way `--hear`
matches an app, and `--setup` gained a `voice` line that names the chosen voice
and lists the rest — a miss is reported with the list rather than silently
substituted, so a typo cannot look like a preference that worked. 83 unit tests
(+2).

Not installed here, and the next real step if OneCore is still not good enough:
Windows 11's offline *natural* (neural) voices — Settings > Accessibility >
Narrator > Add natural voices. Whether they surface in the OneCore category is
unverified; `--setup` will say.

**Frameworks looked at and declined:** `zavora-ai/adk-rust` (638★, v2.2.0, 43
crates) and `microsoft/VibeVoice`. Neither is a fit — see the note in the
decisions below.

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
