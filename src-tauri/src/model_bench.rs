//! How long each model × tier takes on this PC (Round 14 spec §3). Timings are cached in
//! `model-bench.json` keyed by the file's sha, the tier and a machine fingerprint, so a new GPU,
//! more RAM or the GPU switch re-times rather than reusing old numbers.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use photos_ml::MlError;
use serde::{Deserialize, Serialize};

use photos_core::settings::ExperimentalTasks;
use photos_core::retouch::{PARAMS, RETOUCH_LAYOUT};
use photos_jobs::{CancelToken, Lane};
use photos_ml::manifest::{manifest, ModelEntry};
use photos_ml::retouch::Retoucher;
use tauri::{AppHandle, Manager};
use ts_rs::TS;

use crate::develop_retouch::AiTaskKind;
use crate::model_catalogue::{CatalogueModel, Precision, Tier, TierState};
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fingerprint {
    pub os: String,
    pub cpu: String,
    pub cores: usize,
    pub ram_gib: u64,
    pub gpu: String,
    pub ep: String,
}

impl Fingerprint {
    /// A stable join; any field changing makes every timing re-measure.
    pub fn key(&self) -> String {
        format!("{}|{}|{}|{}|{}|{}", self.os, self.cpu, self.cores, self.ram_gib, self.gpu, self.ep)
    }

    /// This machine with `ep`; everything but the EP is read once per run.
    pub fn current(ep: &str) -> Self {
        static BASE: std::sync::OnceLock<Fingerprint> = std::sync::OnceLock::new();
        Fingerprint { ep: ep.to_string(), ..BASE.get_or_init(Self::read).clone() }
    }

    fn read() -> Self {
        use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
        let sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::nothing())
                .with_memory(MemoryRefreshKind::nothing().with_ram()),
        );
        let cpu = sys.cpus().first().map(|c| c.brand().trim().to_string()).unwrap_or_default();
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        let ram_gib = (sys.total_memory() as f64 / (1u64 << 30) as f64).round() as u64;
        Fingerprint {
            os: System::long_os_version().unwrap_or_default(),
            cpu,
            cores,
            ram_gib,
            gpu: gpu_name().to_string(),
            ep: String::new(),
        }
    }
}

/// The GPU adapter name(s), asked once per run; `""` when unknown.
fn gpu_name() -> &'static str {
    static GPU: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    GPU.get_or_init(|| {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            std::process::Command::new("powershell")
                .args(["-NoProfile", "-Command", "(Get-CimInstance Win32_VideoController).Name -join ';'"])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default()
        }
        #[cfg(not(windows))]
        {
            String::new()
        }
    })
}

fn cache_key(sha: &str, tier: Tier, fp: &Fingerprint) -> String {
    format!("{sha}:{}:{}", tier.as_str(), fp.key())
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchCache {
    #[serde(default)]
    pub entries: BTreeMap<String, f32>,
    /// The EP each file (by sha) really opened on with the GPU switch on: one file can fall back
    /// to the CPU while the others run on DirectML, so there is no single EP for the machine.
    #[serde(default)]
    pub ep_by_file: BTreeMap<String, String>,
    /// Tiers that could not run here (`cache_key` → why). They are skipped, not retried, until
    /// *Time again*.
    #[serde(default)]
    pub failed: BTreeMap<String, String>,
    /// Before Round 15 one EP was kept for every file, written by whichever file opened last.
    /// Read once to fill `ep_by_file`, never written again.
    #[serde(default, skip_serializing)]
    pub gpu_ep: Option<String>,
}

/// The EP the app asks for with the GPU switch on or off.
pub fn requested_ep(gpu: bool) -> &'static str {
    if gpu {
        "directml"
    } else {
        "cpu"
    }
}

/// The EP the file `sha` runs on: CPU with the GPU switch off, else what it opened last time.
pub fn ep_for(gpu: bool, cache: &BenchCache, sha: &str) -> String {
    if !gpu {
        return "cpu".into();
    }
    cache.ep_by_file.get(sha).cloned().unwrap_or_else(|| "directml".into())
}

impl BenchCache {
    /// A missing, unreadable or half-written file reads as empty: the models are timed again.
    pub fn load(path: &Path) -> Self {
        let mut c: Self = std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        c.adopt_legacy_ep();
        c
    }

    /// A pre-Round-15 file: each sha takes the EP its timings were stored under, DirectML first
    /// (the global said "cpu" when only one file had fallen back), so nothing is timed again.
    fn adopt_legacy_ep(&mut self) {
        let Some(legacy) = self.gpu_ep.take() else { return };
        let mut seen: BTreeMap<String, (bool, bool)> = BTreeMap::new();
        for key in self.entries.keys() {
            let (Some((sha, _)), Some((_, ep))) = (key.split_once(':'), key.rsplit_once('|')) else { continue };
            let s = seen.entry(sha.to_string()).or_default();
            s.0 |= ep == "directml";
            s.1 |= ep == "cpu";
        }
        for (sha, (dml, cpu)) in seen {
            let ep = if dml { "directml" } else if cpu && legacy == "cpu" { "cpu" } else { continue };
            self.ep_by_file.entry(sha).or_insert_with(|| ep.into());
        }
    }

