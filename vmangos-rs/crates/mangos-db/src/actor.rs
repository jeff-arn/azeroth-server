//! Generic per-database actor: one thread owns one synchronous
//! `mysql` connection and runs jobs serially (architecture
//! sections 5 and 12) — the shape [`crate::login::LoginDb`]
//! established for realmd, generalized for the world,
//! character, and logs databases.
//!
//! `call` blocks the caller on a `sync_channel`; Tokio callers
//! bridge with `spawn_blocking`. Serial-lane (GUID-low) ordering
//! for character writes arrives with the save port (Phase 6);
//! Phase 3 is read-only plus the boot/session writes on record.

use std::sync::mpsc::{Receiver, Sender, SyncSender, channel, sync_channel};

use crate::DbError;

type Job = Box<dyn FnOnce(&mut mysql::Conn) + Send>;

/// Handle to a per-DB actor thread. Clone-free by design: share
/// via `Arc`.
pub struct DbActor {
    tx: Sender<Job>,
}

impl DbActor {
    /// Connect and spawn the actor thread. Blocks until the
    /// initial connection succeeds or fails, so a bad DSN is a
    /// startup error (`Master::StartDB`).
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the URL is invalid or the
    /// connection is rejected.
    pub fn spawn(name: &str, url: &str) -> Result<DbActor, DbError> {
        let opts = mysql::Opts::from_url(url)?;
        let (ready_tx, ready_rx) = sync_channel::<Result<(), DbError>>(1);
        let (tx, rx) = channel::<Job>();
        std::thread::Builder::new()
            .name(name.to_string())
            .spawn(move || actor_loop(opts, rx, ready_tx))
            .map_err(|e| DbError::Driver(mysql::Error::IoError(e)))?;
        ready_rx
            .recv()
            .unwrap_or(Err(DbError::NoRow))
            .map(|()| DbActor { tx })
    }

    /// Run one job on the actor connection and wait for its result.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] from the job, or [`DbError::NoRow`] if
    /// the actor thread has exited.
    pub fn call<T, F>(&self, job: F) -> Result<T, DbError>
    where
        T: Send + 'static,
        F: FnOnce(&mut mysql::Conn) -> Result<T, DbError> + Send + 'static,
    {
        let (done_tx, done_rx) = sync_channel::<Result<T, DbError>>(1);
        self.tx
            .send(Box::new(move |conn| {
                let _ = done_tx.send(job(conn));
            }))
            .map_err(|_| DbError::NoRow)?;
        done_rx.recv().unwrap_or(Err(DbError::NoRow))
    }
}

// By-value is the thread-ownership transfer, as in login::actor_loop.
#[allow(clippy::needless_pass_by_value)]
fn actor_loop(opts: mysql::Opts, rx: Receiver<Job>, ready_tx: SyncSender<Result<(), DbError>>) {
    let mut conn = match mysql::Conn::new(opts.clone()) {
        Ok(conn) => {
            let _ = ready_tx.send(Ok(()));
            conn
        }
        Err(e) => {
            let _ = ready_tx.send(Err(DbError::Driver(e)));
            return;
        }
    };
    while let Ok(job) = rx.recv() {
        // Same ping-and-reconnect guard as the login actor: the
        // C++ driver reconnects transparently, so a dropped dev
        // connection must not wedge the actor forever.
        if conn.ping().is_err()
            && let Ok(fresh) = mysql::Conn::new(opts.clone())
        {
            conn = fresh;
        }
        job(&mut conn);
    }
}
