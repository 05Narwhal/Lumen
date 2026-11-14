//! The models are fetched automatically (ML design §2.3): a plain GET of public files named
//! by the compiled-in manifest, verified by SHA-256 before an atomic rename puts each in
//! place. Nothing is sent but the request itself. Offline is not an error — the next launch
//! or six-hour tick tries again, and every algorithm works without the models meanwhile.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use photos_ml::clip::ImageEncoder;
use photos_ml::faces::{FaceDetector, FaceEmbedder};
use photos_ml::pets::PetEmbedder;
use photos_ml::retouch::Retoucher;
use photos_ml::text::TextEncoder;
use photos_ml::manifest::{manifest, ModelEntry};
use photos_ml::store::ModelStore;
use photos_models::verify;
use tauri::{AppHandle, Emitter, Manager};

use crate::events::{ModelsProgress, MODELS_PROGRESS, MODELS_READY};
use crate::state::AppState;

/// Where `export.py --install-dev` puts a model Julian just trained (Round 12 spec §4):
/// `config_dir/JL-Studios/Lumen/models/dev`, or `$LUMEN_DEV_MODELS` (tests, dev).
pub fn dev_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("LUMEN_DEV_MODELS") {
        return PathBuf::from(p);
    }
    let app_data = photos_core::paths::AppPaths::resolve()
        .map(|p| p.app_data)
        .unwrap_or_else(|_| photos_core::paths::AppPaths::app_data_under(Path::new(".")));
    app_data.join("models").join("dev")
}

/// A trained model in the dev folder. Every way it can be unusable is a state, never an error.
#[derive(Debug, Clone)]
pub enum DevModel {
    Missing,
    Ready { path: PathBuf, entry: Box<ModelEntry> },
    Invalid { reason: String },
}

/// What a dev file was last checked against, so its SHA-256 is computed once per version of
/// the file: a new export (new size or mtime, or a new entry) is checked again on the next call.
type ShaCheck = (std::time::SystemTime, u64, String, bool);
static DEV_CHECKED: Mutex<Option<std::collections::HashMap<PathBuf, ShaCheck>>> = Mutex::new(None);

/// The dev entry `<role>.json` and its file, checked by size and SHA-256.
pub fn dev_model(role: &str) -> DevModel {
    dev_model_in(&dev_dir(), role)
}

/// [`dev_model`] on any folder (the catalogue reads the legacy entry this way).
pub fn dev_model_in(dir: &Path, role: &str) -> DevModel {
    let raw = match std::fs::read_to_string(dir.join(format!("{role}.json"))) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return DevModel::Missing,
        Err(e) => return DevModel::Invalid { reason: e.to_string() },
    };
    let entry: ModelEntry = match serde_json::from_str(&raw) {
        Ok(e) => e,
        Err(e) => return DevModel::Invalid { reason: e.to_string() },
    };
    if !is_plain_name(&entry.file) {
        return DevModel::Invalid { reason: "the entry's file must be a name, not a path".into() };
    }
    let path = dir.join(&entry.file);
    if !path.exists() {
        return DevModel::Invalid { reason: "model file missing".into() };
    }
    if checked(&path, &entry) {
        DevModel::Ready { path, entry: Box::new(entry) }
    } else {
        DevModel::Invalid { reason: "checksum does not match".into() }
    }
}

/// Where each of the text scan's two models is, and its entry (Round 13).
pub type OcrSources = ((PathBuf, ModelEntry), (PathBuf, ModelEntry));

/// The stock text detector and Latin recogniser in the dev folder (Round 13): `None` unless
/// both are there and check out, which turns the feature off rather than failing.
pub fn ocr_sources() -> Option<OcrSources> {
    ocr_sources_in(&dev_dir())
}

pub fn ocr_sources_in(dir: &Path) -> Option<OcrSources> {
    let ready = |role| match dev_model_in(dir, role) {
        DevModel::Ready { path, entry } => Some((path, *entry)),
        DevModel::Missing => None,
        DevModel::Invalid { reason } => {
            tracing::debug!(role, reason, "text model unusable");
            None
        }
    };
    Some((ready("ocr_detection")?, ready("ocr_recognition")?))
}

/// What `ocr_scan.model` records, `"<det id>+<rec id>"`: a new model of either rescans.
pub fn ocr_scan_key() -> Option<String> {
    ocr_scan_key_in(&dev_dir())
}

