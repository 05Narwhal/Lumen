//! Setup's download of the smart-feature models. A model that does not arrive is a state Lumen
//! already handles (it fetches the rest itself), never an error screen: every `FetchError`
//! ends the download as `Later`.
use crate::options::{ModelsOutcome, SetupProgress, SetupTask};
use photos_models::fetch::fetch_one;
use photos_models::{ModelEntry, ModelStore};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{watch, Mutex};

/// The one download Setup runs. `stop` ends it and waits until it has, leaving the `.part` for
/// Lumen to resume.
pub struct ModelsJob {
    cancel: AtomicBool,
    stop_tx: watch::Sender<bool>,
    running: Mutex<()>,
}

impl Default for ModelsJob {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelsJob {
    pub fn new() -> Self {
        Self { cancel: AtomicBool::new(false), stop_tx: watch::channel(false).0, running: Mutex::new(()) }
    }

    /// Clear an earlier stop; called when an install starts.
    pub fn reset(&self) {
        self.cancel.store(false, Ordering::SeqCst);
        self.stop_tx.send_replace(false);
    }

    /// Ask the download to end without waiting (Skip).
    pub fn request_stop(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        self.stop_tx.send_replace(true);
    }

    /// End the download and wait for it to let go of its files (Open Lumen).
    pub async fn stop(&self) {
        self.request_stop();
        // The download holds this lock until it has let go of its files.
        let _idle = self.running.lock().await;
    }

