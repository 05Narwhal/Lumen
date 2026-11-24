//! A model Lumen knows nothing about, run on f32 tensors (Round 24 §8.4): the plugin host's
//! `models.run`. Same execution providers as the other models (DirectML first on Windows, then the
//! CPU); no pre- or post-processing, no knowledge of what the numbers mean.

use std::borrow::Cow;
use std::path::Path;

use ort::session::{Session, SessionInputValue};
use ort::value::Tensor;

use crate::MlError;

/// One named f32 tensor, row-major over `shape`.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedTensor {
    pub name: String,
    pub shape: Vec<usize>,
    pub data: Vec<f32>,
}

pub struct OnnxModel {
    session: Session,
    ep: &'static str,
}

impl OnnxModel {
    /// `gpu` tries DirectML first; if it cannot open the model the CPU takes over.
    pub fn open(path: &Path, gpu: bool) -> Result<Self, MlError> {
        let on_gpu = if gpu {
            crate::retouch::gpu_session(path).inspect_err(|e| tracing::warn!("model {}: GPU unavailable, using the CPU: {e}", path.display())).ok()
        } else {
            None
        };
        let (session, ep) = match on_gpu {
            Some(s) => (s, "directml"),
            None => (Session::builder()?.with_intra_threads(4)?.with_intra_op_spinning(false)?.commit_from_file(path)?, "cpu"),
        };
        Ok(Self { session, ep })
    }

    /// `"directml"` or `"cpu"`.
    pub fn ep(&self) -> &'static str {
        self.ep
    }

    /// Runs the model; every output comes back as f32 (a half-precision output is widened).
    pub fn run(&mut self, inputs: Vec<NamedTensor>) -> Result<Vec<NamedTensor>, MlError> {
        let mut feed: Vec<(Cow<'static, str>, SessionInputValue<'_>)> = Vec::with_capacity(inputs.len());
        for t in inputs {
            if t.shape.iter().product::<usize>() != t.data.len() {
                return Err(MlError::Shape(format!("{}: shape {:?} holds {} values, not {}", t.name, t.shape, t.shape.iter().product::<usize>(), t.data.len())));
            }
            let tensor = Tensor::from_array((t.shape, t.data))?;
            feed.push((Cow::Owned(t.name), SessionInputValue::from(tensor)));
        }
        let names: Vec<String> = self.session.outputs.iter().map(|o| o.name.clone()).collect();
        let out = self.session.run(feed)?;
        let mut result = Vec::with_capacity(names.len());
        for name in names {
            let (shape, data) = out[name.as_str()].try_extract_tensor::<f32>()?;
            result.push(NamedTensor { shape: shape.iter().map(|&d| d.max(0) as usize).collect(), data: data.to_vec(), name });
        }
        Ok(result)
    }
}
