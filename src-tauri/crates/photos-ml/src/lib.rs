//! On-device models (Phase 7). Pixels and samples in, numbers out: this crate decodes no
//! files and opens no sockets. The binary hands it buffers and stores what it returns.

pub mod beats;
pub mod clip;
pub mod denoise;
pub mod faces;
pub mod facet;
pub mod manifest;
pub mod ocr;
pub mod pets;
pub mod retouch;
pub mod store;
pub mod text;
mod prototypes;

pub use prototypes::prototypes;

#[derive(Debug, thiserror::Error)]
pub enum MlError {
    #[error("model not available: {0}")]
    Missing(String),
    #[error("inference failed: {0}")]
    Runtime(String),
    #[error("unexpected tensor shape: {0}")]
    Shape(String),
}

impl From<ort::Error> for MlError {
    fn from(e: ort::Error) -> Self {
        MlError::Runtime(e.to_string())
    }
}
