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

/// The Microsoft application (client) ID: the `PRISM_RS_MSA_CLIENT_ID` environment
/// variable, else the `MSAClientIDOverride` setting. None is built in.
pub fn msa_client_id(setting: &str) -> Option<String> {
    std::env::var("PRISM_RS_MSA_CLIENT_ID")
        .ok()
        .or_else(|| Some(setting.to_string()))
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
}
