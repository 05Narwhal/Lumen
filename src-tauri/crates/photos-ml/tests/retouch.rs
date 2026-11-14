//! Lumen's own Retouch model through `Retoucher` (Round 12-5), on a tiny stand-in made by
//! `fixtures/make_retouch_tiny.py` with the real model's ONNX contract.

use std::path::{Path, PathBuf};

use photos_ml::manifest::{ModelEntry, RetouchSpec};
use photos_ml::retouch::Retoucher;

const SIZE: usize = 256;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn reference() -> (usize, Vec<f32>, Vec<f32>) {
    let r: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(fixture("retouch-tiny.json")).unwrap()).unwrap();
    let vec = |k: &str| r[k].as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect::<Vec<_>>();
    (r["outputs"].as_u64().unwrap() as usize, vec("plain"), vec("with_clip"))
}

fn entry(layout: u32) -> ModelEntry {
    serde_json::from_value(serde_json::json!({
        "id": "retouch-tiny", "role": "retouch", "file": "retouch-tiny.onnx", "bytes": 0, "sha256": "",
        "licence": "Lumen (own weights)",
        "retouch": { "size": SIZE, "layout": layout, "inputs": ["image", "clip", "clip_flag"], "output": "v" }
    }))
    .unwrap()
}

fn open(outputs: usize) -> Retoucher {
    Retoucher::open(&fixture("retouch-tiny.onnx"), &entry(1), 1, outputs, false).unwrap()
}

fn assert_close(got: &[f32], want: &[f32]) {
    assert_eq!(got.len(), want.len());
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert!((g - w).abs() < 1e-5, "v[{i}] = {g}, want {w}");
    }
}

#[test]
fn a_model_made_for_another_layout_is_refused() {
    let (outputs, ..) = reference();
    let err = Retoucher::open(&fixture("retouch-tiny.onnx"), &entry(2), 1, outputs, false).err().expect("refused");
    assert!(err.to_string().contains("layout"), "{err}");
}

#[test]
fn an_entry_without_a_retouch_contract_is_refused() {
    let (outputs, ..) = reference();
    let mut e = entry(1);
    e.retouch = None;
    assert!(Retoucher::open(&fixture("retouch-tiny.onnx"), &e, 1, outputs, false).is_err());
}

#[test]
fn runs_with_and_without_a_clip_embedding() {
    let (outputs, plain, with_clip) = reference();
    let mut r = open(outputs);
    let image = vec![0.5f32; 4 * SIZE * SIZE];
    assert_close(&r.run(&image, None).unwrap(), &plain);
    assert_close(&r.run(&image, Some(&[0.1f32; 512])).unwrap(), &with_clip);
}

#[test]
fn wrong_sizes_are_errors_not_panics() {
    let (outputs, ..) = reference();
    let image = vec![0.5f32; 4 * SIZE * SIZE];
    assert!(open(outputs + 1).run(&image, None).is_err(), "the model's output length must match");
    let mut r = open(outputs);
    assert!(r.run(&image[..100], None).is_err(), "a short image");
    assert!(r.run(&image, Some(&[0.0; 3])).is_err(), "a short clip");
}

#[test]
fn the_spec_parses_back() {
    let spec = entry(1).retouch.unwrap();
    assert_eq!(spec, RetouchSpec { size: 256, layout: 1, inputs: vec!["image".into(), "clip".into(), "clip_flag".into()], output: "v".into() });
}

#[test]
fn cpu_fallback_reports_cpu() {
    let (outputs, plain, _) = reference();
    let mut r = Retoucher::open(&fixture("retouch-tiny.onnx"), &entry(1), 1, outputs, false).unwrap();
    assert_eq!(r.ep(), "cpu");
    assert_close(&r.run(&vec![0.5f32; 4 * SIZE * SIZE], None).unwrap(), &plain);
}

#[test]
fn the_gpu_either_opens_or_falls_back_to_the_cpu() {
    let (outputs, plain, _) = reference();
    let mut r = Retoucher::open(&fixture("retouch-tiny.onnx"), &entry(1), 1, outputs, true).unwrap();
    assert!(matches!(r.ep(), "cpu" | "directml"), "{}", r.ep());
    assert_close(&r.run(&vec![0.5f32; 4 * SIZE * SIZE], None).unwrap(), &plain);
}
