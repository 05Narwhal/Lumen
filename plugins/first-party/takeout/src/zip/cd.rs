//! Central directory parsing: EOCD, the zip64 locator and record, and the entries. A zip with no
//! readable end (a cut-off download) is walked forward by its local headers instead.

use super::ZipEntry;
use miniz_oxide::inflate::stream::{inflate, InflateState};
use miniz_oxide::{DataFormat, MZError, MZFlush, MZStatus};

const EOCD_SIG: u32 = 0x0605_4b50;
const LOC64_SIG: u32 = 0x0706_4b50;
const EOCD64_SIG: u32 = 0x0606_4b50;
const CD_SIG: u32 = 0x0201_4b50;
const LOCAL_SIG: u32 = 0x0403_4b50;
const DESC_SIG: u32 = 0x0807_4b50;
/// Largest EOCD (22 bytes) plus the longest comment.
const EOCD_SEARCH: u64 = 22 + 65535;
const CHUNK: u64 = 1 << 20;
/// The most one read_range call will gather; a bigger central directory is not trusted.
const MAX_RANGE: u64 = 256 << 20;
/// Input and output chunk when inflating only to find where a stream ends.
const INFLATE_CHUNK: usize = 64 << 10;

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    b.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}
fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    b.get(i..i + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}
fn u64_at(b: &[u8], i: usize) -> Option<u64> {
    b.get(i..i + 8).map(|s| u64::from_le_bytes(s.try_into().unwrap()))
}

/// A read of at most `n` bytes (a host may return fewer, never more than asked).
fn read_capped(read_at: &dyn Fn(u64, u32) -> Vec<u8>, start: u64, n: u32) -> Vec<u8> {
    let mut v = read_at(start, n);
    v.truncate(n as usize);
    v
}

/// Reads `[start, start + n)` in chunks, stopping early on a short read. `None` when `n` is above
/// [`MAX_RANGE`].
fn read_range(read_at: &dyn Fn(u64, u32) -> Vec<u8>, start: u64, n: u64) -> Option<Vec<u8>> {
    if n > MAX_RANGE {
        return None;
    }
    let mut out = Vec::new();
    while (out.len() as u64) < n {
        let want = (n - out.len() as u64).min(CHUNK) as u32;
        let Some(at) = start.checked_add(out.len() as u64) else { break };
        let got = read_capped(read_at, at, want);
        if got.is_empty() {
            break;
        }
        out.extend_from_slice(&got);
    }
    Some(out)
}

fn skip_reason(flags: u16, method: u16) -> Option<String> {
    if flags & 1 != 0 {
        Some("encrypted".into())
    } else if method != 0 && method != 8 {
        Some(format!("unsupported compression method {method}"))
    } else {
        None
    }
}

fn name_of(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw).into_owned()
}

/// Work units (a local header, or one inflate chunk) between two progress reports.
const TICK_EVERY: u64 = 500;

/// Counts the work of walking a cut-off zip and reports it, so the host can show it and stop it.
struct Pace<'a> {
    tick: &'a dyn Fn(u64) -> Result<(), String>,
    every: u64,
    work: u64,
    stopped: Option<String>,
}

impl Pace<'_> {
    fn add(&mut self) {
        self.work += 1;
        if self.work.is_multiple_of(self.every) && self.stopped.is_none() {
            if let Err(e) = (self.tick)(self.work) {
                self.stopped = Some(e);
            }
        }
    }
}

/// [`read_index_ticking`] with no reports.
pub fn read_index(read_at: &dyn Fn(u64, u32) -> Vec<u8>, len: u64) -> Result<Vec<ZipEntry>, String> {
    read_index_ticking(read_at, len, &|_| Ok(()))
}

/// Lists a zip's files (directories are left out). Never panics on malformed input; a zip whose
/// end is missing or unreadable yields the entries found by walking its local headers, which can
/// take long: `tick(work)` is called every 500 headers or inflated 64 KB chunks there, and an
/// `Err` from it (the host cancelling) ends the walk with that error.
pub fn read_index_ticking(read_at: &dyn Fn(u64, u32) -> Vec<u8>, len: u64, tick: &dyn Fn(u64) -> Result<(), String>) -> Result<Vec<ZipEntry>, String> {
    let mut pace = Pace { tick, every: TICK_EVERY, work: 0, stopped: None };
    read_index_with(read_at, len, &mut pace)
}

