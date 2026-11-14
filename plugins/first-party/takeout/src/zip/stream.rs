//! Reading the bytes of zip entries: whole (small sidecars) or in ranges (media, through the
//! host's `importer.read`). A range read of a deflated entry keeps an inflate state per key, so
//! sequential reads cost O(n) in total; a backwards offset restarts that entry. Nothing here ever
//! holds a whole entry in memory: input and output move in bounded chunks.

use super::ZipEntry;
use miniz_oxide::inflate::stream::{inflate, InflateState};
use miniz_oxide::{DataFormat, MZError, MZFlush, MZStatus};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

const CHUNK: usize = 64 << 10;
/// How many entries keep their inflate state at once (a dozen or more ingest threads read
/// different entries at the same time; least recently used goes first).
const ACTIVE: usize = 64;

/// `(zip index, offset, len)` to bytes (fewer at the end of the file, none on a failure).
pub type ReadAt = Box<dyn Fn(usize, u64, u32) -> Vec<u8>>;

/// The zips of one import: their file names, each one's entries and a way to read its bytes.
/// An entry is named by its zip's file name and its path inside, never by position, so a zip
/// whose index failed (an empty list) or one added later shifts nothing.
pub struct ZipSet {
    pub names: Vec<String>,
    pub zips: Vec<Vec<ZipEntry>>,
    pub read_at: ReadAt,
    lookup: HashMap<(String, String), (usize, usize)>,
}

/// Separates the zip's file name from the entry's path in a key (a control character no real
/// file name holds).
const SEP: char = '\u{1f}';

/// The key of the entry `entry` of the zip file `zip`.
pub fn make_key(zip: &str, entry: &str) -> String {
    format!("zip:{zip}{SEP}{entry}")
}

fn parse_key(key: &str) -> Option<(&str, &str)> {
    key.strip_prefix("zip:")?.split_once(SEP)
}

/// Bytes `[start, start + n)` of a zip, asking again after a short read; fewer only at the end.
fn gather(read_at: &ReadAt, zi: usize, start: u64, n: u64) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    while (out.len() as u64) < n {
        let want = (n - out.len() as u64).min(1 << 20) as u32;
        let got = read_at(zi, start + out.len() as u64, want);
        if got.is_empty() {
            break;
        }
        out.extend_from_slice(&got[..got.len().min(want as usize)]);
    }
    out
}

impl ZipSet {
    pub fn new(names: Vec<String>, zips: Vec<Vec<ZipEntry>>, read_at: ReadAt) -> ZipSet {
        let mut lookup = HashMap::new();
        for (zi, zip) in zips.iter().enumerate() {
            for (ei, e) in zip.iter().enumerate() {
                lookup.entry((names[zi].clone(), e.name.clone())).or_insert((zi, ei));
            }
        }
        ZipSet { names, zips, read_at, lookup }
    }

    /// The key of entry `ei` of zip `zi`.
    pub fn key_of(&self, zi: usize, ei: usize) -> String {
        make_key(&self.names[zi], &self.zips[zi][ei].name)
    }

    fn entry(&self, zi: usize, ei: usize) -> Result<&ZipEntry, String> {
        let e = self.zips.get(zi).and_then(|z| z.get(ei)).ok_or("no such entry")?;
        match &e.skip {
            Some(r) => Err(format!("{} cannot be read: {r}", e.name)),
            None => Ok(e),
        }
    }

    fn resolve(&self, key: &str) -> Result<(usize, usize), String> {
        let (zip, entry) = parse_key(key).ok_or("not a zip entry key")?;
        self.lookup.get(&(zip.to_string(), entry.to_string())).copied().ok_or_else(|| format!("{entry} is not in {zip}"))
    }

    /// Where the entry's data starts: after its local header, whose name and extra lengths can
    /// differ from the central directory's.
    fn data_start(&self, zi: usize, e: &ZipEntry) -> Result<u64, String> {
        let h = gather(&self.read_at, zi, e.local_offset, 30);
        Ok(e.local_offset + header_len(&h)?)
    }

    /// The entry's whole content, for a small one; `Err` above `cap` bytes. One read fetches the
    /// header and the data together; a second only when the header's name and extra are long.
    pub fn read_whole(&self, zi: usize, ei: usize, cap: u64) -> Result<Vec<u8>, String> {
        let e = self.entry(zi, ei)?;
        if e.size > cap || e.compressed > cap {
            return Err(format!("{} is too large to read whole", e.name));
        }
        // One over-read for header and data; more only if the host answered short or the
        // header's name and extra were long.
        let ask = 30 + 256 + e.compressed;
        let mut buf = (self.read_at)(zi, e.local_offset, ask.min(u32::MAX as u64) as u32);
        buf.truncate(ask as usize);
        let head = header_len(&buf)? as usize;
        let have = buf.len().saturating_sub(head).min(e.compressed as usize);
        let mut data = if buf.len() > head { buf[head..head + have].to_vec() } else { Vec::new() };
        if (data.len() as u64) < e.compressed {
            let more = gather(&self.read_at, zi, e.local_offset + (head + data.len()) as u64, e.compressed - data.len() as u64);
            data.extend(more);
        }
        if data.len() as u64 != e.compressed {
            return Err(format!("{} is cut off", e.name));
        }
        let out = if e.method == 0 {
            data
        } else {
            miniz_oxide::inflate::decompress_to_vec_with_limit(&data, cap as usize).map_err(|_| format!("{} is corrupt", e.name))?
        };
        if out.len() as u64 != e.size {
            return Err(format!("{} is cut off", e.name));
        }
        Ok(out)
    }
}

