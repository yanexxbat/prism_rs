//! Settings, instances and file helpers shared by the other crates.
//! Ported from Prism Launcher (`launcher/settings`, `BaseInstance`, `InstanceList`,
//! `FileSystem`, `Version`). See NOTICE.md.

pub mod fs;
pub mod ini;
pub mod instance;
pub mod settings;
pub mod version;

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{}: {source}", path.display())]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