fn read_index_with(read_at: &dyn Fn(u64, u32) -> Vec<u8>, len: u64, pace: &mut Pace) -> Result<Vec<ZipEntry>, String> {
    if len < 22 {
        return Err("too small to be a zip".into());
    }
    let tail_start = len.saturating_sub(EOCD_SEARCH);
    let tail = read_range(read_at, tail_start, len - tail_start).unwrap_or_default();
    let eocd = (0..tail.len().saturating_sub(21)).rev().find(|&i| u32_at(&tail, i) == Some(EOCD_SIG));
    let Some(i) = eocd else {
        return scan_local(read_at, len, pace);
    };
    let mut count = u16_at(&tail, i + 10).unwrap_or(0) as u64;
    let mut cd_size = u32_at(&tail, i + 12).unwrap_or(0) as u64;
    let mut cd_off = u32_at(&tail, i + 16).unwrap_or(0) as u64;
    if (count == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_off == 0xFFFF_FFFF) && i >= 20 && u32_at(&tail, i - 20) == Some(LOC64_SIG) {
        let rec_off = u64_at(&tail, i - 12).unwrap_or(u64::MAX);
        let rec = read_range(read_at, rec_off, 56).unwrap_or_default();
        if u32_at(&rec, 0) != Some(EOCD64_SIG) {
            return Err("zip64 end record not found".into());
        }
        count = u64_at(&rec, 32).unwrap_or(count);
        cd_size = u64_at(&rec, 40).unwrap_or(cd_size);
        cd_off = u64_at(&rec, 48).unwrap_or(cd_off);
    }
    if cd_size > MAX_RANGE || cd_off.checked_add(cd_size).is_none_or(|e| e > len) {
        return scan_local(read_at, len, pace);
    }
    let cd = read_range(read_at, cd_off, cd_size).unwrap_or_default();
    let (entries, complete) = parse_cd(&cd, count);
    if !complete && entries.is_empty() {
        return scan_local(read_at, len, pace);
    }
    Ok(entries)
}

/// Returns the entries and whether all `count` records parsed.
fn parse_cd(cd: &[u8], count: u64) -> (Vec<ZipEntry>, bool) {
    let mut out = Vec::new();
    let mut p = 0usize;
    for _ in 0..count {
        if u32_at(cd, p) != Some(CD_SIG) || cd.len() < p + 46 {
            return (out, false);
        }
        let flags = u16_at(cd, p + 8).unwrap_or(0);
        let method = u16_at(cd, p + 10).unwrap_or(0);
        let mut comp = u32_at(cd, p + 20).unwrap_or(0) as u64;
        let mut size = u32_at(cd, p + 24).unwrap_or(0) as u64;
        let nlen = u16_at(cd, p + 28).unwrap_or(0) as usize;
        let elen = u16_at(cd, p + 30).unwrap_or(0) as usize;
        let clen = u16_at(cd, p + 32).unwrap_or(0) as usize;
        let mut off = u32_at(cd, p + 42).unwrap_or(0) as u64;
        let end = p + 46 + nlen + elen + clen;
        if cd.len() < end {
            return (out, false);
        }
        let name = name_of(&cd[p + 46..p + 46 + nlen]);
        let extra = &cd[p + 46 + nlen..p + 46 + nlen + elen];
        let ok = apply_zip64(extra, &mut size, &mut comp, &mut off);
        let modified = modified_of(extra, u16_at(cd, p + 12).unwrap_or(0), u16_at(cd, p + 14).unwrap_or(0));
        if !name.ends_with('/') {
            let skip = if ok { skip_reason(flags, method) } else { Some("corrupt zip64 extra".into()) };
            out.push(ZipEntry { name, method, compressed: comp, size, local_offset: off, skip, modified });
        }
        p = end;
    }
    (out, true)
}

/// The zip64 extra field (id 1) holds, in this order, only those of size, compressed size and
/// local offset whose 32-bit value is 0xFFFFFFFF. False when the field is too short for them.
fn apply_zip64(extra: &[u8], size: &mut u64, comp: &mut u64, off: &mut u64) -> bool {
    let mut i = 0;
    while i + 4 <= extra.len() {
        let id = u16_at(extra, i).unwrap_or(0);
        let l = u16_at(extra, i + 2).unwrap_or(0) as usize;
        if id == 1 {
            let body = extra.get(i + 4..i + 4 + l).unwrap_or(&[]);
            let mut q = 0;
            for v in [&mut *size, &mut *comp, &mut *off] {
                if *v == 0xFFFF_FFFF {
                    match u64_at(body, q) {
                        Some(x) => {
                            *v = x;
                            q += 8;
                        }
                        None => return false,
                    }
                }
            }
            return true;
        }
        i += 4 + l;
    }
    true
}

