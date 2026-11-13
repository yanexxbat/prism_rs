//! Accounts and login.
//! Ported from Prism Launcher (`launcher/minecraft/auth`). See NOTICE.md.

pub mod account;
pub mod msa;
pub mod session;

pub use account::{Account, AccountList, AccountState, AccountType};
pub use session::AuthSession;

use std::path::PathBuf;

/// Messages never contain tokens.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The login servers could not be reached; an offline launch is still possible.
    #[error("{0}")]
    Network(String),
    /// The stored login is no longer usable; the user has to log in again.
    #[error("{0}")]
    Expired(String),
    /// The servers refused the login.
    #[error("{0}")]
    Failed(String),
    /// The launcher's own rules forbid this.
    #[error("{0}")]
    Refused(String),
    #[error("{0}")]
    Protocol(String),
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

/// The project's own Microsoft application (client) ID. It identifies the app and is not a secret.
pub const DEFAULT_MSA_CLIENT_ID: &str = "f5663236-e39b-423e-b0e0-b8fa648c4512";

/// The Microsoft application (client) ID: the `PRISM_RS_MSA_CLIENT_ID` environment
/// variable, else the `MSAClientIDOverride` setting, else the built-in one.
pub fn msa_client_id(setting: &str) -> Option<String> {
    let custom = std::env::var("PRISM_RS_MSA_CLIENT_ID").ok().filter(|id| !id.trim().is_empty());
    let id = custom.unwrap_or_else(|| setting.to_string());
    Some(if id.trim().is_empty() { DEFAULT_MSA_CLIENT_ID.to_string() } else { id.trim().to_string() })
}