    /// Written to a temp file then renamed, so a kill mid-save never leaves a torn cache.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, path)
    }

    pub fn get(&self, sha: &str, tier: Tier, fp: &Fingerprint) -> Option<f32> {
        self.entries.get(&cache_key(sha, tier, fp)).copied()
    }

    /// `base` (the EP the app asks for) with the EP the file `sha` really runs on.
    pub fn resolved(&self, sha: &str, base: &Fingerprint) -> Fingerprint {
        Fingerprint { ep: ep_for(base.ep != "cpu", self, sha), ..base.clone() }
    }

    /// The median of `sha`'s `tier` under the EP that file runs on.
    pub fn time(&self, sha: &str, tier: Tier, base: &Fingerprint) -> Option<f32> {
        self.get(sha, tier, &self.resolved(sha, base))
    }

    /// Why `sha`'s `tier` could not run here, if it could not (under its own EP, or under the
    /// asked-for EP when the file would not even open).
    pub fn failure(&self, sha: &str, tier: Tier, base: &Fingerprint) -> Option<&str> {
        self.failed
            .get(&cache_key(sha, tier, &self.resolved(sha, base)))
            .or_else(|| self.failed.get(&cache_key(sha, tier, base)))
            .map(String::as_str)
    }

    pub fn put(&mut self, sha: &str, tier: Tier, fp: &Fingerprint, ms: f32) {
        self.entries.insert(cache_key(sha, tier, fp), ms);
    }
}

/// Untimed runs before timing: DirectML builds and tunes its graph over the first few calls,
/// and one warm-up left that cost in the median (Round 15).
pub const BENCH_WARMUPS: usize = 3;
/// Timed runs; the median of these is the tier's time.
pub const BENCH_TIMED: usize = 7;
/// How long the bench waits for a running transcode before timing anyway.
const TRANSCODE_WAIT: Duration = Duration::from_secs(300);
/// The user's last render this recent counts as busy (video prep's step-aside).
const QUIET: Duration = Duration::from_millis(600);

/// A deterministic test picture of `size`² × 4: a diagonal gradient per channel with noise, so
/// no kernel takes a constant-input shortcut and the timing is what a photo costs.
pub fn bench_image(size: usize) -> Vec<f32> {
    let mut seed = 0x2545_f491_u32;
    let mut noise = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
    };
    let n = size.max(1) as f32;
    let mut out = Vec::with_capacity(4 * size * size);
    for c in 0..4 {
        for y in 0..size {
            for x in 0..size {
                let g = (x as f32 / n * (0.6 + 0.1 * c as f32) + y as f32 / n * 0.4) * 0.8 + 0.1;
                out.push((g + 0.15 * noise()).clamp(0.0, 1.0));
            }
        }
    }
    out
}

/// Waits (polling every `poll`) until the user is idle and no transcode runs, so neither inflates
/// the timings. A transcode is waited on for at most `transcode_max`; false when cancelled.
pub fn wait_quiet(cancel: &CancelToken, user_busy: impl Fn() -> bool, transcoding: impl Fn() -> bool, poll: Duration, transcode_max: Duration) -> bool {
    let start = Instant::now();
    loop {
        if cancel.is_cancelled() {
            return false;
        }
        if !user_busy() && (!transcoding() || start.elapsed() >= transcode_max) {
            return true;
        }
        std::thread::sleep(poll);
    }
}

/// Runs `warmups` untimed then `timed` timed calls and returns the median in ms.
pub fn median_ms(mut run: impl FnMut() -> Result<(), MlError>, warmups: usize, timed: usize) -> Result<f32, MlError> {
    for _ in 0..warmups {
        run()?;
    }
    let mut times = Vec::with_capacity(timed.max(1));
    for _ in 0..timed.max(1) {
        let t = std::time::Instant::now();
        run()?;
        times.push(t.elapsed().as_secs_f32() * 1000.0);
    }
    times.sort_by(f32::total_cmp);
    Ok(times[times.len() / 2])
}

/// `config_dir/JL-Studios/Lumen/model-bench.json`.
pub fn cache_path() -> PathBuf {
    photos_core::paths::AppPaths::resolve()
        .map(|p| p.app_data)
        .unwrap_or_else(|_| photos_core::paths::AppPaths::app_data_under(Path::new(".")))
        .join("model-bench.json")
}

/// The tier *Automatic* runs for `kind`: the highest whose median fits the budget, else the
/// lowest timed; `(High, true)` while any ready tier is still untimed (*Testing this PC…*).
pub fn recommended(kind: AiTaskKind, timings: &[(Tier, Option<f32>)]) -> (Tier, bool) {
    let mut timed: Vec<(Tier, f32)> = Vec::with_capacity(timings.len());
    for (tier, ms) in timings {
        match ms {
            Some(ms) => timed.push((*tier, *ms)),
            None => return (Tier::High, true),
        }
    }
    timed.sort_by_key(|(t, _)| *t);
    let budget = kind.budget_ms().unwrap_or(f32::INFINITY);
    let fits = timed.iter().rev().find(|(_, ms)| *ms <= budget).map(|(t, _)| *t);
    (fits.or(timed.first().map(|(t, _)| *t)).unwrap_or(Tier::High), false)
}

/// Each ready tier of `model` that can run here, with its median under the EP its file runs on
/// (from the asked-for `base`), if timed. A tier that failed is left out: it is not runnable.
pub fn timings(model: &CatalogueModel, cache: &BenchCache, base: &Fingerprint) -> Vec<(Tier, Option<f32>)> {
    Tier::ALL
        .into_iter()
        .filter_map(|t| model.ready(t).map(|(_, entry, _)| (t, &entry.sha256)))
        .filter(|(t, sha)| cache.failure(sha, *t, base).is_none())
        .map(|(t, sha)| (t, cache.time(sha, t, base)))
        .collect()
}

/// An open model ready to be timed: the EP it really opened on and one pass.
pub struct BenchSession {
    pub ep: String,
    pub run: Box<dyn FnMut() -> Result<(), MlError>>,
}

