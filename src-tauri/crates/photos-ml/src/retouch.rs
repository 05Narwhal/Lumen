//! Lumen's own Retouch model (Round 12 spec §4): the prepared preview base and the photo's
//! CLIP embedding in, the Retouch parameter vector `v` out. The binary prepares the input
//! (`photos_media::retouch_input::prepare`) and decodes `v` (`photos_core::retouch::decode`).

use std::path::Path;

use ort::{inputs, session::Session, value::Tensor};

use crate::manifest::{ModelEntry, RetouchSpec};
use crate::MlError;

/// The CLIP embedding's width.
pub const CLIP: usize = 512;

pub struct Retoucher {
    session: Session,
    spec: RetouchSpec,
    outputs: usize,
    ep: &'static str,
}

impl Retoucher {
    /// Opens the model when its entry describes a Retouch model made for `expected_layout`
    /// (`photos_core::retouch::RETOUCH_LAYOUT`) giving `outputs` parameters. `gpu` tries
    /// DirectML first (Windows); if it cannot open the model the CPU takes over.
    pub fn open(path: &Path, entry: &ModelEntry, expected_layout: u32, outputs: usize, gpu: bool) -> Result<Self, MlError> {
        let spec = entry.retouch.clone().ok_or_else(|| MlError::Missing(format!("{} is not a Retouch model", entry.id)))?;
        if spec.layout != expected_layout {
            return Err(MlError::Shape(format!(
                "model made for parameter layout {}, this Lumen uses layout {expected_layout}",
                spec.layout
            )));
        }
        if spec.inputs.len() != 3 {
            return Err(MlError::Shape(format!("expected inputs image, clip, clip_flag; got {:?}", spec.inputs)));
        }
        let gpu = if gpu {
            match gpu_session(path) {
                Ok(s) => Some(s),
                Err(e) => {
                    tracing::warn!("retouch: GPU unavailable, using the CPU: {e}");
                    None
                }
            }
        } else {
            None
        };
        let (session, ep) = match gpu {
            Some(s) => (s, "directml"),
            None => (Session::builder()?.with_intra_threads(2)?.commit_from_file(path)?, "cpu"),
        };
        Ok(Self { session, spec, outputs, ep })
    }

    /// The execution provider the session runs on: `"directml"` or `"cpu"`.
    pub fn ep(&self) -> &'static str {
        self.ep
    }

    /// The input edge in pixels.
    pub fn size(&self) -> usize {
        self.spec.size as usize
    }

    /// `image`: 4×size×size channel-major; `clip`: `None` → zeros and flag 0.
    pub fn run(&mut self, image: &[f32], clip: Option<&[f32]>) -> Result<Vec<f32>, MlError> {
        let size = self.size();
        if image.len() != 4 * size * size {
            return Err(MlError::Shape(format!("image has {} values, expected 4×{size}×{size}", image.len())));
        }
        if clip.is_some_and(|c| c.len() != CLIP) {
            return Err(MlError::Shape(format!("clip has {} values, expected {CLIP}", clip.map_or(0, |c| c.len()))));
        }
        let (clip, flag) = match clip {
            Some(c) => (c.to_vec(), 1.0f32),
            None => (vec![0.0; CLIP], 0.0),
        };
        let image = Tensor::from_array(([1usize, 4, size, size], image.to_vec()))?;
        let clip = Tensor::from_array(([1usize, CLIP], clip))?;
        let flag = Tensor::from_array(([1usize, 1], vec![flag]))?;
        let n = &self.spec.inputs;
        let out = self.session.run(inputs![n[0].clone() => image, n[1].clone() => clip, n[2].clone() => flag])?;
        let (_, v) = out[self.spec.output.as_str()].try_extract_tensor::<f32>()?;
        if v.len() != self.outputs {
            return Err(MlError::Shape(format!("model returned {} parameters, expected {}", v.len(), self.outputs)));
        }
        Ok(v.to_vec())
    }
}

#[cfg(windows)]
pub(crate) fn gpu_session(path: &Path) -> Result<Session, MlError> {
    use ort::execution_providers::DirectMLExecutionProvider;
    // DirectML wants sequential execution and no memory pattern (ONNX Runtime's own advice).
    Ok(Session::builder()?
        .with_parallel_execution(false)?
        .with_memory_pattern(false)?
        .with_execution_providers([DirectMLExecutionProvider::default().build().error_on_failure()])?
        .commit_from_file(path)?)
}

#[cfg(not(windows))]
pub(crate) fn gpu_session(_: &Path) -> Result<Session, MlError> {
    Err(MlError::Missing("no GPU provider on this platform".into()))
}
