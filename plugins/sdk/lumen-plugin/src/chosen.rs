//! The folder the person chose for an importer (needs `import.read-chosen`).
//! Paths are relative to that folder; the host refuses anything outside it.

/// Bytes asked of the host per `read` call.
pub const CHUNK: u32 = 1 << 20;

/// Reads `size` bytes in `chunk`-sized pieces through `read(offset, len)`. A short or empty
/// piece before `size` is an error ("file ended early"), so a truncated file never passes silently.
pub fn read_all_with(
    size: u64,
    chunk: u32,
    mut read: impl FnMut(u64, u32) -> Result<Vec<u8>, String>,
) -> Result<Vec<u8>, String> {
    let chunk = chunk.max(1);
    let mut out = Vec::with_capacity(size.min(64 << 20) as usize);
    while (out.len() as u64) < size {
        let want = (size - out.len() as u64).min(chunk as u64) as u32;
        let got = read(out.len() as u64, want)?;
        if got.is_empty() {
            return Err("file ended early".into());
        }
        if got.len() as u64 > want as u64 {
            return Err("host returned more than asked".into());
        }
        out.extend_from_slice(&got);
    }
    Ok(out)
}

#[cfg(all(target_arch = "wasm32", feature = "importer"))]
mod imp {
    use super::*;
    use crate::bindings::lumen::plugin::chosen as c;
    use crate::Entry;

    /// Lists a folder (`""` is the chosen root).
    pub fn list(dir: &str) -> Result<Vec<Entry>, String> {
        c::list(dir)
    }
    pub fn stat(path: &str) -> Result<Entry, String> {
        c::stat(path)
    }
    /// Up to `len` bytes from `offset`.
    pub fn read(path: &str, offset: u64, len: u32) -> Result<Vec<u8>, String> {
        c::read(path, offset, len)
    }
    /// The whole file, read in 1 MiB pieces.
    pub fn read_all(path: &str) -> Result<Vec<u8>, String> {
        let size = c::stat(path)?.size;
        read_all_with(size, CHUNK, |o, l| c::read(path, o, l))
    }
    /// Reports progress; an `Err` means the host wants the plugin to stop (cancelled or over time).
    pub fn progress(done: u64, total: Option<u64>) -> Result<(), String> {
        c::progress(done, total)
    }
}

#[cfg(all(target_arch = "wasm32", feature = "importer"))]
pub use imp::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(data: Vec<u8>) -> impl FnMut(u64, u32) -> Result<Vec<u8>, String> {
        move |o, l| {
            let s = (o as usize).min(data.len());
            let e = (s + l as usize).min(data.len());
            Ok(data[s..e].to_vec())
        }
    }

    #[test]
    fn read_all_chunks_and_reassembles() {
        let data: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        let mut calls = Vec::new();
        let mut inner = fake(data.clone());
        let got = read_all_with(1000, 300, |o, l| {
            calls.push((o, l));
            inner(o, l)
        })
        .unwrap();
        assert_eq!(got, data);
        assert_eq!(calls, vec![(0, 300), (300, 300), (600, 300), (900, 100)]);
    }

    #[test]
    fn read_all_empty_file_makes_no_call() {
        let got = read_all_with(0, 16, |_, _| panic!("no read expected")).unwrap();
        assert!(got.is_empty());
    }

    #[test]
    fn read_all_errors_when_file_ends_early() {
        let err = read_all_with(100, 40, fake(vec![1; 50])).unwrap_err();
        assert_eq!(err, "file ended early");
    }

    #[test]
    fn read_all_propagates_host_errors() {
        let err = read_all_with(10, 4, |_, _| Err("denied".into())).unwrap_err();
        assert_eq!(err, "denied");
    }
}
