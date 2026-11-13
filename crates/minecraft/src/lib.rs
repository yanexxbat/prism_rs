//! Metadata, components, Java, launching and mod platforms.
//! Ported from Prism Launcher (`launcher/meta`, `launcher/minecraft`, `launcher/java`,
//! `launcher/launch`, `launcher/modplatform`). See NOTICE.md.

pub mod assets;
pub mod java;
pub mod launch;
pub mod meta;
pub mod pack_profile;
pub mod version_file;

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Core(#[from] prism_core::Error),
    #[error(transparent)]
    Net(#[from] prism_net::Error),
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{file}: {reason}")]
    BadVersionFile { file: String, reason: String },
    #[error("metadata {0} is not available offline")]
    MetaOffline(String),
    #[error("metadata checksum mismatch for {file}: expected {expected}, got {actual}")]
    MetaChecksum { file: String, expected: String, actual: String },
    #[error("invalid component id or version: {0}")]
    BadComponent(String),
    #[error("local library is missing: {}", .0.display())]
    MissingLocalLibrary(PathBuf),
    #[error("{0}")]
    Dependencies(String),
    #[error("{0}")]
    Java(String),
    #[error("{0}")]
    Launch(String),
    #[error("{0}")]
    ModPlatform(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn io_err(path: &std::path::Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |source| Error::Io { path: path.to_path_buf(), source }
}
