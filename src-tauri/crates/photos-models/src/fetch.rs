//! One model file, resumed where the connection dropped. Long downloads over some networks
//! break mid-stream ("error decoding response body" on a 176 MB file, measured); each break
//! keeps what arrived and asks for the rest with a `Range`, so a flaky line still finishes.
//! The file is written to `<dest>.part` and renamed onto `dest` only once its hash checks out.

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::manifest::ModelEntry;
use crate::store::{part_path, range_from, verify};

/// How many times one download may be resumed before it is left for the next launch.
pub const ATTEMPTS: usize = 12;

/// Progress is reported after at least this many new bytes.
const PROGRESS_STEP: u64 = 1 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    Http(u16),
    /// Nothing ever arrived: offline, refused, DNS.
    Network(String),
    /// A complete file with the wrong hash; the `.part` was removed.
    Checksum,
    /// Some bytes arrived but not all, after [`ATTEMPTS`] tries; the `.part` is kept.
    Incomplete(String),
    Io(String),
    /// Stopped on request; the `.part` is kept so the next fetch resumes it.
    Cancelled,
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Http(c) => write!(f, "HTTP {c}"),
            FetchError::Network(e) => write!(f, "{e}"),
            FetchError::Checksum => write!(f, "checksum mismatch"),
            FetchError::Incomplete(e) => write!(f, "incomplete after {ATTEMPTS} attempts: {e}"),
            FetchError::Io(e) => write!(f, "{e}"),
            FetchError::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for FetchError {}

