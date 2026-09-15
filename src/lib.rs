#![forbid(unsafe_code)]
//! Async [`rusqlite`](https://docs.rs/rusqlite) wrapper using a dedicated OS thread and
//! a `crossbeam_channel` dispatch loop.
//!
//! # Why this crate?
//!
//! `rusqlite` is inherently synchronous — every call blocks the calling thread. Running
//! SQLite operations directly inside a Tokio task would block the async executor. The
//! standard solution is `spawn_blocking`, but it creates a new thread-pool thread per
//! call and incurs overhead on every dispatch.
//!
//! This crate takes a different approach: a **single OS thread is dedicated to the
//! database connection** for its entire lifetime. Callers send closures over a
//! `Sender<Message>` (crossbeam unbounded channel) and receive results via
//! `oneshot::Receiver<T>`. Because the connection thread is long-lived, there is no
//! per-call thread-spawn cost, and `rusqlite::Connection` never crosses thread
//! boundaries.
//!
//! # Differences from `tokio-rusqlite`
//!
//! The [`tokio-rusqlite`](https://docs.rs/tokio-rusqlite) crate uses `spawn_blocking`
//! (one thread per call from the pool). This crate permanently binds one `std::thread`
//! to one `rusqlite::Connection`, which is cheaper for workloads with many short
//! sequential calls (no thread-pool churn) and guarantees that SQLite's
//! `PRAGMA`/`BEGIN`/`COMMIT` session state is never observed on a different thread.
//!
//! # Threading model
//!
//! ```text
//! async caller
//!   │  conn.call(|c| { … })
//!   │
//!   ▼
//! Sender<Message>  ──crossbeam channel──►  background OS thread
//!                                               │ rusqlite::Connection
//!                                               │ executes closure
//!                                               ▼
//! oneshot::Receiver<R>  ◄── tokio oneshot ──  result
//! ```
//!
//! The [`Connection`] handle is [`Clone`]: all clones share the same channel and the
//! same underlying SQLite connection, so they are all serialized through the single
//! background thread. SQLite's WAL mode is recommended when multiple handles are used
//! concurrently from different clones.
//!
//! # Error handling
//!
//! All fallible operations return [`Error`]. Rusqlite errors are wrapped in
//! [`Error::Rusqlite`]; callers can also return arbitrary errors via [`Error::Other`].
//!
//! # Example
//!
//! ```rust
//! // tokio::runtime::Runtime is available because the tokio dep enables the `rt` feature.
//! let rt = tokio::runtime::Runtime::new().unwrap();
//! rt.block_on(async {
//!     let conn = rhei_tokio_rusqlite::Connection::open_in_memory().await
//!         .expect("open in-memory db");
//!
//!     // DDL + DML in one closure — the connection is not re-entrant, so keep
//!     // transactions inside a single call() to avoid deadlocks.
//!     conn.call(|c| {
//!         c.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)", [])?;
//!         c.execute("INSERT INTO users (name) VALUES (?1)", ["Alice"])?;
//!         c.execute("INSERT INTO users (name) VALUES (?1)", ["Bob"])?;
//!         Ok(())
//!     }).await.expect("setup");
//!
//!     let count: i64 = conn.call(|c| {
//!         c.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
//!             .map_err(Into::into)
//!     }).await.expect("count");
//!
//!     assert_eq!(count, 2);
//!
//!     conn.close().await.expect("close");
//! });
//! ```

mod error;

pub use error::Error;

use crossbeam_channel::{Receiver, Sender};
use std::path::Path;
use tokio::sync::oneshot;

/// A boxed closure sent to the background thread for execution.
type CallFn = Box<dyn FnOnce(&mut rusqlite::Connection) + Send>;

/// Messages sent from the async side to the background thread.
enum Message {
    /// Execute a closure on the rusqlite connection.
    Execute(CallFn),
    /// Close the connection and report the result.
    Close(oneshot::Sender<Result<(), rusqlite::Error>>),
}

