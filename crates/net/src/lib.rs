//! HTTP client and checksum-validated downloads.
//! Ported from Prism Launcher (`launcher/net`: `NetJob`, `Request`, `ChecksumValidator`). See NOTICE.md.

use futures_util::StreamExt;
use sha1::{Digest, Sha1};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("request to {url} failed: {source}")]
    Http {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("checksum mismatch for {url}: expected {expected}, got {actual}")]
    Checksum { url: String, expected: String, actual: String },
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

pub const USER_AGENT: &str = concat!("prism_rs/", env!("CARGO_PKG_VERSION"));

/// One file to fetch. With `sha1` set, the file only lands at `path` if the hash matches.
#[derive(Clone, Debug)]
pub struct Download {
    pub url: String,
    pub path: PathBuf,
    pub sha1: Option<String>,
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha1_hex(data: &[u8]) -> String {
    hex(&Sha1::digest(data))
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex(&sha2::Sha256::digest(data))
}

fn http_err(url: &str) -> impl FnOnce(reqwest::Error) -> Error + '_ {
    move |source| Error::Http { url: url.to_string(), source }
}

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |source| Error::Io { path: path.to_path_buf(), source }
}

impl Client {
    pub fn new() -> Result<Self> {
        let http = reqwest::Client::builder().user_agent(USER_AGENT).build().map_err(http_err("client setup"))?;
        Ok(Client { http })
    }

    /// For requests that need their own headers or a body.
    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    pub async fn get_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let response = self.http.get(url).send().await.and_then(|r| r.error_for_status()).map_err(http_err(url))?;
        Ok(response.bytes().await.map_err(http_err(url))?.to_vec())
    }

    pub async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let response = self.http.get(url).send().await.and_then(|r| r.error_for_status()).map_err(http_err(url))?;
        response.json().await.map_err(http_err(url))
    }

    /// Downloads to `<path>.part` and renames on success; three attempts.
    pub async fn download(&self, download: &Download) -> Result<()> {
        let mut attempt = 1;
        loop {
            match self.download_once(download).await {
                Ok(()) => return Ok(()),
                Err(e) if attempt < 3 => tracing::warn!("attempt {attempt} failed: {e}"),
                Err(e) => return Err(e),
            }
            attempt += 1;
        }
    }

    async fn download_once(&self, download: &Download) -> Result<()> {
        let url = download.url.as_str();
        let response = self.http.get(url).send().await.and_then(|r| r.error_for_status()).map_err(http_err(url))?;
        if let Some(parent) = download.path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(io_err(parent))?;
        }
        let mut part = download.path.clone().into_os_string();
        part.push(".part");
        let part = PathBuf::from(part);

        let mut file = tokio::fs::File::create(&part).await.map_err(io_err(&part))?;
        let mut hasher = Sha1::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(http_err(url))?;
            hasher.update(&chunk);
            file.write_all(&chunk).await.map_err(io_err(&part))?;
        }
        file.flush().await.map_err(io_err(&part))?;
        drop(file);

        if let Some(expected) = &download.sha1 {
            let actual = hex(&hasher.finalize());
            if !expected.eq_ignore_ascii_case(&actual) {
                let _ = tokio::fs::remove_file(&part).await;
                return Err(Error::Checksum { url: url.to_string(), expected: expected.clone(), actual });
            }
        }
        tokio::fs::rename(&part, &download.path).await.map_err(io_err(&download.path))
    }

    /// Runs the downloads `concurrency` at a time and stops at the first failure.
    /// `progress` is called with (finished, total) after each file.
    pub async fn download_all(
        &self,
        downloads: &[Download],
        concurrency: usize,
        progress: impl Fn(usize, usize),
    ) -> Result<()> {
        let total = downloads.len();
        let mut done = 0;
        // Each future owns its client and download, so the whole call can run on another thread.
        let jobs = downloads.iter().cloned().map(|download| {
            let client = self.clone();
            async move { client.download(&download).await }
        });
        let mut stream = futures_util::stream::iter(jobs).buffer_unordered(concurrency.max(1));
        while let Some(result) = stream.next().await {
            result?;
            done += 1;
            progress(done, total);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    /// Serves `body` for every request on a local port.
    async fn serve(body: &'static [u8]) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 2048];
                let _ = socket.read(&mut buf).await;
                let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(body).await;
            }
        });
        format!("http://{addr}/file")
    }

    #[test]
    fn hashes_match_known_vectors() {
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[tokio::test]
    async fn download_checks_sha1() {
        let url = serve(b"abc").await;
        let dir = tempfile::tempdir().expect("tempdir");
        let client = Client::new().expect("client");

        let good = Download {
            url: url.clone(),
            path: dir.path().join("sub/good.bin"),
            sha1: Some("A9993E364706816ABA3E25717850C26C9CD0D89D".into()),
        };
        client.download_all(std::slice::from_ref(&good), 4, |_, _| {}).await.expect("download");
        assert_eq!(std::fs::read(&good.path).expect("read"), b"abc");

        let bad = Download { url, path: dir.path().join("bad.bin"), sha1: Some("00".repeat(20)) };
        let error = client.download(&bad).await.expect_err("must fail");
        assert!(matches!(error, Error::Checksum { .. }), "{error}");
        assert!(!bad.path.exists(), "a file with a wrong hash must not be kept");
        assert!(!dir.path().join("bad.bin.part").exists());
    }
}