/// The entry's modification time as Unix seconds: the extended timestamp extra (id 0x5455, UTC)
/// when it has one, else the DOS time and date taken as UTC.
fn modified_of(extra: &[u8], dos_time: u16, dos_date: u16) -> Option<i64> {
    let mut i = 0;
    while i + 4 <= extra.len() {
        let id = u16_at(extra, i).unwrap_or(0);
        let l = u16_at(extra, i + 2).unwrap_or(0) as usize;
        if id == 0x5455 {
            let body = extra.get(i + 4..i + 4 + l).unwrap_or(&[]);
            if body.first().is_some_and(|f| f & 1 != 0) {
                if let Some(t) = u32_at(body, 1) {
                    return Some(t as i64);
                }
            }
        }
        i += 4 + l;
    }
    let (day, month, year) = ((dos_date & 31) as i64, ((dos_date >> 5) & 15) as i64, 1980 + (dos_date >> 9) as i64);
    let (sec, min, hour) = ((dos_time & 31) as i64 * 2, ((dos_time >> 5) & 63) as i64, (dos_time >> 11) as i64);
    if dos_date == 0 || !(1..=12).contains(&month) || day == 0 || hour > 23 || min > 59 || sec > 59 {
        return None;
    }
    // Days from the civil date (proleptic Gregorian).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hour * 3600 + min * 60 + sec)
}

fn has_zip64(extra: &[u8]) -> bool {
    let mut i = 0;
    while i + 4 <= extra.len() {
        if u16_at(extra, i) == Some(1) {
            return true;
        }
        i += 4 + u16_at(extra, i + 2).unwrap_or(0) as usize;
    }
    false
}

/// Inflates the raw deflate stream at `start` in bounded chunks, discarding the output, to learn
/// its compressed and uncompressed lengths. `Err` carries the progress made when the stream is cut
/// off or corrupt.
fn inflate_extent(read_at: &dyn Fn(u64, u32) -> Vec<u8>, start: u64, len: u64, pace: &mut Pace) -> Result<(u64, u64), (u64, u64)> {
    let mut st = InflateState::new_boxed(DataFormat::Raw);
    let mut out = vec![0u8; INFLATE_CHUNK];
    let (mut consumed, mut written) = (0u64, 0u64);
    let mut buf: Vec<u8> = Vec::new();
    let mut bp = 0usize;
    loop {
        if bp >= buf.len() {
            let left = len.saturating_sub(start.saturating_add(consumed));
            if left == 0 {
                return Err((consumed, written));
            }
            pace.add();
            if pace.stopped.is_some() {
                return Err((consumed, written));
            }
            buf = read_capped(read_at, start + consumed, left.min(INFLATE_CHUNK as u64) as u32);
            bp = 0;
            if buf.is_empty() {
                return Err((consumed, written));
            }
        }
        let r = inflate(&mut st, &buf[bp..], &mut out, MZFlush::None);
        bp += r.bytes_consumed;
        consumed += r.bytes_consumed as u64;
        written += r.bytes_written as u64;
        match r.status {
            Ok(MZStatus::StreamEnd) => return Ok((consumed, written)),
            Ok(_) | Err(MZError::Buf) => {
                if r.bytes_consumed == 0 && r.bytes_written == 0 {
                    return Err((consumed, written));
                }
            }
            Err(_) => return Err((consumed, written)),
        }
    }
}