/// An async handle to a SQLite connection that runs on a dedicated background thread.
///
/// ## Dispatch model
///
/// Constructing a `Connection` (via [`Connection::open`] or
/// [`Connection::open_in_memory`]) spawns exactly one `std::thread`. That thread owns
/// a `rusqlite::Connection` and processes messages from a `Sender<Message>` channel in
/// a loop. Each [`call`](Connection::call) sends a boxed closure down the channel and
/// awaits the result on a `oneshot` channel, so the async caller never blocks the Tokio
/// executor.
///
/// ## Cloning
///
/// `Connection` is cheap to clone: clones share the same `Sender<Message>` and
/// therefore the same background thread. Because all calls are serialized through one
/// thread, concurrent operations from multiple clones are safe but not parallel — they
/// queue behind each other on the channel. For SQLite workloads this is usually fine
/// given SQLite's own serialization guarantees.
///
/// ## Closing
///
/// Dropping the last clone disconnects the channel; the background thread exits its
/// event loop and the `rusqlite::Connection` is dropped cleanly. To receive an explicit
/// confirmation (and any close error), call [`Connection::close`] instead of dropping.
#[derive(Clone)]
pub struct Connection {
    sender: Sender<Message>,
}

impl Connection {
    /// Open a SQLite database file at `path`, returning an async connection handle.
    ///
    /// A background OS thread is spawned immediately. The method returns only after the
    /// thread has successfully opened the database file (or returned an error).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Rusqlite`] if `rusqlite::Connection::open` fails (e.g., the
    /// path is not writable or the file is not a valid SQLite database).
    ///
    /// # Examples
    ///
    /// ```rust
    /// # let rt = tokio::runtime::Runtime::new().unwrap();
    /// # rt.block_on(async {
    /// // Open an in-memory database for illustration; swap with a real path in
    /// // production (Connection::open is tested separately by the test suite).
    /// let conn = rhei_tokio_rusqlite::Connection::open_in_memory().await.unwrap();
    /// conn.close().await.unwrap();
    /// # });
    /// ```
    pub async fn open<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let path = path.as_ref().to_owned();
        let (open_tx, open_rx) = oneshot::channel();

        // Unbounded channel: backpressure is inherent in the request-response
        // pattern — each call() awaits its oneshot before sending the next.
        let (sender, receiver) = crossbeam_channel::unbounded();

        std::thread::spawn(move || match rusqlite::Connection::open(&path) {
            Ok(conn) => {
                let _ = open_tx.send(Ok(()));
                event_loop(conn, receiver);
            }
            Err(e) => {
                let _ = open_tx.send(Err(e));
            }
        });

        open_rx
            .await
            .map_err(|_| Error::ConnectionClosed)?
            .map_err(Error::Rusqlite)?;

