//! The change feed: one `change` row per write from any process, and an in-process
//! broadcast that fires with the new sequence number after every row this process writes.

use rusqlite::Connection;
use tokio::sync::broadcast;

use crate::db::{self, loads_strs};
use crate::error::Result;
use crate::model::{ChangeRow, Changes};

pub struct ChangeFeed {
    tx: broadcast::Sender<i64>,
}

impl Default for ChangeFeed {
    fn default() -> Self {
        ChangeFeed::new()
    }
}

impl ChangeFeed {
    pub fn new() -> ChangeFeed {
        let (tx, _rx) = broadcast::channel(256);
        ChangeFeed { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<i64> {
        self.tx.subscribe()
    }

    /// Tells subscribers that the feed reached `seq`. A feed without listeners drops the value.
    pub fn notify(&self, seq: i64) {
        let _ = self.tx.send(seq);
    }

    /// Writes a change row inside the caller's transaction and notifies after it.
    pub fn log(&self, c: &Connection, what: &str, ids: &[String], origin: &str) -> Result<i64> {
        let seq = db::log_change(c, what, ids, origin)?;
        self.notify(seq);
        Ok(seq)
    }
}

/// The rows after `since`, oldest first, at most `limit`.
pub fn changes_since(c: &Connection, since: i64, limit: i64) -> Result<Changes> {
    let mut stmt = c.prepare("SELECT seq,at,what,ids,origin FROM change WHERE seq>? ORDER BY seq LIMIT ?")?;
    let rows = stmt.query_map([since, limit], |r| {
        Ok(ChangeRow {
            seq: r.get(0)?,
            at: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            what: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            ids: loads_strs(r.get::<_, Option<String>>(3)?.as_deref()),
            origin: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
        })
    })?;
    let changes = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Changes { seq: db::change_seq(c), changes })
}