pub fn ocr_scan_key_in(dir: &Path) -> Option<String> {
    ocr_sources_in(dir).map(|((_, det), (_, rec))| crate::ocr::scan_key(&det.id, &rec.id))
}

/// Smart features and *Read text in photos* are both on (Round 13).
pub fn read_text_on(state: &AppState) -> bool {
    state.settings.read().map(|s| s.smart.enabled && s.smart.experimental.read_text).unwrap_or(false)
}

/// A model entry's `file` is a bare file name, never a path out of its folder.
pub fn is_plain_name(file: &str) -> bool {
    !(file.is_empty() || file.contains(['/', '\\']) || file == "..")
}

/// Whether `path` is the file `entry` names, its SHA-256 computed once per version of the
/// file (size + mtime) and entry: tiers sharing a file pay for one hash.
pub(crate) fn checked(path: &Path, entry: &ModelEntry) -> bool {
    let Ok(meta) = std::fs::metadata(path) else { return false };
    let stamp = (meta.modified().unwrap_or(std::time::UNIX_EPOCH), meta.len());
    let mut checked = DEV_CHECKED.lock().unwrap_or_else(|e| e.into_inner());
    let cache = checked.get_or_insert_with(Default::default);
    match cache.get(path) {
        Some((m, l, sha, ok)) if (*m, *l) == stamp && *sha == entry.sha256 => *ok,
        _ => {
            let ok = verify(path, entry);
            cache.insert(path.to_path_buf(), (stamp.0, stamp.1, entry.sha256.clone(), ok));
            ok
        }
    }
}

/// The loaded models, opened on first use.
#[derive(Default)]
pub struct MlState {
    pub encoder: Mutex<Option<ImageEncoder>>,
    pub faces: Mutex<Option<(FaceDetector, FaceEmbedder)>>,
    /// DINOv2 (Round 4C). The scan's detectors belong to its workers (Round 8A).
    pub pets: Mutex<Option<PetEmbedder>>,
    pub text: Mutex<Option<TextEncoder>>,
    /// The text reader for a photo opened in the viewer (Round 13-3), with its scan key; the
    /// background job opens its own at background priority.
    pub ocr: Mutex<Option<(String, photos_ml::ocr::OcrEngine)>>,
    /// Lumen's own Retouch-contract models (Round 12, 14), one open session per family
    /// (`ray`, `prism`), keyed by file, entry id, SHA-256 and GPU switch so a new export or
    /// tier is opened on the next call.
    pub retoucher: Mutex<std::collections::HashMap<String, (String, Retoucher)>>,
    /// Lumen Facet (Phase 8f), keyed like `retoucher`.
    pub facet: Mutex<Option<(String, photos_ml::facet::FacetModel)>>,
    /// One fetch at a time: a launch and a tick must not download the same file twice.
    pub fetching: std::sync::atomic::AtomicBool,
    /// Why a model's last download failed, by model id, in words for Settings (Round 18 C1).
    /// Cleared when the file arrives; only the latest attempt is kept.
    pub fetch_problems: Mutex<std::collections::HashMap<String, String>>,
}

/// A failed download as the person reads it: what happened and that the switch can be turned
/// off and on again to retry. `error` is `fetch_one`'s raw text.
pub fn fetch_problem(name: &str, error: &str) -> String {
    let why = if error == "HTTP 404 Not Found" || error.starts_with("HTTP 404") {
        "it isn't on the download server yet".to_string()
    } else if error.starts_with("HTTP ") {
        format!("the download server answered {}", error.trim_start_matches("HTTP "))
    } else if error == "checksum mismatch" {
        "the file that arrived was damaged".to_string()
    } else if error.starts_with("incomplete after") {
        "the connection kept dropping".to_string()
    } else {
        "there was no connection to the download server".to_string()
    };
    // "Lumen Facet 1.0.0-d (based on EoMT-S)" reads as "Lumen Facet 1.0.0-d".
    let short = name.split(" (").next().unwrap_or(name);
    format!("Couldn't download {short}: {why}. Turn the switch off and on to try again.")
}

/// The latest failure for `model_id`, if its last download failed.
pub fn fetch_problem_of(state: &AppState, model_id: &str) -> Option<String> {
    state.ml.fetch_problems.lock().ok()?.get(model_id).cloned()
}

pub fn store(app: &AppHandle) -> ModelStore {
    store_in(&app.state::<AppState>())
}

pub fn store_in(state: &AppState) -> ModelStore {
    ModelStore { dir: state.paths.models_dir() }
}