/// The local header's length (30 plus its name and extra) from at least its first 30 bytes.
fn header_len(h: &[u8]) -> Result<u64, String> {
    if h.len() < 30 || h[..4] != [0x50, 0x4b, 0x03, 0x04] {
        return Err("bad local header".into());
    }
    let n = u16::from_le_bytes([h[26], h[27]]) as u64;
    let x = u16::from_le_bytes([h[28], h[29]]) as u64;
    Ok(30 + n + x)
}

/// An open entry: how far its inflate has got.
struct Cursor {
    method: u16,
    data_start: u64,
    compressed: u64,
    size: u64,
    st: Box<InflateState>,
    /// Input read but not yet consumed.
    buf: Vec<u8>,
    bp: usize,
    /// Compressed bytes fetched so far.
    fetched: u64,
    /// Where the next output byte sits in the entry.
    at: u64,
    done: bool,
}

impl Cursor {
    fn open(set: &ZipSet, zi: usize, e: &ZipEntry) -> Result<Cursor, String> {
        Ok(Cursor {
            method: e.method,
            data_start: set.data_start(zi, e)?,
            compressed: e.compressed,
            size: e.size,
            st: InflateState::new_boxed(DataFormat::Raw),
            buf: Vec::new(),
            bp: 0,
            fetched: 0,
            at: 0,
            done: false,
        })
    }

    /// Bytes `[offset, offset + len)`, fewer at the end of the entry. A deflated entry must not
    /// be asked for an offset behind `at` (the caller starts a new cursor).
    fn read(&mut self, set: &ZipSet, zi: usize, offset: u64, len: u32) -> Result<Vec<u8>, String> {
        if offset >= self.size {
            return Ok(Vec::new());
        }
        let want = (len as u64).min(self.size - offset);
        if self.method == 0 {
            let mut out: Vec<u8> = Vec::new();
            while (out.len() as u64) < want {
                let left = want - out.len() as u64;
                let got = (set.read_at)(zi, self.data_start + offset + out.len() as u64, left as u32);
                if got.is_empty() {
                    break;
                }
                out.extend_from_slice(&got[..got.len().min(left as usize)]);
            }
            return Ok(out);
        }
        // Inflate up to `offset`, discarding, then collect.
        let mut sink = vec![0u8; CHUNK];
        while self.at < offset {
            let n = ((offset - self.at) as usize).min(CHUNK);
            if self.step(set, zi, &mut sink[..n])? == 0 {
                return Ok(Vec::new());
            }
        }
        let mut out = vec![0u8; want as usize];
        let mut filled = 0;
        while filled < out.len() {
            let n = self.step(set, zi, &mut out[filled..])?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        out.truncate(filled);
        Ok(out)
    }

    /// Inflates into `out`; the number of bytes written (0 at the end of the stream).
    fn step(&mut self, set: &ZipSet, zi: usize, out: &mut [u8]) -> Result<usize, String> {
        loop {
            if self.done {
                return Ok(0);
            }
            if self.bp >= self.buf.len() && self.fetched < self.compressed {
                let n = (self.compressed - self.fetched).min(CHUNK as u64) as u32;
                self.buf = (set.read_at)(zi, self.data_start + self.fetched, n);
                self.buf.truncate(n as usize);
                self.bp = 0;
                self.fetched += self.buf.len() as u64;
                if self.buf.is_empty() {
                    return Err("zip is cut off".into());
                }
            }
            let r = inflate(&mut self.st, &self.buf[self.bp..], out, MZFlush::None);
            self.bp += r.bytes_consumed;
            self.at += r.bytes_written as u64;
            match r.status {
                Ok(MZStatus::StreamEnd) => {
                    self.done = true;
                    return Ok(r.bytes_written);
                }
                Ok(_) | Err(MZError::Buf) => {
                    if r.bytes_written > 0 {
                        return Ok(r.bytes_written);
                    }
                    if r.bytes_consumed == 0 {
                        // No input used and no output made: more input cannot help.
                        return Err("compressed data ends early".into());
                    }
                }
                Err(_) => return Err("corrupt compressed data".into()),
            }
        }
    }
}

/// Range reads for the host's `importer.read`, one inflate state per recently used key.
pub struct Streams {
    set: Rc<ZipSet>,
    open: RefCell<Vec<(String, Cursor)>>,
}

impl Streams {
    pub fn new(set: Rc<ZipSet>) -> Streams {
        Streams { set, open: RefCell::new(Vec::new()) }
    }

    pub fn read(&self, key: &str, offset: u64, len: u32) -> Result<Vec<u8>, String> {
        let (zi, ei) = self.set.resolve(key)?;
        let e = self.set.entry(zi, ei)?;
        let mut open = self.open.borrow_mut();
        let found = open.iter().position(|(k, _)| k == key);
        // A backwards offset starts that entry over.
        let reusable = found.filter(|&i| open[i].1.method == 0 || open[i].1.at <= offset);
        let i = match reusable {
            Some(i) => {
                // Most recently used goes to the back, so eviction at the front is LRU.
                let c = open.remove(i);
                open.push(c);
                open.len() - 1
            }
            None => {
                if let Some(i) = found {
                    open.remove(i);
                }
                if open.len() >= ACTIVE {
                    open.remove(0);
                }
                open.push((key.to_string(), Cursor::open(&self.set, zi, e)?));
                open.len() - 1
            }
        };
        let out = open[i].1.read(&self.set, zi, offset, len);
        // A finished or failed entry frees its state; a read at its end starts over cheaply.
        if out.is_err() || open[i].1.done {
            open.remove(i);
        }
        out
    }
}