/// One model file and the tiers (with their passes) still to time on it.
type FileTiers<'a> = (&'a ModelEntry, Vec<(Tier, u8)>);

/// Times every ready tier of `all` neither timed nor failed in `cache` under `base`, the EP the
/// app asks for (every one when `force`), one session per file; an ultra timing is `tta` passes.
/// Each timing is stored under the EP its file really opened on, and a tier that errors is
/// recorded in `failed` so it is never retried in a loop. Returns how many tiers were timed.
pub fn bench_in(
    all: &[CatalogueModel],
    cache: &mut BenchCache,
    base: &Fingerprint,
    force: bool,
    cancel: &CancelToken,
    mut open: impl FnMut(&Path, &ModelEntry) -> Result<BenchSession, MlError>,
) -> usize {
    // Group the ready tiers by file: tiers sharing one (high, ultra) share a session.
    let mut files: BTreeMap<PathBuf, FileTiers> = BTreeMap::new();
    for model in all {
        for tier in Tier::ALL {
            let Some((path, entry, tta)) = model.ready(tier) else { continue };
            let sha = &entry.sha256;
            if !force && (cache.time(sha, tier, base).is_some() || cache.failure(sha, tier, base).is_some()) {
                continue;
            }
            files.entry(path.to_path_buf()).or_insert_with(|| (entry, Vec::new())).1.push((tier, tta));
        }
    }
    let mut timed = 0;
    for (path, (entry, tiers)) in files {
        if cancel.is_cancelled() {
            break;
        }
        let sha = &entry.sha256;
        let mut session = match open(&path, entry) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("model bench: {} would not open: {e}", entry.id);
                for (tier, _) in tiers {
                    cache.failed.insert(cache_key(sha, tier, base), e.to_string());
                }
                continue;
            }
        };
        // With the switch off every file is on the CPU anyway; only a GPU run says where it went.
        if base.ep != "cpu" {
            cache.ep_by_file.insert(sha.clone(), session.ep.clone());
        }
        let used = Fingerprint { ep: session.ep.clone(), ..base.clone() };
        for (tier, tta) in tiers {
            if cancel.is_cancelled() {
                break;
            }
            let run = &mut session.run;
            match median_ms(|| (0..tta.max(1)).try_for_each(|_| run()), BENCH_WARMUPS, BENCH_TIMED) {
                Ok(ms) => {
                    cache.put(sha, tier, &used, ms);
                    cache.failed.remove(&cache_key(sha, tier, &used));
                    cache.failed.remove(&cache_key(sha, tier, base));
                    timed += 1;
                }
                Err(e) => {
                    tracing::warn!("model bench: {} {}: {e}", entry.id, tier.as_str());
                    cache.failed.insert(cache_key(sha, tier, &used), e.to_string());
                }
            }
        }
    }
    timed
}

/// One tier of a model as the Experimental tab shows it.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TierView {
    pub tier: Tier,
    pub ready: bool,
    pub reason: Option<String>,
    pub precision: Option<Precision>,
    pub tta: u8,
    pub val_de: Option<f32>,
    pub ms: Option<f32>,
    /// Where `ms` was measured: `"cpu"` or a GPU EP such as `"directml"`.
    pub ep: Option<String>,
    /// Why this tier could not run here; it is then never chosen.
    pub failed: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModelView {
    pub id: String,
    pub name: String,
    pub version: String,
    pub legacy: bool,
    pub tiers: Vec<TierView>,
}

/// One row of Settings → Experimental.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExperimentalTask {
    pub task: AiTaskKind,
    pub family: String,
    /// Has an on/off switch (Auto, Retouch).
    pub switchable: bool,
    /// Spectrum: the built-in model's name when the CLIP models are present.
    pub builtin: Option<String>,
    /// Newest first.
    pub models: Vec<ModelView>,
    /// The model id the task runs.
    pub model: Option<String>,
    pub recommended: Tier,
    pub testing: bool,
    /// The tier the task runs.
    pub tier: Option<Tier>,
    pub budget_ms: Option<f32>,
    /// Why the model's last download failed, in words for the row (Facet, Round 18 C1).
    pub problem: Option<String>,
}

pub const SPECTRUM_BUILTIN: &str = "Lumen Spectrum 1.0 (based on CLIP ViT-B/32)";
/// Round 13: the stock PP-OCRv5 detector + Latin recogniser from the dev folder, until a models release.
pub const GLYPH_BUILTIN: &str = "Lumen Glyph 1.0.0-d (based on PP-OCRv5)";
pub const TASKS: [AiTaskKind; 5] = [AiTaskKind::Auto, AiTaskKind::Retouch, AiTaskKind::Search, AiTaskKind::Text, AiTaskKind::Masks];

