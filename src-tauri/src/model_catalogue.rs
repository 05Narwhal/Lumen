//! The dev model catalogue (Round 14 spec §2): `dev/<family>/<id>/model.json`, each naming
//! up to three tiers (`low`, `high`, `ultra`) that may share a file, plus the legacy
//! `dev/retouch.json` read as one Ray model at tier `high`. Every way a tier can be unusable
//! is that tier's state, never an error.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use photos_core::retouch::RETOUCH_LAYOUT;
use photos_ml::manifest::ModelEntry;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::models::{self, DevModel};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Tier {
    Low,
    High,
    Ultra,
}

impl Tier {
    pub const ALL: [Tier; 3] = [Tier::Low, Tier::High, Tier::Ultra];

    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Low => "low",
            Tier::High => "high",
            Tier::Ultra => "ultra",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Precision {
    Int8,
    Fp16,
    Fp32,
}

#[derive(Debug, Clone)]
pub enum TierState {
    Ready { path: PathBuf, entry: Box<ModelEntry>, precision: Precision, tta: u8, val_de: Option<f32> },
    Invalid { reason: String },
}

#[derive(Debug, Clone)]
pub struct CatalogueModel {
    pub id: String,
    pub name: String,
    pub family: String,
    pub version: String,
    /// Read from the legacy `retouch.json`.
    pub legacy: bool,
    /// Only the tiers `model.json` names.
    pub tiers: BTreeMap<Tier, TierState>,
}

impl CatalogueModel {
    pub fn ready(&self, t: Tier) -> Option<(&Path, &ModelEntry, u8)> {
        match self.tiers.get(&t)? {
            TierState::Ready { path, entry, tta, .. } => Some((path.as_path(), entry.as_ref(), *tta)),
            TierState::Invalid { .. } => None,
        }
    }

    /// The highest tier that runs.
    pub fn best_ready(&self) -> Option<Tier> {
        Tier::ALL.into_iter().rev().find(|t| self.ready(*t).is_some())
    }
}

#[derive(Deserialize)]
struct RawModel {
    name: String,
    family: String,
    version: String,
    #[serde(default)]
    tiers: BTreeMap<Tier, RawTier>,
}

#[derive(Deserialize)]
struct RawTier {
    #[serde(flatten)]
    entry: ModelEntry,
    #[serde(default = "fp32")]
    precision: Precision,
    #[serde(default = "one")]
    tta: u8,
    #[serde(default)]
    val_de: Option<f32>,
}

fn fp32() -> Precision {
    Precision::Fp32
}

fn one() -> u8 {
    1
}

/// `"1.0.1-d"` → `(1, 0, 1, 0)`: stages d < a < b < r < release.
pub fn version_key(v: &str) -> (u32, u32, u32, u8) {
    let (nums, stage) = v.split_once('-').unwrap_or((v, ""));
    let mut n = nums.split('.').map(|x| x.parse::<u32>().unwrap_or(0));
    let stage = match stage.chars().next() {
        None => 4,
        Some('d') => 0,
        Some('a') => 1,
        Some('b') => 2,
        Some('r') => 3,
        Some(_) => 0,
    };
    (n.next().unwrap_or(0), n.next().unwrap_or(0), n.next().unwrap_or(0), stage)
}

/// Ray and Prism run through the Retouch head: an entry without its block, or made for another
/// parameter layout, cannot run here. Other families have their own contracts.
fn retouch_problem(family: &str, entry: &ModelEntry) -> Option<String> {
    if family != "ray" && family != "prism" {
        return None;
    }
    match &entry.retouch {
        None => Some("not a Retouch model".into()),
        Some(spec) if spec.layout != RETOUCH_LAYOUT => Some(format!("made for another version of Lumen (parameter layout {})", spec.layout)),
        Some(_) => None,
    }
}

fn tier_state(dir: &Path, family: &str, raw: RawTier) -> TierState {
    if let Some(reason) = retouch_problem(family, &raw.entry) {
        return TierState::Invalid { reason };
    }
    if !models::is_plain_name(&raw.entry.file) {
        return TierState::Invalid { reason: "the entry's file must be a name, not a path".into() };
    }
    let path = dir.join(&raw.entry.file);
    if !path.exists() {
        return TierState::Invalid { reason: "model file missing".into() };
    }
    if !models::checked(&path, &raw.entry) {
        return TierState::Invalid { reason: "checksum does not match".into() };
    }
    TierState::Ready { path, entry: Box::new(raw.entry), precision: raw.precision, tta: raw.tta.max(1), val_de: raw.val_de }
}

fn read_model(dir: &Path) -> Option<CatalogueModel> {
    let raw = std::fs::read_to_string(dir.join("model.json")).ok()?;
    let model: RawModel = match serde_json::from_str(&raw) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(dir = %dir.display(), error = %e, "skipping a dev model whose model.json does not parse");
            return None;
        }
    };
    let id = dir.file_name()?.to_string_lossy().into_owned();
    let tiers = model.tiers.into_iter().map(|(t, raw)| (t, tier_state(dir, &model.family, raw))).collect();
    Some(CatalogueModel { id, name: model.name, family: model.family, version: model.version, legacy: false, tiers })
}