        Ok(Connection { sender })
    }

    /// Open a private in-memory SQLite database.
    ///
    /// In-memory databases are useful for tests and temporary scratch space. Each call
    /// creates an independent database; the data is discarded when the last
    /// `Connection` clone is dropped.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Rusqlite`] if `rusqlite::Connection::open_in_memory` fails
    /// (very unlikely in practice, but included for completeness).
    pub async fn open_in_memory() -> Result<Self, Error> {
        let (open_tx, open_rx) = oneshot::channel();

        let (sender, receiver) = crossbeam_channel::unbounded();

        std::thread::spawn(move || match rusqlite::Connection::open_in_memory() {
            Ok(conn) => {
                let _ = open_tx.send(Ok(()));
                event_loop(conn, receiver);
            }
            Err(e) => {
                let _ = open_tx.send(Err(e));
            }
        });

        open_rx
            .await
            .map_err(|_| Error::ConnectionClosed)?
            .map_err(Error::Rusqlite)?;

        Ok(Connection { sender })
    }

    /// Execute a closure on the background rusqlite thread and await its result.
    ///
    /// The closure receives a `&mut rusqlite::Connection` and must return
    /// `Result<R, Error>`. It runs synchronously on the dedicated background thread;
    /// the async caller is suspended on a `oneshot` channel until the result is ready.
    ///
    /// ## Tips
    ///
    /// - Keep the closure short: it holds the connection for its entire duration.
    /// - Use a single `call` to wrap a `BEGIN … COMMIT` transaction rather than
    ///   spreading statements across multiple calls; the connection is not re-entrant.
    /// - Closures must be `Send + 'static` — capture by value (use `move |c| { … }`).
    ///
    /// # Errors
    ///
    /// Returns whatever `Error` the closure returns, or [`Error::ConnectionClosed`] if
    /// the background thread has exited (e.g., after [`Connection::close`] or if all
    /// senders were dropped before this call was dispatched).
    ///
    /// # Examples
    ///
    /// ```rust
    /// # let rt = tokio::runtime::Runtime::new().unwrap();
    /// # rt.block_on(async {
    /// let conn = rhei_tokio_rusqlite::Connection::open_in_memory().await.unwrap();
    /// conn.call(|c| {
    ///     c.execute("CREATE TABLE kv (k TEXT PRIMARY KEY, v INTEGER)", [])?;
    ///     c.execute("INSERT INTO kv VALUES ('answer', 42)", [])?;
    ///     Ok(())
    /// }).await.unwrap();
    ///
    /// let val: i64 = conn.call(|c| {
    ///     c.query_row("SELECT v FROM kv WHERE k = 'answer'", [], |row| row.get(0))
    ///         .map_err(Into::into)
    /// }).await.unwrap();
    /// assert_eq!(val, 42);
    /// conn.close().await.unwrap();
    /// # });
    /// ```
    pub async fn call<F, R>(&self, f: F) -> Result<R, Error>
    where
        F: FnOnce(&mut rusqlite::Connection) -> Result<R, Error> + Send + 'static,
        R: Send + 'static,
    {
        let (result_tx, result_rx) = oneshot::channel();

        let callback: CallFn = Box::new(move |conn| {
            let result = f(conn);
            let _ = result_tx.send(result);
        });

        self.sender
            .send(Message::Execute(callback))
            .map_err(|_| Error::ConnectionClosed)?;

        result_rx.await.map_err(|_| Error::ConnectionClosed)?
    }

    /// Explicitly close the connection and wait for the background thread to confirm.
    ///
    /// This is optional: dropping the last `Connection` clone also closes the
    /// background thread cleanly. `close` is useful when you need to observe — and
    /// propagate — any error that rusqlite reports during shutdown (e.g., a WAL
    /// checkpoint failure).
    ///
    /// After `close` returns, any remaining clones of this handle will return
    /// [`Error::ConnectionClosed`] on the next [`Connection::call`].
    ///
    /// # Errors
    ///
    /// - [`Error::ConnectionClosed`] — the connection was already closed or all channel
    ///   senders were dropped before the `Close` message could be dispatched.
    /// - [`Error::Close`] — `rusqlite::Connection::close` reported an error (wraps the
    ///   inner `rusqlite::Error`).
    pub async fn close(self) -> Result<(), Error> {
        let (close_tx, close_rx) = oneshot::channel();

        self.sender
            .send(Message::Close(close_tx))
            .map_err(|_| Error::ConnectionClosed)?;

        close_rx
            .await
            .map_err(|_| Error::ConnectionClosed)?
            .map_err(Error::Close)
    }
}

