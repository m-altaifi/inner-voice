//! Evidence-backed knowledge. The worker owns this connection; the audio router
//! only sees an immutable bounded snapshot, never a database lock.
use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

pub const BATCH_BYTES: usize = 16_000;
pub const INTERVAL: i64 = 300;
pub const MAX_CLAIMS: usize = 20_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EpisodeRef {
    pub id: i64,
    pub source: String,
    pub offset: i64,
    pub speaker: String,
    pub time: i64,
    pub text: String,
    pub fingerprint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KnowledgeClaim {
    pub id: i64,
    pub scope: String,
    pub subject: String,
    pub key: String,
    pub text: String,
    pub kind: String,
    pub status: String,
    pub open: bool,
    pub updated: i64,
    pub evidence: Vec<EpisodeRef>,
}

#[derive(Clone, Debug)]
pub struct OpenItem {
    pub claim: KnowledgeClaim,
}

#[derive(Clone, Debug)]
pub struct ConsolidationBatch {
    pub episodes: Vec<EpisodeRef>,
    pub input: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extraction {
    pub claims: Vec<Candidate>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub scope: String,
    pub subject: String,
    pub key: String,
    pub text: String,
    pub kind: String,
    pub evidence: Vec<i64>,
    pub correction: bool,
}

pub struct Store {
    conn: Connection,
}

fn fingerprint(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

pub fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

impl Store {
    pub fn open(folder: Option<&Path>) -> Result<Self> {
        let conn = match folder {
            Some(folder) => Connection::open(folder.join("memory.sqlite3")),
            None => Connection::open_in_memory(),
        }
        .context("opening learned memory")?;
        conn.busy_timeout(std::time::Duration::from_millis(100))?;
        conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(
            version <= 2,
            "memory schema is newer than this application; leave it intact"
        );
        if version == 0 {
            conn.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY, value INTEGER NOT NULL);
                 CREATE TABLE IF NOT EXISTS sources(path TEXT PRIMARY KEY, position INTEGER NOT NULL);
                 CREATE TABLE IF NOT EXISTS episodes(
                   id INTEGER PRIMARY KEY AUTOINCREMENT, source TEXT NOT NULL, position INTEGER NOT NULL,
                   speaker TEXT NOT NULL, time INTEGER NOT NULL, text TEXT NOT NULL,
                   fingerprint TEXT NOT NULL, state INTEGER NOT NULL DEFAULT 0,
                   attempts INTEGER NOT NULL DEFAULT 0,
                   UNIQUE(source,position,fingerprint));
                 CREATE INDEX IF NOT EXISTS episode_pending ON episodes(state,id);
                 CREATE TABLE IF NOT EXISTS claims(
                   id INTEGER PRIMARY KEY AUTOINCREMENT, scope TEXT NOT NULL, subject TEXT NOT NULL,
                   field TEXT NOT NULL, text TEXT NOT NULL, kind TEXT NOT NULL,
                   status TEXT NOT NULL DEFAULT 'reported', is_open INTEGER NOT NULL,
                   updated INTEGER NOT NULL, origin TEXT NOT NULL);
                 CREATE INDEX IF NOT EXISTS claim_identity ON claims(scope,subject,field);
                 CREATE TABLE IF NOT EXISTS evidence(
                   claim INTEGER NOT NULL REFERENCES claims(id) ON DELETE CASCADE,
                   episode INTEGER NOT NULL REFERENCES episodes(id) ON DELETE CASCADE,
                   PRIMARY KEY(claim,episode));
                 CREATE TABLE IF NOT EXISTS requests(time INTEGER NOT NULL);
                 CREATE TABLE IF NOT EXISTS corrections(
                   id INTEGER PRIMARY KEY, claim INTEGER REFERENCES claims(id) ON DELETE CASCADE,
                   time INTEGER NOT NULL, action TEXT NOT NULL, previous TEXT NOT NULL);
                 PRAGMA user_version=2;
                 COMMIT;",
            )?;
        }
        if version == 1 {
            conn.execute_batch("PRAGMA foreign_keys=OFF; BEGIN IMMEDIATE;
                CREATE TABLE episodes_v2(id INTEGER PRIMARY KEY AUTOINCREMENT,source TEXT NOT NULL,position INTEGER NOT NULL,speaker TEXT NOT NULL,time INTEGER NOT NULL,text TEXT NOT NULL,fingerprint TEXT NOT NULL,state INTEGER NOT NULL DEFAULT 0,attempts INTEGER NOT NULL DEFAULT 0,UNIQUE(source,position,fingerprint));
                INSERT INTO episodes_v2 SELECT * FROM episodes;
                DROP TABLE episodes; ALTER TABLE episodes_v2 RENAME TO episodes;
                CREATE INDEX episode_pending ON episodes(state,id);
                CREATE TABLE claims_v2(id INTEGER PRIMARY KEY AUTOINCREMENT,scope TEXT NOT NULL,subject TEXT NOT NULL,field TEXT NOT NULL,text TEXT NOT NULL,kind TEXT NOT NULL,status TEXT NOT NULL DEFAULT 'reported',is_open INTEGER NOT NULL,updated INTEGER NOT NULL,origin TEXT NOT NULL);
                INSERT INTO claims_v2 SELECT * FROM claims;
                DROP TABLE claims; ALTER TABLE claims_v2 RENAME TO claims;
                CREATE INDEX claim_identity ON claims(scope,subject,field);
                PRAGMA user_version=2; COMMIT; PRAGMA foreign_keys=ON;")?;
        }
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS evidence_episode ON evidence(episode);
            CREATE INDEX IF NOT EXISTS episode_source ON episodes(source,position);
            CREATE INDEX IF NOT EXISTS episode_time ON episodes(time);",
        )?;
        Ok(Self { conn })
    }

    pub fn enable_at(&self, now: i64) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO metadata VALUES('first_enabled',?1)",
            [now],
        )?;
        Ok(())
    }

    pub fn enabled_before(&self) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM metadata WHERE key='first_enabled'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .is_some())
    }

    pub fn register(&self, source: &str, position: i64) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO sources VALUES(?1,?2)",
            params![source, position],
        )?;
        Ok(())
    }

    pub fn sources(&self) -> Result<Vec<(String, i64)>> {
        Ok(self
            .conn
            .prepare("SELECT path,position FROM sources ORDER BY path")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn ingest(
        &mut self,
        source: &str,
        position: i64,
        next: i64,
        speaker: &str,
        time: i64,
        text: &str,
    ) -> Result<()> {
        ensure!(
            text.len() <= 12_000
                && source.len() <= 1024
                && speaker.len() <= 200
                && position >= 0
                && next >= position
                && time >= 0,
            "episode exceeds storage limit"
        );
        let probe = EpisodeRef {
            id: i64::MAX,
            source: source.into(),
            offset: position,
            speaker: speaker.into(),
            time,
            text: text.into(),
            fingerprint: fingerprint(text),
        };
        ensure!(
            serde_json::to_vec(&probe)?.len() + 2 <= BATCH_BYTES,
            "encoded episode exceeds extraction batch limit"
        );
        let tx = self.conn.transaction()?;
        if !text.trim().is_empty() && speaker != "COACH" && !speaker.starts_with("RESEARCH") {
            tx.execute("INSERT OR IGNORE INTO episodes(source,position,speaker,time,text,fingerprint) VALUES(?1,?2,?3,?4,?5,?6)",
                params![source,position,speaker,time,text,fingerprint(text)])?;
        }
        tx.execute(
            "UPDATE sources SET position=?2 WHERE path=?1",
            params![source, next],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn advance(&self, source: &str, position: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE sources SET position=?2 WHERE path=?1",
            params![source, position],
        )?;
        Ok(())
    }

    pub fn skip_record(&mut self, source: &str, position: i64) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE sources SET position=?2 WHERE path=?1",
            params![source, position],
        )?;
        tx.execute("INSERT INTO metadata VALUES('skipped_records',1) ON CONFLICT(key) DO UPDATE SET value=value+1",[])?;
        tx.commit()?;
        Ok(())
    }

    pub fn evidence_matches(
        &self,
        source: &str,
        check: &mut dyn FnMut(EpisodeRef) -> Result<bool>,
    ) -> Result<bool> {
        let mut stmt=self.conn.prepare("SELECT e.id,e.source,e.position,e.speaker,e.time,e.text,e.fingerprint FROM episodes e WHERE e.source=?1 AND (e.state=0 OR EXISTS(SELECT 1 FROM evidence v WHERE v.episode=e.id)) ORDER BY e.position")?;
        for row in stmt.query_map([source], episode_row)? {
            if !check(row?)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn batch(&self) -> Result<Option<ConsolidationBatch>> {
        let mut stmt = self.conn.prepare("SELECT id,source,position,speaker,time,text,fingerprint FROM episodes WHERE state=0 ORDER BY id LIMIT 64")?;
        let records = stmt.query_map([], episode_row)?;
        let mut episodes = Vec::new();
        let mut used = 2;
        for record in records {
            let record = record?;
            let bytes = serde_json::to_string(&record)?.len() + 1;
            if used + bytes > BATCH_BYTES {
                break;
            }
            used += bytes;
            episodes.push(record);
        }
        if episodes.is_empty() {
            return Ok(None);
        }
        let input = serde_json::to_string(&episodes)?;
        Ok(Some(ConsolidationBatch { episodes, input }))
    }

    /// Reserve before sending: a crash/restart cannot buy a thirteenth request.
    pub fn reserve_request(&mut self, now: i64) -> Result<bool> {
        let tx = self.conn.transaction()?;
        let last: Option<i64> = tx.query_row("SELECT MAX(time) FROM requests", [], |r| r.get(0))?;
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM requests WHERE time>?1",
            [now - 3600],
            |r| r.get(0),
        )?;
        if last.is_some_and(|t| now.saturating_sub(t) < INTERVAL) || count >= 12 {
            return Ok(false);
        }
        tx.execute("DELETE FROM requests WHERE time<=?1", [now - 3600])?;
        tx.execute("INSERT INTO requests VALUES(?1)", [now])?;
        tx.commit()?;
        Ok(true)
    }

    pub fn fail_batch(&mut self, batch: &ConsolidationBatch) -> Result<()> {
        let tx = self.conn.transaction()?;
        for e in &batch.episodes {
            tx.execute("UPDATE episodes SET attempts=attempts+1,state=CASE WHEN attempts>=2 THEN 2 ELSE 0 END WHERE id=?1 AND state=0",[e.id])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn counts(&self) -> Result<(i64, i64, i64)> {
        Ok((
            self.conn
                .query_row("SELECT COUNT(*) FROM episodes WHERE state=0", [], |r| {
                    r.get(0)
                })?,
            self.conn
                .query_row("SELECT (SELECT COUNT(*) FROM episodes WHERE state=2)+COALESCE((SELECT value FROM metadata WHERE key='skipped_records'),0)", [], |r| {
                    r.get(0)
                })?,
            self.conn.query_row(
                "SELECT COALESCE(MAX(value),0) FROM metadata WHERE key='last_success'",
                [],
                |r| r.get(0),
            )?,
        ))
    }

    pub fn apply(&mut self, batch: &ConsolidationBatch, output: &str, now: i64) -> Result<()> {
        ensure!(output.len() <= 32_000, "extraction exceeds response limit");
        let result: Extraction = serde_json::from_str(output).context("invalid memory JSON")?;
        ensure!(result.claims.len() <= 24, "too many extracted claims");
        for c in &result.claims {
            validate(c, batch)?;
        }
        let tx = self.conn.transaction()?;
        for e in &batch.episodes {
            let current = tx
                .query_row(
                    "SELECT state FROM episodes WHERE id=?1 AND fingerprint=?2",
                    params![e.id, e.fingerprint],
                    |r| r.get::<_, i64>(0),
                )
                .optional()?;
            ensure!(
                current == Some(0),
                "batch evidence changed, was deleted, or was already processed"
            );
        }
        for c in result.claims {
            let first = batch
                .episodes
                .iter()
                .find(|e| e.id == c.evidence[0])
                .unwrap();
            let origin = speaker_identity(first);
            // Unknown scope is session-local; never join unrelated projects on a guess.
            let scope = if c.scope.trim().is_empty() {
                format!("session:{}", first.source)
            } else {
                normalized(&c.scope)
            };
            let subject = normalized(&c.subject);
            let key = normalized(&c.key);
            let text = c.text.trim();
            let explicit_correction = c.correction
                && first.speaker != "THEM"
                && c.evidence.iter().any(|id| {
                    let e = batch.episodes.iter().find(|e| e.id == *id).unwrap();
                    let t = normalized(&e.text);
                    speaker_identity(e) == origin
                        && [
                            "correction",
                            "correcting",
                            "actually",
                            "has taken over",
                            "replaces",
                            "instead of",
                            "no longer",
                        ]
                        .iter()
                        .any(|s| crate::memory_context::scope_matches(&t, s))
                });
            // Ordinary repetition stays attached to its previous revision.
            // An explicit return to a superseded value needs a new revision;
            // reusing the hidden row would silently discard the correction.
            // Dismissed and user-confirmed items still take the duplicate path.
            let duplicate: Option<i64> = tx.query_row("SELECT id FROM claims WHERE scope=?1 AND subject=?2 AND field=?3 AND text=?4 AND kind=?5 AND origin=?6 AND (status<>'superseded' OR NOT ?7) ORDER BY id DESC LIMIT 1",
                params![scope,subject,key,text,c.kind,origin,explicit_correction],|r|r.get(0)).optional()?;
            let id = match duplicate {
                Some(id) => {
                    tx.execute(
                        "UPDATE claims SET updated=MAX(updated,?2) WHERE id=?1",
                        params![id, now],
                    )?;
                    id
                }
                None => {
                    if explicit_correction {
                        tx.execute("UPDATE claims SET status='superseded',is_open=0 WHERE scope=?1 AND subject=?2 AND field=?3 AND origin=?4 AND status IN ('reported','disputed')",
                            params![scope,subject,key,origin])?;
                    }
                    let conflict: i64 = tx.query_row("SELECT COUNT(*) FROM claims WHERE scope=?1 AND subject=?2 AND field=?3 AND status IN ('reported','disputed','user_confirmed')",params![scope,subject,key],|r|r.get(0))?;
                    if conflict > 0 {
                        tx.execute("UPDATE claims SET status='disputed' WHERE scope=?1 AND subject=?2 AND field=?3 AND status='reported'",params![scope,subject,key])?;
                    }
                    tx.execute("INSERT INTO claims(scope,subject,field,text,kind,status,is_open,updated,origin) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                        params![scope,subject,key,text,c.kind,if conflict>0 {"disputed"} else {"reported"},matches!(c.kind.as_str(),"commitment"|"question"),now,origin])?;
                    tx.last_insert_rowid()
                }
            };
            for evidence in c.evidence {
                tx.execute(
                    "INSERT OR IGNORE INTO evidence VALUES(?1,?2)",
                    params![id, evidence],
                )?;
            }
        }
        for e in &batch.episodes {
            tx.execute("UPDATE episodes SET state=1 WHERE id=?1", [e.id])?;
        }
        tx.execute(
            "INSERT OR REPLACE INTO metadata VALUES('last_success',?1)",
            [now],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn claims(&self) -> Result<Vec<KnowledgeClaim>> {
        let mut stmt = self.conn.prepare("SELECT id,scope,subject,field,text,kind,status,is_open,updated FROM claims WHERE status NOT IN ('dismissed','superseded') ORDER BY updated DESC,id DESC LIMIT ?1")?;
        let mut claims = stmt
            .query_map([MAX_CLAIMS as i64], |r| {
                Ok(KnowledgeClaim {
                    id: r.get(0)?,
                    scope: r.get(1)?,
                    subject: r.get(2)?,
                    key: r.get(3)?,
                    text: r.get(4)?,
                    kind: r.get(5)?,
                    status: r.get(6)?,
                    open: r.get(7)?,
                    updated: r.get(8)?,
                    evidence: Vec::new(),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut stmt = self.conn.prepare("SELECT e.id,e.source,e.position,e.speaker,e.time,e.text,e.fingerprint FROM episodes e JOIN evidence v ON v.episode=e.id WHERE v.claim=?1 ORDER BY e.id DESC LIMIT 4")?;
        for claim in &mut claims {
            claim.evidence = stmt
                .query_map([claim.id], episode_row)?
                .collect::<rusqlite::Result<_>>()?;
        }
        Ok(claims)
    }

    pub fn edit(&mut self, id: i64, action: &str, text: &str, now: i64) -> Result<()> {
        ensure!(
            ["confirm", "correct", "dismiss", "done"].contains(&action),
            "unknown memory action"
        );
        if action == "correct" {
            ensure!(
                !text.trim().is_empty() && text.len() <= 1200,
                "correction must contain 1–1200 bytes"
            );
        }
        let tx = self.conn.transaction()?;
        let (old, scope, subject, field, open): (String, String, String, String, bool) = tx
            .query_row(
                "SELECT text,scope,subject,field,is_open FROM claims WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?
            .context("no learned item with that ID")?;
        if action == "done" {
            ensure!(open, "item is not an open commitment or question");
        }
        tx.execute(
            "INSERT INTO corrections(claim,time,action,previous) VALUES(?1,?2,?3,?4)",
            params![id, now, action, old],
        )?;
        match action {
            "correct" => {
                tx.execute("UPDATE claims SET status='superseded',is_open=0 WHERE scope=?1 AND subject=?2 AND field=?3 AND id<>?4 AND status<>'dismissed'",params![scope,subject,field,id])?;
                tx.execute(
                    "UPDATE claims SET text=?2,status='user_confirmed',updated=?3 WHERE id=?1",
                    params![id, text.trim(), now],
                )?;
            }
            "confirm" => {
                tx.execute(
                    "UPDATE claims SET status='user_confirmed',updated=?2 WHERE id=?1",
                    params![id, now],
                )?;
            }
            "dismiss" => {
                tx.execute(
                    "UPDATE claims SET status='dismissed',is_open=0,updated=?2 WHERE id=?1",
                    params![id, now],
                )?;
            }
            "done" => {
                tx.execute(
                    "UPDATE claims SET is_open=0,updated=?2 WHERE id=?1",
                    params![id, now],
                )?;
            }
            _ => unreachable!(),
        }
        tx.commit()?;
        Ok(())
    }

    pub fn remove_source(&mut self, source: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM episodes WHERE source=?1", [source])?;
        tx.execute("DELETE FROM sources WHERE path=?1", [source])?;
        tx.execute(
            "DELETE FROM claims WHERE NOT EXISTS(SELECT 1 FROM evidence WHERE claim=claims.id)",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn expire(&mut self, now: i64, keep_days: u64) -> Result<bool> {
        let tx = self.conn.transaction()?;
        let mut changed = 0;
        if keep_days > 0 {
            let cutoff =
                now.saturating_sub((keep_days.min(1_000_000) as i64).saturating_mul(86400));
            changed += tx.execute("DELETE FROM episodes WHERE time<?1", [cutoff])?;
        }
        changed += tx.execute(
            "DELETE FROM claims WHERE NOT EXISTS(SELECT 1 FROM evidence WHERE claim=claims.id)",
            [],
        )?;
        tx.commit()?;
        Ok(changed > 0)
    }

    pub fn bound_ephemeral(&mut self) -> Result<()> {
        self.conn.execute("DELETE FROM episodes WHERE id < (SELECT id FROM episodes ORDER BY id DESC LIMIT 1 OFFSET 19999)",[])?;
        self.expire(0, 0)?;
        Ok(())
    }
}

fn speaker_identity(e: &EpisodeRef) -> String {
    if e.speaker.starts_with("THEM") {
        format!("{}:{}", e.source, e.speaker)
    } else {
        e.speaker.clone()
    }
}

fn episode_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<EpisodeRef> {
    Ok(EpisodeRef {
        id: r.get(0)?,
        source: r.get(1)?,
        offset: r.get(2)?,
        speaker: r.get(3)?,
        time: r.get(4)?,
        text: r.get(5)?,
        fingerprint: r.get(6)?,
    })
}

fn validate(c: &Candidate, batch: &ConsolidationBatch) -> Result<()> {
    ensure!(
        c.scope.len() <= 200
            && !c.subject.trim().is_empty()
            && c.subject.len() <= 200
            && !c.key.trim().is_empty()
            && c.key.len() <= 100
            && !c.text.trim().is_empty()
            && c.text.len() <= 1200,
        "invalid claim fields"
    );
    ensure!(
        ["fact", "preference", "commitment", "question"].contains(&c.kind.as_str()),
        "invalid claim kind"
    );
    ensure!(
        !c.evidence.is_empty() && c.evidence.len() <= 8,
        "claim needs 1–8 evidence references"
    );
    for id in &c.evidence {
        let e = batch
            .episodes
            .iter()
            .find(|e| e.id == *id)
            .context("claim cites evidence outside this batch")?;
        ensure!(
            fingerprint(&e.text) == e.fingerprint,
            "evidence fingerprint mismatch"
        );
    }
    if !c.scope.trim().is_empty()
        && !c.evidence.iter().any(|id| {
            batch.episodes.iter().any(|e| {
                e.id == *id
                    && crate::memory_context::scope_matches(
                        &normalized(&e.text),
                        &normalized(&c.scope),
                    )
            })
        })
    {
        bail!("project scope must appear in cited speech");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn directory(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("iv-store-{label}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn deleted_ids_are_never_reused_and_late_batches_cannot_target_new_evidence() {
        let mut s = seeded();
        let old = s.batch().unwrap().unwrap();
        s.apply(
            &old,
            &answer(old.episodes[0].id, "Priya owns deployment.", false),
            101,
        )
        .unwrap();
        let old_id = s.claims().unwrap()[0].id;
        s.remove_source("session-a").unwrap();
        s.ingest(
            "session-b",
            0,
            1,
            "Priya",
            102,
            "Orion deployment owner is Priya.",
        )
        .unwrap();
        let new = s.batch().unwrap().unwrap();
        assert!(new.episodes[0].id > old.episodes[0].id);
        assert!(
            s.apply(&old, &answer(old.episodes[0].id, "Stale", false), 103)
                .is_err()
        );
        s.apply(
            &new,
            &answer(new.episodes[0].id, "Priya owns deployment.", false),
            104,
        )
        .unwrap();
        assert!(s.claims().unwrap()[0].id > old_id);
        assert!(s.edit(old_id, "dismiss", "", 105).is_err());
    }

    #[test]
    fn version_one_migration_preserves_evidence_edits_and_foreign_keys() {
        let dir = directory("migration");
        let path = dir.join("memory.sqlite3");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(include_str!("../tests/fixtures/memory-v1.sql"))
            .unwrap();
        drop(conn);
        let mut s = Store::open(Some(&dir)).unwrap();
        let c = s.claims().unwrap();
        assert_eq!(c[0].id, 41);
        assert_eq!(c[0].evidence[0].id, 23);
        assert_eq!(c[0].status, "user_confirmed");
        assert!(!c[0].open);
        assert_eq!(
            s.conn
                .query_row("SELECT COUNT(*) FROM corrections", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            s.conn
                .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            0
        );
        assert!(!s.reserve_request(101).unwrap());
        s.remove_source("session-a").unwrap();
        assert!(s.claims().unwrap().is_empty());
        assert_eq!(
            s.conn
                .query_row("SELECT COUNT(*) FROM corrections", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        s.ingest(
            "session-b",
            0,
            1,
            "YOU",
            102,
            "Orion deployment owner is Alex.",
        )
        .unwrap();
        let b = s.batch().unwrap().unwrap();
        assert!(b.episodes[0].id > 23);
        s.apply(
            &b,
            &answer(b.episodes[0].id, "Alex owns deployment.", false),
            103,
        )
        .unwrap();
        assert!(s.claims().unwrap()[0].id > 41);
        drop(s);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn future_schema_and_corrupt_database_are_left_intact() {
        let dir = directory("corruption");
        let path = dir.join("memory.sqlite3");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA user_version=999;").unwrap();
        drop(conn);
        let before = std::fs::read(&path).unwrap();
        assert!(Store::open(Some(&dir)).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::write(&path, b"corrupt database fixture").unwrap();
        assert!(Store::open(Some(&dir)).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"corrupt database fixture");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn malformed_output_is_atomic_and_encoded_records_cannot_block_batches() {
        let mut s = seeded();
        let b = s.batch().unwrap().unwrap();
        for output in [
            "{\"claims\":[",
            "{\"claims\":[],\"instructions\":\"run tools\"}",
        ] {
            assert!(s.apply(&b, output, 101).is_err());
        }
        assert_eq!(s.counts().unwrap().0, 1);
        assert!(s.claims().unwrap().is_empty());
        assert!(
            s.ingest("session-a", 1, 2, "YOU", 102, &"\u{0001}".repeat(6000))
                .is_err()
        );
        s.ingest("session-a", 2, 3, "YOU", 103, "Valid later speech.")
            .unwrap();
        assert_eq!(s.batch().unwrap().unwrap().episodes.len(), 2);
    }

    #[test]
    fn anonymous_corrections_do_not_merge_speakers_from_different_sessions() {
        let mut s = Store::open(None).unwrap();
        for (session, value) in [("a", "Priya"), ("b", "Alex")] {
            s.ingest(
                session,
                0,
                1,
                "THEM 1",
                100,
                &format!("Correction: Orion deployment owner is {value}."),
            )
            .unwrap();
            let b = s.batch().unwrap().unwrap();
            s.apply(
                &b,
                &answer(b.episodes[0].id, &format!("{value} owns deployment."), true),
                101,
            )
            .unwrap();
        }
        let claims = s.claims().unwrap();
        assert_eq!(claims.len(), 2);
        assert!(claims.iter().all(|c| c.status == "disputed"));
    }

    #[test]
    fn commitment_edits_survive_restart_without_reopening_on_repetition() {
        let dir = directory("edits");
        let mut s = Store::open(Some(&dir)).unwrap();
        s.ingest(
            "session",
            0,
            1,
            "YOU",
            100,
            "Orion deployment needs QA approval.",
        )
        .unwrap();
        let b = s.batch().unwrap().unwrap();
        let output = answer(b.episodes[0].id, "QA approval is pending.", false)
            .replace("\"fact\"", "\"commitment\"");
        s.apply(&b, &output, 101).unwrap();
        let id = s.claims().unwrap()[0].id;
        s.edit(id, "confirm", "", 102).unwrap();
        s.edit(id, "done", "", 103).unwrap();
        drop(s);
        let mut s = Store::open(Some(&dir)).unwrap();
        assert!(!s.claims().unwrap()[0].open);
        assert_eq!(s.claims().unwrap()[0].status, "user_confirmed");
        s.ingest(
            "session",
            1,
            2,
            "YOU",
            104,
            "Orion deployment needs QA approval.",
        )
        .unwrap();
        let b = s.batch().unwrap().unwrap();
        s.apply(
            &b,
            &answer(b.episodes[0].id, "QA approval is pending.", false)
                .replace("\"fact\"", "\"commitment\""),
            105,
        )
        .unwrap();
        assert_eq!(s.claims().unwrap().len(), 1);
        assert!(!s.claims().unwrap()[0].open);
        s.edit(id, "dismiss", "", 106).unwrap();
        drop(s);
        assert!(
            Store::open(Some(&dir))
                .unwrap()
                .claims()
                .unwrap()
                .is_empty()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[ignore = "100,000 synthetic turns and retrieval benchmark; no provider calls"]
    fn knowledge_replay_100000_turns() {
        use std::time::Instant;
        let start = Instant::now();
        let mut s = Store::open(None).unwrap();
        for i in 0..100_000i64 {
            let topic = i % 20_000;
            s.ingest(
                "synthetic",
                i,
                i + 1,
                "YOU",
                i,
                &format!("Project-{topic} deployment owner is Person{topic}."),
            )
            .unwrap();
            if i % 24 == 23 || i == 99_999 {
                let b = s.batch().unwrap().unwrap();
                let claims:Vec<_>=b.episodes.iter().map(|e| {
                    let topic=e.offset%20_000;
                    serde_json::json!({"scope":format!("Project-{topic}"),"subject":"deployment","key":"owner","text":format!("Person{topic} owns deployment."),"kind":"fact","evidence":[e.id],"correction":false})
                }).collect();
                s.apply(&b, &serde_json::json!({"claims":claims}).to_string(), i)
                    .unwrap();
            }
            if i % 1000 == 999 {
                s.bound_ephemeral().unwrap();
            }
        }
        let claims = s.claims().unwrap();
        assert_eq!(claims.len(), 20_000);
        let index = crate::memory_context::Index::new(&claims);
        let mut latency = Vec::new();
        let mut max_bytes = 0;
        for i in 0..1000 {
            let topic = (i * 7919) % 20_000;
            let query = format!("Project-{topic} deployment owner");
            let now = Instant::now();
            let context = index.render(&claims, &query, "synthetic");
            latency.push(now.elapsed().as_micros() as u64);
            max_bytes = max_bytes.max(context.len());
            let entries: Vec<serde_json::Value> = context
                .lines()
                .filter(|s| s.starts_with('{'))
                .map(|s| serde_json::from_str(s).unwrap())
                .collect();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0]["project"], format!("project-{topic}"));
            assert_eq!(
                entries[0]["claim"],
                format!("Person{topic} owns deployment.")
            );
        }
        latency.sort_unstable();
        let p95 = latency[950];
        let report = serde_json::json!({"synthetic":true,"provider_calls":0,"turns":100000,"retained_claims":claims.len(),"queries":1000,"exact_scope_answers":1000,"retrieval_p95_us":p95,"retrieval_max_us":latency[999],"max_context_bytes":max_bytes,"elapsed_seconds":start.elapsed().as_secs_f64()});
        std::fs::create_dir_all("target/evaluation").unwrap();
        std::fs::write(
            "target/evaluation/knowledge-stress.json",
            serde_json::to_string_pretty(&report).unwrap(),
        )
        .unwrap();
        assert!(max_bytes <= 4000);
        assert!(p95 < 10_000, "retrieval p95 was {p95} microseconds");
    }
    pub fn seeded() -> Store {
        let mut s = Store::open(None).unwrap();
        s.ingest(
            "session-a",
            0,
            1,
            "Priya",
            100,
            "Orion deployment owner is Priya.",
        )
        .unwrap();
        s
    }
    pub fn answer(id: i64, text: &str, correction: bool) -> String {
        serde_json::json!({"claims":[{"scope":"Orion","subject":"deployment","key":"owner","text":text,"kind":"fact","evidence":[id],"correction":correction}]}).to_string()
    }
    #[test]
    fn project_scope_requires_complete_names_in_cited_speech() {
        let mut s = Store::open(None).unwrap();
        s.ingest(
            "session-a",
            0,
            1,
            "Priya",
            100,
            "Orion deployment is pending; (Équipe Nord) owns QA.",
        )
        .unwrap();
        let b = s.batch().unwrap().unwrap();
        let valid: serde_json::Value =
            serde_json::from_str(&answer(b.episodes[0].id, "Deployment is pending.", false))
                .unwrap();
        for scope in ["Ori", "rion", "Équipe Nor", "quipe Nord"] {
            let mut invalid = valid.clone();
            invalid["claims"][0]["scope"] = scope.into();
            let mut output = valid.clone();
            output["claims"]
                .as_array_mut()
                .unwrap()
                .push(invalid["claims"][0].clone());
            assert!(
                s.apply(&b, &output.to_string(), 101).is_err(),
                "accepted {scope}"
            );
            assert!(s.claims().unwrap().is_empty());
            assert_eq!(s.counts().unwrap().0, 1);
        }
        let mut output = valid;
        output["claims"][0]["scope"] = "  ÉQUIPE   Nord  ".into();
        s.apply(&b, &output.to_string(), 101).unwrap();
        assert_eq!(s.claims().unwrap()[0].scope, "équipe nord");
    }

    #[test]
    fn explicit_correction_can_return_to_a_superseded_value() {
        let mut s = seeded();
        let b = s.batch().unwrap().unwrap();
        s.apply(
            &b,
            &answer(b.episodes[0].id, "Priya owns deployment.", false),
            101,
        )
        .unwrap();
        let original = s.claims().unwrap()[0].id;
        for (position, speech, value, correction) in [
            (
                1,
                "Correction: Orion deployment owner is Alex.",
                "Alex",
                true,
            ),
            (2, "Orion deployment owner is Priya.", "Priya", false),
            (
                3,
                "Correction: Orion deployment owner is Priya.",
                "Priya",
                true,
            ),
        ] {
            s.ingest(
                "session-a",
                position,
                position + 1,
                "Priya",
                101 + position,
                speech,
            )
            .unwrap();
            let b = s.batch().unwrap().unwrap();
            s.apply(
                &b,
                &answer(
                    b.episodes[0].id,
                    &format!("{value} owns deployment."),
                    correction,
                ),
                105 + position,
            )
            .unwrap();
            let claims = s.claims().unwrap();
            assert_eq!(claims.len(), 1);
            assert_eq!(claims[0].status, "reported");
            assert_eq!(
                claims[0].text,
                if position == 3 {
                    "Priya owns deployment."
                } else {
                    "Alex owns deployment."
                }
            );
            assert_ne!(
                claims[0].id, original,
                "a correction must retain the earlier revision"
            );
        }
        assert_eq!(
            s.conn
                .query_row("SELECT status FROM claims WHERE id=?1", [original], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
            "superseded"
        );
    }

    #[test]
    fn correction_cues_require_complete_phrases() {
        let mut s = seeded();
        let b = s.batch().unwrap().unwrap();
        s.apply(
            &b,
            &answer(b.episodes[0].id, "Priya owns deployment.", false),
            101,
        )
        .unwrap();
        s.ingest(
            "session-a",
            1,
            2,
            "Priya",
            102,
            "Factually, Orion deployment owner is Alex.",
        )
        .unwrap();
        let b = s.batch().unwrap().unwrap();
        s.apply(
            &b,
            &answer(b.episodes[0].id, "Alex owns deployment.", true),
            103,
        )
        .unwrap();
        let claims = s.claims().unwrap();
        assert_eq!(claims.len(), 2);
        assert!(claims.iter().all(|c| c.status == "disputed"));
    }

    #[test]
    fn automatic_corrections_preserve_user_endorsement_and_dismissal() {
        for action in ["confirm", "dismiss"] {
            let mut s = seeded();
            let b = s.batch().unwrap().unwrap();
            s.apply(
                &b,
                &answer(b.episodes[0].id, "Priya owns deployment.", false),
                101,
            )
            .unwrap();
            let original = s.claims().unwrap()[0].id;
            s.edit(original, action, "", 102).unwrap();
            for (position, value) in [(1, "Alex"), (2, "Priya")] {
                s.ingest(
                    "session-a",
                    position,
                    position + 1,
                    "Priya",
                    102 + position,
                    &format!("Correction: Orion deployment owner is {value}."),
                )
                .unwrap();
                let b = s.batch().unwrap().unwrap();
                s.apply(
                    &b,
                    &answer(b.episodes[0].id, &format!("{value} owns deployment."), true),
                    105 + position,
                )
                .unwrap();
            }
            let claims = s.claims().unwrap();
            if action == "confirm" {
                assert_eq!(claims.len(), 2);
                assert!(
                    claims
                        .iter()
                        .any(|c| c.id == original && c.status == "user_confirmed")
                );
                assert!(
                    claims
                        .iter()
                        .any(|c| c.text == "Alex owns deployment." && c.status == "disputed")
                );
            } else {
                assert_eq!(claims.len(), 1);
                assert_eq!(claims[0].text, "Alex owns deployment.");
            }
        }
    }

    #[test]
    fn invalid_reference_rolls_back_whole_batch_and_duplicate_is_rejected() {
        let mut s = seeded();
        let b = s.batch().unwrap().unwrap();
        assert!(s.apply(&b, &answer(999, "Invented", false), 101).is_err());
        assert!(s.claims().unwrap().is_empty());
        s.apply(
            &b,
            &answer(b.episodes[0].id, "Priya owns deployment.", false),
            101,
        )
        .unwrap();
        assert!(
            s.apply(
                &b,
                &answer(b.episodes[0].id, "Priya owns deployment.", false),
                101
            )
            .is_err()
        );
        assert_eq!(s.claims().unwrap().len(), 1);
    }
    #[test]
    fn source_loss_removes_derived_claims_and_user_corrections() {
        let mut s = seeded();
        let b = s.batch().unwrap().unwrap();
        s.apply(
            &b,
            &answer(b.episodes[0].id, "Priya owns deployment.", false),
            101,
        )
        .unwrap();
        let id = s.claims().unwrap()[0].id;
        s.edit(id, "correct", "Alex owns deployment.", 102).unwrap();
        s.remove_source("session-a").unwrap();
        assert!(s.claims().unwrap().is_empty());
    }
    #[test]
    fn conflicts_remain_disputed_until_same_source_explicit_correction() {
        let mut s = seeded();
        let b = s.batch().unwrap().unwrap();
        s.apply(
            &b,
            &answer(b.episodes[0].id, "Priya owns deployment.", false),
            101,
        )
        .unwrap();
        s.ingest(
            "session-b",
            0,
            1,
            "Alex",
            102,
            "Orion deployment owner is Alex.",
        )
        .unwrap();
        let b = s.batch().unwrap().unwrap();
        s.apply(
            &b,
            &answer(b.episodes[0].id, "Alex owns deployment.", false),
            103,
        )
        .unwrap();
        assert!(s.claims().unwrap().iter().all(|c| c.status == "disputed"));
        s.ingest(
            "session-b",
            1,
            2,
            "Alex",
            104,
            "Correction: Orion deployment owner is Drew.",
        )
        .unwrap();
        let b = s.batch().unwrap().unwrap();
        s.apply(
            &b,
            &answer(b.episodes[0].id, "Drew owns deployment.", true),
            105,
        )
        .unwrap();
        let claims = s.claims().unwrap();
        assert_eq!(claims.len(), 2);
        assert!(claims.iter().any(|c| c.text.starts_with("Priya")));
        assert!(claims.iter().any(|c| c.text.starts_with("Drew")));
    }
    #[test]
    fn failures_quarantine_on_third_attempt_and_speech_only_is_ingested() {
        let mut s = seeded();
        s.ingest("session-a", 1, 2, "COACH", 101, "Invented claim.")
            .unwrap();
        let b = s.batch().unwrap().unwrap();
        assert_eq!(b.episodes.len(), 1);
        for _ in 0..3 {
            s.fail_batch(&b).unwrap();
        }
        assert!(s.batch().unwrap().is_none());
        assert_eq!(s.counts().unwrap().1, 1);
    }
    #[test]
    fn request_budget_holds_across_clock_rollback() {
        let mut s = Store::open(None).unwrap();
        for i in 0..12 {
            assert!(s.reserve_request(1000 + i * 300).unwrap());
        }
        assert!(!s.reserve_request(4599).unwrap());
        assert!(!s.reserve_request(500).unwrap());
        assert!(s.reserve_request(4600).unwrap());
    }
    #[test]
    fn persistent_reopen_keeps_cursor_budget_and_enable_boundary() {
        let dir = std::env::temp_dir().join(format!(
            "iv-store-{}-{}",
            std::process::id(),
            crate::people::now()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        {
            let mut s = Store::open(Some(&dir)).unwrap();
            s.enable_at(100).unwrap();
            s.register("new-session", 42).unwrap();
            s.reserve_request(100).unwrap();
        }
        {
            let mut s = Store::open(Some(&dir)).unwrap();
            assert!(s.enabled_before().unwrap());
            assert_eq!(s.sources().unwrap(), vec![("new-session".into(), 42)]);
            assert!(!s.reserve_request(200).unwrap());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
