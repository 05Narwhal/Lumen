//! Faces (Phase 7b): YuNet finds them, SFace describes them. Both follow OpenCV's own
//! wrappers exactly — `FaceDetectorYN` (BGR 0..255 input padded to a multiple of 32, the
//! decoding below, NMS) and `FaceRecognizerSF` (a similarity warp of five landmarks onto the
//! ArcFace template, RGB 0..255 input) — because that is what the published weights expect,
//! and the test fixture is OpenCV's answer on a public-domain photograph.

use std::path::Path;

use ort::{inputs, session::Session, value::Tensor};
use photos_core::embedding::{quantize, Vector};

use crate::manifest::ModelEntry;
use crate::MlError;

/// Where the five landmarks of a 112×112 aligned face belong: eyes, nose tip, mouth corners.
pub const ARCFACE_112: [[f32; 2]; 5] = [[38.2946, 51.6963], [73.5318, 51.5014], [56.0252, 71.7366], [41.5493, 92.3655], [70.7299, 92.2041]];

const STRIDES: [usize; 3] = [8, 16, 32];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Face {
    /// x, y, width, height in source pixels.
    pub bbox: [f32; 4],
    /// Right eye, left eye, nose tip, right and left mouth corner, in source pixels.
    pub landmarks: [[f32; 2]; 5],
    pub score: f32,
}

/// One stride's raw outputs: classification, objectness, box (4) and landmarks (10) per anchor.
pub struct StrideOutput<'a> {
    pub cls: &'a [f32],
    pub obj: &'a [f32],
    pub bbox: &'a [f32],
    pub kps: &'a [f32],
}

fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let (x1, y1) = (a[0].max(b[0]), a[1].max(b[1]));
    let (x2, y2) = ((a[0] + a[2]).min(b[0] + b[2]), (a[1] + a[3]).min(b[1] + b[3]));
    let inter = (x2 - x1).max(0.0) * (y2 - y1).max(0.0);
    let union = a[2] * a[3] + b[2] * b[3] - inter;
    if union > 0.0 { inter / union } else { 0.0 }
}

