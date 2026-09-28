//! G1 simulation-only runtime. No native input, network or credential access.
//! The SQLite ledger is NOT an encrypted production message store.
#![forbid(unsafe_code)]

mod model;
mod runtime;
mod service;
pub mod simulation;

pub use model::*;
pub use runtime::Runtime;
pub use service::*;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("state directory is already owned by another runtime")]
    Busy,
    #[error("unsupported or unsafe state directory")]
    UnsafeState,
    #[error("unsupported database schema")]
    Schema,
    #[error("invalid input: {0}")]
    Invalid(&'static str),
    #[error("operation blocked: {0}")]
    Blocked(&'static str),
    #[error("stale task or duplicate transition")]
    Stale,
    #[error("queue capacity reached")]
    Backpressure,
    #[error("record not found")]
    NotFound,
    #[error("local storage I/O failed")]
    Io(#[from] std::io::Error),
    #[error("database operation failed")]
    Sql(#[from] rusqlite::Error),
    #[error("invalid serialized data")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