/// The event loop running on the dedicated background thread.
///
/// Processes messages until the channel is closed or a `Close` message is received.
fn event_loop(mut conn: rusqlite::Connection, receiver: Receiver<Message>) {
    while let Ok(msg) = receiver.recv() {
        match msg {
            Message::Execute(f) => f(&mut conn),
            Message::Close(sender) => {
                let result = conn.close().map_err(|(_conn, err)| err);
                let _ = sender.send(result);
                return;
            }
        }
    }
    // Channel closed (all senders dropped) — connection drops naturally here.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_open_in_memory() {
        let conn = Connection::open_in_memory().await.unwrap();

        conn.call(|conn| {
            conn.execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
                [],
            )?;
            conn.execute("INSERT INTO test (name) VALUES (?1)", ["Alice"])?;
            Ok(())
        })
        .await
        .unwrap();

        let name: String = conn
            .call(|conn| {
                conn.query_row("SELECT name FROM test WHERE id = 1", [], |row| row.get(0))
                    .map_err(Into::into)
            })
            .await
            .unwrap();

        assert_eq!(name, "Alice");
    }

    #[tokio::test]
    async fn test_call_returns_result() {
        let conn = Connection::open_in_memory().await.unwrap();

        conn.call(|conn| {
            conn.execute("CREATE TABLE nums (val INTEGER)", [])?;
            conn.execute("INSERT INTO nums VALUES (10)", [])?;
            conn.execute("INSERT INTO nums VALUES (20)", [])?;
            conn.execute("INSERT INTO nums VALUES (30)", [])?;
            Ok(())
        })
        .await
        .unwrap();

        let sum: i64 = conn
            .call(|conn| {
                conn.query_row("SELECT SUM(val) FROM nums", [], |row| row.get(0))
                    .map_err(Into::into)
            })
            .await
            .unwrap();

        assert_eq!(sum, 60);
    }

    #[tokio::test]
    async fn test_clone_and_concurrent() {
        let conn = Connection::open_in_memory().await.unwrap();

        conn.call(|conn| {
            conn.execute(
                "CREATE TABLE concurrent (id INTEGER PRIMARY KEY, val INTEGER NOT NULL)",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let mut handles = Vec::new();
        for i in 0..10 {
            let conn_clone = conn.clone();
            handles.push(tokio::spawn(async move {
                conn_clone
                    .call(move |conn| {
                        conn.execute(
                            "INSERT INTO concurrent (id, val) VALUES (?1, ?2)",
                            rusqlite::params![i, i * 10],
                        )?;
                        Ok(())
                    })
                    .await
                    .unwrap();
            }));
        }

        for handle in handles {
            handle.await.unwrap();
        }

        let count: i64 = conn
            .call(|conn| {
                conn.query_row("SELECT COUNT(*) FROM concurrent", [], |row| row.get(0))
                    .map_err(Into::into)
            })
            .await
            .unwrap();

        assert_eq!(count, 10);
    }

    #[tokio::test]
    async fn test_close() {
        let conn = Connection::open_in_memory().await.unwrap();

        conn.call(|conn| {
            conn.execute("CREATE TABLE t (id INTEGER)", [])?;
            Ok(())
        })
        .await
        .unwrap();

        // Clone before closing so we can test that the clone also fails.
        let conn2 = conn.clone();

        conn.close().await.unwrap();

        // Subsequent calls on the clone should fail with ConnectionClosed.
        let result = conn2
            .call(|conn| {
                conn.execute("SELECT 1", []).map_err(Error::Rusqlite)?;
                Ok(())
            })
            .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            Error::ConnectionClosed => {} // expected
            other => panic!("expected ConnectionClosed, got: {other}"),
        }
    }

    #[tokio::test]
    async fn test_drop() {
        let conn = Connection::open_in_memory().await.unwrap();

        conn.call(|conn| {
            conn.execute("CREATE TABLE t (id INTEGER)", [])?;
            conn.execute("INSERT INTO t VALUES (1)", [])?;
            Ok(())
        })
        .await
        .unwrap();

        // Drop the connection — the background thread should exit gracefully.
        drop(conn);

        // Give the thread a moment to exit. If it panics, the test will fail.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}
