//! A Takeout of 100,000 photos with their sidecars in one stored zip: `enumerate` is one host
//! call, so it must report progress every 500 reads and lists (the host's 30 s limit applies
//! between reports) and finish in sensible time.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, RefCell};
use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use takeout::fs::{Ticking, PROGRESS_EVERY};
use takeout::scan::scan;
use takeout::zip::{read_index, ZipFs, ZipSet};
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

const N: usize = 100_000;

/// Tracks the heap's high-water mark (the plugin's wasm memory is capped at 512 MB).
struct Peak;
static CUR: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Peak {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = System.alloc(l);
        if !p.is_null() {
            let c = CUR.fetch_add(l.size(), Ordering::Relaxed) + l.size();
            PEAK.fetch_max(c, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l);
        CUR.fetch_sub(l.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = System.realloc(p, l, n);
        if !q.is_null() {
            let c = if n >= l.size() { CUR.fetch_add(n - l.size(), Ordering::Relaxed) + n - l.size() } else { CUR.fetch_sub(l.size() - n, Ordering::Relaxed) - (l.size() - n) };
            PEAK.fetch_max(c, Ordering::Relaxed);
        }
        q
    }
}
#[global_allocator]
static A: Peak = Peak;

/// A sidecar of about 1 KB the way Google writes them.
fn sidecar(i: usize) -> String {
    format!(
        r#"{{"title":"IMG_{i:06}.jpg","description":"","imageViews":"{v}","creationTime":{{"timestamp":"{t}","formatted":"Jul 14, 2017, 2:40:00 AM UTC"}},"photoTakenTime":{{"timestamp":"{t}","formatted":"Jul 14, 2017, 2:40:00 AM UTC"}},"geoData":{{"latitude":45.{i:06},"longitude":13.{i:06},"altitude":112.5,"latitudeSpan":0.0,"longitudeSpan":0.0}},"geoDataExif":{{"latitude":45.{i:06},"longitude":13.{i:06},"altitude":112.5,"latitudeSpan":0.0,"longitudeSpan":0.0}},"people":[{{"name":"Person A{i}"}},{{"name":"Person B{i}"}}],"url":"https://photos.google.com/photo/AF1QipN{i:06}abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOP","googlePhotosOrigin":{{"mobileUpload":{{"deviceFolder":{{"localFolderName":""}},"deviceType":"ANDROID_PHONE"}}}},"appSource":{{"androidPackageName":"com.google.android.apps.photos"}}}}"#,
        v = i % 97,
        t = 1_500_000_000 + i
    )
}

#[test]
fn large_enumerate_progress() {
    let dir = std::env::temp_dir().join(format!("lumen-takeout-large-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("takeout-001.zip");

    let t0 = Instant::now();
    {
        let mut w = zip::ZipWriter::new(BufWriter::new(File::create(&path).unwrap()));
        let opt = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for i in 0..N {
            let folder = format!("Takeout/Google Photos/Photos from {}", 2000 + i % 20);
            w.start_file(format!("{folder}/IMG_{i:06}.jpg"), opt).unwrap();
            w.write_all(&[0xff, 0xd8, (i % 251) as u8, (i / 251 % 251) as u8]).unwrap();
            w.start_file(format!("{folder}/IMG_{i:06}.jpg.json"), opt).unwrap();
            w.write_all(sidecar(i).as_bytes()).unwrap();
        }
        w.finish().unwrap().flush().unwrap();
    }
    let size = std::fs::metadata(&path).unwrap().len();
    eprintln!("wrote {N} photos + {N} sidecars, {} MB, in {:?}", size >> 20, t0.elapsed());

    let file = RefCell::new(File::open(&path).unwrap());
    let read_at = |o: u64, n: u32| {
        let mut f = file.borrow_mut();
        let mut b = vec![0u8; n as usize];
        if f.seek(SeekFrom::Start(o)).is_err() {
            return Vec::new();
        }
        let got = f.read(&mut b).unwrap_or(0);
        b.truncate(got);
        b
    };
    let t1 = Instant::now();
    let index = read_index(&read_at, size).unwrap();
    eprintln!("central directory: {} entries in {:?}", index.len(), t1.elapsed());
    assert_eq!(index.len(), 2 * N);

    // The set owns its own handle to the file.
    let path2 = path.clone();
    let set = Rc::new(ZipSet::new(
        vec!["takeout-001.zip".to_string()],
        vec![index],
        Box::new(move |_, o, n| {
            let mut f = File::open(&path2).unwrap();
            f.seek(SeekFrom::Start(o)).unwrap();
            let mut b = vec![0u8; n as usize];
            let got = f.read(&mut b).unwrap_or(0);
            b.truncate(got);
            b
        }),
    ));
    let zfs = ZipFs::new(set);

    let reports = RefCell::new(Vec::<(u64, Instant)>::new());
    let worst_gap = Cell::new(Duration::ZERO);
    let last = Cell::new(Instant::now());
    let ticks = Ticking::new(
        &zfs,
        Box::new(|n| {
            let now = Instant::now();
            worst_gap.set(worst_gap.get().max(now - last.get()));
            last.set(now);
            reports.borrow_mut().push((n, now));
            Ok(())
        }),
    );
    PEAK.store(CUR.load(Ordering::Relaxed), Ordering::Relaxed);
    let t2 = Instant::now();
    let found = scan(&ticks, true);
    let total = t2.elapsed();
    worst_gap.set(worst_gap.get().max(last.get().elapsed()));
    eprintln!("heap high-water mark during enumerate: {} MB (baseline {} MB)", PEAK.load(Ordering::Relaxed) >> 20, CUR.load(Ordering::Relaxed) >> 20);
    drop(ticks);
    let reports = reports.into_inner();
    eprintln!(
        "enumerate: {} items in {total:?}; {} progress reports (every {PROGRESS_EVERY}); longest gap between reports {:?}",
        found.len(),
        reports.len(),
        worst_gap.get()
    );

    assert_eq!(found.len(), N);
    assert!(found.iter().all(|(_, x)| x.title.is_some()));
    assert!(found.iter().all(|(i, _)| i.key.starts_with("zip:takeout-001.zip\u{1f}")));
    // One read per sidecar at least: a report every 500 of them, always at a multiple of 500.
    assert!(reports.len() >= N / PROGRESS_EVERY as usize, "{} reports", reports.len());
    assert!(reports.iter().all(|(n, _)| n % PROGRESS_EVERY == 0));
    assert!(reports.windows(2).all(|w| w[1].0 == w[0].0 + PROGRESS_EVERY));
    assert!(worst_gap.get() < Duration::from_secs(30), "a gap of {:?} between reports would trip the host's 30 s limit", worst_gap.get());
    assert!(total < Duration::from_secs(120), "{total:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
