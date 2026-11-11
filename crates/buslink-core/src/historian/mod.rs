//! SQLite historian (PRD R20, architecture D9). WAL mode, batched inserts,
//! per-minute rollups from GOOD samples only, an append-only command audit
//! table. Non-GOOD samples are stored with a NULL value so trends break at
//! gaps; the raw registers are always kept losslessly.

pub mod batch;

use crate::command::audit::{AuditError, AuditRecord, AuditSink};
use crate::quality::{Quality, Sample};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

pub const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS sessions (
  id TEXT PRIMARY KEY,
  started_ms INTEGER NOT NULL,
  app_version TEXT NOT NULL,
  platform TEXT NOT NULL,
  logging_interval_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS devices (
  id INTEGER PRIMARY KEY,
  session TEXT NOT NULL REFERENCES sessions(id),
  device_key TEXT NOT NULL,
  name TEXT NOT NULL,
  profile_id TEXT NOT NULL,
  profile_version TEXT NOT NULL,
  unit_id INTEGER NOT NULL,
  link TEXT NOT NULL,
  UNIQUE(session, device_key)
);
CREATE TABLE IF NOT EXISTS tags (
  id INTEGER PRIMARY KEY,
  device INTEGER NOT NULL REFERENCES devices(id),
  key TEXT NOT NULL,
  unit TEXT,
  UNIQUE(device, key)
);
CREATE TABLE IF NOT EXISTS samples (
  tag INTEGER NOT NULL REFERENCES tags(id),
  ts_ms INTEGER NOT NULL,
  value_ts_ms INTEGER,
  ts_src_ms INTEGER,
  seq INTEGER NOT NULL,
  raw BLOB,
  raw_value TEXT,
  value REAL,
  text TEXT,
  quality INTEGER NOT NULL,
  exception INTEGER,
  age_ms INTEGER
);
CREATE INDEX IF NOT EXISTS samples_tag_ts ON samples(tag, ts_ms);
CREATE TABLE IF NOT EXISTS events (
  id INTEGER PRIMARY KEY,
  session TEXT,
  ts_ms INTEGER NOT NULL,
  kind TEXT NOT NULL,
  device TEXT,
  tag TEXT,
  severity TEXT NOT NULL,
  text TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS command_audit (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  ts_ms INTEGER NOT NULL,
  session_id TEXT NOT NULL,
  user TEXT NOT NULL,
  role TEXT NOT NULL,
  event TEXT,
  device_id TEXT NOT NULL,
  profile_id TEXT NOT NULL,
  profile_version TEXT NOT NULL,
  unit_id INTEGER,
  function INTEGER,
  address INTEGER,
  register_id TEXT,
  command_id TEXT,
  preview_id INTEGER,
  old_value REAL,
  requested_value REAL,
  raw_value_hex TEXT,
  request_hex TEXT,
  response_hex TEXT,
  exception INTEGER,
  ack TEXT,
  readback TEXT,
  readback_value REAL,
  note TEXT
);
CREATE TRIGGER IF NOT EXISTS command_audit_no_update BEFORE UPDATE ON command_audit
  BEGIN SELECT RAISE(ABORT, 'command_audit is append-only'); END;
CREATE TRIGGER IF NOT EXISTS command_audit_no_delete BEFORE DELETE ON command_audit
  BEGIN SELECT RAISE(ABORT, 'command_audit is append-only'); END;
CREATE TABLE IF NOT EXISTS outbox (
  seq INTEGER PRIMARY KEY,
  topic TEXT NOT NULL,
  payload BLOB NOT NULL,
  tries INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS samples_1m (
  tag INTEGER NOT NULL REFERENCES tags(id),
  minute INTEGER NOT NULL,
  n INTEGER NOT NULL,
  min REAL NOT NULL,
  max REAL NOT NULL,
  sum REAL NOT NULL,
  PRIMARY KEY(tag, minute)
);
"#;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum HistorianError {
    #[error("database error: {0}")]
    Db(String),
    #[error("device '{0}' is not registered in the current session")]
    UnknownDevice(String),
    #[error("no session has been started")]
    NoSession,
    #[error("value out of range for storage: {0}")]
    Range(&'static str),
}

impl From<rusqlite::Error> for HistorianError {
    fn from(e: rusqlite::Error) -> Self {
        HistorianError::Db(e.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SessionInfo {
    pub id: String,
    pub started_ms: i64,
    pub app_version: String,
    pub platform: String,
    pub logging_interval_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DeviceRow {
    pub session: String,
    pub device_id: String,
    pub name: String,
    pub profile_id: String,
    pub profile_version: String,
    pub unit_id: u8,
    pub link: String,
}

/// A sample as stored. `value` is NULL (None) for every non-GOOD sample.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StoredSample {
    pub session: String,
    pub device_id: String,
    pub tag_id: String,
    pub unit: Option<String>,
    pub profile_id: String,
    pub profile_version: String,
    pub ts_ms: i64,
    pub value_ts_ms: Option<i64>,
    pub ts_src_ms: Option<i64>,
    pub seq: u64,
    pub raw: Vec<u16>,
    /// Exact decoded raw value (64-bit integers as digits).
    pub raw_value: Option<String>,
    pub value: Option<f64>,
    pub text: Option<String>,
    pub quality: Quality,
    pub age_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EventRow {
    pub session: Option<String>,
    pub ts_ms: i64,
    pub kind: String,
    pub device: Option<String>,
    pub tag: Option<String>,
    pub severity: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Rollup {
    pub minute_start_ms: i64,
    pub n: u64,
    pub min: f64,
    pub max: f64,
    pub sum: f64,
}

impl Rollup {
    pub fn mean(&self) -> f64 {
        if self.n == 0 {
            f64::NAN
        } else {
            self.sum / self.n as f64
        }
    }
}

pub struct Historian {
    conn: Connection,
    session: Option<String>,
    devices: HashMap<String, i64>,
    tags: HashMap<(i64, String), i64>,
}

fn regs_to_blob(regs: &[u16]) -> Vec<u8> {
    regs.iter().flat_map(|r| r.to_be_bytes()).collect()
}

fn blob_to_regs(b: &[u8]) -> Vec<u16> {
    b.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect()
}

fn to_i64(v: u64, what: &'static str) -> Result<i64, HistorianError> {
    i64::try_from(v).map_err(|_| HistorianError::Range(what))
}

impl Historian {
    pub fn open(path: &Path) -> Result<Self, HistorianError> {
        let conn = Connection::open(path)?;
        Self::init(conn, true)
    }

    pub fn open_in_memory() -> Result<Self, HistorianError> {
        Self::init(Connection::open_in_memory()?, false)
    }

    fn init(conn: Connection, wal: bool) -> Result<Self, HistorianError> {
        if wal {
            let mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
            if !mode.eq_ignore_ascii_case("wal") {
                return Err(HistorianError::Db(format!("journal_mode is {mode}, expected wal")));
            }
            conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
        }
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(SCHEMA)?;
        conn.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', ?1)",
            params![SCHEMA_VERSION.to_string()],
        )?;
        Ok(Self {
            conn,
            session: None,
            devices: HashMap::new(),
            tags: HashMap::new(),
        })
    }

    pub fn journal_mode(&self) -> Result<String, HistorianError> {
        Ok(self.conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?)
    }

    pub fn begin_session(
        &mut self,
        id: &str,
        started_ms: i64,
        app_version: &str,
        platform: &str,
        logging_interval_ms: u64,
    ) -> Result<(), HistorianError> {
        self.conn.execute(
            "INSERT INTO sessions(id, started_ms, app_version, platform, logging_interval_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, started_ms, app_version, platform, to_i64(logging_interval_ms, "logging interval")?],
        )?;
        self.session = Some(id.to_owned());
        self.devices.clear();
        self.tags.clear();
        Ok(())
    }

    pub fn session(&self) -> Option<&str> {
        self.session.as_deref()
    }

    pub fn register_device(&mut self, d: &DeviceRow) -> Result<(), HistorianError> {
        let session = self.session.clone().ok_or(HistorianError::NoSession)?;
        self.conn.execute(
            "INSERT INTO devices(session, device_key, name, profile_id, profile_version, unit_id, link)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                session,
                d.device_id,
                d.name,
                d.profile_id,
                d.profile_version,
                d.unit_id,
                d.link
            ],
        )?;
        self.devices.insert(d.device_id.clone(), self.conn.last_insert_rowid());
        Ok(())
    }

    fn tag_row(&mut self, device_key: &str, tag: &str, unit: Option<&str>) -> Result<i64, HistorianError> {
        let dev = *self
            .devices
            .get(device_key)
            .ok_or_else(|| HistorianError::UnknownDevice(device_key.to_owned()))?;
        if let Some(id) = self.tags.get(&(dev, tag.to_owned())) {
            return Ok(*id);
        }
        self.conn.execute(
            "INSERT OR IGNORE INTO tags(device, key, unit) VALUES (?1, ?2, ?3)",
            params![dev, tag, unit],
        )?;
        let id: i64 = self.conn.query_row(
            "SELECT id FROM tags WHERE device = ?1 AND key = ?2",
            params![dev, tag],
            |r| r.get(0),
        )?;
        self.tags.insert((dev, tag.to_owned()), id);
        Ok(id)
    }

    /// Insert a batch in one transaction.
    pub fn insert_samples(&mut self, samples: &[Sample]) -> Result<usize, HistorianError> {
        let mut rows = Vec::with_capacity(samples.len());
        for s in samples {
            let tag = self.tag_row(&s.device_id, &s.tag_id, s.unit.as_deref())?;
            rows.push((tag, s));
        }
        let tx = self.conn.transaction()?;
        {
            let mut st = tx.prepare(
                "INSERT INTO samples(tag, ts_ms, value_ts_ms, ts_src_ms, seq, raw, raw_value, value, text, quality, exception, age_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )?;
            for (tag, s) in &rows {
                let good = s.quality.is_good();
                let v = s.value.as_ref().filter(|_| good);
                let age = s.age_ms.map(|a| to_i64(a, "age")).transpose()?;
                st.execute(params![
                    tag,
                    s.ts_ms,
                    s.value_ts_ms,
                    s.ts_source_ms,
                    to_i64(s.seq, "seq")?,
                    s.raw.as_deref().map(regs_to_blob),
                    v.map(|v| v.raw.display()),
                    v.and_then(|v| v.engineering).filter(|x| x.is_finite()),
                    v.and_then(|v| v.text.clone()),
                    s.quality.code(),
                    s.quality.exception_code(),
                    age,
                ])?;
            }
        }
        tx.commit()?;
        Ok(rows.len())
    }

    pub fn add_event(&mut self, e: &EventRow) -> Result<(), HistorianError> {
        self.conn.execute(
            "INSERT INTO events(session, ts_ms, kind, device, tag, severity, text) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![e.session, e.ts_ms, e.kind, e.device, e.tag, e.severity, e.text],
        )?;
        Ok(())
    }

    pub fn sessions(&self) -> Result<Vec<SessionInfo>, HistorianError> {
        let mut st = self.conn.prepare(
            "SELECT id, started_ms, app_version, platform, logging_interval_ms FROM sessions ORDER BY started_ms",
        )?;
        let rows = st.query_map([], |r| {
            Ok(SessionInfo {
                id: r.get(0)?,
                started_ms: r.get(1)?,
                app_version: r.get(2)?,
                platform: r.get(3)?,
                logging_interval_ms: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn devices(&self, session: &str) -> Result<Vec<DeviceRow>, HistorianError> {
        let mut st = self.conn.prepare(
            "SELECT session, device_key, name, profile_id, profile_version, unit_id, link FROM devices WHERE session = ?1 ORDER BY id",
        )?;
        let rows = st.query_map(params![session], |r| {
            Ok(DeviceRow {
                session: r.get(0)?,
                device_id: r.get(1)?,
                name: r.get(2)?,
                profile_id: r.get(3)?,
                profile_version: r.get(4)?,
                unit_id: r.get(5)?,
                link: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Samples of a session (optionally one device/tag), time-ordered, in `[from, to)`.
    pub fn samples(
        &self,
        session: &str,
        device: Option<&str>,
        tag: Option<&str>,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<Vec<StoredSample>, HistorianError> {
        let mut st = self.conn.prepare(
            "SELECT d.session, d.device_key, t.key, t.unit, d.profile_id, d.profile_version,
                    s.ts_ms, s.value_ts_ms, s.ts_src_ms, s.seq, s.raw, s.raw_value, s.value, s.text, s.quality, s.exception, s.age_ms
             FROM samples s JOIN tags t ON t.id = s.tag JOIN devices d ON d.id = t.device
             WHERE d.session = ?1 AND (?2 IS NULL OR d.device_key = ?2) AND (?3 IS NULL OR t.key = ?3)
               AND s.ts_ms >= ?4 AND s.ts_ms < ?5
             ORDER BY s.ts_ms, s.seq",
        )?;
        let rows = st.query_map(params![session, device, tag, from_ms, to_ms], |r| {
            let q: u8 = r.get(14)?;
            let exc: Option<u8> = r.get(15)?;
            let raw: Option<Vec<u8>> = r.get(10)?;
            let seq: i64 = r.get(9)?;
            let age: Option<i64> = r.get(16)?;
            Ok(StoredSample {
                session: r.get(0)?,
                device_id: r.get(1)?,
                tag_id: r.get(2)?,
                unit: r.get(3)?,
                profile_id: r.get(4)?,
                profile_version: r.get(5)?,
                ts_ms: r.get(6)?,
                value_ts_ms: r.get(7)?,
                ts_src_ms: r.get(8)?,
                seq: u64::try_from(seq).unwrap_or(0),
                raw: raw.as_deref().map(blob_to_regs).unwrap_or_default(),
                raw_value: r.get(11)?,
                value: r.get(12)?,
                text: r.get(13)?,
                // Unknown codes are surfaced as DECODE_ERROR rather than dropped.
                quality: Quality::from_code(q, exc).unwrap_or(Quality::DecodeError),
                age_ms: age.and_then(|a| u64::try_from(a).ok()),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Stored sample counts per device and quality label (`GOOD`, `TIMEOUT`, ...), for the session report.
    pub fn quality_counts(&self, session: &str) -> Result<Vec<(String, String, u64)>, HistorianError> {
        let mut st = self.conn.prepare(
            "SELECT d.device_key, s.quality, s.exception, COUNT(*)
             FROM samples s JOIN tags t ON t.id = s.tag JOIN devices d ON d.id = t.device
             WHERE d.session = ?1 GROUP BY d.device_key, s.quality, s.exception ORDER BY d.device_key, s.quality",
        )?;
        let rows = st.query_map(params![session], |r| {
            let q: u8 = r.get(1)?;
            let exc: Option<u8> = r.get(2)?;
            let n: i64 = r.get(3)?;
            Ok((
                r.get::<_, String>(0)?,
                Quality::from_code(q, exc)
                    .unwrap_or(Quality::DecodeError)
                    .as_str()
                    .to_owned(),
                u64::try_from(n).unwrap_or(0),
            ))
        })?;
        // EXCEPTION rows differ by code; merge them under one label.
        let mut out: Vec<(String, String, u64)> = Vec::new();
        for row in rows {
            let (d, q, n) = row?;
            match out.iter_mut().find(|(od, oq, _)| *od == d && *oq == q) {
                Some(e) => e.2 += n,
                None => out.push((d, q, n)),
            }
        }
        Ok(out)
    }

    pub fn events(&self, session: &str) -> Result<Vec<EventRow>, HistorianError> {
        let mut st = self.conn.prepare(
            "SELECT session, ts_ms, kind, device, tag, severity, text FROM events WHERE session = ?1 OR session IS NULL ORDER BY ts_ms, id",
        )?;
        let rows = st.query_map(params![session], |r| {
            Ok(EventRow {
                session: r.get(0)?,
                ts_ms: r.get(1)?,
                kind: r.get(2)?,
                device: r.get(3)?,
                tag: r.get(4)?,
                severity: r.get(5)?,
                text: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn append_audit(&mut self, a: &AuditRecord) -> Result<(), HistorianError> {
        self.conn.execute(
            "INSERT INTO command_audit(ts_ms, session_id, user, role, event, device_id, profile_id, profile_version, unit_id,
               function, address, register_id, command_id, preview_id, old_value, requested_value, raw_value_hex, request_hex,
               response_hex, exception, ack, readback, readback_value, note)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
            params![
                a.ts_ms,
                a.session_id,
                a.user,
                a.role,
                a.event.map(|e| e.as_str()),
                a.device_id,
                a.profile_id,
                a.profile_version,
                a.unit_id,
                a.function,
                a.address,
                a.register_id,
                a.command_id,
                a.preview_id.map(|p| to_i64(p, "preview id")).transpose()?,
                a.old_value,
                a.requested_value,
                a.raw_value_hex,
                a.request_hex,
                a.response_hex,
                a.exception,
                a.ack,
                a.readback,
                a.readback_value,
                a.note,
            ],
        )?;
        Ok(())
    }

    /// Audit rows of a session as (event, record) in insertion order.
    pub fn audit(&self, session: &str) -> Result<Vec<AuditRow>, HistorianError> {
        let mut st = self.conn.prepare(
            "SELECT id, ts_ms, session_id, user, role, event, device_id, profile_id, profile_version, unit_id, function, address,
                    register_id, command_id, old_value, requested_value, raw_value_hex, request_hex, response_hex, exception, ack,
                    readback, readback_value, note
             FROM command_audit WHERE session_id = ?1 ORDER BY id",
        )?;
        let rows = st.query_map(params![session], |r| {
            Ok(AuditRow {
                id: r.get(0)?,
                ts_ms: r.get(1)?,
                session_id: r.get(2)?,
                user: r.get(3)?,
                role: r.get(4)?,
                event: r.get(5)?,
                device_id: r.get(6)?,
                profile_id: r.get(7)?,
                profile_version: r.get(8)?,
                unit_id: r.get(9)?,
                function: r.get(10)?,
                address: r.get(11)?,
                register_id: r.get(12)?,
                command_id: r.get(13)?,
                old_value: r.get(14)?,
                requested_value: r.get(15)?,
                raw_value_hex: r.get(16)?,
                request_hex: r.get(17)?,
                response_hex: r.get(18)?,
                exception: r.get(19)?,
                ack: r.get(20)?,
                readback: r.get(21)?,
                readback_value: r.get(22)?,
                note: r.get(23)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Recompute per-minute rollups for every complete minute before `until_ms`
    /// that has not been rolled up yet. GOOD samples only.
    pub fn rollup_until(&mut self, until_ms: i64) -> Result<usize, HistorianError> {
        let done: i64 = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'rollup_until_minute'", [], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .and_then(|s| s.parse().ok())
            .unwrap_or(i64::MIN / 60_000);
        let until_minute = until_ms.div_euclid(60_000);
        if until_minute <= done {
            return Ok(0);
        }
        let tx = self.conn.transaction()?;
        let n = tx.execute(
            "INSERT OR REPLACE INTO samples_1m(tag, minute, n, min, max, sum)
             SELECT tag, ts_ms / 60000, COUNT(*), MIN(value), MAX(value), SUM(value) FROM samples
             WHERE quality = 0 AND value IS NOT NULL AND ts_ms >= ?1 AND ts_ms < ?2
             GROUP BY tag, ts_ms / 60000",
            params![done.saturating_mul(60_000).max(0), until_minute * 60_000],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES ('rollup_until_minute', ?1)",
            params![until_minute.to_string()],
        )?;
        tx.commit()?;
        Ok(n)
    }

    pub fn rollups(
        &self,
        session: &str,
        device: &str,
        tag: &str,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<Vec<Rollup>, HistorianError> {
        let mut st = self.conn.prepare(
            "SELECT m.minute, m.n, m.min, m.max, m.sum FROM samples_1m m JOIN tags t ON t.id = m.tag JOIN devices d ON d.id = t.device
             WHERE d.session = ?1 AND d.device_key = ?2 AND t.key = ?3 AND m.minute * 60000 >= ?4 AND m.minute * 60000 < ?5
             ORDER BY m.minute",
        )?;
        let rows = st.query_map(params![session, device, tag, from_ms, to_ms], |r| {
            let minute: i64 = r.get(0)?;
            let n: i64 = r.get(1)?;
            Ok(Rollup {
                minute_start_ms: minute * 60_000,
                n: u64::try_from(n).unwrap_or(0),
                min: r.get(2)?,
                max: r.get(3)?,
                sum: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Retention: delete samples, rollups and events older than `before_ms`.
    /// The command audit is never deleted.
    pub fn delete_before(&mut self, before_ms: i64) -> Result<usize, HistorianError> {
        let tx = self.conn.transaction()?;
        let mut n = tx.execute("DELETE FROM samples WHERE ts_ms < ?1", params![before_ms])?;
        n += tx.execute(
            "DELETE FROM samples_1m WHERE minute * 60000 < ?1",
            params![before_ms.div_euclid(60_000) * 60_000],
        )?;
        n += tx.execute("DELETE FROM events WHERE ts_ms < ?1", params![before_ms])?;
        tx.commit()?;
        Ok(n)
    }

    /// Direct SQL for tests that check database-level guarantees.
    pub fn execute_raw(&self, sql: &str) -> Result<usize, HistorianError> {
        Ok(self.conn.execute(sql, [])?)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AuditRow {
    pub id: i64,
    pub ts_ms: i64,
    pub session_id: String,
    pub user: String,
    pub role: String,
    pub event: Option<String>,
    pub device_id: String,
    pub profile_id: String,
    pub profile_version: String,
    pub unit_id: Option<u8>,
    pub function: Option<u8>,
    pub address: Option<u16>,
    pub register_id: Option<String>,
    pub command_id: Option<String>,
    pub old_value: Option<f64>,
    pub requested_value: Option<f64>,
    pub raw_value_hex: Option<String>,
    pub request_hex: Option<String>,
    pub response_hex: Option<String>,
    pub exception: Option<u8>,
    pub ack: Option<String>,
    pub readback: Option<String>,
    pub readback_value: Option<f64>,
    pub note: Option<String>,
}

impl AuditSink for Historian {
    fn append(&mut self, record: &AuditRecord) -> Result<(), AuditError> {
        self.append_audit(record).map_err(|e| AuditError(e.to_string()))
    }
}