    /// Fetch every entry that is not ready into `store`. The first failure of any kind ends the
    /// run as `Later`; so does a stop.
    pub async fn download(
        &self,
        store: &ModelStore,
        entries: &[ModelEntry],
        url_of: &(dyn Fn(&ModelEntry) -> String + Sync),
        progress: &mut (dyn FnMut(SetupProgress) + Send),
    ) -> ModelsOutcome {
        let _running = self.running.lock().await;
        let total: u64 = entries.iter().map(|e| e.bytes).sum();
        let mut done: u64 = 0;
        let Ok(client) = reqwest::Client::builder().user_agent(concat!("Lumen-Setup/", env!("CARGO_PKG_VERSION"))).build() else {
            return ModelsOutcome::Later;
        };
        if std::fs::create_dir_all(&store.dir).is_err() {
            return ModelsOutcome::Later;
        }
        for entry in entries {
            if self.cancel.load(Ordering::SeqCst) {
                return ModelsOutcome::Later;
            }
            if store.is_ready(entry) {
                done += entry.bytes;
                progress(SetupProgress { task: SetupTask::Models, done, total });
                continue;
            }
            let url = url_of(entry);
            let dest = store.path_of(entry);
            let base = done;
            let mut on_bytes = |received: u64| progress(SetupProgress { task: SetupTask::Models, done: base + received, total });
            let mut stop = self.stop_tx.subscribe();
            // Dropping the fetch on a stop frees a stalled connection at once; its `.part` stays.
            let result = tokio::select! {
                r = fetch_one(&client, &url, &dest, entry, &self.cancel, &mut on_bytes) => r,
                _ = stop.wait_for(|stopped| *stopped) => return ModelsOutcome::Later,
            };
            if result.is_err() {
                return ModelsOutcome::Later;
            }
            done += entry.bytes;
            progress(SetupProgress { task: SetupTask::Models, done, total });
        }
        ModelsOutcome::Complete
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photos_models::fetch::FetchError;
    use sha2::{Digest, Sha256};
    use std::path::Path;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn entry(file: &str, body: &[u8]) -> ModelEntry {
        serde_json::from_value(serde_json::json!({
            "id": format!("{file}@1"), "role": "image_embedding", "file": file,
            "bytes": body.len(), "sha256": format!("{:x}", Sha256::digest(body)), "licence": "MIT"
        }))
        .unwrap()
    }

    type Reply = Arc<dyn Fn(Option<u64>) -> Vec<u8> + Send + Sync>;

    /// A local HTTP server that answers every GET with `reply(range start)` and closes. Returns
    /// its base url and a counter of requests served.
    async fn serve(reply: Reply) -> (String, Arc<std::sync::atomic::AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let h = hits.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let reply = reply.clone();
                let h = h.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_lowercase();
                    let from = req
                        .lines()
                        .find_map(|l| l.strip_prefix("range: bytes="))
                        .and_then(|r| r.split('-').next())
                        .and_then(|s| s.trim().parse().ok());
                    h.fetch_add(1, Ordering::SeqCst);
                    let _ = sock.write_all(&reply(from)).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        (base, hits)
    }

    fn ok(body: &[u8], from: Option<u64>) -> Vec<u8> {
        let (status, slice) = match from {
            Some(f) => ("206 Partial Content", &body[f as usize..]),
            None => ("200 OK", body),
        };
        let mut r = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", slice.len()).into_bytes();
        r.extend_from_slice(slice);
        r
    }

    fn store(dir: &Path) -> ModelStore {
        ModelStore { dir: dir.to_path_buf() }
    }

    async fn run(job: &ModelsJob, st: &ModelStore, entries: &[ModelEntry], base: &str) -> (ModelsOutcome, Vec<SetupProgress>) {
        let mut seen = Vec::new();
        let base = base.to_string();
        let url = move |e: &ModelEntry| format!("{base}{}", e.file);
        let out = job.download(st, entries, &url, &mut |p| seen.push(p)).await;
        (out, seen)
    }

    #[tokio::test]
    async fn a_download_that_arrives_is_complete_and_ready() {
        let body = vec![7u8; 3000];
        let e = entry("a.onnx", &body);
        let b = body.clone();
        let (base, _) = serve(Arc::new(move |f| ok(&b, f))).await;
        let dir = tempfile::tempdir().unwrap();
        let st = store(dir.path());
        let (out, seen) = run(&ModelsJob::new(), &st, std::slice::from_ref(&e), &base).await;
        assert_eq!(out, ModelsOutcome::Complete);
        assert!(st.is_ready(&e));
        let last = seen.last().expect("progress after the last model");
        assert_eq!((last.task, last.done, last.total), (SetupTask::Models, 3000, 3000));
    }

    #[tokio::test]
    async fn a_ready_model_is_skipped() {
        let body = vec![1u8; 500];
        let e = entry("r.onnx", &body);
        let (base, hits) = serve(Arc::new(|_| b"HTTP/1.1 500 x\r\nContent-Length: 0\r\n\r\n".to_vec())).await;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("r.onnx"), &body).unwrap();
        let (out, _) = run(&ModelsJob::new(), &store(dir.path()), &[e], &base).await;
        assert_eq!(out, ModelsOutcome::Complete);
        assert_eq!(hits.load(Ordering::SeqCst), 0, "no request for a file that is already whole");
    }

