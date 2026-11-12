//! Bounded SQLite outbox (architecture data model, research/02 §4).
//!
//! Every message to publish is stored here first and removed only after the
//! broker acknowledged it. Rows come back in insertion order, so a replay keeps
//! the original order, `ts` and `seq`. When the table is full the OLDEST rows go
//! and are counted: a gap in `seq` is visible to consumers, never hidden.

use rusqlite::{Connection, params};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("outbox: {0}")]
pub struct OutboxError(pub String);

impl From<rusqlite::Error> for OutboxError {
    fn from(e: rusqlite::Error) -> Self {
        OutboxError(e.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewRow {
    pub topic: String,
    pub payload: Vec<u8>,
    pub enq_ms: i64,
    pub sample_seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: i64,
    pub topic: String,
    pub payload: Vec<u8>,
    pub enq_ms: i64,
    pub sample_seq: u64,
}

pub struct Outbox {
    conn: Connection,
    max: usize,
    depth: usize,
    /// Rows removed because the table was full, since this value was opened.
    pub dropped: u64,
}

impl Outbox {
    pub fn open(path: Option<&Path>, max: usize) -> Result<Outbox, OutboxError> {
        let conn = match path {
            Some(p) => {
                let c = Connection::open(p)?;
                c.pragma_update(None, "journal_mode", "WAL")?;
                c.pragma_update(None, "synchronous", "NORMAL")?;
                c
            }
            None => Connection::open_in_memory()?,
        };
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS outbox (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                topic TEXT NOT NULL,
                payload BLOB NOT NULL,
                enq_ms INTEGER NOT NULL,
                sample_seq INTEGER NOT NULL
             );",
        )?;
        let depth: i64 = conn.query_row("SELECT COUNT(*) FROM outbox", [], |r| r.get(0))?;
        let mut o = Outbox {
            conn,
            max: max.max(1),
            depth: usize::try_from(depth).unwrap_or(0),
            dropped: 0,
        };
        o.trim()?;
        Ok(o)
    }

    pub fn depth(&self) -> usize {
        self.depth
    }

    pub fn max(&self) -> usize {
        self.max
    }

    fn trim(&mut self) -> Result<(), OutboxError> {
        if self.depth > self.max {
            let extra = self.depth - self.max;
            let n = self.conn.execute(
                "DELETE FROM outbox WHERE id IN (SELECT id FROM outbox ORDER BY id LIMIT ?1)",
                params![i64::try_from(extra).unwrap_or(i64::MAX)],
            )?;
            self.depth -= n;
            self.dropped += n as u64;
        }
        Ok(())
    }

    /// Append rows in one transaction; returns how many old rows the bound pushed out.
    pub fn push_many(&mut self, rows: &[NewRow]) -> Result<u64, OutboxError> {
        if rows.is_empty() {
            return Ok(0);
        }
        let before = self.dropped;
        let tx = self.conn.transaction()?;
        {
            let mut st =
                tx.prepare("INSERT INTO outbox (topic, payload, enq_ms, sample_seq) VALUES (?1, ?2, ?3, ?4)")?;
            for r in rows {
                st.execute(params![
                    r.topic,
                    r.payload,
                    r.enq_ms,
                    i64::try_from(r.sample_seq).unwrap_or(i64::MAX)
                ])?;
            }
        }
        tx.commit()?;
        self.depth += rows.len();
        self.trim()?;
        Ok(self.dropped - before)
    }

    pub fn push(&mut self, row: NewRow) -> Result<u64, OutboxError> {
        self.push_many(std::slice::from_ref(&row))
    }

    /// Up to `limit` rows with `id > after`, oldest first.
    pub fn peek_after(&self, after: i64, limit: usize) -> Result<Vec<Row>, OutboxError> {
        let mut st = self
            .conn
            .prepare("SELECT id, topic, payload, enq_ms, sample_seq FROM outbox WHERE id > ?1 ORDER BY id LIMIT ?2")?;
        let rows = st
            .query_map(params![after, i64::try_from(limit).unwrap_or(i64::MAX)], |r| {
                Ok(Row {
                    id: r.get(0)?,
                    topic: r.get(1)?,
                    payload: r.get(2)?,
                    enq_ms: r.get(3)?,
                    sample_seq: u64::try_from(r.get::<_, i64>(4)?).unwrap_or(0),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Remove one row (acknowledged, expired or refused). Unknown ids are fine.
    pub fn delete(&mut self, id: i64) -> Result<(), OutboxError> {
        let n = self.conn.execute("DELETE FROM outbox WHERE id = ?1", params![id])?;
        self.depth = self.depth.saturating_sub(n);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::check;

    fn row(seq: u64) -> NewRow {
        NewRow {
            topic: format!("t/{seq}"),
            payload: seq.to_be_bytes().to_vec(),
            enq_ms: 1_000 + seq as i64,
            sample_seq: seq,
        }
    }

    /// M3: insertion order, bounded depth, the oldest go first and are counted.
    #[test]
    fn m3_order_bound_and_overflow() {
        check("m3_outbox", 300, |rng| {
            let max = 3 + rng.below(30) as usize;
            let mut o = Outbox::open(None, max).unwrap();
            let mut model: Vec<u64> = Vec::new();
            let mut dropped = 0u64;
            let mut next = 1u64;
            for _ in 0..(10 + rng.below(120)) {
                match rng.below(4) {
                    0 | 1 => {
                        let n = 1 + rng.below(5);
                        let rows: Vec<_> = (0..n).map(|i| row(next + i)).collect();
                        let pushed_out = o.push_many(&rows).unwrap();
                        model.extend(next..next + n);
                        next += n;
                        let over = model.len().saturating_sub(max);
                        model.drain(..over);
                        dropped += over as u64;
                        assert_eq!(pushed_out, over as u64);
                    }
                    2 => {
                        if let Some(first) = o.peek_after(0, 1).unwrap().first() {
                            assert_eq!(Some(first.sample_seq), model.first().copied());
                            o.delete(first.id).unwrap();
                            model.remove(0);
                        }
                    }
                    _ => {
                        // Peek past a cursor keeps order.
                        let all = o.peek_after(0, usize::MAX >> 1).unwrap();
                        assert_eq!(all.iter().map(|r| r.sample_seq).collect::<Vec<_>>(), model);
                        assert!(all.windows(2).all(|w| w[0].id < w[1].id));
                    }
                }
                assert!(o.depth() <= max);
                assert_eq!(o.depth(), model.len());
                assert_eq!(o.dropped, dropped);
            }
        });
    }

    #[test]
    fn rows_survive_reopen_in_order() {
        let dir = std::env::temp_dir().join(format!("buslink-outbox-{}-{}", std::process::id(), rand_tag()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("o.sqlite3");
        {
            let mut o = Outbox::open(Some(&path), 100).unwrap();
            o.push_many(&[row(1), row(2), row(3)]).unwrap();
            let first = o.peek_after(0, 1).unwrap()[0].id;
            o.delete(first).unwrap();
        }
        let o = Outbox::open(Some(&path), 100).unwrap();
        assert_eq!(o.depth(), 2);
        assert_eq!(
            o.peek_after(0, 10)
                .unwrap()
                .iter()
                .map(|r| r.sample_seq)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        // A smaller bound on reopen trims the oldest.
        drop(o);
        let o = Outbox::open(Some(&path), 1).unwrap();
        assert_eq!(
            o.peek_after(0, 10)
                .unwrap()
                .iter()
                .map(|r| r.sample_seq)
                .collect::<Vec<_>>(),
            vec![3]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    fn rand_tag() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    }
}
