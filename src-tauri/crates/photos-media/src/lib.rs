//! Lumen media: format detection, metadata, decoding, resizing and derivative paths.
//! Depends on nothing from Tauri.

pub mod compare;
pub mod decode;
pub mod develop;
pub mod encode;
pub mod error;
pub mod exiftool;
pub mod failure;
pub mod ffmpeg;
pub mod hdr;
pub mod raw_lib;
pub mod recover;
pub mod heif;
pub mod icc;
pub mod kind;
pub mod metadata;
pub mod metadata_transfer;
pub mod trim_export;
pub mod video_frames;
pub mod beats;
pub mod frame;
pub mod retouch_input;
pub mod slideshow;
pub mod store;
pub mod thumbs;
pub mod xmp;

pub use decode::{
    apply_orientation, decode_preview, decode_preview_hdr, decode_preview_with, decoder_orients, effective_orientation,
    orientation_swaps,
};
pub use error::MediaError;
pub use exiftool::ExifTool;
pub use ffmpeg::{playback_for, playback_with, proxy_plan, PROXY_MAX_BITRATE, REMUX_MAX_BYTES, Ffmpeg, Playback, Probe, ProxyPlan, VideoSupport};
pub use hdr::HdrKind;
pub use kind::*;
pub use metadata::{apply_probe, capture_in_head, choose_capture, exif_thumbnail, read_metadata, read_metadata_with, CaptureSource, CaptureTime, Metadata, RawDates, METADATA_VERSION};
pub use store::DerivativeStore;
pub use thumbs::{dhash, dominant_color, sharpness, encode_jpeg, encode_jpeg_bytes, encode_png, resize_long_edge, thumbhash_for};

/// Thumbnail long edges served by the protocol.
pub const THUMB_SIZES: &[u32] = &[256, 1024, 2560];
pub const JPEG_QUALITY: u8 = 82;