/// A zip with no usable central directory: walk local headers from the start. Entries written
/// with a data descriptor (bit 3, sizes zero in the header, as Go's archive/zip does) are sized by
/// inflating their stream. A cut-off entry is listed with `skip: Some("truncated")`.
fn scan_local(read_at: &dyn Fn(u64, u32) -> Vec<u8>, len: u64, pace: &mut Pace) -> Result<Vec<ZipEntry>, String> {
    let mut out = Vec::new();
    let mut pos = 0u64;
    let entry = |name: String, method, compressed, size, local_offset, skip| ZipEntry { name, method, compressed, size, local_offset, skip, modified: None };
    while pos.saturating_add(30) <= len {
        pace.add();
        if let Some(e) = &pace.stopped {
            return Err(e.clone());
        }
        let h = read_capped(read_at, pos, 30);
        if h.len() < 30 || u32_at(&h, 0) != Some(LOCAL_SIG) {
            break;
        }
        let flags = u16_at(&h, 6).unwrap_or(0);
        let method = u16_at(&h, 8).unwrap_or(0);
        let mut comp = u32_at(&h, 18).unwrap_or(0) as u64;
        let mut size = u32_at(&h, 22).unwrap_or(0) as u64;
        let nlen = u16_at(&h, 26).unwrap_or(0) as usize;
        let elen = u16_at(&h, 28).unwrap_or(0) as usize;
        let var = read_capped(read_at, pos + 30, (nlen + elen) as u32);
        if var.len() < nlen + elen {
            let part = &var[..var.len().min(nlen)];
            if !part.is_empty() {
                out.push(entry(name_of(part), method, 0, 0, pos, Some("truncated".into())));
            }
            break;
        }
        let name = name_of(&var[..nlen]);
        let extra = &var[nlen..];
        let mut off = pos;
        let z64_ok = apply_zip64(extra, &mut size, &mut comp, &mut off);
        let data_start = pos + 30 + (nlen + elen) as u64;
        let is_dir = name.ends_with('/');
        let skip = if z64_ok { skip_reason(flags, method) } else { Some("corrupt zip64 extra".into()) };
        if flags & 8 != 0 && comp == 0 {
            if is_dir {
                pos = data_start;
                continue;
            }
            // Sizes live after the data: only a deflate stream can be measured.
            let reason = skip.clone().or_else(|| (method != 8).then(|| "stored entry with a data descriptor cannot be sized".to_string()));
            if let Some(r) = reason {
                out.push(entry(name, method, 0, 0, pos, Some(r)));
                break;
            }
            let extent = inflate_extent(read_at, data_start, len, pace);
            if let Some(e) = &pace.stopped {
                return Err(e.clone());
            }
            match extent {
                Err((c, w)) => {
                    out.push(entry(name, method, c, w, pos, Some("truncated".into())));
                    break;
                }
                Ok((c, w)) => {
                    let end = data_start + c;
                    let d = read_capped(read_at, end, 24);
                    let sig = if u32_at(&d, 0) == Some(DESC_SIG) { 4 } else { 0 };
                    let dlen = sig + 4 + if has_zip64(extra) { 16 } else { 8 };
                    if d.len() < dlen {
                        out.push(entry(name, method, c, w, pos, Some("truncated".into())));
                        break;
                    }
                    out.push(entry(name, method, c, w, pos, None));
                    pos = end + dlen as u64;
                }
            }
            continue;
        }
        let next = data_start.checked_add(comp);
        match next {
            Some(n) if n <= len => {
                if !is_dir {
                    out.push(entry(name, method, comp, size, pos, skip));
                }
                pos = n;
            }
            _ => {
                if !is_dir {
                    out.push(entry(name, method, comp, size, pos, Some("truncated".into())));
                }
                break;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;

    fn make(entries: &[(&str, &[u8], CompressionMethod)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (n, d, m) in entries {
            w.start_file(*n, SimpleFileOptions::default().compression_method(*m)).unwrap();
            w.write_all(d).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    fn reader(data: &[u8]) -> impl Fn(u64, u32) -> Vec<u8> + '_ {
        move |o, n| {
            let s = (o as usize).min(data.len());
            let e = (s + n as usize).min(data.len());
            data[s..e].to_vec()
        }
    }

    /// A central directory record with the given raw 32-bit fields.
    fn cd_entry(name: &str, flags: u16, method: u16, comp: u32, size: u32, off: u32, extra: &[u8]) -> Vec<u8> {
        let mut cd = Vec::new();
        cd.extend(CD_SIG.to_le_bytes());
        cd.extend([45u8, 0, 45, 0]);
        cd.extend(flags.to_le_bytes());
        cd.extend(method.to_le_bytes());
        cd.extend([0u8; 8]); // time, date, crc
        cd.extend(comp.to_le_bytes());
        cd.extend(size.to_le_bytes());
        cd.extend((name.len() as u16).to_le_bytes());
        cd.extend((extra.len() as u16).to_le_bytes());
        cd.extend([0u8; 6]); // comment length, disk, internal attrs
        cd.extend([0u8; 4]); // external attrs
        cd.extend(off.to_le_bytes());
        cd.extend(name.as_bytes());
        cd.extend(extra);
        cd
    }

    fn zip64_extra(vals: &[u64]) -> Vec<u8> {
        let mut e = Vec::new();
        e.extend(1u16.to_le_bytes());
        e.extend((vals.len() as u16 * 8).to_le_bytes());
        for v in vals {
            e.extend(v.to_le_bytes());
        }
        e
    }

    /// A central directory plus a plain (non-zip64) EOCD with a comment.
    fn plain_archive(cd: &[u8], count: u16, comment: &[u8]) -> Vec<u8> {
        let mut z = cd.to_vec();
        z.extend(EOCD_SIG.to_le_bytes());
        z.extend([0u8; 4]);
        z.extend(count.to_le_bytes());
        z.extend(count.to_le_bytes());
        z.extend((cd.len() as u32).to_le_bytes());
        z.extend(0u32.to_le_bytes());
        z.extend((comment.len() as u16).to_le_bytes());
        z.extend(comment);
        z
    }

    /// The end of a zip64 archive: `cd`, zip64 record, locator, EOCD. `cd_size` is written as given.
    fn zip64_tail(cd: &[u8], cd_off: u64, count: u64, cd_size: u64, rec_off: u64) -> Vec<u8> {
        let mut t = cd.to_vec();
        t.extend(EOCD64_SIG.to_le_bytes());
        t.extend(44u64.to_le_bytes());
        t.extend([45u8, 0, 45, 0]);
        t.extend([0u8; 8]);
        t.extend(count.to_le_bytes());
        t.extend(count.to_le_bytes());
        t.extend(cd_size.to_le_bytes());
        t.extend(cd_off.to_le_bytes());
        t.extend(LOC64_SIG.to_le_bytes());
        t.extend(0u32.to_le_bytes());
        t.extend(rec_off.to_le_bytes());
        t.extend(1u32.to_le_bytes());
        t.extend(EOCD_SIG.to_le_bytes());
        t.extend([0u8; 4]);
        t.extend(0xFFFFu16.to_le_bytes());
        t.extend(0xFFFFu16.to_le_bytes());
        t.extend(0xFFFF_FFFFu32.to_le_bytes());
        t.extend(0xFFFF_FFFFu32.to_le_bytes());
        t.extend(0u16.to_le_bytes());
        t
    }

    /// A virtual archive `len` long whose bytes from `base` on are `tail`, zeros before. Records
    /// the largest single read asked of it.
    fn virt<'a>(tail: &'a [u8], base: u64, len: u64, max_n: &'a Cell<u32>) -> impl Fn(u64, u32) -> Vec<u8> + 'a {
        move |o, n| {
            max_n.set(max_n.get().max(n));
            let n = n.min(len.saturating_sub(o).min(u32::MAX as u64) as u32);
            (0..n as u64).map(|k| if o + k >= base { tail[(o + k - base) as usize] } else { 0 }).collect()
        }
    }

    #[test]
    fn reads_small_zip() {
        let data = make(&[
            ("Takeout/a.jpg", &[1u8; 500], CompressionMethod::Deflated),
            ("Takeout/b.json", b"{}", CompressionMethod::Stored),
        ]);
        let e = read_index(&reader(&data), data.len() as u64).unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].name, "Takeout/a.jpg");
        assert_eq!((e[0].method, e[0].size, e[0].local_offset), (8, 500, 0));
        assert!(e[0].compressed < 500);
        assert_eq!((e[1].name.as_str(), e[1].method, e[1].size, e[1].compressed), ("Takeout/b.json", 0, 2, 2));
        assert!(e.iter().all(|x| x.skip.is_none()));
    }

    #[test]
    fn zip64_offsets() {
        // One entry at offset 5 GB in a fake 6 GB archive; only the tail exists.
        let off: u64 = 5 << 30;
        let extra = zip64_extra(&[4 << 30, (1 << 30) + 7, off]);
        let cd = cd_entry("big/p.jpg", 0, 8, 0xFFFF_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF, &extra);
        let len: u64 = 6 << 30;
        let base = len - (cd.len() as u64 + 98);
        let tail = zip64_tail(&cd, base, 1, cd.len() as u64, base + cd.len() as u64);
        assert_eq!(base + tail.len() as u64, len);
        let max = Cell::new(0);
        let e = read_index(&virt(&tail, base, len, &max), len).unwrap();
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].name, "big/p.jpg");
        assert_eq!(e[0].local_offset, off);
        assert_eq!(e[0].size, 4 << 30);
        assert_eq!(e[0].compressed, (1 << 30) + 7);
    }

    #[test]
    fn zip64_offset_only() {
        let cd = cd_entry("a.jpg", 0, 8, 100, 120, 0xFFFF_FFFF, &zip64_extra(&[5 << 30]));
        let z = plain_archive(&cd, 1, b"");
        let e = read_index(&reader(&z), z.len() as u64).unwrap();
        assert_eq!((e[0].compressed, e[0].size, e[0].local_offset, e[0].skip.clone()), (100, 120, 5 << 30, None));
    }

    #[test]
    fn zip64_size_and_offset_only() {
        let cd = cd_entry("a.jpg", 0, 8, 77, 0xFFFF_FFFF, 0xFFFF_FFFF, &zip64_extra(&[3 << 30, 5 << 30]));
        let z = plain_archive(&cd, 1, b"");
        let e = read_index(&reader(&z), z.len() as u64).unwrap();
        assert_eq!((e[0].compressed, e[0].size, e[0].local_offset), (77, 3 << 30, 5 << 30));
    }

    #[test]
    fn zip64_after_other_extra_field() {
        let mut extra = vec![0x55, 0x54, 5, 0, 1, 0, 0, 0, 0]; // extended timestamp
        extra.extend(zip64_extra(&[5 << 30]));
        let cd = cd_entry("a.jpg", 0, 0, 10, 10, 0xFFFF_FFFF, &extra);
        let z = plain_archive(&cd, 1, b"");
        let e = read_index(&reader(&z), z.len() as u64).unwrap();
        assert_eq!(e[0].local_offset, 5 << 30);
    }

    #[test]
    fn short_zip64_extra_is_corrupt() {
        let mut extra = vec![1, 0, 4, 0];
        extra.extend([0u8; 4]);
        let cd = cd_entry("a.jpg", 0, 0, 10, 10, 0xFFFF_FFFF, &extra);
        let z = plain_archive(&cd, 1, b"");
        let e = read_index(&reader(&z), z.len() as u64).unwrap();
        assert_eq!(e[0].skip.as_deref(), Some("corrupt zip64 extra"));
    }

    #[test]
    fn eocd_comment_present() {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        w.set_comment("a zip with a comment, 1234567890");
        w.start_file("a.jpg", SimpleFileOptions::default().compression_method(CompressionMethod::Stored)).unwrap();
        w.write_all(b"hello").unwrap();
        let data = w.finish().unwrap().into_inner();
        let e = read_index(&reader(&data), data.len() as u64).unwrap();
        assert_eq!((e.len(), e[0].name.as_str(), e[0].size), (1, "a.jpg", 5));
        let cd = cd_entry("b.jpg", 0, 0, 1, 1, 0, &[]);
        let z = plain_archive(&cd, 1, &[b'x'; 300]);
        assert_eq!(read_index(&reader(&z), z.len() as u64).unwrap()[0].name, "b.jpg");
    }

    #[test]
    fn forged_sizes_do_not_allocate_or_panic() {
        // zip64 record claiming a 1 TB central directory in a fake 2 TB archive.
        let len: u64 = 2 << 40;
        let cd = cd_entry("a.jpg", 0, 0, 1, 1, 0, &[]);
        let base = len - (cd.len() as u64 + 98);
        let tail = zip64_tail(&cd, 0, u64::MAX, 1 << 40, base + cd.len() as u64);
        let max = Cell::new(0);
        let e = read_index(&virt(&tail, base, len, &max), len).unwrap();
        assert!(e.is_empty());
        assert!(max.get() as u64 <= CHUNK, "asked for {} bytes at once", max.get());
        // A real central directory, a huge count and an entry with 64 KB name and extra lengths.
        let mut big = cd_entry("a.jpg", 0, 0, 1, 1, 0, &[]);
        big.extend(cd_entry("b.jpg", 0, 0, 1, 1, 0, &[]));
        let mut bad = cd_entry("c", 0, 0, 1, 1, 0, &[]);
        bad[28..30].copy_from_slice(&0xFFFFu16.to_le_bytes());
        bad[30..32].copy_from_slice(&0xFFFFu16.to_le_bytes());
        big.extend(bad);
        let base = len - (big.len() as u64 + 98);
        let tail = zip64_tail(&big, base, u64::MAX, big.len() as u64, base + big.len() as u64);
        let max = Cell::new(0);
        let e = read_index(&virt(&tail, base, len, &max), len).unwrap();
        assert_eq!(e.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["a.jpg", "b.jpg"]);
        assert!(max.get() as u64 <= CHUNK);
    }

    #[test]
    fn truncated_zip_partial() {
        let data = make(&[
            ("a.jpg", &[7u8; 2000], CompressionMethod::Stored),
            ("b.jpg", &[8u8; 2000], CompressionMethod::Stored),
            ("c.jpg", &[9u8; 2000], CompressionMethod::Stored),
        ]);
        // Cut inside the third entry's data: no central directory survives.
        let cut = 2 * (30 + 5 + 2000) + 1000;
        let e = read_index(&reader(&data[..cut]), cut as u64).unwrap();
        assert_eq!(e.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["a.jpg", "b.jpg", "c.jpg"]);
        assert_eq!((e[0].skip.clone(), e[1].skip.clone()), (None, None));
        assert_eq!(e[2].skip.as_deref(), Some("truncated"));
        // Garbage never panics.
        assert!(read_index(&reader(&[0u8; 100]), 100).unwrap().is_empty());
        assert!(read_index(&reader(&[1u8; 5]), 5).is_err());
    }

    /// Pseudo-random bytes, so the deflate stream is bigger than one inflate chunk.
    fn noise(n: usize) -> Vec<u8> {
        let mut x = 0x1234_5678u32;
        (0..n)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (x >> 24) as u8
            })
            .collect()
    }

    /// A local entry the way Go writes it: bit 3, zero sizes in the header, sizes after the data.
    fn local_bit3(name: &str, data: &[u8], sig: bool, z64: bool) -> (Vec<u8>, usize) {
        let comp = miniz_oxide::deflate::compress_to_vec(data, 6);
        let mut z = Vec::new();
        z.extend(LOCAL_SIG.to_le_bytes());
        z.extend([45u8, 0, 8, 0, 8, 0]); // version, flags (bit 3), method
        z.extend([0u8; 4]); // time, date
        z.extend([0u8; 12]); // crc, comp, size
        z.extend((name.len() as u16).to_le_bytes());
        let extra = if z64 { zip64_extra(&[0, 0]) } else { Vec::new() };
        z.extend((extra.len() as u16).to_le_bytes());
        z.extend(name.as_bytes());
        z.extend(&extra);
        let data_at = z.len();
        z.extend(&comp);
        if sig {
            z.extend(DESC_SIG.to_le_bytes());
        }
        z.extend([0u8; 4]); // crc
        if z64 {
            z.extend((comp.len() as u64).to_le_bytes());
            z.extend((data.len() as u64).to_le_bytes());
        } else {
            z.extend((comp.len() as u32).to_le_bytes());
            z.extend((data.len() as u32).to_le_bytes());
        }
        (z, data_at)
    }

    #[test]
    fn truncated_zip_with_data_descriptors() {
        let big = noise(200_000);
        let (e1, _) = local_bit3("one.jpg", &big, true, false);
        let (e2, _) = local_bit3("two.jpg", b"hello hello hello hello", false, false);
        let (e3, _) = local_bit3("three.jpg", &noise(5000), true, true);
        let (e4, at) = local_bit3("four.jpg", &noise(90_000), true, false);
        let mut z = [e1.clone(), e2.clone(), e3.clone()].concat();
        let four_at = z.len() as u64;
        z.extend(&e4[..at + 40_000]); // cut-off inside the fourth entry's data
        let e = read_index(&reader(&z), z.len() as u64).unwrap();
        let names: Vec<_> = e.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["one.jpg", "two.jpg", "three.jpg", "four.jpg"]);
        assert_eq!((e[0].size, e[0].skip.clone()), (200_000, None));
        assert!(e[0].compressed > INFLATE_CHUNK as u64);
        assert_eq!((e[1].size, e[1].local_offset), (23, e1.len() as u64));
        assert_eq!((e[2].size, e[2].skip.clone()), (5000, None));
        assert_eq!((e[3].local_offset, e[3].skip.as_deref()), (four_at, Some("truncated")));
        // The complete zip's entries match without the cut.
        z.extend(&e4[at + 40_000..]);
        let e = read_index(&reader(&z), z.len() as u64).unwrap();
        assert_eq!((e.len(), e[3].size, e[3].skip.clone()), (4, 90_000, None));
    }

    #[test]
    fn stored_entry_with_data_descriptor_is_skipped() {
        let mut z = Vec::new();
        z.extend(LOCAL_SIG.to_le_bytes());
        z.extend([20u8, 0, 8, 0, 0, 0]);
        z.extend([0u8; 16]);
        z.extend(1u16.to_le_bytes()); // name length
        z.extend(0u16.to_le_bytes());
        z.push(b'a');
        z.extend([5u8; 40]);
        let e = read_index(&reader(&z), z.len() as u64).unwrap();
        assert_eq!(e.len(), 1);
        assert!(e[0].skip.as_deref().unwrap().contains("cannot be sized"));
    }

    #[test]
    fn encrypted_entry_skipped() {
        let mut data = make(&[("a.jpg", &[1u8; 100], CompressionMethod::Stored), ("b.jpg", &[2u8; 100], CompressionMethod::Stored)]);
        // Set general-purpose bit 0 on the first central entry.
        let cd = (0..data.len() - 4).find(|&i| u32_at(&data, i) == Some(CD_SIG)).unwrap();
        data[cd + 8] |= 1;
        let e = read_index(&reader(&data), data.len() as u64).unwrap();
        assert_eq!(e[0].skip.as_deref(), Some("encrypted"));
        assert_eq!(e[1].skip, None);
    }

    #[test]
    fn cut_off_zip_reports_progress_and_stops_when_told() {
        let entries: Vec<(String, Vec<u8>)> = (0..2000).map(|i| (format!("p/{i}.jpg"), vec![1u8; 10])).collect();
        let refs: Vec<(&str, &[u8], CompressionMethod)> = entries.iter().map(|(n, d)| (n.as_str(), d.as_slice(), CompressionMethod::Stored)).collect();
        let data = make(&refs);
        // No central directory: the walk reads 2000 local headers.
        let cut = 2000 * (30 + 10) + entries.iter().map(|(n, _)| n.len()).sum::<usize>();
        let cut = &data[..cut];
        let seen = std::cell::RefCell::new(Vec::new());
        let e = read_index_ticking(&reader(cut), cut.len() as u64, &|n| {
            seen.borrow_mut().push(n);
            Ok(())
        })
        .unwrap();
        assert_eq!(e.len(), 2000);
        assert_eq!(*seen.borrow(), [500, 1000, 1500, 2000]);
        // A failing report ends the walk with its error.
        let calls = Cell::new(0);
        let r = read_index_ticking(&reader(cut), cut.len() as u64, &|_| {
            calls.set(calls.get() + 1);
            Err("cancelled".into())
        });
        assert_eq!(r, Err("cancelled".to_string()));
        assert_eq!(calls.get(), 1);
        // A complete zip reads its central directory and never reports.
        let calls = Cell::new(0);
        read_index_ticking(&reader(&data), data.len() as u64, &|_| {
            calls.set(calls.get() + 1);
            Ok(())
        })
        .unwrap();
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn measuring_a_stream_reports_and_stops() {
        let big = noise(400_000);
        let (e1, _) = local_bit3("one.jpg", &big, true, false);
        let seen = Cell::new(0u64);
        let mut pace = Pace { tick: &|n| { seen.set(n); Ok(()) }, every: 3, work: 0, stopped: None };
        let e = scan_local(&reader(&e1), e1.len() as u64, &mut pace).unwrap();
        assert_eq!(e[0].size, 400_000);
        assert!(seen.get() >= 6, "inflating 400 KB of input made {} work units", seen.get());
        let mut pace = Pace { tick: &|_| Err("stop".into()), every: 3, work: 0, stopped: None };
        assert_eq!(scan_local(&reader(&e1), e1.len() as u64, &mut pace), Err("stop".to_string()));
    }

    #[test]
    fn modified_prefers_the_extended_timestamp() {
        let mut extra = vec![0x55, 0x54, 5, 0, 1];
        extra.extend(1_558_096_210u32.to_le_bytes());
        assert_eq!(modified_of(&extra, 0, 0), Some(1_558_096_210));
        assert_eq!(modified_of(&[], 0, 0), None);
        assert_eq!(modified_of(&[], 0, (1 << 5) | 1), Some(315_532_800));
    }
}
