//! API 1.3 (Round 25 P4b): `proposals.submit` reaches the host only with a proposal permission,
//! each proposal needs its kind's permission, and a bad or ungranted proposal refuses the batch.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use photos_core::proposals::{NewProposal, ProposalValue};
use photos_plugins::host::{Engine, HostCtx, ProposalsHost, ToolsInstance};
use photos_plugins::PluginPermission;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/wasm/{name}.wasm"))
}

#[derive(Default)]
struct Fake {
    got: Mutex<Vec<(String, Vec<NewProposal>)>>,
}

impl ProposalsHost for Fake {
    fn submit(&self, plugin: &str, batch: Vec<NewProposal>) -> Result<(), String> {
        self.got.lock().unwrap().push((plugin.into(), batch));
        Ok(())
    }
}

fn tools(granted: Vec<PluginPermission>) -> Result<(tempfile::TempDir, Arc<Fake>, Arc<ToolsInstance>), photos_plugins::PluginError> {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(&dir.path().join("cache"), Arc::new(|_, _| {})).unwrap();
    let fake = Arc::new(Fake::default());
    let (data, f) = (dir.path().join("data"), fake.clone());
    let ctx = move || HostCtx {
        plugin: "com.test.x".into(),
        granted: granted.clone(),
        settings: Default::default(),
        data_dir: data.clone(),
        log: Arc::new(|_: &str| {}),
        cancel: None,
        chosen: None,
        photos: None,
        drafts: None,
        models: None,
        library: None,
        places: None,
        proposals: Some(f.clone() as Arc<dyn ProposalsHost>),
    };
    let t = engine.tools_at("com.test.x", &fixture("proposals_ok"), ctx)?;
    Ok((dir, fake, t))
}

fn base() -> Vec<PluginPermission> {
    vec![PluginPermission::Storage, PluginPermission::LibraryRead]
}

#[test]
fn every_kind_arrives_checked_and_tidied() {
    let mut granted = base();
    granted.extend(PluginPermission::PROPOSING);
    let (_d, fake, t) = tools(granted).unwrap();
    assert_eq!(t.call("all", "{}").unwrap(), "ok");
    let got = fake.got.lock().unwrap();
    let (plugin, batch) = &got[0];
    assert_eq!(plugin, "com.test.x");
    let values: Vec<_> = batch.iter().map(|p| p.value.clone()).collect();
    assert_eq!(
        values,
        [
            ProposalValue::Location { lat: 46.0, lon: 14.5 },
            ProposalValue::Date("2021-05-04T10:00:00".into()),
            ProposalValue::Tags(vec!["beach".into(), "sea".into()]),
            ProposalValue::Label("Green".into()),
            ProposalValue::Person { face: 7, name: "Ana".into() },
            ProposalValue::Title("Sunset".into()),
            ProposalValue::Caption("A long day".into()),
        ]
    );
    assert!(batch.iter().all(|p| p.asset_id == 1 && p.confidence == 0.75 && p.reason.as_deref() == Some("because")));
}

#[test]
fn each_kind_needs_its_own_permission() {
    let mut granted = base();
    granted.push(PluginPermission::CaptionsWrite);
    let (_d, fake, t) = tools(granted).unwrap();
    assert_eq!(t.call("title", "{}").unwrap(), "ok");
    // The location in "all" needs places.write: the whole batch is refused.
    let e = t.call("all", "{}").unwrap_err();
    assert!(e.message.contains("places.write"), "{e}");
    assert_eq!(fake.got.lock().unwrap().len(), 1);
}

#[test]
fn one_bad_proposal_refuses_the_batch() {
    let mut granted = base();
    granted.extend(PluginPermission::PROPOSING);
    let (_d, fake, t) = tools(granted).unwrap();
    let e = t.call("bad", "{}").unwrap_err();
    assert!(e.message.contains("Orange"), "{e}");
    assert!(fake.got.lock().unwrap().is_empty());
}

#[test]
fn proposals_without_a_proposal_permission_are_refused() {
    let err = tools(base()).err().expect("refused");
    assert_eq!(err.code, "plugin_permission", "{err}");
    assert!(err.message.contains("lumen:plugin/proposals"), "{err}");
}
