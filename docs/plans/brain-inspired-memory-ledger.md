# Memory and awareness implementation ledger

Accepted 2026-09-10. Automatic attributed learning starts with new sessions;
existing transcripts keep their original local recall behavior. Learning uses
the configured provider, at most once every five minutes / twelve requests per
rolling hour, and pauses when advice is muted. No automatic merging of PRs.

## Delivery

Private repository: `m-altaifi/inner-voice`. Preserve existing Git history and
uncommitted work. Stacked PRs target their immediate dependency. Statuses are
`planned`, `in_progress`, `blocked`, `in_review`, `done`; only merged work is done.

| ID | Task / branch suffix | Dependency | Status | Acceptance | PR / evidence |
|---|---|---|---|---|---|
| M01 | `01-ledger`: architecture and task ledger | repository | in_review | accepted decisions and review sequence recorded | [PR 1](https://github.com/m-altaifi/inner-voice/pull/1) |
| M02 | `02-foundation`: preserve existing changes | M01 | in_review | 158 unit + 2 integration tests passed | [PR 2](https://github.com/m-altaifi/inner-voice/pull/2) |
| M03 | `03-store`: SQLite knowledge and provenance | M02 | in_review | six storage tests pass | [PR 3](https://github.com/m-altaifi/inner-voice/pull/3) |
| M04 | `04-learning`: bounded background consolidation | M03 | in_review | two worker tests pass, including actual local HTTP and mute gating | [PR 4](https://github.com/m-altaifi/inner-voice/pull/4) |
| M05 | `05-retrieval`: revisions and contextual recall | M04 | in_review | nine store/retrieval tests pass; scope and 4 KB limits verified | PR pending |
| M06 | `06-awareness`: working memory and open items | M05 | planned | reset transient context, retain relevant commitments | pending |
| M07 | `07-controls`: inspect and correct memory | M06 | planned | command panes, source inspection, learning status | pending |
| M08 | `08-validation`: retention and release evidence | M07 | planned | fault tests, 100k replay, paced hour, restart, release checks | pending |

## Implementation contract

- JSONL remains source evidence; `<IV_LOG>/memory.sqlite3` stores derived claims,
  evidence references, open items, extraction progress and user changes. Database
  sidecars stay ignored. SQLite foreign keys and transactional batch commits are required.
- `EpisodeRef`, `KnowledgeClaim`, `OpenItem`, `ConsolidationBatch`,
  `AwarenessSnapshot` retain explicit provenance and stable IDs. Speech is data,
  never authorization; advice/research never becomes its own evidence.
- Claim states: reported, user_confirmed, disputed, superseded. Confirmation is
  user endorsement, not independent verification. Repetition adds no authority.
  Only same-source explicit correction or user correction supersedes a claim.
- `--learning` / `IV_LEARNING` defaults off for compatibility. New sessions only;
  first enable records a boundary. No logging means process-local knowledge only.
- Same provider/model, separate worker, no tools/switches. 16 KB source batches,
  1,500 output tokens, 30-second timeout. Persist progress and request timing;
  retries consume scheduled slots, three failures quarantine a batch.
- Pause/mute stops automatic requests and retires results. Re-arming permits
  pending new-session speech. Expose backlog, failures, last success and lost work.
- Local retrieval and awareness add at most 4 KB to a prompt. Capture and advice
  never wait for consolidation or a database lock. Failures preserve existing coaching.
- Hourly/source changes clear participation/topic assumptions; durable open items
  remain but enter context only when relevant. Evidence removal invalidates
  derived knowledge. Existing retention applies; no historical import in v1.
- Commands: `/memory [topic]`, `/awareness`, `/commitments`, `/learning on|off`,
  plus stable-ID confirmation, correction, dismissal and completion. Dismissal
  suppresses a learned claim; original transcripts remain. Preserve hotkeys/focus.

## Checks required per task

- [ ] Implement behavior and meaningful regression tests.
- [ ] Run relevant release tests through `build.ps1`; no paid tests by default.
- [ ] Check formatting, Clippy, diff whitespace and build.
- [ ] Run preview checks for routing/pane changes.
- [ ] Record actual results and limitations in this ledger and the PR body.
- [ ] Push a reviewable branch and open its dependent PR; do not merge.

Final evaluation separates infrastructure success from decision quality. Cover
malformed provider output, disputed owners, speaker ambiguity, cross-project
isolation, source deletion, logging disabled, corruption, restart, rate limiting
and cancellation. Replay 100,000 turns and target local retrieval p95 <10 ms on
the development machine. A one-hour paced session must cross context rotation
and restart. Synthetic data only for paid evaluation; paid tests remain opt-in.

Deferred: embeddings, model training, screen monitoring, historical extraction.
