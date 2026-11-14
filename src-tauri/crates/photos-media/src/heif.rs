//! A HEIF writer of Lumen's own (Phase 8 spec §5): FFmpeg encodes HEVC but has no HEIF
//! muxer, so this assembles the container the way iPhones write it — a `grid` image of
//! 512 × 512 `hvc1` tiles, with the colour declared as `nclx` *and* an ICC profile (ExifTool
//! adds EXIF and XMP afterwards but cannot add an ICC profile to HEIF).
//!
//! Pure: it takes the encoder's Annex B stream (one picture per tile, in raster order) and
//! returns the file's bytes.

use crate::MediaError;

pub const TILE: u32 = 512;

/// How the pixels were encoded, as declared in `colr nclx` (ITU-T H.273 code points).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Nclx {
    pub primaries: u16,
    pub transfer: u16,
    pub matrix: u16,
    pub full_range: bool,
}

/// sRGB: BT.709 primaries, the sRGB curve, BT.709 matrix, full range.
pub const NCLX_SRGB: Nclx = Nclx { primaries: 1, transfer: 13, matrix: 1, full_range: true };

// ---------------------------------------------------------------------------------------
// HEVC bitstream

/// NAL unit types (H.265 table 7-1) Lumen cares about.
const NAL_VPS: u8 = 32;
const NAL_SPS: u8 = 33;
const NAL_PPS: u8 = 34;

fn nal_type(nal: &[u8]) -> u8 {
    (nal[0] >> 1) & 0x3f
}

/// The NAL units of an Annex B stream (start codes removed, trailing zeros trimmed).
pub fn split_annex_b(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::with_capacity(starts.len());
    for (k, &s) in starts.iter().enumerate() {
        let mut end = if k + 1 < starts.len() { starts[k + 1] - 3 } else { data.len() };
        while end > s && data[end - 1] == 0 {
            end -= 1;
        }
        if end > s + 1 {
            out.push(&data[s..end]);
        }
    }
    out
}

/// One coded picture: its slice NAL units, and the parameter sets that came before it.
#[derive(Debug, Default)]
pub struct Picture<'a> {
    pub params: Vec<&'a [u8]>,
    pub slices: Vec<&'a [u8]>,
}

/// Group NAL units into pictures: a VCL unit whose `first_slice_segment_in_pic_flag` is set
/// starts a new one. SEI and access-unit delimiters are dropped; HEIF has no use for them.
pub fn pictures<'a>(nals: &[&'a [u8]]) -> Vec<Picture<'a>> {
    let mut out: Vec<Picture<'a>> = Vec::new();
    let mut pending: Vec<&'a [u8]> = Vec::new();
    for &nal in nals {
        let t = nal_type(nal);
        if t < 32 {
            let first = nal.len() > 2 && nal[2] & 0x80 != 0;
            if first || out.is_empty() {
                out.push(Picture { params: std::mem::take(&mut pending), slices: Vec::new() });
            }
            out.last_mut().expect("a picture").slices.push(nal);
        } else if matches!(t, NAL_VPS | NAL_SPS | NAL_PPS) {
            pending.push(nal);
        }
    }
    out
}

/// The RBSP of a NAL unit: its payload with emulation-prevention bytes (00 00 03) removed.
pub fn rbsp(nal: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(nal.len());
    let mut zeros = 0;
    for &b in &nal[2.min(nal.len())..] {
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        out.push(b);
    }
    out
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn bit(&mut self) -> Result<u32, MediaError> {
        let byte = *self.data.get(self.pos / 8).ok_or_else(|| MediaError::Decode("hevc: SPS ends early".into()))?;
        let b = (byte >> (7 - self.pos % 8)) & 1;
        self.pos += 1;
        Ok(b as u32)
    }
    fn bits(&mut self, n: u32) -> Result<u32, MediaError> {
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | self.bit()?;
        }
        Ok(v)
    }
    fn skip(&mut self, n: usize) {
        self.pos += n;
    }
    /// Unsigned exp-Golomb.
    fn ue(&mut self) -> Result<u32, MediaError> {
        let mut zeros = 0;
        while self.bit()? == 0 {
            zeros += 1;
            if zeros > 31 {
                return Err(MediaError::Decode("hevc: bad exp-Golomb code".into()));
            }
        }
        Ok((1u32 << zeros) - 1 + self.bits(zeros)?)
    }
}

/// What `hvcC` needs from a sequence parameter set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpsInfo {
    /// profile space, tier, profile, compatibility flags, constraint flags and level: the
    /// twelve bytes `hvcC` copies verbatim.
    pub general: [u8; 12],
    pub chroma_format: u8,
    pub bit_depth_luma: u8,
    pub bit_depth_chroma: u8,
    pub width: u32,
    pub height: u32,
}