/// The rows for `all` with the timings in `cache` under `fp` and the person's choices.
/// `clip_ready` / `ocr_ready`: Spectrum's and Glyph's stock models are present; `facet`: the
/// masks model's name when it is (Phase 8f).
pub fn catalog_rows(all: &[CatalogueModel], cache: &BenchCache, fp: &Fingerprint, tasks: &ExperimentalTasks, clip_ready: bool, ocr_ready: bool, facet: Option<&str>) -> Vec<ExperimentalTask> {
    TASKS
        .into_iter()
        .map(|kind| {
            let family = kind.family();
            let models = crate::model_catalogue::family_models(all, family);
            let choice = match kind {
                AiTaskKind::Auto => tasks.auto.clone(),
                AiTaskKind::Retouch => tasks.retouch.clone(),
                _ => Default::default(),
            };
            let recommend = |m: &CatalogueModel| recommended(kind, &timings(m, cache, fp)).0;
            let effective = crate::develop_retouch::resolve_in(all, kind, &choice, recommend);
            let (recommended, testing) = match &effective {
                Some((m, _)) => recommended(kind, &timings(m, cache, fp)),
                None => (Tier::High, false),
            };
            ExperimentalTask {
                task: kind,
                family: family.to_string(),
                switchable: matches!(kind, AiTaskKind::Auto | AiTaskKind::Retouch | AiTaskKind::Text | AiTaskKind::Masks),
                builtin: match kind {
                    AiTaskKind::Search if clip_ready => Some(SPECTRUM_BUILTIN.to_string()),
                    AiTaskKind::Text if ocr_ready => Some(GLYPH_BUILTIN.to_string()),
                    AiTaskKind::Masks => facet.map(str::to_string),
                    _ => None,
                },
                models: models.iter().map(|m| model_view(m, cache, fp)).collect(),
                model: effective.as_ref().map(|(m, _)| m.id.clone()),
                recommended,
                testing,
                tier: effective.map(|(_, t)| t),
                budget_ms: kind.budget_ms(),
                problem: None,
            }
        })
        .collect()
}

fn model_view(m: &CatalogueModel, cache: &BenchCache, fp: &Fingerprint) -> ModelView {
    let tiers = m
        .tiers
        .iter()
        .map(|(tier, state)| match state {
            TierState::Ready { entry, precision, tta, val_de, .. } => {
                let failed = cache.failure(&entry.sha256, *tier, fp).map(str::to_string);
                let ms = cache.time(&entry.sha256, *tier, fp).filter(|_| failed.is_none());
                TierView {
                    tier: *tier,
                    ready: true,
                    reason: None,
                    precision: Some(*precision),
                    tta: *tta,
                    val_de: *val_de,
                    ms,
                    ep: ms.map(|_| cache.resolved(&entry.sha256, fp).ep),
                    failed,
                }
            }
            TierState::Invalid { reason } => TierView {
                tier: *tier,
                ready: false,
                reason: Some(reason.clone()),
                precision: None,
                tta: 1,
                val_de: None,
                ms: None,
                ep: None,
                failed: None,
            },
        })
        .collect();
    ModelView { id: m.id.clone(), name: m.name.clone(), version: m.version.clone(), legacy: m.legacy, tiers }
}

/// The families timed: those that run on the Retouch contract.
const BENCHED: [AiTaskKind; 2] = [AiTaskKind::Auto, AiTaskKind::Retouch];

/// Times what is missing from `model-bench.json` for this machine (all when `force`) on the
/// session the app would use: same file, same GPU switch.
pub fn bench_all(state: &AppState, force: bool, cancel: &CancelToken) {
    let path = cache_path();
    let mut cache = BenchCache::load(&path);
    let gpu = crate::models::use_gpu(state);
    let fp = Fingerprint::current(requested_ep(gpu));
    let all: Vec<_> = crate::model_catalogue::catalogue()
        .into_iter()
        .filter(|m| BENCHED.iter().any(|k| k.family() == m.family))
        .collect();
    let before = cache.clone();
    // First the user goes idle (video prep carries on meanwhile); then video prep starts nothing
    // new while this times, and a transcode running already is waited out: one taking every core
    // made CPU timings several times too slow.
    let poll = Duration::from_millis(250);
    if !wait_quiet(cancel, || photos_jobs::user_busy(QUIET), || false, poll, TRANSCODE_WAIT) {
        return;
    }
    let _pause = crate::video_prep::BenchPause::begin();
    if !wait_quiet(cancel, || photos_jobs::user_busy(QUIET), crate::video_prep::transcoding, poll, TRANSCODE_WAIT) {
        return;
    }
    let timed = bench_in(&all, &mut cache, &fp, force, cancel, |path, entry| {
        let mut r = Retoucher::open(path, entry, RETOUCH_LAYOUT, PARAMS.len(), gpu)?;
        let size = r.size();
        let image = bench_image(size);
        Ok(BenchSession { ep: r.ep().to_string(), run: Box::new(move || r.run(&image, None).map(|_| ())) })
    });
    if cache != before {
        tracing::info!(timed, failed = cache.failed.len(), "model bench: timed model tiers on this PC");
        if let Err(e) = cache.save(&path) {
            tracing::warn!("model bench: could not save {}: {e}", path.display());
        }
    }
}