/// YuNet's anchors decoded into faces above `score`, then greedy NMS at `nms`. `pad_w` and
/// `pad_h` are the padded input's size (multiples of 32).
pub fn decode_yunet(outputs: &[StrideOutput<'_>; 3], pad_w: usize, pad_h: usize, score: f32, nms: f32) -> Vec<Face> {
    let mut faces = Vec::new();
    for (o, stride) in outputs.iter().zip(STRIDES) {
        let (cols, rows) = (pad_w / stride, pad_h / stride);
        for r in 0..rows {
            for c in 0..cols {
                let i = r * cols + c;
                if i >= o.cls.len() {
                    continue;
                }
                let s = (o.cls[i].clamp(0.0, 1.0) * o.obj[i].clamp(0.0, 1.0)).sqrt();
                if s < score {
                    continue;
                }
                let st = stride as f32;
                let b = &o.bbox[i * 4..i * 4 + 4];
                let (cx, cy) = ((c as f32 + b[0]) * st, (r as f32 + b[1]) * st);
                let (w, h) = (b[2].exp() * st, b[3].exp() * st);
                let k = &o.kps[i * 10..i * 10 + 10];
                let mut landmarks = [[0f32; 2]; 5];
                for (n, l) in landmarks.iter_mut().enumerate() {
                    *l = [(k[2 * n] + c as f32) * st, (k[2 * n + 1] + r as f32) * st];
                }
                faces.push(Face { bbox: [cx - w / 2.0, cy - h / 2.0, w, h], landmarks, score: s });
            }
        }
    }
    faces.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut kept: Vec<Face> = Vec::new();
    for f in faces {
        if kept.iter().all(|k| iou(&k.bbox, &f.bbox) <= nms) {
            kept.push(f);
        }
    }
    kept
}

/// Round 4I: the tiles a `width × height` image is looked at in, `(x, y, w, h)`: along each
/// axis longer than `side`, one span at each end (so they overlap in the middle and a face
/// cut by one tile's edge is whole in the other); a shorter axis is one span.
pub fn tiles(width: u32, height: u32, side: u32) -> Vec<(u32, u32, u32, u32)> {
    let spans = |len: u32| if len <= side { vec![(0, len)] } else { vec![(0, side), (len - side, side)] };
    let mut out = Vec::new();
    for (y, h) in spans(height) {
        for (x, w) in spans(width) {
            out.push((x, y, w, h));
        }
    }
    out
}

/// Faces found by several passes over one photo (the whole of it and its tiles) made one list:
/// best first, a face dropped when it overlaps a better one by `nms` IoU or lies mostly inside
/// it (a tile's edge cuts a face into a smaller box that IoU alone would keep).
pub fn merge_faces(mut faces: Vec<Face>, nms: f32) -> Vec<Face> {
    const INSIDE: f32 = 0.6;
    faces.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut kept: Vec<Face> = Vec::new();
    for f in faces {
        let clashes = kept.iter().any(|k| {
            let (x1, y1) = (k.bbox[0].max(f.bbox[0]), k.bbox[1].max(f.bbox[1]));
            let (x2, y2) = ((k.bbox[0] + k.bbox[2]).min(f.bbox[0] + f.bbox[2]), (k.bbox[1] + k.bbox[3]).min(f.bbox[1] + f.bbox[3]));
            let inter = (x2 - x1).max(0.0) * (y2 - y1).max(0.0);
            let smaller = (k.bbox[2] * k.bbox[3]).min(f.bbox[2] * f.bbox[3]);
            iou(&k.bbox, &f.bbox) > nms || (smaller > 0.0 && inter / smaller > INSIDE)
        });
        if !clashes {
            kept.push(f);
        }
    }
    kept
}

/// The least-squares similarity (rotation, uniform scale, shift) taking `src` onto `dst`, as
/// a 2×3 matrix. Closed form: with both sets centred, `a = Σ s·d`, `b = Σ s×d` and the scaled
/// rotation is `[[a, −b], [b, a]] / Σ|s|²`.
pub fn similarity_transform(src: &[[f32; 2]; 5], dst: &[[f32; 2]; 5]) -> [[f32; 3]; 2] {
    let mean = |p: &[[f32; 2]; 5]| {
        let (x, y) = p.iter().fold((0.0, 0.0), |(x, y), q| (x + q[0], y + q[1]));
        [x / 5.0, y / 5.0]
    };
    let (ms, md) = (mean(src), mean(dst));
    let (mut a, mut b, mut ss) = (0f32, 0f32, 0f32);
    for (s, d) in src.iter().zip(dst) {
        let (sx, sy, dx, dy) = (s[0] - ms[0], s[1] - ms[1], d[0] - md[0], d[1] - md[1]);
        a += sx * dx + sy * dy;
        b += sx * dy - sy * dx;
        ss += sx * sx + sy * sy;
    }
    let (ca, sb) = if ss > 0.0 { (a / ss, b / ss) } else { (1.0, 0.0) };
    let tx = md[0] - (ca * ms[0] - sb * ms[1]);
    let ty = md[1] - (sb * ms[0] + ca * ms[1]);
    [[ca, -sb, tx], [sb, ca, ty]]
}

/// A 112×112 RGB crop with the face's landmarks on the ArcFace template: every output pixel
/// is sampled bilinearly from where the inverse warp puts it in the source (OpenCV's
/// `warpAffine` with `INTER_LINEAR` and a zero border).
pub fn align_crop(rgb: &[u8], width: u32, height: u32, landmarks: &[[f32; 2]; 5]) -> Vec<u8> {
    let m = similarity_transform(landmarks, &ARCFACE_112);
    // Invert [[a, −b], [b, a]] and the shift.
    let (a, b) = (m[0][0], m[1][0]);
    let det = a * a + b * b;
    let inv = [[a / det, b / det], [-b / det, a / det]];
    let (w, h) = (width as i64, height as i64);
    let px = |x: i64, y: i64, ch: usize| -> f32 {
        if x < 0 || y < 0 || x >= w || y >= h { 0.0 } else { rgb[((y * w + x) * 3) as usize + ch] as f32 }
    };
    let mut out = vec![0u8; 112 * 112 * 3];
    for oy in 0..112 {
        for ox in 0..112 {
            let (u, v) = (ox as f32 - m[0][2], oy as f32 - m[1][2]);
            let sx = inv[0][0] * u + inv[0][1] * v;
            let sy = inv[1][0] * u + inv[1][1] * v;
            let (x0, y0) = (sx.floor() as i64, sy.floor() as i64);
            let (fx, fy) = (sx - x0 as f32, sy - y0 as f32);
            for ch in 0..3 {
                let top = px(x0, y0, ch) * (1.0 - fx) + px(x0 + 1, y0, ch) * fx;
                let bottom = px(x0, y0 + 1, ch) * (1.0 - fx) + px(x0 + 1, y0 + 1, ch) * fx;
                out[(oy * 112 + ox) * 3 + ch] = (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// Thresholds the manifest gives the face models.
fn spec_f32(entry: &ModelEntry, key: &str, default: f32) -> f32 {
    entry.faces.as_ref().and_then(|f| f.get(key)).and_then(|v| v.as_f64()).map(|v| v as f32).unwrap_or(default)
}

pub struct FaceDetector {
    session: Session,
    score: f32,
    nms: f32,
    min_side: f32,
}

impl FaceDetector {
    pub fn open(path: &Path, entry: &ModelEntry) -> Result<Self, MlError> {
        Ok(Self {
            session: Session::builder()?.with_intra_threads(2)?.commit_from_file(path)?,
            score: spec_f32(entry, "score", 0.8),
            nms: spec_f32(entry, "nms", 0.3),
            min_side: spec_f32(entry, "min_side_px", 24.0),
        })
    }

    /// The faces in a packed RGB image, best first.
    pub fn detect(&mut self, rgb: &[u8], width: u32, height: u32) -> Result<Vec<Face>, MlError> {
        let (w, h) = (width as usize, height as usize);
        let (pw, ph) = (((w.max(1) - 1) / 32 + 1) * 32, ((h.max(1) - 1) / 32 + 1) * 32);
        // BGR, 0..255, NCHW, zero-padded right and bottom — what FaceDetectorYN feeds.
        let mut input = vec![0f32; 3 * pw * ph];
        for y in 0..h {
            for x in 0..w {
                let p = (y * w + x) * 3;
                for (ch, from) in [2usize, 1, 0].iter().enumerate() {
                    input[ch * pw * ph + y * pw + x] = rgb[p + from] as f32;
                }
            }
        }
        let t = Tensor::from_array(([1usize, 3, ph, pw], input))?;
        let out = self.session.run(inputs!["input" => t])?;
        let get = |name: String| out[name.as_str()].try_extract_tensor::<f32>().map(|(_, d)| d.to_vec());
        let mut raw = Vec::with_capacity(3);
        for s in STRIDES {
            raw.push((get(format!("cls_{s}"))?, get(format!("obj_{s}"))?, get(format!("bbox_{s}"))?, get(format!("kps_{s}"))?));
        }
        let outputs = [0, 1, 2].map(|i| StrideOutput { cls: &raw[i].0, obj: &raw[i].1, bbox: &raw[i].2, kps: &raw[i].3 });
        Ok(decode_yunet(&outputs, pw, ph, self.score, self.nms)
            .into_iter()
            .filter(|f| f.bbox[2].min(f.bbox[3]) >= self.min_side)
            .collect())
    }

    /// Round 4I's second look: the whole image, then each `tiles(…, side)` at its own
    /// resolution (small faces in a group photo are too few pixels for the whole-image pass),
    /// merged into one list in the image's coordinates.
    pub fn detect_tiled(&mut self, rgb: &[u8], width: u32, height: u32, side: u32) -> Result<Vec<Face>, MlError> {
        let mut all = self.detect(rgb, width, height)?;
        let parts = tiles(width, height, side);
        if parts.len() > 1 {
            for (x, y, w, h) in parts {
                let mut tile = Vec::with_capacity((w * h * 3) as usize);
                for row in y..y + h {
                    let start = ((row * width + x) * 3) as usize;
                    tile.extend_from_slice(&rgb[start..start + (w * 3) as usize]);
                }
                for mut f in self.detect(&tile, w, h)? {
                    f.bbox[0] += x as f32;
                    f.bbox[1] += y as f32;
                    for l in f.landmarks.iter_mut() {
                        l[0] += x as f32;
                        l[1] += y as f32;
                    }
                    all.push(f);
                }
            }
        }
        Ok(merge_faces(all, self.nms))
    }
}

pub struct FaceEmbedder {
    session: Session,
}

impl FaceEmbedder {
    pub fn open(path: &Path, _entry: &ModelEntry) -> Result<Self, MlError> {
        Ok(Self { session: Session::builder()?.with_intra_threads(2)?.commit_from_file(path)? })
    }

    /// The 128-d description of an aligned 112×112 RGB face, quantised.
    pub fn embed(&mut self, aligned_rgb: &[u8]) -> Result<Vector, MlError> {
        if aligned_rgb.len() != 112 * 112 * 3 {
            return Err(MlError::Shape(format!("{} bytes, not an aligned face", aligned_rgb.len())));
        }
        let plane = 112 * 112;
        let mut input = vec![0f32; 3 * plane];
        for (i, px) in aligned_rgb.as_chunks::<3>().0.iter().enumerate() {
            for ch in 0..3 {
                input[ch * plane + i] = px[ch] as f32;
            }
        }
        let t = Tensor::from_array(([1usize, 3, 112, 112], input))?;
        let out = self.session.run(inputs!["data" => t])?;
        let (_, data) = out["fc1"].try_extract_tensor::<f32>()?;
        Ok(quantize(data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty(n: usize) -> (Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>) {
        (vec![0.0; n], vec![0.0; n], vec![0.0; n * 4], vec![0.0; n * 10])
    }

    #[test]
    fn decodes_an_anchor_into_a_box_and_landmarks() {
        // A 64×64 input: 8×8 anchors at stride 8, 4×4 at 16, 2×2 at 32.
        let (mut c8, mut o8, mut b8, mut k8) = empty(64);
        let (c16, o16, b16, k16) = empty(16);
        let (c32, o32, b32, k32) = empty(4);
        let i = 2 * 8 + 3; // row 2, column 3
        c8[i] = 0.81;
        o8[i] = 1.0;
        b8[i * 4..i * 4 + 4].copy_from_slice(&[0.5, 0.5, 1.0f32.ln() + 2.0f32.ln(), 3.0f32.ln()]);
        k8[i * 10..i * 10 + 2].copy_from_slice(&[0.25, -0.5]);
        let out = [
            StrideOutput { cls: &c8, obj: &o8, bbox: &b8, kps: &k8 },
            StrideOutput { cls: &c16, obj: &o16, bbox: &b16, kps: &k16 },
            StrideOutput { cls: &c32, obj: &o32, bbox: &b32, kps: &k32 },
        ];
        let faces = decode_yunet(&out, 64, 64, 0.5, 0.3);
        assert_eq!(faces.len(), 1);
        let f = faces[0];
        assert!((f.score - 0.9).abs() < 1e-5, "sqrt(cls · obj)");
        // Centre (3.5, 2.5) × 8 = (28, 20); size (2, 3) × 8 = (16, 24).
        assert!((f.bbox[0] - 20.0).abs() < 1e-4 && (f.bbox[1] - 8.0).abs() < 1e-4, "{:?}", f.bbox);
        assert!((f.bbox[2] - 16.0).abs() < 1e-4 && (f.bbox[3] - 24.0).abs() < 1e-4);
        assert!((f.landmarks[0][0] - 26.0).abs() < 1e-4 && (f.landmarks[0][1] - 12.0).abs() < 1e-4);
    }

    #[test]
    fn nms_keeps_the_better_of_two_overlapping_faces() {
        let f = |x: f32, s: f32| Face { bbox: [x, 0.0, 10.0, 10.0], landmarks: [[0.0; 2]; 5], score: s };
        assert!(iou(&f(0.0, 1.0).bbox, &f(1.0, 1.0).bbox) > 0.3);
        assert_eq!(iou(&f(0.0, 1.0).bbox, &f(50.0, 1.0).bbox), 0.0);
    }

    /// Round 4I: a 2560 × 1920 photo is looked at in 1536 px tiles, two along each axis,
    /// overlapping, together covering every pixel; a side no longer than a tile is one span.
    #[test]
    fn tiles_cover_the_photo_with_overlap() {
        let t = tiles(2560, 1920, 1536);
        assert_eq!(t, vec![(0, 0, 1536, 1536), (1024, 0, 1536, 1536), (0, 384, 1536, 1536), (1024, 384, 1536, 1536)]);
        for y in [0, 1000, 1919] {
            for x in [0, 1300, 2559] {
                assert!(t.iter().any(|&(tx, ty, tw, th)| x >= tx && x < tx + tw && y >= ty && y < ty + th), "({x}, {y}) is in a tile");
            }
        }
        assert_eq!(tiles(1400, 1000, 1536), vec![(0, 0, 1400, 1000)]);
        assert_eq!(tiles(2560, 1200, 1536), vec![(0, 0, 1536, 1200), (1024, 0, 1536, 1200)]);
    }

    /// The same face seen by the whole-photo pass and by two tiles (one of which cuts it at
    /// its edge) is one face, the best-scored; a face beside it stays.
    #[test]
    fn nms_across_tiles_keeps_one_box_per_face() {
        let f = |x: f32, w: f32, s: f32| Face { bbox: [x, 100.0, w, 40.0], landmarks: [[0.0; 2]; 5], score: s };
        let merged = merge_faces(vec![f(500.0, 40.0, 0.85), f(501.0, 39.0, 0.92), f(500.0, 22.0, 0.81), f(900.0, 40.0, 0.9)], 0.3);
        assert_eq!(merged.len(), 2, "{merged:?}");
        assert_eq!(merged[0].score, 0.92);
        assert_eq!(merged[1].bbox[0], 900.0);
    }

    #[test]
    fn the_template_maps_onto_itself() {
        let m = similarity_transform(&ARCFACE_112, &ARCFACE_112);
        for (r, want) in m.iter().zip([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]) {
            for (a, b) in r.iter().zip(want) {
                assert!((a - b).abs() < 1e-4, "{m:?}");
            }
        }
    }

    #[test]
    fn a_known_rotation_scale_and_shift_is_recovered() {
        let (s, t, dx, dy) = (1.7f32, 0.3f32, 12.0f32, -5.0f32);
        let src = ARCFACE_112.map(|p| [p[0] * 2.0 + 100.0, p[1] * 2.0 + 40.0]);
        let dst = src.map(|p| [s * (t.cos() * p[0] - t.sin() * p[1]) + dx, s * (t.sin() * p[0] + t.cos() * p[1]) + dy]);
        let m = similarity_transform(&src, &dst);
        assert!((m[0][0] - s * t.cos()).abs() < 1e-3 && (m[1][0] - s * t.sin()).abs() < 1e-3, "{m:?}");
        assert!((m[0][2] - dx).abs() < 1e-2 && (m[1][2] - dy).abs() < 1e-2, "{m:?}");
    }

    #[test]
    fn an_aligned_crop_is_112_square_and_puts_the_eyes_on_the_template() {
        // A 200×200 grey image with a white dot where the right eye is.
        let mut rgb = vec![60u8; 200 * 200 * 3];
        let lm = ARCFACE_112.map(|p| [p[0] + 40.0, p[1] + 30.0]);
        let (ex, ey) = (lm[0][0].round() as usize, lm[0][1].round() as usize);
        for c in 0..3 {
            rgb[(ey * 200 + ex) * 3 + c] = 255;
        }
        let crop = align_crop(&rgb, 200, 200, &lm);
        assert_eq!(crop.len(), 112 * 112 * 3);
        let (tx, ty) = (ARCFACE_112[0][0].round() as usize, ARCFACE_112[0][1].round() as usize);
        assert!(crop[(ty * 112 + tx) * 3] > 150, "the eye lands on the template");
    }
}