pub fn parse_sps(nal: &[u8]) -> Result<SpsInfo, MediaError> {
    let r = rbsp(nal);
    if r.len() < 13 {
        return Err(MediaError::Decode("hevc: SPS too short".into()));
    }
    let max_sub_layers_minus1 = (r[0] >> 1) & 0x7;
    let mut general = [0u8; 12];
    general.copy_from_slice(&r[1..13]);
    let mut b = Bits { data: &r, pos: 8 + 96 };
    // profile_tier_level's sub-layer part (H.265 7.3.3).
    let n = max_sub_layers_minus1 as usize;
    let mut present = Vec::with_capacity(n);
    for _ in 0..n {
        present.push((b.bit()?, b.bit()?));
    }
    if n > 0 {
        b.skip(2 * (8 - n));
    }
    for (profile, level) in present {
        if profile == 1 {
            b.skip(88);
        }
        if level == 1 {
            b.skip(8);
        }
    }
    b.ue()?; // sps_seq_parameter_set_id
    let chroma_format = b.ue()? as u8;
    if chroma_format == 3 {
        b.skip(1); // separate_colour_plane_flag
    }
    let mut width = b.ue()?;
    let mut height = b.ue()?;
    if b.bit()? == 1 {
        // Conformance window, in chroma units.
        let (l, r_, t, bo) = (b.ue()?, b.ue()?, b.ue()?, b.ue()?);
        let (sx, sy) = match chroma_format {
            1 => (2, 2),
            2 => (2, 1),
            _ => (1, 1),
        };
        width -= sx * (l + r_);
        height -= sy * (t + bo);
    }
    let bit_depth_luma = b.ue()? as u8 + 8;
    let bit_depth_chroma = b.ue()? as u8 + 8;
    Ok(SpsInfo { general, chroma_format, bit_depth_luma, bit_depth_chroma, width, height })
}

// ---------------------------------------------------------------------------------------
// Boxes

fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(8 + body.len());
    v.extend_from_slice(&(8 + body.len() as u32).to_be_bytes());
    v.extend_from_slice(kind);
    v.extend_from_slice(body);
    v
}

fn full(kind: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(4 + body.len());
    b.push(version);
    b.extend_from_slice(&flags.to_be_bytes()[1..]);
    b.extend_from_slice(body);
    bx(kind, &b)
}

fn hvcc(sps: &SpsInfo, vps: &[u8], sps_nal: &[u8], pps: &[u8]) -> Vec<u8> {
    let mut b = vec![1u8];
    b.extend_from_slice(&sps.general);
    b.extend_from_slice(&0xF000u16.to_be_bytes()); // min_spatial_segmentation_idc = 0
    b.push(0xFC); // parallelismType = 0
    b.push(0xFC | (sps.chroma_format & 3));
    b.push(0xF8 | ((sps.bit_depth_luma - 8) & 7));
    b.push(0xF8 | ((sps.bit_depth_chroma - 8) & 7));
    b.extend_from_slice(&0u16.to_be_bytes()); // avgFrameRate
    b.push(0x0F); // one temporal layer, nested, 4-byte NAL lengths
    b.push(3);
    for (t, nal) in [(NAL_VPS, vps), (NAL_SPS, sps_nal), (NAL_PPS, pps)] {
        b.push(0x80 | t); // array_completeness
        b.extend_from_slice(&1u16.to_be_bytes());
        b.extend_from_slice(&(nal.len() as u16).to_be_bytes());
        b.extend_from_slice(nal);
    }
    bx(b"hvcC", &b)
}

fn ispe(w: u32, h: u32) -> Vec<u8> {
    let mut b = w.to_be_bytes().to_vec();
    b.extend_from_slice(&h.to_be_bytes());
    full(b"ispe", 0, 0, &b)
}

fn colr_nclx(c: Nclx) -> Vec<u8> {
    let mut b = b"nclx".to_vec();
    for v in [c.primaries, c.transfer, c.matrix] {
        b.extend_from_slice(&v.to_be_bytes());
    }
    b.push(if c.full_range { 0x80 } else { 0 });
    bx(b"colr", &b)
}

fn colr_icc(icc: &[u8]) -> Vec<u8> {
    let mut b = b"prof".to_vec();
    b.extend_from_slice(icc);
    bx(b"colr", &b)
}

fn pixi(bits: u8) -> Vec<u8> {
    full(b"pixi", 0, 0, &[3, bits, bits, bits])
}