/// Queues `bench_all` on the maintenance lane after `settle` (start-up), once at a time.
pub fn schedule(app: &AppHandle, force: bool, settle: Duration) {
    let a = app.clone();
    app.state::<AppState>().jobs.submit("maintenance:model-bench", Lane::Maintenance, move |cancel| {
        let until = Instant::now() + settle;
        while Instant::now() < until {
            if cancel.is_cancelled() {
                return;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        bench_all(&a.state::<AppState>(), force, cancel);
    });
}

/// The tier *Automatic* runs for `model` on this machine, from the cached timings.
pub fn recommended_now(state: &AppState, kind: AiTaskKind, model: &CatalogueModel) -> Tier {
    let cache = BenchCache::load(&cache_path());
    let fp = Fingerprint::current(requested_ep(crate::models::use_gpu(state)));
    recommended(kind, &timings(model, &cache, &fp)).0
}

/// Settings → Experimental's rows; schedules a bench when a ready tier is neither timed nor
/// failed, at most once per app run per fingerprint (the tab refetches while testing, and a
/// refetch must never queue the bench again).
pub fn experimental_catalog(app: &AppHandle, state: &AppState) -> Vec<ExperimentalTask> {
    static SCHEDULED: std::sync::Mutex<std::collections::BTreeSet<String>> = std::sync::Mutex::new(std::collections::BTreeSet::new());
    let cache = BenchCache::load(&cache_path());
    let fp = Fingerprint::current(requested_ep(crate::models::use_gpu(state)));
    let tasks = state.settings.read().map(|s| s.smart.experimental.tasks.clone()).unwrap_or_default();
    let clip_ready = manifest().by_role("image_embedding").is_some_and(|e| crate::models::store_in(state).is_ready(e));
    let ocr_ready = crate::models::ocr_sources().is_some();
    let facet = crate::facet::model_source(state).map(|(_, e)| e.name.unwrap_or(e.id));
    let mut rows = catalog_rows(&crate::model_catalogue::catalogue(), &cache, &fp, &tasks, clip_ready, ocr_ready, facet.as_deref());
    // A Facet download that failed says why on its row, until the file arrives.
    if facet.is_none() {
        if let Some(entry) = manifest().by_role("segmentation") {
            if let Some(row) = rows.iter_mut().find(|r| r.task == AiTaskKind::Masks) {
                row.problem = crate::models::fetch_problem_of(state, &entry.id);
            }
        }
    }
    if rows.iter().any(|r| r.testing) && SCHEDULED.lock().is_ok_and(|mut s| s.insert(fp.key())) {
        schedule(app, false, Duration::ZERO);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_catalogue::tests::{write_legacy, write_model};

    fn fp(ep: &str) -> Fingerprint {
        Fingerprint {
            os: "Windows 11".into(),
            cpu: "Ryzen".into(),
            cores: 16,
            ram_gib: 32,
            gpu: "RTX".into(),
            ep: ep.into(),
        }
    }

    #[test]
    fn cache_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model-bench.json");
        let mut c = BenchCache::default();
        c.put("abc", Tier::High, &fp("cpu"), 412.5);
        c.save(&path).unwrap();
        let back = BenchCache::load(&path);
        assert_eq!(back.get("abc", Tier::High, &fp("cpu")), Some(412.5));
        assert!(!dir.path().join("model-bench.json.tmp").exists());
    }

    #[test]
    fn a_corrupt_cache_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model-bench.json");
        std::fs::write(&path, "{\"entr").unwrap();
        assert!(BenchCache::load(&path).entries.is_empty());
        assert!(BenchCache::load(&dir.path().join("absent.json")).entries.is_empty());
    }

    #[test]
    fn fingerprint_changes_with_the_ep() {
        assert_ne!(fp("cpu").key(), fp("directml").key());
        assert_eq!(fp("cpu").key(), fp("cpu").key());
    }

    #[test]
    fn keys_separate_tiers_and_shas() {
        let mut c = BenchCache::default();
        c.put("abc", Tier::High, &fp("cpu"), 100.0);
        c.put("abc", Tier::Ultra, &fp("cpu"), 500.0);
        c.put("def", Tier::High, &fp("cpu"), 200.0);
        assert_eq!(c.get("abc", Tier::High, &fp("cpu")), Some(100.0));
        assert_eq!(c.get("abc", Tier::Ultra, &fp("cpu")), Some(500.0));
        assert_eq!(c.get("def", Tier::High, &fp("cpu")), Some(200.0));
        assert_eq!(c.get("abc", Tier::Low, &fp("cpu")), None);
        assert_eq!(c.get("abc", Tier::High, &fp("directml")), None);
    }

    #[test]
    fn median_of_five_ignores_the_warmup() {
        let mut calls = 0;
        let ms = median_ms(
            || {
                calls += 1;
                std::thread::sleep(std::time::Duration::from_millis(if calls == 1 { 50 } else { 1 }));
                Ok(())
            },
            1,
            5,
        )
        .unwrap();
        assert_eq!(calls, 6);
        assert!(ms < 20.0, "median {ms} ms");
    }

    #[test]
    fn recommended_picks_the_highest_tier_in_budget() {
        let t = |l, h, u| [(Tier::Low, Some(l)), (Tier::High, Some(h)), (Tier::Ultra, Some(u))];
        assert_eq!(recommended(AiTaskKind::Auto, &t(50.0, 200.0, 900.0)), (Tier::Ultra, false));
        assert_eq!(recommended(AiTaskKind::Auto, &t(50.0, 200.0, 4100.0)), (Tier::High, false));
        assert_eq!(recommended(AiTaskKind::Auto, &t(1100.0, 2000.0, 9000.0)), (Tier::Low, false));
        // Retouch's 1500 ms budget lets a 1200 ms Ultra through where Auto's does not.
        assert_eq!(recommended(AiTaskKind::Auto, &t(50.0, 200.0, 1200.0)), (Tier::High, false));
        assert_eq!(recommended(AiTaskKind::Retouch, &t(50.0, 200.0, 1200.0)), (Tier::Ultra, false));
    }

    #[test]
    fn recommended_waits_for_every_timing() {
        assert_eq!(recommended(AiTaskKind::Auto, &[(Tier::Low, Some(10.0)), (Tier::High, None)]), (Tier::High, true));
        // Only the legacy High, too slow: the lowest timed tier.
        assert_eq!(recommended(AiTaskKind::Auto, &[(Tier::High, Some(3000.0))]), (Tier::High, false));
    }

    #[test]
    fn catalog_rows_for_an_empty_folder() {
        let dir = tempfile::tempdir().unwrap();
        let all = crate::model_catalogue::catalogue_in(dir.path());
        let rows = catalog_rows(&all, &BenchCache::default(), &fp("cpu"), &ExperimentalTasks::default(), false, false, None);
        let families: Vec<_> = rows.iter().map(|r| r.family.as_str()).collect();
        assert_eq!(families, ["ray", "prism", "spectrum", "glyph", "facet"]);
        assert!(rows.iter().all(|r| r.models.is_empty() && r.model.is_none() && r.tier.is_none() && !r.testing));
        assert_eq!(rows.iter().map(|r| r.switchable).collect::<Vec<_>>(), [true, true, false, true, true]);
        assert_eq!(rows[0].budget_ms, Some(1000.0));
        assert_eq!(rows[1].budget_ms, Some(1500.0));
        assert_eq!(rows[2].builtin, None);
        assert_eq!(rows[3].builtin, None);
        let rows = catalog_rows(&all, &BenchCache::default(), &fp("cpu"), &ExperimentalTasks::default(), true, false, None);
        assert_eq!(rows[2].builtin.as_deref(), Some(SPECTRUM_BUILTIN));
        assert_eq!(rows[3].builtin, None);
    }

    /// Round 13: Glyph is switched on its row and names the stock text models once both are there.
    #[test]
    fn glyph_names_the_text_models_when_ready() {
        let dir = tempfile::tempdir().unwrap();
        let all = crate::model_catalogue::catalogue_in(dir.path());
        let rows = catalog_rows(&all, &BenchCache::default(), &fp("cpu"), &ExperimentalTasks::default(), false, true, None);
        assert_eq!(rows[3].task, AiTaskKind::Text);
        assert_eq!(rows[3].builtin.as_deref(), Some(GLYPH_BUILTIN));
        assert!(rows[3].switchable);
    }

    /// Phase 8f: Facet has a switch and names its model once it is here.
    #[test]
    fn facet_names_its_model_when_ready() {
        let dir = tempfile::tempdir().unwrap();
        let all = crate::model_catalogue::catalogue_in(dir.path());
        let rows = catalog_rows(&all, &BenchCache::default(), &fp("cpu"), &ExperimentalTasks::default(), false, false, None);
        assert_eq!(rows[4].task, AiTaskKind::Masks);
        assert!(rows[4].switchable);
        assert_eq!(rows[4].builtin, None);
        let rows = catalog_rows(&all, &BenchCache::default(), &fp("cpu"), &ExperimentalTasks::default(), false, false, Some("Lumen Facet 1.0.0-d"));
        assert_eq!(rows[4].builtin.as_deref(), Some("Lumen Facet 1.0.0-d"));
    }

    #[test]
    fn catalog_rows_show_tiers_timings_and_the_choice() {
        let dir = tempfile::tempdir().unwrap();
        write_legacy(dir.path(), b"legacy ray");
        let all = crate::model_catalogue::catalogue_in(dir.path());
        let mut cache = BenchCache::default();
        let rows = catalog_rows(&all, &cache, &fp("cpu"), &ExperimentalTasks::default(), false, false, None);
        let auto = &rows[0];
        assert_eq!(auto.models.len(), 1);
        assert!(auto.models[0].legacy);
        assert_eq!(auto.models[0].tiers.len(), 1);
        assert_eq!(auto.models[0].tiers[0].tier, Tier::High);
        assert_eq!((auto.recommended, auto.testing, auto.tier), (Tier::High, true, Some(Tier::High)));
        assert_eq!(auto.model.as_deref(), Some(all[0].id.as_str()));
        let sha = all[0].ready(Tier::High).unwrap().1.sha256.clone();
        cache.put(&sha, Tier::High, &fp("cpu"), 640.0);
        let rows = catalog_rows(&all, &cache, &fp("cpu"), &ExperimentalTasks::default(), false, false, None);
        assert_eq!(rows[0].models[0].tiers[0].ms, Some(640.0));
        assert!(!rows[0].testing);
        assert!(rows[1].models.is_empty(), "the legacy Ray is not a Prism");
    }

    fn three_tiers(dir: &Path) -> Vec<CatalogueModel> {
        write_model(
            dir,
            "ray",
            "lumen-ray-1.0.2-d",
            "1.0.2-d",
            &[
                ("low", "low.onnx", b"low", "fp16", 1, None),
                ("high", "high.onnx", b"high", "fp32", 1, Some(3.9)),
                ("ultra", "high.onnx", b"high", "fp32", 5, Some(3.7)),
            ],
        );
        crate::model_catalogue::catalogue_in(dir)
    }

    #[test]
    fn bench_all_times_only_whats_missing() {
        let dir = tempfile::tempdir().unwrap();
        let all = three_tiers(dir.path());
        let mut cache = BenchCache::default();
        let opens = std::cell::Cell::new(0);
        let passes = std::rc::Rc::new(std::cell::Cell::new(0));
        let open = |_: &Path, _: &ModelEntry| {
            opens.set(opens.get() + 1);
            let p = passes.clone();
            Ok(BenchSession {
                ep: "cpu".into(),
                run: Box::new(move || {
                    p.set(p.get() + 1);
                    Ok(())
                }),
            })
        };
        let cancel = CancelToken::new();
        assert_eq!(bench_in(&all, &mut cache, &fp("cpu"), false, &cancel, open), 3);
        assert_eq!(opens.get(), 2, "one session per file");
        // low 10 + high 10 + ultra 10 × 5 passes.
        assert_eq!(passes.get(), 10 + 10 + 50);
        assert_eq!(cache.entries.len(), 3);
        let again = |_: &Path, _: &ModelEntry| {
            opens.set(opens.get() + 1);
            Ok(BenchSession { ep: "cpu".into(), run: Box::new(|| Ok(())) })
        };
        assert_eq!(bench_in(&all, &mut cache, &fp("cpu"), false, &cancel, again), 0);
        assert_eq!(opens.get(), 2, "nothing missing, nothing opened");
        let forced = |_: &Path, _: &ModelEntry| Ok(BenchSession { ep: "cpu".into(), run: Box::new(|| Ok(())) });
        assert_eq!(bench_in(&all, &mut cache, &fp("cpu"), true, &cancel, forced), 3);
    }

    #[test]
    fn a_gpu_that_falls_back_is_timed_as_cpu() {
        let dir = tempfile::tempdir().unwrap();
        let all = three_tiers(dir.path());
        let mut cache = BenchCache::default();
        let open = |_: &Path, _: &ModelEntry| Ok(BenchSession { ep: "cpu".into(), run: Box::new(|| Ok(())) });
        bench_in(&all, &mut cache, &fp("directml"), false, &CancelToken::new(), open);
        let sha = all[0].ready(Tier::High).unwrap().1.sha256.clone();
        assert_eq!(ep_for(true, &cache, &sha), "cpu");
        assert_eq!(ep_for(false, &cache, &sha), "cpu");
        assert_eq!(ep_for(true, &BenchCache::default(), &sha), "directml");
        assert!(cache.get(&sha, Tier::High, &fp("cpu")).is_some());
        assert!(cache.get(&sha, Tier::High, &fp("directml")).is_none());
    }

    #[test]
    fn a_cancelled_bench_stops() {
        let dir = tempfile::tempdir().unwrap();
        let all = three_tiers(dir.path());
        let cancel = CancelToken::new();
        cancel.cancel();
        let open = |_: &Path, _: &ModelEntry| Ok(BenchSession { ep: "cpu".into(), run: Box::new(|| Ok(())) });
        assert_eq!(bench_in(&all, &mut BenchCache::default(), &fp("cpu"), false, &cancel, open), 0);
    }

    /// Two ready models of one family, high + ultra each, in files `a.onnx` and `b.onnx`.
    fn two_model_files() -> (tempfile::TempDir, Vec<CatalogueModel>) {
        let dir = tempfile::tempdir().unwrap();
        for (id, version, file, bytes) in [("lumen-ray-1.0.2-d", "1.0.2-d", "a.onnx", &b"a"[..]), ("lumen-ray-1.0.3-d", "1.0.3-d", "b.onnx", &b"b"[..])] {
            write_model(dir.path(), "ray", id, version, &[("high", file, bytes, "fp32", 1, None), ("ultra", file, bytes, "fp32", 5, None)]);
        }
        let all = crate::model_catalogue::catalogue_in(dir.path());
        assert_eq!(all.len(), 2);
        (dir, all)
    }

    #[test]
    fn the_warmups_absorb_the_graph_build() {
        let mut calls = 0;
        let ms = median_ms(
            || {
                calls += 1;
                std::thread::sleep(std::time::Duration::from_millis(if calls <= 3 { 50 } else { 5 }));
                Ok(())
            },
            BENCH_WARMUPS,
            BENCH_TIMED,
        )
        .unwrap();
        assert_eq!(calls, 10);
        assert!(ms < 10.0, "median {ms} ms");
    }

    #[test]
    fn the_bench_image_is_a_real_looking_picture() {
        let a = bench_image(64);
        assert_eq!(a.len(), 4 * 64 * 64);
        assert_eq!(a, bench_image(64), "deterministic");
        assert!(a.iter().all(|v| (0.0..=1.0).contains(v)));
        let mean = a.iter().sum::<f32>() / a.len() as f32;
        let var = a.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / a.len() as f32;
        assert!(var > 0.01, "not flat: variance {var}");
        // Neighbouring pixels differ (noise), so no kernel sees a constant tile.
        assert!(a.windows(2).filter(|w| w[0] != w[1]).count() > a.len() / 2);
    }

    #[test]
    fn the_bench_waits_for_quiet_but_not_forever_on_a_transcode() {
        let cancel = CancelToken::new();
        let busy = std::cell::Cell::new(3);
        let quiet = wait_quiet(
            &cancel,
            || {
                busy.set(busy.get() - 1);
                busy.get() > 0
            },
            || false,
            Duration::from_millis(1),
            Duration::from_secs(60),
        );
        assert!(quiet && busy.get() == 0, "waited out the user");
        let t = Instant::now();
        assert!(wait_quiet(&cancel, || false, || true, Duration::from_millis(5), Duration::from_millis(40)), "a transcode that never ends is waited on only so long");
        assert!(t.elapsed() < Duration::from_secs(2));
        cancel.cancel();
        assert!(!wait_quiet(&cancel, || true, || false, Duration::from_millis(1), Duration::from_secs(60)), "cancelled: give up");
    }

    fn named(p: &Path, name: &str) -> bool {
        p.file_name().is_some_and(|n| n == name)
    }

    #[test]
    fn a_file_that_fell_back_to_the_cpu_is_found_under_its_own_ep() {
        // Two files: A opens on DirectML, B falls back to the CPU (Julian's 2026-10-08 case).
        let (dir, all) = two_model_files();
        let mut cache = BenchCache::default();
        let base = fp("directml");
        bench_in(&all, &mut cache, &base, false, &CancelToken::new(), |p, _| {
            Ok(BenchSession { ep: if named(p, "b.onnx") { "cpu".into() } else { "directml".into() }, run: Box::new(|| Ok(())) })
        });
        for m in &all {
            assert!(timings(m, &cache, &base).iter().all(|(_, ms)| ms.is_some()), "{:?}", cache.entries.keys());
        }
        let again = bench_in(&all, &mut cache, &base, false, &CancelToken::new(), |_, _| panic!("nothing left to time"));
        assert_eq!(again, 0);
        let b = all.iter().find(|m| named(m.ready(Tier::High).unwrap().0, "b.onnx")).unwrap();
        let view = model_view(b, &cache, &base);
        assert!(view.tiers.iter().all(|t| t.ep.as_deref() == Some("cpu")), "{:?}", view.tiers);
        // With the GPU switch off everything is looked up (and timed) on the CPU.
        assert!(timings(b, &cache, &fp("cpu")).iter().all(|(_, ms)| ms.is_some()));
        drop(dir);
    }

    #[test]
    fn a_tier_that_fails_is_not_retried_and_does_not_keep_testing() {
        let dir = tempfile::tempdir().unwrap();
        let all = three_tiers(dir.path());
        let mut cache = BenchCache::default();
        let base = fp("directml");
        let timed = bench_in(&all, &mut cache, &base, false, &CancelToken::new(), |p, _| {
            let low = named(p, "low.onnx");
            Ok(BenchSession {
                ep: "directml".into(),
                run: Box::new(move || if low { Err(MlError::Runtime("80070057 The parameter is incorrect".into())) } else { Ok(()) }),
            })
        });
        assert_eq!(timed, 2, "high and ultra");
        assert_eq!(cache.failed.len(), 1);
        let again = bench_in(&all, &mut cache, &base, false, &CancelToken::new(), |_, _| panic!("a failed tier is not retried"));
        assert_eq!(again, 0);
        let rows = catalog_rows(&all, &cache, &base, &ExperimentalTasks::default(), false, false, None);
        assert!(!rows[0].testing);
        assert_eq!(rows[0].recommended, Tier::Ultra, "the failed Low is skipped, not untimed");
        let low = rows[0].models[0].tiers.iter().find(|t| t.tier == Tier::Low).unwrap();
        assert!(low.failed.as_deref().is_some_and(|w| w.contains("80070057")), "{low:?}");
        assert_eq!(low.ms, None);

        // A file that will not open fails every tier it holds, once.
        let mut cache = BenchCache::default();
        let timed = bench_in(&all, &mut cache, &base, false, &CancelToken::new(), |p, _| {
            if named(p, "high.onnx") {
                Err(MlError::Runtime("no session".into()))
            } else {
                Ok(BenchSession { ep: "directml".into(), run: Box::new(|| Ok(())) })
            }
        });
        assert_eq!((timed, cache.failed.len()), (1, 2), "low timed; high and ultra failed");
        assert_eq!(bench_in(&all, &mut cache, &base, false, &CancelToken::new(), |_, _| panic!("not retried")), 0);
        assert_eq!(recommended(AiTaskKind::Auto, &timings(&all[0], &cache, &base)), (Tier::Low, false));
        // *Time again* (force) retries, and a success clears the failure.
        let ok = |_: &Path, _: &ModelEntry| Ok(BenchSession { ep: "directml".into(), run: Box::new(|| Ok(())) });
        assert_eq!(bench_in(&all, &mut cache, &base, true, &CancelToken::new(), ok), 3);
        assert!(cache.failed.is_empty(), "{:?}", cache.failed);
    }

    #[test]
    fn julians_old_cache_recommends_ultra() {
        // The model-bench.json of 2026-10-08: `gpu_ep` "cpu" (one file fell back) while every other
        // timing sits under DirectML. Its shas are replaced by the fixture files' own.
        let dir = tempfile::tempdir().unwrap();
        let all = three_tiers(dir.path());
        let sha = |t: Tier| all[0].ready(t).unwrap().1.sha256.clone();
        let machine = "Windows 11 Pro|AMD Ryzen 9 5900X 12-Core Processor|24|64|NVIDIA GeForce RTX 3080 Ti";
        let json = serde_json::json!({
            "entries": {
                format!("{}:low:{machine}|cpu", sha(Tier::Low)): 47.754,
                format!("{}:high:{machine}|directml", sha(Tier::High)): 9.8412,
                format!("{}:ultra:{machine}|directml", sha(Tier::Ultra)): 55.4582,
            },
            "gpu_ep": "cpu"
        });
        let path = dir.path().join("model-bench.json");
        std::fs::write(&path, json.to_string()).unwrap();
        let cache = BenchCache::load(&path);
        let base = Fingerprint {
            os: "Windows 11 Pro".into(),
            cpu: "AMD Ryzen 9 5900X 12-Core Processor".into(),
            cores: 24,
            ram_gib: 64,
            gpu: "NVIDIA GeForce RTX 3080 Ti".into(),
            ep: "directml".into(),
        };
        assert_eq!(timings(&all[0], &cache, &base), [(Tier::Low, Some(47.754)), (Tier::High, Some(9.8412)), (Tier::Ultra, Some(55.4582))]);
        assert_eq!(recommended(AiTaskKind::Auto, &timings(&all[0], &cache, &base)), (Tier::Ultra, false));
        assert_eq!(bench_in(&all, &mut BenchCache::load(&path), &base, false, &CancelToken::new(), |_, _| panic!("no re-timing")), 0);
        // Saving drops the old global EP.
        cache.save(&path).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("gpu_ep"));
    }

    #[test]
    fn current_fingerprint_names_this_machine() {
        let f = Fingerprint::current("cpu");
        assert_eq!(f.ep, "cpu");
        assert!(f.cores >= 1);
    }
}
