#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("decode failed: {0}")]
    Decode(String),
    #[error("unsupported format: {0}")]
    Unsupported(String),
}

impl From<image::ImageError> for MediaError {
    fn from(e: image::ImageError) -> Self {
        MediaError::Decode(e.to_string())
    }
}