fn legacy_model(dir: &Path) -> Option<CatalogueModel> {
    let state = match models::dev_model_in(dir, "retouch") {
        DevModel::Missing => return None,
        DevModel::Invalid { reason } => {
            let id = "retouch".to_string();
            let tiers = BTreeMap::from([(Tier::High, TierState::Invalid { reason })]);
            return Some(CatalogueModel { name: id.clone(), id, family: "ray".into(), version: "0".into(), legacy: true, tiers });
        }
        DevModel::Ready { path, entry } => (path, entry),
    };
    let (path, entry) = state;
    let id = entry.id.clone();
    let name = entry.name.clone().unwrap_or_else(|| id.clone());
    let problem = retouch_problem("ray", &entry);
    let version = id
        .strip_prefix("lumen-ray-")
        .map(|rest| {
            ["-low", "-high", "-ultra"].iter().fold(rest, |r, s| r.strip_suffix(s).unwrap_or(r)).to_string()
        })
        .unwrap_or_else(|| "0".into());
    let val_de = entry.trained.as_ref().and_then(|t| t.get("val_de")).and_then(|v| v.as_f64()).map(|v| v as f32);
    let state = match problem {
        Some(reason) => TierState::Invalid { reason },
        None => TierState::Ready { path, entry, precision: Precision::Fp32, tta: 1, val_de },
    };
    let tiers = BTreeMap::from([(Tier::High, state)]);
    Some(CatalogueModel { id, name, family: "ray".into(), version, legacy: true, tiers })
}

/// Every model in `dir`, grouped by family, newest first within a family.
pub fn catalogue_in(dir: &Path) -> Vec<CatalogueModel> {
    let mut out = Vec::new();
    if let Ok(families) = std::fs::read_dir(dir) {
        for family in families.flatten().filter(|e| e.path().is_dir()) {
            let Ok(ids) = std::fs::read_dir(family.path()) else {
                tracing::warn!(dir = %family.path().display(), "skipping an unreadable model family folder");
                continue;
            };
            out.extend(ids.flatten().filter(|e| e.path().is_dir()).filter_map(|e| read_model(&e.path())));
        }
    }
    out.extend(legacy_model(dir));
    out.sort_by(|a, b| {
        a.family
            .cmp(&b.family)
            .then_with(|| version_key(&b.version).cmp(&version_key(&a.version)))
            .then_with(|| a.legacy.cmp(&b.legacy))
            .then_with(|| a.id.cmp(&b.id))
    });
    out
}

pub fn catalogue() -> Vec<CatalogueModel> {
    catalogue_in(&models::dev_dir())
}

/// One family's models, newest first.
pub fn family_models(all: &[CatalogueModel], family: &str) -> Vec<CatalogueModel> {
    all.iter().filter(|m| m.family == family).cloned().collect()
}

