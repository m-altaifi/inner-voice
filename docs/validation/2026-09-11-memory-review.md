# Memory PR review follow-up — 2026-09-11

PR #8 includes three fixes found while reviewing the completed memory stack.
All eight dependent PRs remain open. GitHub reported them as mergeable, with no
review comments or automated check runs at the start of this review.

| Failure reproduced before the fix | Result after the fix |
|---|---|
| An extraction could assign `Ori` to speech naming only `Orion` | Project scope must match a complete phrase in cited speech; an invalid claim rejects the whole batch without consuming pending evidence |
| `Factually` satisfied the substring check for the correction cue `actually` | Correction cues require complete phrase matches; this example keeps both competing claims disputed |
| A speaker corrected Priya to Alex, then explicitly back to Priya; the final correction attached to the superseded row and left Alex active | The return creates a new revision with its own evidence and ID, preserves the old revision, and supersedes the intervening value |

Four regression tests cover these cases, including Unicode project names,
case/whitespace normalization, punctuation boundaries, and preservation of user
confirmations and dismissals. Ordinary repetition of a superseded claim remains
hidden. The three failure regressions were run against the original code and
failed before the implementation changed.

## Validation

Windows x64, release profile, default CUDA feature. The full suite passed
**188 unit tests and two integration tests**, including GPU transcription and
WASAPI loopback. Six opt-in tests were ignored and the live provider test was
filtered. Clippy with warnings denied, formatting, whitespace checks and the
offline release build all passed.

```powershell
.\build.ps1 test --release --offline -- --skip live_provider_returns_advice
.\build.ps1 clippy --release --offline --all-targets -- -D warnings
.\build.ps1 fmt -- --check
git diff --check
.\build.ps1 build --release --offline
```

This follow-up changes claim validation and revision handling only. It makes no
schema migration and does not reprocess historical extractions. Phrase checks
cannot establish that the model interpreted the source accurately.

No paid provider calls were made. The GUI smoke, 100,000-turn benchmark and
one-hour paced run were not repeated for this follow-up; their dated evidence
and limits remain in the [original validation report](2026-09-10-memory.md).