/// Smart features are on (Settings → Smart features).
pub fn enabled(app: &AppHandle) -> bool {
    enabled_in(&app.state::<AppState>())
}

pub fn enabled_in(state: &AppState) -> bool {
    state.settings.read().map(|s| s.smart.enabled).unwrap_or(false)
}

/// The face detector and embedder, opened on first use; `None` while either is missing.
pub fn with_faces<T>(app: &AppHandle, f: impl FnOnce(&mut FaceDetector, &mut FaceEmbedder) -> T) -> Option<T> {
    if !enabled(app) {
        return None;
    }
    let (det, emb) = (manifest().by_role("face_detection")?, manifest().by_role("face_embedding")?);
    let store = store(app);
    if !store.is_ready(det) || !store.is_ready(emb) {
        return None;
    }
    let state = app.state::<AppState>();
    let mut guard = state.ml.faces.lock().ok()?;
    if guard.is_none() {
        match (FaceDetector::open(&store.path_of(det), det), FaceEmbedder::open(&store.path_of(emb), emb)) {
            (Ok(d), Ok(e)) => *guard = Some((d, e)),
            (Err(e), _) | (_, Err(e)) => {
                tracing::warn!("face models: {e}");
                return None;
            }
        }
    }
    guard.as_mut().map(|(d, e)| f(d, e))
}

/// Where the pet detector is (Round 4C), for the scan's workers to open their own copies
/// (Round 8A); `None` while smart features are off or either pet model is missing.
pub fn pet_detector_source(app: &AppHandle) -> Option<(std::path::PathBuf, &'static ModelEntry)> {
    if !enabled(app) {
        return None;
    }
    let (det, emb) = (manifest().by_role("pet_detection")?, manifest().by_role("pet_embedding")?);
    let store = store(app);
    (store.is_ready(det) && store.is_ready(emb)).then(|| (store.path_of(det), det))
}

/// The pet embedder (Round 4C), opened on first use and shared: only photos with an animal
/// reach it, so one copy keeps up with every detector.
pub fn with_pet_embedder<T>(app: &AppHandle, f: impl FnOnce(&mut PetEmbedder) -> T) -> Option<T> {
    if !enabled(app) {
        return None;
    }
    let entry = manifest().by_role("pet_embedding")?;
    let store = store(app);
    if !store.is_ready(entry) {
        return None;
    }
    let state = app.state::<AppState>();
    let mut guard = state.ml.pets.lock().ok()?;
    if guard.is_none() {
        match PetEmbedder::open(&store.path_of(entry), entry) {
            Ok(e) => *guard = Some(e),
            Err(e) => {
                tracing::warn!("pet embedder: {e}");
                return None;
            }
        }
    }
    guard.as_mut().map(f)
}

/// The text encoder for search by meaning, opened on first use.
pub fn with_text<T>(state: &AppState, f: impl FnOnce(&mut TextEncoder) -> T) -> Option<T> {
    if !enabled_in(state) {
        return None;
    }
    let (model, tokenizer) = (manifest().by_role("text_embedding")?, manifest().by_role("tokenizer")?);
    let store = store_in(state);
    if !store.is_ready(model) || !store.is_ready(tokenizer) {
        return None;
    }
    let mut guard = state.ml.text.lock().ok()?;
    if guard.is_none() {
        match TextEncoder::open(&store.path_of(model), &store.path_of(tokenizer), model) {
            Ok(e) => *guard = Some(e),
            Err(e) => {
                tracing::warn!("text encoder: {e}");
                return None;
            }
        }
    }
    guard.as_mut().map(f)
}

/// Runs `f` with the image encoder, opening it on first use; `None` while the model is
/// missing or smart features are off.
pub fn with_encoder<T>(app: &AppHandle, f: impl FnOnce(&mut ImageEncoder) -> T) -> Option<T> {
    if !enabled(app) {
        return None;
    }
    let entry = manifest().by_role("image_embedding")?;
    let store = store(app);
    if !store.is_ready(entry) {
        return None;
    }
    let state = app.state::<AppState>();
    let mut guard = state.ml.encoder.lock().ok()?;
    if guard.is_none() {
        match ImageEncoder::open(&store.path_of(entry), entry) {
            Ok(e) => *guard = Some(e),
            Err(e) => {
                tracing::warn!("image encoder: {e}");
                return None;
            }
        }
    }
    guard.as_mut().map(f)
}