/// Fetch `url` to `dest`, verified against `entry`. `progress` gets the bytes received so far;
/// `cancel` is checked between chunks and attempts.
pub async fn fetch_one(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    entry: &ModelEntry,
    cancel: &AtomicBool,
    progress: &mut (dyn FnMut(u64) + Send),
) -> Result<(), FetchError> {
    let part = part_path(dest);
    let mut last_error = String::from("no attempt made");
    for _ in 0..ATTEMPTS {
        if cancel.load(Ordering::SeqCst) {
            return Err(FetchError::Cancelled);
        }
        let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        let Some(range) = range_from(have, entry.bytes) else { break };
        let mut req = client.get(url);
        if let Some(r) = &range {
            req = req.header(reqwest::header::RANGE, r);
        }
        let mut resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                last_error = e.to_string();
                continue;
            }
        };
        // 206 continues the file; a 200 is the whole file again (a server that ignores ranges).
        let append = resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
        if !append && !resp.status().is_success() {
            return Err(FetchError::Http(resp.status().as_u16()));
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(&part)
            .map_err(|e| FetchError::Io(e.to_string()))?;
        let mut received = if append { have } else { 0 };
        let mut last_emit = received;
        loop {
            match resp.chunk().await {
                Ok(Some(chunk)) => {
                    file.write_all(&chunk).map_err(|e| FetchError::Io(e.to_string()))?;
                    received += chunk.len() as u64;
                    if received - last_emit > PROGRESS_STEP {
                        last_emit = received;
                        progress(received);
                    }
                    if cancel.load(Ordering::SeqCst) {
                        let _ = file.flush();
                        return Err(FetchError::Cancelled);
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    last_error = e.to_string();
                    break;
                }
            }
        }
    }
    if !verify(&part, entry) {
        // A complete file with the wrong hash is corrupt: start over next time. A short one
        // is kept, so the next fetch resumes it.
        let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        if have >= entry.bytes {
            let _ = std::fs::remove_file(&part);
            return Err(FetchError::Checksum);
        }
        if have == 0 {
            return Err(FetchError::Network(last_error));
        }
        return Err(FetchError::Incomplete(last_error));
    }
    std::fs::rename(&part, dest).map_err(|e| FetchError::Io(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    fn entry_for(body: &[u8]) -> ModelEntry {
        serde_json::from_str(&format!(
            r#"{{ "id": "t", "role": "x", "file": "t.onnx", "bytes": {}, "sha256": "{:x}", "licence": "MIT" }}"#,
            body.len(),
            Sha256::digest(body)
        ))
        .unwrap()
    }

    /// One thread, one canned response per connection; returns the base URL and the requests seen.
    fn serve(responses: Vec<Vec<u8>>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/t.onnx", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for resp in responses {
                let Ok((mut s, _)) = listener.accept() else { return };
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).unwrap_or(0);
                log.lock().unwrap().push(String::from_utf8_lossy(&buf[..n]).into_owned());
                let _ = s.write_all(&resp);
                let _ = s.flush();
            }
        });
        (url, seen)
    }

    fn reply(status: &str, extra: &str, body: &[u8]) -> Vec<u8> {
        let mut v = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n", body.len()).into_bytes();
        v.extend_from_slice(body);
        v
    }

    fn body(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i % 251) as u8).collect()
    }

    async fn run(url: &str, dest: &Path, e: &ModelEntry, cancel: &AtomicBool) -> Result<(), FetchError> {
        fetch_one(&reqwest::Client::new(), url, dest, e, cancel, &mut |_| {}).await
    }

    #[tokio::test]
    async fn a_fresh_fetch_verifies_and_renames() {
        let b = body(5000);
        let (url, _) = serve(vec![reply("200 OK", "", &b)]);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("t.onnx");
        run(&url, &dest, &entry_for(&b), &AtomicBool::new(false)).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b);
        assert!(!part_path(&dest).exists());
    }

    #[tokio::test]
    async fn a_short_part_resumes_with_range() {
        let b = body(5000);
        let (url, seen) = serve(vec![reply("206 Partial Content", "", &b[2000..])]);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("t.onnx");
        std::fs::write(part_path(&dest), &b[..2000]).unwrap();
        run(&url, &dest, &entry_for(&b), &AtomicBool::new(false)).await.unwrap();
        assert!(seen.lock().unwrap()[0].to_ascii_lowercase().contains("range: bytes=2000-"));
        assert_eq!(std::fs::read(&dest).unwrap(), b);
    }

    #[tokio::test]
    async fn a_bad_hash_deletes_the_part() {
        let b = body(3000);
        let mut wrong = b.clone();
        wrong[10] ^= 0xff;
        let (url, _) = serve(vec![reply("200 OK", "", &wrong)]);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("t.onnx");
        let r = run(&url, &dest, &entry_for(&b), &AtomicBool::new(false)).await;
        assert_eq!(r, Err(FetchError::Checksum));
        assert!(!part_path(&dest).exists() && !dest.exists());
    }

    #[tokio::test]
    async fn a_404_is_http_404() {
        let b = body(10);
        let (url, _) = serve(vec![reply("404 Not Found", "", b"")]);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("t.onnx");
        let r = run(&url, &dest, &entry_for(&b), &AtomicBool::new(false)).await;
        assert_eq!(r, Err(FetchError::Http(404)));
        assert_eq!(r.unwrap_err().to_string(), "HTTP 404");
        assert!(!part_path(&dest).exists());
    }

    #[tokio::test]
    async fn offline_is_network() {
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let b = body(10);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("t.onnx");
        let r = run(&format!("http://127.0.0.1:{port}/t.onnx"), &dest, &entry_for(&b), &AtomicBool::new(false)).await;
        assert!(matches!(r, Err(FetchError::Network(_)) | Err(FetchError::Incomplete(_))), "{r:?}");
        assert!(!dest.exists());
    }

    #[tokio::test]
    async fn a_cancelled_fetch_keeps_the_part() {
        let b = body(3 << 20);
        let (url, _) = serve(vec![reply("200 OK", "", &b)]);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("t.onnx");
        let cancel = AtomicBool::new(false);
        let r = fetch_one(&reqwest::Client::new(), &url, &dest, &entry_for(&b), &cancel, &mut |_| {
            cancel.store(true, Ordering::SeqCst)
        })
        .await;
        assert_eq!(r, Err(FetchError::Cancelled));
        let kept = std::fs::metadata(part_path(&dest)).unwrap().len();
        assert!(kept > 1 << 20 && kept <= b.len() as u64, "kept {kept}");
        assert!(!dest.exists());
    }
}