fn infe(id: u16, kind: &[u8; 4], hidden: bool) -> Vec<u8> {
    let mut b = id.to_be_bytes().to_vec();
    b.extend_from_slice(&0u16.to_be_bytes());
    b.extend_from_slice(kind);
    b.push(0); // empty name
    full(b"infe", 2, if hidden { 1 } else { 0 }, &b)
}

/// The `grid` item's payload (ISO/IEC 23008-12 6.6.2.3.2).
fn grid_payload(rows: u32, cols: u32, w: u32, h: u32) -> Vec<u8> {
    let wide = w > 0xFFFF || h > 0xFFFF;
    let mut b = vec![0u8, if wide { 1 } else { 0 }, (rows - 1) as u8, (cols - 1) as u8];
    if wide {
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&h.to_be_bytes());
    } else {
        b.extend_from_slice(&(w as u16).to_be_bytes());
        b.extend_from_slice(&(h as u16).to_be_bytes());
    }
    b
}

/// Assemble a HEIC of `width × height` from `stream`, the Annex B output of encoding
/// `ceil(w/512) × ceil(h/512)` tiles of 512 × 512 in raster order.
pub fn write_grid(stream: &[u8], width: u32, height: u32, nclx: Nclx, icc: &[u8]) -> Result<Vec<u8>, MediaError> {
    let cols = width.div_ceil(TILE);
    let rows = height.div_ceil(TILE);
    if cols == 0 || rows == 0 || cols > 256 || rows > 256 {
        return Err(MediaError::Unsupported(format!("heif: {width}×{height} is outside a 256×256 grid of tiles")));
    }
    let nals = split_annex_b(stream);
    let pics = pictures(&nals);
    if pics.len() != (rows * cols) as usize {
        return Err(MediaError::Decode(format!("heif: expected {} tiles, the encoder gave {}", rows * cols, pics.len())));
    }
    let find = |t: u8| pics[0].params.iter().copied().find(|n| nal_type(n) == t).ok_or_else(|| MediaError::Decode(format!("heif: no NAL type {t} before the first tile")));
    let (vps, sps_nal, pps) = (find(NAL_VPS)?, find(NAL_SPS)?, find(NAL_PPS)?);
    let sps = parse_sps(sps_nal)?;
    if sps.width != TILE || sps.height != TILE {
        return Err(MediaError::Decode(format!("heif: tiles are {}×{}, not {TILE}×{TILE}", sps.width, sps.height)));
    }

    // Item 1 is the grid (the primary image); tiles are 2… in raster order.
    let tiles: Vec<Vec<u8>> = pics
        .iter()
        .map(|p| {
            let mut d = Vec::new();
            for s in &p.slices {
                d.extend_from_slice(&(s.len() as u32).to_be_bytes());
                d.extend_from_slice(s);
            }
            d
        })
        .collect();
    let grid = grid_payload(rows, cols, width, height);
    let n = tiles.len() as u16;
    let ids: Vec<u16> = (2..2 + n).collect();

    let ftyp = bx(b"ftyp", b"heic\0\0\0\0mif1heic");
    let meta_for = |mdat_start: u32| -> Vec<u8> {
        let hdlr = full(b"hdlr", 0, 0, &[0, 0, 0, 0, b'p', b'i', b'c', b't', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let pitm = full(b"pitm", 0, 0, &1u16.to_be_bytes());
        // iloc v0: 4-byte offsets and lengths, no base offset, one extent each.
        let mut iloc = vec![0x44, 0x00];
        iloc.extend_from_slice(&(n + 1).to_be_bytes());
        let mut at = mdat_start + 8;
        let mut entry = |id: u16, len: usize| {
            iloc.extend_from_slice(&id.to_be_bytes());
            iloc.extend_from_slice(&0u16.to_be_bytes());
            iloc.extend_from_slice(&1u16.to_be_bytes());
            iloc.extend_from_slice(&at.to_be_bytes());
            iloc.extend_from_slice(&(len as u32).to_be_bytes());
            at += len as u32;
        };
        entry(1, grid.len());
        for (id, t) in ids.iter().zip(&tiles) {
            entry(*id, t.len());
        }
        let iloc = full(b"iloc", 0, 0, &iloc);
        let mut iinf = (n + 1).to_be_bytes().to_vec();
        iinf.extend_from_slice(&infe(1, b"grid", false));
        for id in &ids {
            // Tiles are hidden: they are parts of the picture, not pictures of their own.
            iinf.extend_from_slice(&infe(*id, b"hvc1", true));
        }
        let iinf = full(b"iinf", 0, 0, &iinf);
        let mut dimg = 1u16.to_be_bytes().to_vec();
        dimg.extend_from_slice(&n.to_be_bytes());
        for id in &ids {
            dimg.extend_from_slice(&id.to_be_bytes());
        }
        let iref = full(b"iref", 0, 0, &bx(b"dimg", &dimg));
        // Properties, 1-based: 1 hvcC, 2 tile ispe, 3 image ispe, 4 nclx, 5 ICC, 6 pixi.
        let mut ipco = Vec::new();
        for p in [hvcc(&sps, vps, sps_nal, pps), ispe(TILE, TILE), ispe(width, height), colr_nclx(nclx), colr_icc(icc), pixi(sps.bit_depth_luma)] {
            ipco.extend_from_slice(&p);
        }
        let ipco = bx(b"ipco", &ipco);
        let mut ipma = ((n + 1) as u32).to_be_bytes().to_vec();
        ipma.extend_from_slice(&1u16.to_be_bytes());
        ipma.extend_from_slice(&[4, 3, 0x84, 5, 6]); // grid: ispe, nclx (essential), ICC, pixi
        for id in &ids {
            ipma.extend_from_slice(&id.to_be_bytes());
            ipma.extend_from_slice(&[3, 0x81, 2, 6]); // tile: hvcC (essential), ispe, pixi
        }
        let ipma = full(b"ipma", 0, 0, &ipma);
        let mut iprp = ipco;
        iprp.extend_from_slice(&ipma);
        let iprp = bx(b"iprp", &iprp);
        let mut body = Vec::new();
        for b in [hdlr, pitm, iloc, iinf, iref, iprp] {
            body.extend_from_slice(&b);
        }
        full(b"meta", 0, 0, &body)
    };
    // The meta box's size does not depend on the offsets inside it, so measure then write.
    let meta_len = meta_for(0).len() as u32;
    let meta = meta_for(ftyp.len() as u32 + meta_len);
    let mut mdat = grid.clone();
    for t in &tiles {
        mdat.extend_from_slice(t);
    }
    let mut out = ftyp;
    out.extend_from_slice(&meta);
    out.extend_from_slice(&bx(b"mdat", &mdat));
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn annex_b_splits_on_both_start_code_lengths() {
        let data = [0, 0, 0, 1, 0x40, 0x01, 0xAA, 0, 0, 1, 0x42, 0x01, 0xBB, 0xCC, 0, 0];
        let nals = split_annex_b(&data);
        assert_eq!(nals, vec![&[0x40, 0x01, 0xAA][..], &[0x42, 0x01, 0xBB, 0xCC][..]]);
        assert_eq!(nal_type(nals[0]), NAL_VPS);
        assert_eq!(nal_type(nals[1]), NAL_SPS);
    }

    #[test]
    fn emulation_prevention_bytes_are_removed() {
        assert_eq!(rbsp(&[0x42, 0x01, 0, 0, 3, 1, 0, 0, 3, 0]), vec![0, 0, 1, 0, 0, 0]);
    }

    #[test]
    fn pictures_start_at_a_first_slice_and_keep_their_parameter_sets() {
        let vps = [0x40u8, 1, 0];
        let slice_first = [0x26u8, 1, 0x80, 9]; // IDR_W_RADL, first_slice_segment_in_pic
        let slice_more = [0x26u8, 1, 0x00, 9];
        let sei = [0x4Eu8, 1, 5];
        let nals: Vec<&[u8]> = vec![&vps, &sei, &slice_first, &slice_more, &vps, &slice_first];
        let p = pictures(&nals);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].params, vec![&vps[..]]);
        assert_eq!(p[0].slices.len(), 2);
        assert_eq!(p[1].slices.len(), 1);
    }

    /// An SPS for 512×512 Main 4:2:0 8-bit, as x265 writes it, with one sub-layer.
    pub(crate) fn sample_sps() -> Vec<u8> {
        // Bits after the 2-byte NAL header: vps_id 0, max_sub_layers_minus1 0, nesting 1;
        // PTL: space 0, tier 0, profile 1, compat 0x60000000, constraint 0x90…, level 93;
        // sps_id ue(0)=1, chroma ue(1)=010, width ue(512), height ue(512), conformance 0,
        // bit depths ue(0)=1, ue(0)=1.
        let mut bits = String::new();
        bits += "0000"; // vps id
        bits += "000"; // max_sub_layers_minus1
        bits += "1";
        bits += "00"; // profile space
        bits += "0"; // tier
        bits += "00001"; // profile idc: Main
        bits += &format!("{:032b}", 0x6000_0000u32);
        bits += &format!("{:048b}", 0x9000_0000_0000u64);
        bits += &format!("{:08b}", 93);
        let ue = |v: u32| {
            let x = v + 1;
            let n = 32 - x.leading_zeros();
            format!("{}{:0width$b}", "0".repeat((n - 1) as usize), x, width = n as usize)
        };
        bits += &ue(0);
        bits += &ue(1);
        bits += &ue(512);
        bits += &ue(512);
        bits += "0";
        bits += &ue(0);
        bits += &ue(0);
        bits += "1";
        while !bits.len().is_multiple_of(8) {
            bits.push('0');
        }
        let mut out = vec![0x42, 0x01];
        for chunk in bits.as_bytes().chunks(8) {
            out.push(u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 2).unwrap());
        }
        out
    }

    #[test]
    fn the_sps_gives_what_hvcc_needs() {
        let s = parse_sps(&sample_sps()).unwrap();
        assert_eq!((s.width, s.height), (512, 512));
        assert_eq!(s.chroma_format, 1);
        assert_eq!((s.bit_depth_luma, s.bit_depth_chroma), (8, 8));
        assert_eq!(s.general[0], 0x01, "profile Main");
        assert_eq!(s.general[11], 93, "level 3.1");
    }

    /// Walk boxes at one level: (type, body).
    pub(crate) fn boxes(data: &[u8]) -> Vec<([u8; 4], &[u8])> {
        let mut out = Vec::new();
        let mut i = 0;
        while i + 8 <= data.len() {
            let size = u32::from_be_bytes(data[i..i + 4].try_into().unwrap()) as usize;
            let kind: [u8; 4] = data[i + 4..i + 8].try_into().unwrap();
            out.push((kind, &data[i + 8..i + size]));
            i += size;
        }
        out
    }

    #[test]
    fn the_file_is_a_grid_of_hidden_tiles_with_its_colour_declared() {
        // Two tiles across (a 700×300 image) with fake slice data.
        let sps = sample_sps();
        let mut stream = Vec::new();
        for _ in 0..2 {
            for nal in [&[0x40u8, 1, 0xC][..], &sps, &[0x44u8, 1, 0xC1]] {
                stream.extend_from_slice(&[0, 0, 0, 1]);
                stream.extend_from_slice(nal);
            }
            stream.extend_from_slice(&[0, 0, 0, 1, 0x26, 1, 0x80, 0xAB, 0xCD]);
        }
        let icc = crate::icc::srgb();
        let file = write_grid(&stream, 700, 300, NCLX_SRGB, icc).unwrap();
        let top = boxes(&file);
        let kinds: Vec<&[u8; 4]> = top.iter().map(|(k, _)| k).collect();
        assert_eq!(kinds, vec![b"ftyp", b"meta", b"mdat"]);
        assert_eq!(&top[0].1[..4], b"heic");
        let meta = boxes(&top[1].1[4..]);
        let inner: Vec<[u8; 4]> = meta.iter().map(|(k, _)| *k).collect();
        assert_eq!(inner, vec![*b"hdlr", *b"pitm", *b"iloc", *b"iinf", *b"iref", *b"iprp"]);
        // The iloc offsets point at the grid payload and each tile's length-prefixed slice.
        // Body: version/flags (4), sizes (2), count (2), then 14 bytes per item with its
        // extent offset 6 bytes in.
        let iloc = meta[2].1;
        let first_offset = u32::from_be_bytes(iloc[14..18].try_into().unwrap()) as usize;
        assert_eq!(&file[first_offset..first_offset + 4], &[0, 0, 0, 1], "grid: version 0, 16-bit, 1 row, 2 columns");
        let tile_offset = u32::from_be_bytes(iloc[28..32].try_into().unwrap()) as usize;
        assert_eq!(&file[tile_offset..tile_offset + 9], &[0, 0, 0, 5, 0x26, 1, 0x80, 0xAB, 0xCD]);
        // The ICC profile rides in a colr box.
        let window = file.windows(4).position(|w| w == b"prof").unwrap();
        assert_eq!(&file[window + 4..window + 8], &(icc.len() as u32).to_be_bytes());
    }

    #[test]
    fn a_tile_count_mismatch_is_an_error() {
        let sps = sample_sps();
        let mut stream = vec![0, 0, 0, 1, 0x40, 1, 0xC, 0, 0, 0, 1];
        stream.extend_from_slice(&sps);
        stream.extend_from_slice(&[0, 0, 0, 1, 0x44, 1, 0xC1, 0, 0, 0, 1, 0x26, 1, 0x80, 1]);
        assert!(write_grid(&stream, 1024, 512, NCLX_SRGB, crate::icc::srgb()).is_err());
    }
}