    #[tokio::test]
    async fn a_failed_download_is_later_never_an_error() {
        let body = vec![9u8; 2000];
        let e = entry("f.onnx", &body);
        let dir = tempfile::tempdir().unwrap();
        let st = store(dir.path());
        let job = ModelsJob::new();

        // Http: a 404.
        let (base, _) = serve(Arc::new(|_| b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec())).await;
        assert_eq!(run(&job, &st, std::slice::from_ref(&e), &base).await.0, ModelsOutcome::Later, "Http");

        // Network: nothing listens.
        let dead = {
            let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
            format!("http://{}/", l.local_addr().unwrap())
        };
        assert_eq!(run(&job, &st, std::slice::from_ref(&e), &dead).await.0, ModelsOutcome::Later, "Network");

        // Checksum: the right size, the wrong bytes.
        let wrong = vec![1u8; 2000];
        let (base, _) = serve(Arc::new(move |f| ok(&wrong, f))).await;
        assert_eq!(run(&job, &st, std::slice::from_ref(&e), &base).await.0, ModelsOutcome::Later, "Checksum");
        assert!(!st.is_ready(&e));

        // Incomplete: promises 2000 bytes, sends 100 and hangs up, every time.
        let (base, _) = serve(Arc::new(|_| {
            let mut r = b"HTTP/1.1 200 OK\r\nContent-Length: 2000\r\nConnection: close\r\n\r\n".to_vec();
            r.extend_from_slice(&[9u8; 100]);
            r
        }))
        .await;
        assert_eq!(run(&job, &st, std::slice::from_ref(&e), &base).await.0, ModelsOutcome::Later, "Incomplete");

        // Io: the .part cannot be opened because a folder sits there.
        let io_dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(io_dir.path().join("f.onnx.part")).unwrap();
        let b = body.clone();
        let (base, _) = serve(Arc::new(move |f| ok(&b, f))).await;
        assert_eq!(run(&job, &store(io_dir.path()), std::slice::from_ref(&e), &base).await.0, ModelsOutcome::Later, "Io");

        // Cancelled: stopped before it began.
        let b = body.clone();
        let (base, hits) = serve(Arc::new(move |f| ok(&b, f))).await;
        job.request_stop();
        assert_eq!(run(&job, &st, std::slice::from_ref(&e), &base).await.0, ModelsOutcome::Later, "Cancelled");
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        // Compile-time reminder: a new FetchError variant must be decided here.
        let _ = |e: FetchError| match e {
            FetchError::Http(_) | FetchError::Network(_) | FetchError::Checksum | FetchError::Incomplete(_) | FetchError::Io(_) | FetchError::Cancelled => (),
        };
    }

    #[tokio::test]
    async fn a_failure_stops_the_run_so_an_offline_machine_does_not_retry_every_model() {
        let a = entry("a.onnx", b"aaaa");
        let b = entry("b.onnx", b"bbbb");
        let (base, hits) = serve(Arc::new(|_| b"HTTP/1.1 503 x\r\nContent-Length: 0\r\n\r\n".to_vec())).await;
        let dir = tempfile::tempdir().unwrap();
        let (out, _) = run(&ModelsJob::new(), &store(dir.path()), &[a, b], &base).await;
        assert_eq!(out, ModelsOutcome::Later);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn open_lumen_stops_the_download_first() {
        // A server that sends the headers and 10 bytes, then goes quiet: a stalled connection.
        let body = vec![5u8; 100_000];
        let e = entry("s.onnx", &body);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        let (sent_tx, sent_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = sock.read(&mut buf).await;
            let _ = sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100000\r\n\r\n0123456789").await;
            let _ = sent_tx.send(());
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        });

        let dir = tempfile::tempdir().unwrap();
        let st = store(dir.path());
        let job = Arc::new(ModelsJob::new());
        let log = Arc::new(std::sync::Mutex::new(Vec::<&'static str>::new()));

        let dl = {
            let (job, log, e) = (job.clone(), log.clone(), e.clone());
            let st = st.clone();
            let base = base.clone();
            tokio::spawn(async move {
                let url = move |e: &ModelEntry| format!("{base}{}", e.file);
                let out = job.download(&st, &[e], &url, &mut |_| {}).await;
                log.lock().unwrap().push("download ended");
                out
            })
        };
        sent_rx.await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // What the open_lumen command does: stop, then launch.
        let stopped = tokio::time::timeout(std::time::Duration::from_secs(5), job.stop()).await;
        assert!(stopped.is_ok(), "stop must not wait for a stalled connection");
        log.lock().unwrap().push("lumen launched");

        assert_eq!(dl.await.unwrap(), ModelsOutcome::Later);
        assert_eq!(*log.lock().unwrap(), vec!["download ended", "lumen launched"]);
        assert!(photos_models::part_path(&st.path_of(&e)).exists(), "the .part stays for Lumen to resume");
        assert!(!st.is_ready(&e));
    }
}
