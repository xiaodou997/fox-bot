//! G1c: continuous host with bounded tasks, independent control, credentials and device ownership.
//! Continuous mode remains synthetic; G3c-1 exposes a separate opt-in native single-send test.
#![forbid(unsafe_code)]
pub mod config;
pub mod credentials;
pub mod g2d_real;
pub mod g3c_real;
pub mod native_bridge;
pub mod native_read;
pub mod native_send;
pub mod ownership;
pub mod scheduler;
pub use config::*;
pub use scheduler::*;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HostError {
    #[error("invalid host configuration or command")]
    Config,
    #[error("local runtime operation failed")]
    Storage,
    #[error("device execution scope is busy")]
    Busy,
    #[error("device execution scope is no longer valid")]
    Ownership,
    #[error("required credential is missing")]
    CredentialMissing,
    #[error("system credential store is unavailable")]
    CredentialUnavailable,
    #[error(
        "credential access requires foreground authorization; unattended Keychain reads do not prompt"
    )]
    CredentialInteractionRequired,
    #[error(
        "Keychain denied credential access; authorize this build in the foreground without replacing the existing key"
    )]
    CredentialAccessDenied,
    #[error("credential already exists; replacement requires an explicit rotation workflow")]
    CredentialExists,
    #[error("credential is invalid")]
    CredentialInvalid,
    #[error("host has stopped or its bounded input queue is full")]
    Closed,
    #[error("native read worker queue is full")]
    Backpressure,
    #[error("native read worker is paused")]
    Paused,
    #[error("native read worker failed or returned an invalid result")]
    NativeWorker,
    #[error("native snapshot identity or trust is insufficient")]
    Untrusted,
    #[error("protected mode is not supported on this platform/build")]
    Unsupported,
}
pub type Result<T> = std::result::Result<T, HostError>;
impl From<foxbot_core::Error> for HostError {
    fn from(_: foxbot_core::Error) -> Self {
        Self::Storage
    }
}