/// The model and tier to run: the chosen model (else the newest that runs) at the requested
/// tier, else the nearest ready one, lower tiers first. `None` when nothing in the family runs.
pub fn pick(all: &[CatalogueModel], family: &str, model: Option<&str>, tier: Tier) -> Option<(CatalogueModel, Tier)> {
    let runs = |m: &&CatalogueModel| m.family == family && m.best_ready().is_some();
    let chosen = model.and_then(|id| all.iter().filter(runs).find(|m| m.id == id));
    let m = chosen.or_else(|| all.iter().find(runs))?;
    let lower = Tier::ALL.into_iter().rev().filter(|t| *t < tier);
    let higher = Tier::ALL.into_iter().filter(|t| *t > tier);
    let t = std::iter::once(tier).chain(lower).chain(higher).find(|t| m.ready(*t).is_some())?;
    Some((m.clone(), t))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    pub fn sha_of(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn entry_json(id: &str, file: &str, bytes: &[u8]) -> serde_json::Value {
        serde_json::json!({
            "id": id, "role": "retouch", "file": file, "bytes": bytes.len(), "sha256": sha_of(bytes),
            "licence": "Lumen (own weights)",
            "retouch": { "size": 256, "layout": 1, "inputs": ["image", "clip", "clip_flag"], "output": "v" },
            "trained": { "run": "r2", "val_de": 3.81 }
        })
    }

    /// A tier: (tier, file, bytes, precision, tta, val_de).
    pub type FixtureTier<'a> = (&'a str, &'a str, &'a [u8], &'a str, u8, Option<f32>);

    pub fn write_model(dir: &Path, family: &str, id: &str, version: &str, tiers: &[FixtureTier]) -> PathBuf {
        let folder = dir.join(family).join(id);
        std::fs::create_dir_all(&folder).unwrap();
        let mut map = serde_json::Map::new();
        for (tier, file, bytes, precision, tta, val_de) in tiers {
            if models::is_plain_name(file) {
                std::fs::write(folder.join(file), bytes).unwrap();
            }
            let mut e = entry_json(&format!("{id}-{tier}"), file, bytes);
            let o = e.as_object_mut().unwrap();
            o.insert("precision".into(), (*precision).into());
            o.insert("tta".into(), (*tta).into());
            if let Some(v) = val_de {
                o.insert("val_de".into(), (*v).into());
            }
            map.insert((*tier).into(), e);
        }
        let doc = serde_json::json!({ "name": format!("Lumen {id}"), "family": family, "version": version,
            "purpose": "test", "tiers": map });
        std::fs::write(folder.join("model.json"), doc.to_string()).unwrap();
        folder
    }

    pub fn write_legacy(dir: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(dir).unwrap();
        let file = "lumen-ray-1.0.1-d-high.onnx";
        std::fs::write(dir.join(file), bytes).unwrap();
        let mut e = entry_json("lumen-ray-1.0.1-d-high", file, bytes);
        e.as_object_mut().unwrap().insert("name".into(), "Lumen Ray-high 1.0.1-d".into());
        std::fs::write(dir.join("retouch.json"), e.to_string()).unwrap();
    }

    fn three_tiers(dir: &Path) -> PathBuf {
        write_model(
            dir,
            "ray",
            "lumen-ray-1.0.1-d",
            "1.0.1-d",
            &[
                ("low", "low.onnx", b"low-bytes", "int8", 1, Some(4.1)),
                ("high", "high.onnx", b"high-bytes", "fp32", 1, Some(3.9)),
                ("ultra", "high.onnx", b"high-bytes", "fp32", 5, None),
            ],
        )
    }

    fn ready_parts(m: &CatalogueModel, t: Tier) -> (PathBuf, Precision, u8, Option<f32>) {
        match &m.tiers[&t] {
            TierState::Ready { path, precision, tta, val_de, .. } => (path.clone(), *precision, *tta, *val_de),
            TierState::Invalid { reason } => panic!("{t:?} invalid: {reason}"),
        }
    }

    fn reason(m: &CatalogueModel, t: Tier) -> String {
        match &m.tiers[&t] {
            TierState::Invalid { reason } => reason.clone(),
            TierState::Ready { .. } => panic!("{t:?} is ready"),
        }
    }

    #[test]
    fn new_layout_reads_every_tier() {
        let dir = tempfile::tempdir().unwrap();
        three_tiers(dir.path());
        let all = catalogue_in(dir.path());
        assert_eq!(all.len(), 1);
        let m = &all[0];
        assert_eq!((m.id.as_str(), m.family.as_str(), m.version.as_str(), m.legacy), ("lumen-ray-1.0.1-d", "ray", "1.0.1-d", false));
        let (_, p, tta, de) = ready_parts(m, Tier::Low);
        assert_eq!((p, tta, de), (Precision::Int8, 1, Some(4.1)));
        let (_, p, tta, de) = ready_parts(m, Tier::High);
        assert_eq!((p, tta, de), (Precision::Fp32, 1, Some(3.9)));
        let (_, p, tta, de) = ready_parts(m, Tier::Ultra);
        assert_eq!((p, tta, de), (Precision::Fp32, 5, None));
        assert_eq!(m.best_ready(), Some(Tier::Ultra));
    }

    #[test]
    fn shared_file_is_hashed_once() {
        let dir = tempfile::tempdir().unwrap();
        three_tiers(dir.path());
        let m = &catalogue_in(dir.path())[0];
        assert_eq!(ready_parts(m, Tier::High).0, ready_parts(m, Tier::Ultra).0);
    }

    #[test]
    fn bad_sha_or_missing_file_makes_only_that_tier_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let folder = three_tiers(dir.path());
        std::fs::write(folder.join("low.onnx"), b"LOW-bytes").unwrap();
        let m = &catalogue_in(dir.path())[0];
        assert_eq!(reason(m, Tier::Low), "checksum does not match");
        assert!(m.ready(Tier::High).is_some());
        std::fs::remove_file(folder.join("high.onnx")).unwrap();
        let m = &catalogue_in(dir.path())[0];
        assert_eq!(reason(m, Tier::High), "model file missing");
        assert_eq!(reason(m, Tier::Ultra), "model file missing");
        assert_eq!(m.best_ready(), None);
    }

    #[test]
    fn a_bad_model_json_is_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        three_tiers(dir.path());
        let bad = dir.path().join("ray").join("broken");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("model.json"), "{ not json").unwrap();
        let all = catalogue_in(dir.path());
        assert_eq!(all.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["lumen-ray-1.0.1-d"]);
    }

    #[test]
    fn legacy_retouch_json_is_one_ray_model_at_high() {
        let dir = tempfile::tempdir().unwrap();
        write_legacy(dir.path(), b"legacy-bytes");
        let all = catalogue_in(dir.path());
        assert_eq!(all.len(), 1);
        let m = &all[0];
        assert_eq!((m.family.as_str(), m.legacy, m.version.as_str()), ("ray", true, "1.0.1-d"));
        assert_eq!((m.id.as_str(), m.name.as_str()), ("lumen-ray-1.0.1-d-high", "Lumen Ray-high 1.0.1-d"));
        assert_eq!(m.tiers.keys().copied().collect::<Vec<_>>(), [Tier::High]);
        let (_, p, tta, de) = ready_parts(m, Tier::High);
        assert_eq!((p, tta, de), (Precision::Fp32, 1, Some(3.81)));
    }

    #[test]
    fn newest_first() {
        let dir = tempfile::tempdir().unwrap();
        write_model(dir.path(), "ray", "lumen-ray-1.0", "1.0", &[("high", "a.onnx", b"a", "fp32", 1, None)]);
        write_model(dir.path(), "ray", "lumen-ray-1.0.1-d", "1.0.1-d", &[("high", "b.onnx", b"b", "fp32", 1, None)]);
        write_legacy(dir.path(), b"legacy");
        let ids: Vec<String> = family_models(&catalogue_in(dir.path()), "ray").into_iter().map(|m| m.id).collect();
        // A release outranks every pre-release of its own number; 1.0.1-d is newer than 1.0.
        assert_eq!(ids, ["lumen-ray-1.0.1-d", "lumen-ray-1.0.1-d-high", "lumen-ray-1.0"]);
        assert!(version_key("1.0.1-d") > version_key("1.0"));
        assert!(version_key("1.0.1") > version_key("1.0.1-r"));
        assert!(version_key("1.0.1-a") > version_key("1.0.1-d"));
    }

    #[test]
    fn pick_falls_back_when_the_chosen_model_or_tier_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let folder = three_tiers(dir.path());
        write_legacy(dir.path(), b"legacy");
        let all = catalogue_in(dir.path());
        let (m, t) = pick(&all, "ray", Some("deleted"), Tier::High).unwrap();
        assert_eq!((m.id.as_str(), t), ("lumen-ray-1.0.1-d", Tier::High));
        let (m, t) = pick(&all, "ray", Some("lumen-ray-1.0.1-d-high"), Tier::Ultra).unwrap();
        assert_eq!((m.id.as_str(), t), ("lumen-ray-1.0.1-d-high", Tier::High));
        std::fs::write(folder.join("low.onnx"), b"corrupt").unwrap();
        let all = catalogue_in(dir.path());
        let (_, t) = pick(&all, "ray", None, Tier::Low).unwrap();
        assert_eq!(t, Tier::High);
        assert!(pick(&all, "prism", None, Tier::High).is_none());
    }

    #[test]
    fn file_names_must_not_be_paths() {
        let dir = tempfile::tempdir().unwrap();
        write_model(dir.path(), "ray", "x", "1.0", &[("high", "../x.onnx", b"x", "fp32", 1, None)]);
        let m = &catalogue_in(dir.path())[0];
        assert_eq!(reason(m, Tier::High), "the entry's file must be a name, not a path");
    }

    #[test]
    fn a_retouch_model_for_another_layout_is_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let folder = write_model(dir.path(), "prism", "p", "1.0", &[("high", "p.onnx", b"p", "fp32", 1, None)]);
        let doc = std::fs::read_to_string(folder.join("model.json")).unwrap().replace("\"layout\":1", "\"layout\":99");
        std::fs::write(folder.join("model.json"), doc).unwrap();
        // A legacy file without a retouch block is not a Retouch model at all.
        write_legacy(dir.path(), b"r");
        let legacy = std::fs::read_to_string(dir.path().join("retouch.json")).unwrap();
        let mut v: serde_json::Value = serde_json::from_str(&legacy).unwrap();
        v.as_object_mut().unwrap().remove("retouch");
        std::fs::write(dir.path().join("retouch.json"), v.to_string()).unwrap();
        let all = catalogue_in(dir.path());
        let prism = all.iter().find(|m| m.family == "prism").unwrap();
        assert_eq!(reason(prism, Tier::High), "made for another version of Lumen (parameter layout 99)");
        let ray = all.iter().find(|m| m.family == "ray").unwrap();
        assert_eq!(reason(ray, Tier::High), "not a Retouch model");
    }
}