/// Whether ML sessions should try the GPU (Settings → Smart features, Windows only).
pub fn use_gpu(state: &AppState) -> bool {
    cfg!(windows) && state.settings.read().map(|s| s.smart.gpu).unwrap_or(true)
}

/// Runs `f` with the Retouch-contract model at `path` for `family` (`ray`, `prism`), opening it
/// when the family has none open or a different file, entry or GPU switch; `None` when it is
/// made for another layout or fails to open. Ignores every switch: callers check them.
pub fn with_retoucher<T>(state: &AppState, family: &str, path: &Path, entry: &ModelEntry, f: impl FnOnce(&mut Retoucher) -> T) -> Option<T> {
    let gpu = use_gpu(state);
    let key = format!("{}:{}:{}:{gpu}", path.display(), entry.id, entry.sha256);
    let mut guard = state.ml.retoucher.lock().ok()?;
    if guard.get(family).is_none_or(|(k, _)| *k != key) {
        // Drop the old session before opening the next: two are never alive at once.
        guard.remove(family);
        match Retoucher::open(path, entry, photos_core::retouch::RETOUCH_LAYOUT, photos_core::retouch::PARAMS.len(), gpu) {
            Ok(r) => {
                guard.insert(family.to_string(), (key, r));
            }
            Err(e) => {
                tracing::warn!("retouch model {}: {e}", entry.id);
                return None;
            }
        }
    }
    guard.get_mut(family).map(|(_, r)| f(r))
}

/// Whether Lumen wants `entry` on this machine. An Experimental model behind a switch (Facet,
/// Phase 8f) is fetched only once its switch is on, and Settings doesn't list it as missing
/// before; a copy in the dev folder needs no fetch.
pub fn wanted(entry: &ModelEntry, experimental: &photos_core::settings::Experimental) -> bool {
    wanted_with(entry, experimental, || matches!(dev_model(&entry.role), DevModel::Ready { .. }))
}

fn wanted_with(entry: &ModelEntry, experimental: &photos_core::settings::Experimental, in_dev: impl FnOnce() -> bool) -> bool {
    if photos_models::EXPERIMENTAL_ROLES.contains(&entry.role.as_str()) {
        experimental.ai_masks && !in_dev()
    } else {
        true
    }
}

/// The Facet session (Phase 8f), opened on first use and kept per file, entry and GPU switch.
pub fn with_facet<T>(state: &AppState, path: &Path, entry: &ModelEntry, f: impl FnOnce(&mut photos_ml::facet::FacetModel) -> T) -> Option<T> {
    let gpu = use_gpu(state);
    let key = format!("{}:{}:{}:{gpu}", path.display(), entry.id, entry.sha256);
    let mut guard = state.ml.facet.lock().ok()?;
    if guard.as_ref().is_none_or(|(k, _)| *k != key) {
        *guard = None;
        match photos_ml::facet::FacetModel::open(path, entry, gpu) {
            Ok(m) => *guard = Some((key, m)),
            Err(e) => {
                tracing::warn!("facet model {}: {e}", entry.id);
                return None;
            }
        }
    }
    guard.as_mut().map(|(_, m)| f(m))
}

/// Fetch whatever is missing, one file at a time, off the main thread. Returns at once.
pub fn fetch_all(app: &AppHandle) {
    if !enabled(app) {
        return;
    }
    let state = app.state::<AppState>();
    if state.ml.fetching.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let store = store(&app);
        let _ = std::fs::create_dir_all(&store.dir);
        let client = app.state::<AppState>().http.clone();
        let mut fetched = false;
        let mut failed = false;
        let experimental = app.state::<AppState>().settings.read().map(|s| s.smart.experimental.clone()).unwrap_or_default();
        for entry in &manifest().models {
            if store.is_ready(entry) || !wanted(entry, &experimental) {
                continue;
            }
            let problems = &app.state::<AppState>().ml.fetch_problems;
            match fetch_one(&app, &client, &store, entry).await {
                Ok(()) => {
                    fetched = true;
                    if let Ok(mut p) = problems.lock() {
                        p.remove(&entry.id);
                    }
                }
                Err(e) => {
                    tracing::info!(model = %entry.id, "model fetch deferred: {e}");
                    failed = true;
                    if let Ok(mut p) = problems.lock() {
                        p.insert(entry.id.clone(), fetch_problem(entry.name.as_deref().unwrap_or(&entry.id), &e));
                    }
                }
            }
        }
        app.state::<AppState>().ml.fetching.store(false, std::sync::atomic::Ordering::SeqCst);
        // Settings shows the outcome either way (the file, or why not).
        if fetched || failed {
            let _ = app.emit(MODELS_READY, ());
        }
        if fetched {
            crate::embeddings::schedule(&app);
        }
    });
}

