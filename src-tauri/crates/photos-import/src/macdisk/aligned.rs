//! A raw partition only accepts reads of whole sectors at sector-aligned offsets, and the
//! filesystem parsers read a few kilobytes here and there. Two patterns matter, and they
//! want opposite things from a USB hard drive:
//!
//! * **Catalog walks** read 4–8 KiB B-tree nodes all over the disk, and the upper nodes
//!   again and again. They get 64 KiB blocks, kept in a 32 MiB cache.
//! * **File streams** read one run of bytes front to back. After three sequential misses
//!   the reader fetches 4 MiB at a time into a separate window that bypasses the cache,
//!   so copying a video does not push the catalog out of it.
//!
//! One 4 MiB window for everything made a walk over a 1.3 M-file catalog read megabytes per
//! node, which on a spinning disk took minutes.

use std::collections::HashMap;
use std::io::{self, Read, Seek, SeekFrom};

pub struct AlignedReader<R> {
    inner: R,
    len: u64,
    pos: u64,
    /// Block index → (bytes, last use).
    cache: HashMap<u64, (Vec<u8>, u64)>,
    clock: u64,
    stream_start: u64,
    stream: Vec<u8>,
    /// The block after the last one fetched, and how many fetches in a row were sequential.
    next_block: Option<u64>,
    run: u32,
}

impl<R: Read + Seek> AlignedReader<R> {
    pub const BLOCK: u64 = 64 << 10;
    pub const WINDOW: u64 = 4 << 20;
    const CACHE_BLOCKS: usize = 512;
    const STREAM_AFTER: u32 = 2;

    /// `len` is the partition's size, a whole number of sectors.
    pub fn new(inner: R, len: u64) -> Self {
        Self { inner, len, pos: 0, cache: HashMap::new(), clock: 0, stream_start: 0, stream: Vec::new(), next_block: None, run: 0 }
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// One seek and one read of whole sectors: `start` is block-aligned and the partition
    /// is a whole number of sectors, so every length here is too.
    fn fetch(&mut self, start: u64, max: u64) -> io::Result<Vec<u8>> {
        let mut buf = vec![0u8; max.min(self.len - start) as usize];
        self.inner.seek(SeekFrom::Start(start))?;
        self.inner.read_exact(&mut buf)?;
        Ok(buf)
    }

    fn cache_insert(&mut self, block: u64, bytes: Vec<u8>) {
        if self.cache.len() >= Self::CACHE_BLOCKS {
            if let Some(oldest) = self.cache.iter().min_by_key(|(_, (_, used))| *used).map(|(k, _)| *k) {
                self.cache.remove(&oldest);
            }
        }
        self.clock += 1;
        self.cache.insert(block, (bytes, self.clock));
    }

    /// Copies from `bytes` (which starts at `start`) into `out`, from `self.pos`.
    fn serve(pos: &mut u64, bytes: &[u8], start: u64, out: &mut [u8]) -> usize {
        let offset = (*pos - start) as usize;
        let n = out.len().min(bytes.len() - offset);
        out[..n].copy_from_slice(&bytes[offset..offset + n]);
        *pos += n as u64;
        n
    }
}

impl<R: Read + Seek> Read for AlignedReader<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.len || out.is_empty() {
            return Ok(0);
        }
        if self.pos >= self.stream_start && self.pos < self.stream_start + self.stream.len() as u64 {
            return Ok(Self::serve(&mut self.pos, &self.stream, self.stream_start, out));
        }
        let block = self.pos / Self::BLOCK;
        let start = block * Self::BLOCK;
        if let Some((bytes, used)) = self.cache.get_mut(&block) {
            self.clock += 1;
            *used = self.clock;
            return Ok(Self::serve(&mut self.pos, bytes, start, out));
        }
        self.run = if self.next_block == Some(block) { self.run + 1 } else { 0 };
        if self.run >= Self::STREAM_AFTER {
            let bytes = self.fetch(start, Self::WINDOW)?;
            self.next_block = Some(block + (bytes.len() as u64).div_ceil(Self::BLOCK));
            self.stream = bytes;
            self.stream_start = start;
            return Ok(Self::serve(&mut self.pos, &self.stream, start, out));
        }
        let bytes = self.fetch(start, Self::BLOCK)?;
        self.next_block = Some(block + 1);
        let n = Self::serve(&mut self.pos, &bytes, start, out);
        self.cache_insert(block, bytes);
        Ok(n)
    }
}

