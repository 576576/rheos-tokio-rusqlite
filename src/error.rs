use std::fmt;

/// Errors returned by the async [`Connection`](crate::Connection) wrapper.
///
/// Most database errors surface as [`Error::Rusqlite`]. The other variants indicate
/// lifecycle issues with the connection itself rather than SQL-level failures.
///
/// # Converting from `rusqlite::Error`
///
/// `Error` implements `From<rusqlite::Error>`, so closures passed to
/// [`Connection::call`](crate::Connection::call) can use `?` on rusqlite methods
/// directly:
///
/// ```rust
/// # let rt = tokio::runtime::Runtime::new().unwrap();
/// # rt.block_on(async {
/// let conn = rhei_tokio_rusqlite::Connection::open_in_memory().await.unwrap();
/// let n: i64 = conn.call(|c| {
///     // rusqlite::Error is converted to Error::Rusqlite via From impl
///     c.query_row("SELECT 1 + 1", [], |r| r.get(0)).map_err(Into::into)
/// }).await.unwrap();
/// assert_eq!(n, 2);
/// conn.close().await.unwrap();
/// # });
/// ```
#[derive(Debug)]
pub enum Error {
    /// A rusqlite-level error (SQL syntax, constraint violation, I/O, etc.).
    ///
    /// This is the most common variant. The inner [`rusqlite::Error`] provides full
    /// detail about the failure.
    Rusqlite(rusqlite::Error),
    /// The background thread has already stopped — the channel is disconnected.
    ///
    /// This occurs when [`Connection::close`](crate::Connection::close) has been called
    /// or when the last `Connection` clone was dropped before this operation was
    /// dispatched.
    ConnectionClosed,
    /// `rusqlite::Connection::close` returned an error during explicit shutdown.
    ///
    /// SQLite may refuse to close if there are outstanding prepared statements. The
    /// inner [`rusqlite::Error`] describes the reason.
    Close(rusqlite::Error),
    /// An arbitrary error string produced inside a [`call`](crate::Connection::call)
    /// closure.
    ///
    /// Use this variant when the closure needs to signal a domain-level failure that is
    /// not a rusqlite error. Construct it via `Error::Other("message".into())`.
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Rusqlite(e) => write!(f, "rusqlite error: {e}"),
            Error::ConnectionClosed => write!(f, "connection is already closed"),
            Error::Close(e) => write!(f, "error closing connection: {e}"),
            Error::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Rusqlite(e) | Error::Close(e) => Some(e),
            Error::ConnectionClosed | Error::Other(_) => None,
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Rusqlite(e)
    }
}