/// One model file, resumed where the connection dropped (`photos_models::fetch`), emitting
/// `MODELS_PROGRESS` as bytes arrive.
async fn fetch_one(app: &AppHandle, client: &reqwest::Client, store: &ModelStore, entry: &ModelEntry) -> Result<(), String> {
    let dest = store.path_of(entry);
    let never = std::sync::atomic::AtomicBool::new(false);
    let mut progress = |received: u64| {
        let _ = app.emit(MODELS_PROGRESS, ModelsProgress { file: entry.file.clone(), received, total: entry.bytes });
    };
    photos_models::fetch::fetch_one(client, &manifest().url_of(entry), &dest, entry, &never, &mut progress)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_download_says_why_in_plain_words() {
        let name = "Lumen Facet 1.0.0-d (based on EoMT-S)";
        let missing = fetch_problem(name, "HTTP 404 Not Found");
        assert!(missing.starts_with("Couldn't download Lumen Facet 1.0.0-d: it isn't on the download server yet."), "{missing}");
        assert!(fetch_problem(name, "HTTP 503 Service Unavailable").contains("answered 503"));
        assert!(fetch_problem(name, "checksum mismatch").contains("damaged"));
        assert!(fetch_problem(name, "incomplete after 12 attempts: boom").contains("dropping"));
        assert!(fetch_problem(name, "error sending request").contains("no connection"));
    }

    #[test]
    fn facet_is_wanted_only_with_its_switch_on_and_no_dev_copy() {
        let mut seg = entry(1, "x");
        seg.role = "segmentation".into();
        let off = photos_core::settings::Experimental::default();
        let on = photos_core::settings::Experimental { ai_masks: true, ..Default::default() };
        assert!(!wanted_with(&seg, &off, || false));
        assert!(wanted_with(&seg, &on, || false));
        assert!(!wanted_with(&seg, &on, || true));
        // Every other model is wanted whatever the switches say.
        assert!(wanted_with(&entry(1, "x"), &off, || false));
    }

    fn entry(bytes: u64, sha256: &str) -> ModelEntry {
        ModelEntry {
            id: "t@1".into(),
            name: None,
            role: "image_embedding".into(),
            file: "t.onnx".into(),
            bytes,
            sha256: sha256.into(),
            licence: "MIT".into(),
            input: None,
            aesthetic: None,
            audio: None,
            faces: None,
            text: None,
            denoise: None,
            pets: None,
            retouch: None,
            trained: None,
            ocr_det: None,
            ocr_rec: None,
        }
    }

    /// `LUMEN_DEV_MODELS` is process-wide: the dev-folder tests take turns.
    static DEV_ENV: Mutex<()> = Mutex::new(());

    fn with_dev_dir(f: impl FnOnce(&Path)) {
        let _guard = DEV_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let dev = dir.path().join("dev");
        std::env::set_var("LUMEN_DEV_MODELS", &dev);
        f(&dev);
        std::env::remove_var("LUMEN_DEV_MODELS");
    }

    fn sha_of(bytes: &[u8]) -> String {
        use sha2::Digest;
        format!("{:x}", sha2::Sha256::digest(bytes))
    }

    fn write_dev(dev: &Path, id: &str, file: &str, bytes: &[u8], sha: &str) {
        std::fs::create_dir_all(dev).unwrap();
        if !file.contains(['/', '\\']) {
            std::fs::write(dev.join(file), bytes).unwrap();
        }
        let entry = serde_json::json!({
            "id": id, "role": "retouch", "file": file, "bytes": bytes.len(), "sha256": sha,
            "licence": "Lumen (own weights)",
            "retouch": { "size": 256, "layout": 1, "inputs": ["image", "clip", "clip_flag"], "output": "v" },
            "trained": { "run": "r2", "val_de": 3.81 }
        });
        std::fs::write(dev.join("retouch.json"), entry.to_string()).unwrap();
    }

    #[test]
    fn no_dev_folder_means_missing() {
        with_dev_dir(|dev| {
            assert_eq!(dev_dir(), dev);
            assert!(matches!(dev_model("retouch"), DevModel::Missing));
        });
    }

    #[test]
    fn a_dev_entry_with_its_file_is_ready() {
        with_dev_dir(|dev| {
            write_dev(dev, "retouch-dev-r2", "retouch-dev.onnx", b"model", &sha_of(b"model"));
            match dev_model("retouch") {
                DevModel::Ready { path, entry } => {
                    assert_eq!(path, dev.join("retouch-dev.onnx"));
                    assert_eq!(entry.id, "retouch-dev-r2");
                    assert_eq!(entry.retouch.as_ref().unwrap().layout, 1);
                    assert_eq!(entry.trained.as_ref().unwrap()["run"], "r2");
                }
                other => panic!("expected Ready, got {other:?}"),
            }
        });
    }

    #[test]
    fn a_dev_entry_that_does_not_check_out_is_invalid() {
        with_dev_dir(|dev| {
            write_dev(dev, "a", "retouch-dev.onnx", b"model", &"0".repeat(64));
            assert!(matches!(dev_model("retouch"), DevModel::Invalid { reason } if reason == "checksum does not match"));

            write_dev(dev, "a", "retouch-dev.onnx", b"model", &sha_of(b"model"));
            std::fs::remove_file(dev.join("retouch-dev.onnx")).unwrap();
            assert!(matches!(dev_model("retouch"), DevModel::Invalid { reason } if reason == "model file missing"));

            write_dev(dev, "a", "../retouch-dev.onnx", b"model", &sha_of(b"model"));
            assert!(matches!(dev_model("retouch"), DevModel::Invalid { reason } if reason.contains("not a path")));

            std::fs::write(dev.join("retouch.json"), "{ not json").unwrap();
            assert!(matches!(dev_model("retouch"), DevModel::Invalid { reason } if reason.contains("key must be a string")));
        });
    }

    #[test]
    fn a_new_dev_export_is_seen_without_restarting() {
        with_dev_dir(|dev| {
            write_dev(dev, "r1", "retouch-dev.onnx", b"first", &sha_of(b"first"));
            assert!(matches!(dev_model("retouch"), DevModel::Ready { entry, .. } if entry.id == "r1"));
            write_dev(dev, "r2", "retouch-dev.onnx", b"second model", &sha_of(b"second model"));
            assert!(matches!(dev_model("retouch"), DevModel::Ready { entry, .. } if entry.id == "r2"));
            // Same entry, file swapped for one that does not match: caught, not served from cache.
            std::fs::write(dev.join("retouch-dev.onnx"), b"tampered").unwrap();
            assert!(matches!(dev_model("retouch"), DevModel::Invalid { .. }));
        });
    }

    fn write_ocr(dev: &Path, role: &str, id: &str, bytes: &[u8], sha: &str) {
        std::fs::create_dir_all(dev).unwrap();
        let file = format!("{role}.onnx");
        std::fs::write(dev.join(&file), bytes).unwrap();
        let entry = serde_json::json!({ "id": id, "role": role, "file": file, "bytes": bytes.len(), "sha256": sha, "licence": "Apache-2.0" });
        std::fs::write(dev.join(format!("{role}.json")), entry.to_string()).unwrap();
    }

    /// Round 13: the text scan needs both stock models in the dev folder; anything less is "off".
    #[test]
    fn ocr_needs_both_text_models() {
        let dir = tempfile::tempdir().unwrap();
        let dev = dir.path();
        assert!(ocr_sources_in(dev).is_none(), "an empty folder");
        write_ocr(dev, "ocr_detection", "pp-ocrv5-mobile-det@1", b"det", &sha_of(b"det"));
        assert!(ocr_sources_in(dev).is_none(), "the recogniser is missing");
        write_ocr(dev, "ocr_recognition", "pp-ocrv5-latin-mobile-rec@1", b"rec", "0000");
        assert!(ocr_sources_in(dev).is_none(), "a checksum that does not match");
        write_ocr(dev, "ocr_recognition", "pp-ocrv5-latin-mobile-rec@1", b"rec", &sha_of(b"rec"));
        let ((det, _), (rec, rec_entry)) = ocr_sources_in(dev).expect("both ready");
        assert_eq!((det, rec), (dev.join("ocr_detection.onnx"), dev.join("ocr_recognition.onnx")));
        assert_eq!(rec_entry.id, "pp-ocrv5-latin-mobile-rec@1");
        assert_eq!(ocr_scan_key_in(dev).as_deref(), Some("pp-ocrv5-mobile-det@1+pp-ocrv5-latin-mobile-rec@1/736"));
    }
}