impl<R: Read + Seek> Seek for AlignedReader<R> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = match to {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
            SeekFrom::End(d) => self.len.checked_add_signed(d),
        };
        self.pos = target.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before the start of the partition"))?;
        Ok(self.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Refuses seeks and reads that are not whole 512-byte sectors, as a raw device does.
    struct Strict(std::io::Cursor<Vec<u8>>);

    impl Read for Strict {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            assert!(self.0.position().is_multiple_of(512) && buf.len().is_multiple_of(512), "unaligned read at {} len {}", self.0.position(), buf.len());
            self.0.read(buf)
        }
    }

    impl Seek for Strict {
        fn seek(&mut self, p: SeekFrom) -> io::Result<u64> {
            let at = self.0.seek(p)?;
            assert!(at.is_multiple_of(512), "unaligned seek to {at}");
            Ok(at)
        }
    }

    #[test]
    fn serves_any_read_from_whole_sectors() {
        let data: Vec<u8> = (0..(9u32 << 20)).map(|i| (i % 251) as u8).collect();
        let mut r = AlignedReader::new(Strict(std::io::Cursor::new(data.clone())), data.len() as u64);
        let mut buf = vec![0u8; 3000];
        r.seek(SeekFrom::Start(1_234_567)).unwrap();
        r.read_exact(&mut buf).unwrap();
        assert_eq!(buf, data[1_234_567..1_234_567 + 3000]);
        // Across a window boundary.
        let edge = AlignedReader::<Strict>::WINDOW as usize;
        r.seek(SeekFrom::Start(edge as u64 - 10)).unwrap();
        r.read_exact(&mut buf[..100]).unwrap();
        assert_eq!(buf[..100], data[edge - 10..edge + 90]);
        // Relative seeks, the end, and past it.
        r.seek(SeekFrom::Start(100)).unwrap();
        assert_eq!(r.seek(SeekFrom::Current(-40)).unwrap(), 60);
        assert_eq!(r.seek(SeekFrom::End(-5)).unwrap(), data.len() as u64 - 5);
        let mut tail = Vec::new();
        r.read_to_end(&mut tail).unwrap();
        assert_eq!(tail, data[data.len() - 5..]);
        assert_eq!(r.read(&mut buf).unwrap(), 0);
        assert_eq!(r.len(), data.len() as u64);
    }

    /// Records every read the device serves, to see what the reader asks the disk for.
    struct Counting {
        inner: Strict,
        reads: std::rc::Rc<std::cell::RefCell<Vec<usize>>>,
    }

    impl Read for Counting {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = self.inner.read(buf)?;
            self.reads.borrow_mut().push(n);
            Ok(n)
        }
    }

    impl Seek for Counting {
        fn seek(&mut self, p: SeekFrom) -> io::Result<u64> {
            self.inner.seek(p)
        }
    }

    #[test]
    fn random_reads_fetch_small_cached_blocks_and_streams_fetch_large_ones() {
        let data: Vec<u8> = (0..(64u32 << 20)).map(|i| (i % 253) as u8).collect();
        let reads = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let dev = Counting { inner: Strict(std::io::Cursor::new(data.clone())), reads: reads.clone() };
        let mut r = AlignedReader::new(dev, data.len() as u64);
        let mut node = vec![0u8; 8192];

        // A B-tree walk: 8 KiB nodes scattered over the disk, and the root read again and again.
        for at in [40_000_000u64, 3_000_000, 50_000_000, 40_000_000, 3_000_000, 40_000_000] {
            r.seek(SeekFrom::Start(at)).unwrap();
            r.read_exact(&mut node).unwrap();
            assert_eq!(node, data[at as usize..at as usize + 8192]);
        }
        let random: Vec<usize> = reads.borrow().clone();
        // 50,000,000 + 8 KiB straddles two blocks: four distinct blocks, each fetched once.
        assert_eq!(random.len(), 4, "{random:?}");
        assert!(random.iter().all(|&n| n <= AlignedReader::<Counting>::BLOCK as usize), "{random:?}");

        // Streaming a 20 MB photo: after the first block, the reader switches to big reads.
        reads.borrow_mut().clear();
        r.seek(SeekFrom::Start(10_000_000)).unwrap();
        let mut file = vec![0u8; 20_000_000];
        r.read_exact(&mut file).unwrap();
        assert_eq!(file, data[10_000_000..30_000_000]);
        let stream: Vec<usize> = reads.borrow().clone();
        assert!(stream.len() <= 8, "{} device reads for 20 MB", stream.len());
        assert!(stream.iter().any(|&n| n as u64 == AlignedReader::<Counting>::WINDOW));

        // The stream did not push the B-tree's blocks out of the cache.
        reads.borrow_mut().clear();
        r.seek(SeekFrom::Start(40_000_000)).unwrap();
        r.read_exact(&mut node).unwrap();
        assert!(reads.borrow().is_empty());
    }
}
