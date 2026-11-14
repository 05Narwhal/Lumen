//! Duplicates (Round 4D): exact copies and near-identical shots, found from what the
//! catalogue already holds — the 64-bit difference hash (`assets.phash`, from ingest), the
//! capture time, the pixel size, `derived_from` and, when present, the CLIP embedding.
//! Nothing is stored but the user's *Not duplicates* answers.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::Result;

/// How alike a set's photos are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Likeness {
    /// The same picture stored more than once, perhaps in another format or size.
    Exact,
    /// Taken moments apart and almost identical, but not the same picture.
    Similar,
}

/// One photo of a set, with what the keeper is chosen by and the screen shows.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DuplicateMember {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub width: i64,
    #[ts(type = "number")]
    pub height: i64,
    #[ts(type = "number")]
    pub bytes: i64,
    #[ts(type = "number | null")]
    pub captured_at: Option<i64>,
    pub sharpness: Option<f32>,
    #[ts(type = "number")]
    pub rating: i64,
    pub favorite: bool,
    pub edited: bool,
    /// How many albums it is in.
    #[ts(type = "number")]
    pub albums: i64,
    /// The file's extension, upper case (`HEIC`, `JPG`, `DNG`).
    pub format: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DuplicateSet {
    /// The member ids, sorted and joined by commas: stable while the set is.
    pub key: String,
    pub likeness: Likeness,
    /// Members in the order the screen shows them: the suggested keeper first.
    pub members: Vec<DuplicateMember>,
    /// The suggested keeper.
    #[ts(type = "number")]
    pub keep: i64,
}

/// Exact: hashes at most this many bits apart (re-encoding and resizing move a few bits).
pub const EXACT_BITS: u32 = 2;
/// Exact: aspect ratios within this fraction (a copy is the same shape, whatever its size).
pub const EXACT_ASPECT: f64 = 0.01;
/// Similar: taken within this many seconds of each other.
pub const SIMILAR_SECONDS: i64 = 10;
/// Similar: hashes at most this many bits apart, when both have embeddings…
pub const SIMILAR_BITS: u32 = 6;
/// …and their embeddings at least this close (a tripod timelapse sits at about 0.93; 0.96
/// still paired too many shots that were merely alike).
pub const SIMILAR_COSINE: f32 = 0.985;
/// Similar without embeddings: stricter on the hash…
pub const SIMILAR_BITS_NO_EMBEDDING: u32 = 4;
/// …and on time.
pub const SIMILAR_SECONDS_NO_EMBEDDING: i64 = 3;
/// Exact: the two photos' pixel signatures (`FileLook::pixels`, 32×32 from the 256 px
/// thumbnails) differ by at most this much on average, of 255. A re-encode, a resize or a
/// HEIC→JPEG copy changes almost nothing; an edit (exposure, colour, a filter) changes far
/// more, and is *Similar* at most. On Julian's copy (2026-10-03) the exact candidates split
/// cleanly: 4 true copies below 1, nothing from 1 to 2, and from 2.13 up only edits (brighter,
/// warmer, cropped) or different pictures (two boarding-pass screenshots at 2.13).
pub const EXACT_PIXEL_DIFF: f32 = 1.5;
/// Exact, when both have embeddings: at least this close. A re-encode or a resize sits at
/// 0.99–1.00; unrelated flat frames that share a hash sit far below (replay, 2026-09-26).
pub const EXACT_COSINE: f32 = 0.98;
/// No photo joins a set unless its embedding is at least this close to every member's, so a
/// chain of small steps cannot join photos that are not alike.
pub const SET_COSINE: f32 = 0.90;
/// A hash with fewer set bits than this (or fewer clear ones) is a flat or dark frame: it says
/// little, and it never matches on its own.
pub const MIN_HASH_BITS: u32 = 10;
/// Same prefix and extension, numbers at most this far apart, taken in the same second: the
/// frames of a burst, not copies of one picture.
pub const BURST_SPAN: u32 = 20;
/// The hash pass skips a bucket this full: thousands of blank frames share a hash and are
/// not worth comparing pairwise.
const MAX_BUCKET: usize = 2000;

/// A photo as the finder sees it.
struct Candidate {
    id: i64,
    at: Option<i64>,
    /// The date is the camera's (not missing, not the file's modified time).
    dated: bool,
    hash: u64,
    /// Width over height as displayed.
    aspect: f64,
    derived_from: Option<i64>,
    filename: String,
    /// Lower-case extension.
    ext: String,
    relative_path: String,
    content_hash: String,
}

/// Whether a hash says enough to match on: a flat or dark frame hashes to nearly all zeros.
fn informative(hash: u64) -> bool {
    let n = hash.count_ones();
    (MIN_HASH_BITS..=64 - MIN_HASH_BITS).contains(&n)
}

/// `IMG_3689.HEIC` → `("img_", 3689, "heic")`.
fn counter(filename: &str) -> Option<(String, u64, String)> {
    let (stem, ext) = filename.rsplit_once('.')?;
    let digits = stem.len() - stem.bytes().rev().take_while(u8::is_ascii_digit).count();
    let n = stem[digits..].parse().ok()?;
    Some((stem[..digits].to_ascii_lowercase(), n, ext.to_ascii_lowercase()))
}

/// Two frames of one burst: the camera numbered them one after the other.
fn burst_frames(a: &Candidate, b: &Candidate) -> bool {
    match (counter(&a.filename), counter(&b.filename)) {
        (Some((pa, na, ea)), Some((pb, nb, eb))) => pa == pb && ea == eb && na != nb && na.abs_diff(nb) <= BURST_SPAN as u64,
        _ => false,
    }
}

/// An edge between two candidates (indices), strongest first when sorted.
#[derive(Clone, Copy)]
struct Edge {
    a: usize,
    b: usize,
    exact: bool,
    bits: u32,
    cosine: f32,
}

impl Edge {
    fn strength(&self) -> (u8, u32, i64) {
        (if self.exact { 0 } else { 1 }, self.bits, -(self.cosine * 10_000.0) as i64)
    }
}

fn same_shape(a: &Candidate, b: &Candidate) -> bool {
    (a.aspect - b.aspect).abs() <= EXACT_ASPECT * b.aspect
}

/// An original and its own export were put side by side on purpose.
fn related(a: &Candidate, b: &Candidate) -> bool {
    a.derived_from == Some(b.id) || b.derived_from == Some(a.id)
}

/// A camera shooting RAW + JPEG (or a RAW and its PNG/TIFF render): both are wanted.
fn raw_pair(a: &Candidate, b: &Candidate) -> bool {
    crate::formats::is_raw(&a.ext) != crate::formats::is_raw(&b.ext)
}

/// What an editing application leaves in a file it wrote (EXIF `Software`, XMP `CreatorTool`).
const EDITOR_MARKERS: &[&[u8]] = &[
    b"Lightroom",
    b"Photoshop",
    b"Capture One",
    b"darktable",
    b"RawTherapee",
    b"Luminar",
    b"DxO",
    b"Snapseed",
    b"Affinity Photo",
    b"GIMP",
    b"ON1 Photo",
    b"Pixelmator",
];

/// Whether a file was written by an editing application, from its first 256 KB (EXIF and XMP
/// sit at the front). Read-only; an unreadable file is taken as unedited.
pub fn written_by_editor(path: &std::path::Path) -> bool {
    use std::io::Read;
    let Ok(f) = std::fs::File::open(path) else { return false };
    let mut head = Vec::with_capacity(256 * 1024);
    if f.take(256 * 1024).read_to_end(&mut head).is_err() {
        return false;
    }
    EDITOR_MARKERS.iter().any(|m| head.windows(m.len()).any(|w| w == *m))
}

/// What the finder may read beyond the catalogue (the app passes it; tests may not).
pub struct FileLook<'a> {
    /// The library's `originals/` folder: tells an editor's export from its original.
    pub originals: &'a std::path::Path,
    /// How far apart two photos' pixels are, by content hash: the mean absolute difference
    /// (0..255) of small signatures of their thumbnails. `None` when either cannot be read.
    pub pixels: &'a dyn Fn(&str, &str) -> Option<f32>,
}

/// Which candidates an editing application wrote, and whether two have the same pixels,
/// each looked up once and only when asked.
struct EditorCheck<'p> {
    look: Option<&'p FileLook<'p>>,
    cache: HashMap<i64, bool>,
}

impl EditorCheck<'_> {
    fn edited(&mut self, c: &Candidate) -> bool {
        let Some(look) = self.look else { return false };
        *self.cache.entry(c.id).or_insert_with(|| {
            // An original never changes, so what its head says is kept for the session (Round
            // 7): only the first search after launch reads the files.
            static SEEN: std::sync::LazyLock<std::sync::Mutex<HashMap<std::path::PathBuf, bool>>> = std::sync::LazyLock::new(Default::default);
            let path = look.originals.join(&c.relative_path);
            let known = SEEN.lock().unwrap_or_else(|p| p.into_inner()).get(&path).copied();
            known.unwrap_or_else(|| {
                let edited = written_by_editor(&path);
                SEEN.lock().unwrap_or_else(|p| p.into_inner()).insert(path, edited);
                edited
            })
        })
    }

    /// Whether two photos are the same picture pixel for pixel, as far as can be told: an
    /// unreadable thumbnail says nothing against it.
    fn same_pixels(&self, a: &Candidate, b: &Candidate) -> bool {
        let Some(look) = self.look else { return true };
        (look.pixels)(&a.content_hash, &b.content_hash).is_none_or(|d| d <= EXACT_PIXEL_DIFF)
    }

    /// An original beside its edited export (from Lightroom, say): both are wanted.
    fn original_and_edit(&mut self, a: &Candidate, b: &Candidate) -> bool {
        self.edited(a) != self.edited(b)
    }
}

fn load_candidates(conn: &Connection) -> Result<Vec<Candidate>> {
    // Photos in the Hidden folder or a hiding album are left out altogether: a set of them
    // would put hidden photos on a screen anyone can open.
    let mut stmt = conn.prepare(
        "SELECT a.id, a.captured_at, a.capture_source, a.phash, a.width, a.height, a.orientation, a.derived_from, a.filename, a.extension, a.relative_path, a.content_hash
         FROM assets a
         WHERE a.state = 0 AND a.media_type != 1 AND a.concealed = 0 AND a.phash IS NOT NULL
           AND a.width > 0 AND a.height > 0",
    )?;
    let rows = stmt.query_map([], |r| {
        let (w, h): (i64, i64) = (r.get(4)?, r.get(5)?);
        let swaps = matches!(r.get::<_, i64>(6)?, 5..=8);
        let (w, h) = if swaps { (h, w) } else { (w, h) };
        let at: Option<i64> = r.get(1)?;
        let source: Option<String> = r.get(2)?;
        Ok(Candidate {
            id: r.get(0)?,
            at,
            dated: at.is_some() && source.as_deref() != Some("file_time"),
            hash: r.get::<_, i64>(3)? as u64,
            aspect: w as f64 / h as f64,
            derived_from: r.get(7)?,
            filename: r.get(8)?,
            ext: r.get::<_, String>(9)?.to_ascii_lowercase(),
            relative_path: r.get(10)?,
            content_hash: r.get(11)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// Every photo's embedding, read in one scan (Round 7: 32 ms for 22k, where fetching them one
/// pair at a time kept a connection for the whole search).
struct Embeddings {
    vectors: HashMap<i64, Vec<i8>>,
}

impl Embeddings {
    fn read(conn: &Connection) -> Result<Self> {
        let mut stmt = conn.prepare("SELECT asset_id, vec FROM asset_embeddings")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, crate::embedding::from_blob(&r.get::<_, Vec<u8>>(1)?))))?;
        Ok(Self { vectors: rows.collect::<std::result::Result<_, _>>()? })
    }

    /// Their cosine, or `None` when either has no embedding.
    fn cosine(&mut self, a: i64, b: i64) -> Result<Option<f32>> {
        Ok(match (self.vectors.get(&a), self.vectors.get(&b)) {
            (Some(va), Some(vb)) => Some(crate::embedding::cosine(va, vb)),
            _ => None,
        })
    }
}

/// An exact copy, as far as the embeddings and the hashes can tell: the embeddings agree
/// when both exist, and without them both hashes must say enough to be trusted.
fn exact_by_content(a: &Candidate, b: &Candidate, cosine: Option<f32>) -> bool {
    match cosine {
        Some(c) => c >= EXACT_COSINE,
        None => informative(a.hash) && informative(b.hash),
    }
}

/// Every pair that looks alike, before dismissals.
fn edges(c: &[Candidate], vectors: &mut Embeddings, editors: &mut EditorCheck) -> Result<Vec<Edge>> {
    let mut out = Vec::new();
    let mut seen: HashSet<(usize, usize)> = HashSet::new();

    // Taken close together: a sweep over capture time.
    let mut by_time: Vec<usize> = (0..c.len()).filter(|&i| c[i].at.is_some()).collect();
    by_time.sort_by_key(|&i| (c[i].at, c[i].id));
    for (k, &i) in by_time.iter().enumerate() {
        let ti = c[i].at.unwrap_or_default();
        for &j in &by_time[k + 1..] {
            let dt = c[j].at.unwrap_or_default() - ti;
            if dt > SIMILAR_SECONDS {
                break;
            }
            if related(&c[i], &c[j]) || raw_pair(&c[i], &c[j]) {
                continue;
            }
            let bits = (c[i].hash ^ c[j].hash).count_ones();
            if bits > SIMILAR_BITS {
                continue;
            }
            let cos = vectors.cosine(c[i].id, c[j].id)?;
            let exact = dt == 0 && bits <= EXACT_BITS && same_shape(&c[i], &c[j]) && !burst_frames(&c[i], &c[j]) && exact_by_content(&c[i], &c[j], cos);
            let similar = match cos {
                Some(cos) => (cos >= SIMILAR_COSINE).then_some(cos),
                None => (bits <= SIMILAR_BITS_NO_EMBEDDING
                    && dt <= SIMILAR_SECONDS_NO_EMBEDDING
                    && informative(c[i].hash)
                    && informative(c[j].hash))
                .then_some(0.0),
            };
            // The files are read last, and only for a pair that is otherwise a match (Round 7:
            // reading them for every pair a burst made took 11 s).
            if (!exact && similar.is_none()) || editors.original_and_edit(&c[i], &c[j]) {
                continue;
            }
            if exact && editors.same_pixels(&c[i], &c[j]) {
                seen.insert((i.min(j), i.max(j)));
                out.push(Edge { a: i, b: j, exact: true, bits, cosine: cos.unwrap_or(1.0) });
                continue;
            }
            if let Some(cosine) = similar {
                seen.insert((i.min(j), i.max(j)));
                out.push(Edge { a: i, b: j, exact: false, bits, cosine });
            }
        }
    }

    // Copies whose dates differ because one lost its EXIF: the same hash, found through four
    // 16-bit slices of it (two hashes within two bits share at least two slices whole).
    for slice in 0..4u32 {
        let mut buckets: HashMap<u16, Vec<usize>> = HashMap::new();
        for (i, cand) in c.iter().enumerate() {
            buckets.entry((cand.hash >> (slice * 16)) as u16).or_default().push(i);
        }
        for members in buckets.values().filter(|m| m.len() > 1 && m.len() <= MAX_BUCKET) {
            for (k, &i) in members.iter().enumerate() {
                for &j in &members[k + 1..] {
                    let key = (i.min(j), i.max(j));
                    if seen.contains(&key) || (c[i].dated && c[j].dated) || related(&c[i], &c[j]) || raw_pair(&c[i], &c[j]) {
                        continue;
                    }
                    // Without a shared moment the hash is all there is: it must say enough.
                    if !informative(c[i].hash) || !informative(c[j].hash) {
                        continue;
                    }
                    let bits = (c[i].hash ^ c[j].hash).count_ones();
                    if bits <= EXACT_BITS && same_shape(&c[i], &c[j]) {
                        let cos = vectors.cosine(c[i].id, c[j].id)?;
                        if exact_by_content(&c[i], &c[j], cos) && !editors.original_and_edit(&c[i], &c[j]) && editors.same_pixels(&c[i], &c[j]) {
                            seen.insert(key);
                            out.push(Edge { a: i, b: j, exact: true, bits, cosine: cos.unwrap_or(1.0) });
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

fn dismissed(conn: &Connection) -> Result<HashSet<(i64, i64)>> {
    let mut stmt = conn.prepare("SELECT a, b FROM duplicate_dismissals")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// Joins pairs into sets, strongest likeness first, never joining two photos the user said
/// are not duplicates: an edge that would is dropped, so the weaker link gives way.
fn group(c: &[Candidate], mut edges: Vec<Edge>, apart: &HashSet<(i64, i64)>, vectors: &mut Embeddings) -> Result<Vec<(Vec<usize>, bool)>> {
    edges.sort_by_key(Edge::strength);
    let mut root: Vec<usize> = (0..c.len()).collect();
    let mut members: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut exact: HashMap<usize, bool> = HashMap::new();
    fn find(root: &mut [usize], mut x: usize) -> usize {
        while root[x] != x {
            root[x] = root[root[x]];
            x = root[x];
        }
        x
    }
    for e in edges {
        let (ra, rb) = (find(&mut root, e.a), find(&mut root, e.b));
        if ra == rb {
            continue;
        }
        let ma = members.get(&ra).cloned().unwrap_or_else(|| vec![ra]);
        let mb = members.get(&rb).cloned().unwrap_or_else(|| vec![rb]);
        let crosses = ma.iter().any(|&x| {
            mb.iter().any(|&y| {
                let (p, q) = (c[x].id.min(c[y].id), c[x].id.max(c[y].id));
                apart.contains(&(p, q))
            })
        });
        if crosses {
            continue;
        }
        // Every pair across the join must still look alike, or a chain of small steps would
        // carry the set somewhere else.
        let mut unlike = false;
        'pairs: for &x in &ma {
            for &y in &mb {
                if vectors.cosine(c[x].id, c[y].id)?.is_some_and(|cos| cos < SET_COSINE) {
                    unlike = true;
                    break 'pairs;
                }
            }
        }
        if unlike {
            continue;
        }
        let both_exact = exact.get(&ra).copied().unwrap_or(true) && exact.get(&rb).copied().unwrap_or(true) && e.exact;
        let (big, small) = if ma.len() >= mb.len() { (ra, rb) } else { (rb, ra) };
        root[small] = big;
        let mut joined = ma;
        joined.extend(mb);
        members.remove(&small);
        exact.remove(&small);
        members.insert(big, joined);
        exact.insert(big, both_exact);
    }
    Ok(members.into_iter().map(|(r, m)| (m, exact.get(&r).copied().unwrap_or(false))).collect())
}

/// RAW over the formats a camera or phone writes losslessly or natively, over JPEG.
fn format_rank(ext: &str) -> u8 {
    let e = ext.to_ascii_lowercase();
    if crate::formats::is_raw(&e) {
        3
    } else if matches!(e.as_str(), "heic" | "heif" | "png" | "tif" | "tiff" | "jxl" | "avif") {
        2
    } else if matches!(e.as_str(), "jpg" | "jpeg") {
        1
    } else {
        0
    }
}

fn members(conn: &Connection, ids: &[i64]) -> Result<Vec<(DuplicateMember, bool)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT a.id, a.width, a.height, a.orientation, a.byte_size, a.captured_at, a.sharpness, a.rating, a.is_favorite,
                (EXISTS (SELECT 1 FROM edits e WHERE e.asset_id = a.id) OR EXISTS (SELECT 1 FROM develop_drafts dd WHERE dd.asset_id = a.id)),
                (SELECT COUNT(*) FROM album_items i WHERE i.asset_id = a.id),
                a.extension, a.derived_from IS NOT NULL
         FROM assets a WHERE a.id = ?1",
    )?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        out.push(stmt.query_row([id], |r| {
            let (w, h): (Option<i64>, Option<i64>) = (r.get(1)?, r.get(2)?);
            let swaps = matches!(r.get::<_, i64>(3)?, 5..=8);
            let (w, h) = (w.unwrap_or(0), h.unwrap_or(0));
            let (width, height) = if swaps { (h, w) } else { (w, h) };
            Ok((
                DuplicateMember {
                    id: r.get(0)?,
                    width,
                    height,
                    bytes: r.get(4)?,
                    captured_at: r.get(5)?,
                    sharpness: r.get::<_, Option<f64>>(6)?.map(|s| s as f32),
                    rating: r.get(7)?,
                    favorite: r.get::<_, i64>(8)? != 0,
                    edited: r.get::<_, i64>(9)? != 0,
                    albums: r.get(10)?,
                    format: r.get::<_, String>(11)?.to_ascii_uppercase(),
                },
                r.get::<_, i64>(12)? != 0,
            ))
        })?);
    }
    Ok(out)
}

/// The suggested keeper first: not an export, more pixels, the original format, sharper,
/// edited, rated or a favourite, in more albums, the lower id.
fn keeper_order(list: &mut [(DuplicateMember, bool)]) {
    list.sort_by(|(a, a_export), (b, b_export)| {
        a_export
            .cmp(b_export)
            .then((b.width * b.height).cmp(&(a.width * a.height)))
            .then(format_rank(&b.format).cmp(&format_rank(&a.format)))
            .then(b.sharpness.unwrap_or(0.0).total_cmp(&a.sharpness.unwrap_or(0.0)))
            .then(b.edited.cmp(&a.edited))
            .then((b.rating > 0 || b.favorite).cmp(&(a.rating > 0 || a.favorite)))
            .then(b.albums.cmp(&a.albums))
            .then(a.id.cmp(&b.id))
    });
}

/// What resolving a set changed, so it can be undone exactly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ResolveUndo {
    #[ts(type = "number")]
    pub keep: i64,
    /// The keeper's fields before the merge.
    pub favorite: bool,
    #[ts(type = "number")]
    pub rating: i64,
    #[ts(type = "number")]
    pub color_label: i64,
    #[ts(type = "number")]
    pub flag: i64,
    pub keywords: Option<String>,
    pub title: Option<String>,
    pub caption: Option<String>,
    /// Albums the keeper joined.
    #[ts(type = "Array<number>")]
    pub albums_added: Vec<i64>,
    /// The photos moved to the Trash.
    #[ts(type = "Array<number>")]
    pub removed: Vec<i64>,
}

/// The fields a merge touches, as stored.
struct Marks {
    favorite: bool,
    rating: i64,
    color_label: i64,
    flag: i64,
    keywords: Option<String>,
    title: Option<String>,
    caption: Option<String>,
}

fn marks(conn: &Connection, id: i64) -> Result<Marks> {
    Ok(conn.query_row(
        "SELECT is_favorite, rating, color_label, flag, keywords, title, caption FROM assets WHERE id = ?1",
        [id],
        |r| {
            Ok(Marks {
                favorite: r.get::<_, i64>(0)? != 0,
                rating: r.get(1)?,
                color_label: r.get(2)?,
                flag: r.get(3)?,
                keywords: r.get(4)?,
                title: r.get(5)?,
                caption: r.get(6)?,
            })
        },
    )?)
}

fn write_marks(conn: &Connection, id: i64, m: &Marks) -> Result<()> {
    conn.execute(
        "UPDATE assets SET is_favorite = ?2, rating = ?3, color_label = ?4, flag = ?5, keywords = ?6, title = ?7, caption = ?8 WHERE id = ?1",
        params![id, m.favorite as i64, m.rating, m.color_label, m.flag, m.keywords, m.title, m.caption],
    )?;
    Ok(())
}

/// Refuses a set whose keeper is in the Trash or is also to be removed.
fn check(conn: &Connection, keep: i64, remove: &[i64]) -> Result<()> {
    let active: bool = conn.query_row("SELECT COUNT(*) FROM assets WHERE id = ?1 AND state = 0", [keep], |r| r.get::<_, i64>(0))? == 1;
    if !active {
        return Err(crate::CoreError::Invalid("The photo to keep is in the Trash.".into()));
    }
    if remove.contains(&keep) {
        return Err(crate::CoreError::Invalid("A photo can't be both kept and removed.".into()));
    }
    Ok(())
}

/// Keeps `keep` and moves `remove` to the Trash, in one transaction. First the keeper takes
/// on what the others carried: every album they were in (at their place in it), favourite if
/// any was, the highest rating, a label and a flag if it had none, every keyword, and a title
/// or caption if it had none. Faces are left alone: they go to the Trash with their photo and
/// come back with it.
pub fn resolve(conn: &Connection, keep: i64, remove: &[i64], now: i64) -> Result<ResolveUndo> {
    let tx = conn.unchecked_transaction()?;
    let undo = resolve_in(&tx, keep, remove, now)?;
    tx.commit()?;
    Ok(undo)
}

/// `resolve` inside the caller's transaction. Photos that are not in the library any more or
/// are already in the Trash are left out, so `removed` is exactly what this call moved.
fn resolve_in(tx: &Connection, keep: i64, remove: &[i64], now: i64) -> Result<ResolveUndo> {
    check(tx, keep, remove)?;
    let mut active = Vec::with_capacity(remove.len());
    for &id in remove {
        if tx.query_row("SELECT COUNT(*) FROM assets WHERE id = ?1 AND state = 0", [id], |r| r.get::<_, i64>(0))? == 1 {
            active.push(id);
        }
    }
    let remove = &active[..];
    let before = marks(tx, keep)?;
    let mut merged = Marks { keywords: None, ..marks(tx, keep)? };
    let mut keywords: Vec<String> = before.keywords.as_deref().map(crate::export::split_keywords).unwrap_or_default();
    let mut albums_added = Vec::new();
    for &r in remove {
        let m = marks(tx, r)?;
        merged.favorite |= m.favorite;
        merged.rating = merged.rating.max(m.rating);
        if merged.color_label == 0 {
            merged.color_label = m.color_label;
        }
        if merged.flag == 0 {
            merged.flag = m.flag;
        }
        for k in m.keywords.as_deref().map(crate::export::split_keywords).unwrap_or_default() {
            if !keywords.contains(&k) {
                keywords.push(k);
            }
        }
        if merged.title.as_deref().is_none_or(str::is_empty) {
            merged.title = m.title;
        }
        if merged.caption.as_deref().is_none_or(str::is_empty) {
            merged.caption = m.caption;
        }
        let mut stmt = tx.prepare_cached(
            "SELECT album_id, position FROM album_items i WHERE asset_id = ?1
               AND NOT EXISTS (SELECT 1 FROM album_items k WHERE k.album_id = i.album_id AND k.asset_id = ?2)",
        )?;
        let rows: Vec<(i64, f64)> = stmt.query_map([r, keep], |x| Ok((x.get(0)?, x.get(1)?)))?.collect::<std::result::Result<_, _>>()?;
        for (album, position) in rows {
            tx.execute("INSERT INTO album_items (album_id, asset_id, position, added_at) VALUES (?1, ?2, ?3, ?4)", params![album, keep, position, now])?;
            albums_added.push(album);
        }
    }
    merged.keywords = if keywords.is_empty() { before.keywords.clone() } else { Some(keywords.join("\n")) };
    write_marks(tx, keep, &merged)?;
    crate::assets::trash_assets(tx, remove, now)?;
    crate::fts::index_ids(tx, &[keep])?;
    Ok(ResolveUndo {
        keep,
        favorite: before.favorite,
        rating: before.rating,
        color_label: before.color_label,
        flag: before.flag,
        keywords: before.keywords,
        title: before.title,
        caption: before.caption,
        albums_added,
        removed: remove.to_vec(),
    })
}

/// Resolves several sets, or none: every set is checked before any is changed, so a
/// refusal never leaves some resolved with no Undo.
pub fn resolve_all(conn: &Connection, sets: &[(i64, Vec<i64>)], now: i64) -> Result<Vec<ResolveUndo>> {
    for (keep, remove) in sets {
        check(conn, *keep, remove)?;
    }
    // One transaction for the lot: whatever fails midway leaves no set resolved and no Undo lost.
    let tx = conn.unchecked_transaction()?;
    let undos = sets.iter().map(|(keep, remove)| resolve_in(&tx, *keep, remove, now)).collect::<Result<Vec<_>>>()?;
    tx.commit()?;
    Ok(undos)
}

/// Undoes `resolve`s, newest first: the keeper's fields and albums as they were, and the
/// removed photos out of the Trash.
pub fn undo_resolve(conn: &Connection, undos: &[ResolveUndo]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    for u in undos.iter().rev() {
        let m = Marks {
            favorite: u.favorite,
            rating: u.rating,
            color_label: u.color_label,
            flag: u.flag,
            keywords: u.keywords.clone(),
            title: u.title.clone(),
            caption: u.caption.clone(),
        };
        write_marks(&tx, u.keep, &m)?;
        for album in &u.albums_added {
            tx.execute("DELETE FROM album_items WHERE album_id = ?1 AND asset_id = ?2", params![album, u.keep])?;
        }
        crate::assets::restore_assets(&tx, &u.removed)?;
        crate::fts::index_ids(&tx, &[u.keep])?;
    }
    tx.commit()?;
    Ok(())
}

/// Every set, newest first. With a `FileLook`, an original and a copy an editing application
/// wrote are never paired, and *Exact* also needs the same pixels (an edit is *Similar* at most).
pub fn find_sets_in(conn: &Connection, look: Option<&FileLook>) -> Result<Vec<DuplicateSet>> {
    let input = SetsInput::read(conn)?;
    let groups = input.group(look)?;
    sets_of(conn, &groups)
}

/// `find_sets_in` for the app (Round 7): the catalogue is read in one short hold, pairs are
/// judged (reading files) with no connection, and only the sets' members are read at the end.
/// Holding one connection throughout took 11 s on Julian's library.
pub fn find_sets_on(db: &crate::Db, look: Option<&FileLook>) -> Result<Vec<DuplicateSet>> {
    let input = db.with_conn(SetsInput::read)?;
    let groups = input.group(look)?;
    if groups.is_empty() {
        return Ok(Vec::new());
    }
    db.with_conn(|c| sets_of(c, &groups))
}

/// What the finder needs from the catalogue.
struct SetsInput {
    candidates: Vec<Candidate>,
    vectors: Embeddings,
    apart: HashSet<(i64, i64)>,
}

impl SetsInput {
    fn read(conn: &Connection) -> Result<Self> {
        Ok(Self { candidates: load_candidates(conn)?, vectors: Embeddings::read(conn)?, apart: dismissed(conn)? })
    }

    /// Each set's photo ids (sorted) and whether it is exact.
    fn group(mut self, look: Option<&FileLook>) -> Result<Vec<(Vec<i64>, bool)>> {
        let c = &self.candidates;
        let mut editors = EditorCheck { look, cache: HashMap::new() };
        let edges = edges(c, &mut self.vectors, &mut editors)?;
        if edges.is_empty() {
            return Ok(Vec::new());
        }
        Ok(group(c, edges, &self.apart, &mut self.vectors)?
            .into_iter()
            .map(|(group, exact)| {
                let mut ids: Vec<i64> = group.iter().map(|&i| c[i].id).collect();
                ids.sort_unstable();
                (ids, exact)
            })
            .collect())
    }
}

fn sets_of(conn: &Connection, groups: &[(Vec<i64>, bool)]) -> Result<Vec<DuplicateSet>> {
    let mut out = Vec::new();
    for (ids, exact) in groups {
        let exact = *exact;
        let mut list = members(conn, ids)?;
        keeper_order(&mut list);
        let keep = list[0].0.id;
        out.push(DuplicateSet {
            key: ids.iter().map(i64::to_string).collect::<Vec<_>>().join(","),
            likeness: if exact { Likeness::Exact } else { Likeness::Similar },
            members: list.into_iter().map(|(m, _)| m).collect(),
            keep,
        });
    }
    out.sort_by_key(|s| (std::cmp::Reverse(s.members.iter().filter_map(|m| m.captured_at).max()), s.key.clone()));
    Ok(out)
}

/// Every set, newest first, without looking at the files.
pub fn find_sets(conn: &Connection) -> Result<Vec<DuplicateSet>> {
    find_sets_in(conn, None)
}

/// `(sets, photos that could go)` for the Home card.
pub fn count_sets(conn: &Connection) -> Result<(usize, usize)> {
    count_sets_in(conn, None)
}

pub fn count_sets_in(conn: &Connection, look: Option<&FileLook>) -> Result<(usize, usize)> {
    let sets = find_sets_in(conn, look)?;
    let removable = sets.iter().map(|s| s.members.len() - 1).sum();
    Ok((sets.len(), removable))
}

/// *Not duplicates*: every pair within `ids` stays apart from now on.
pub fn dismiss(conn: &Connection, ids: &[i64], now: i64) -> Result<()> {
    let mut stmt = conn.prepare_cached("INSERT OR IGNORE INTO duplicate_dismissals (a, b, created_at) VALUES (?1, ?2, ?3)")?;
    for (i, a) in ids.iter().enumerate() {
        for b in &ids[i + 1..] {
            let (lo, hi) = if a < b { (a, b) } else { (b, a) };
            if lo != hi {
                stmt.execute(params![lo, hi, now])?;
            }
        }
    }
    Ok(())
}

/// Undoes `dismiss` for the pairs within `ids`.
pub fn undismiss(conn: &Connection, ids: &[i64]) -> Result<()> {
    let mut stmt = conn.prepare_cached("DELETE FROM duplicate_dismissals WHERE a = ?1 AND b = ?2")?;
    for (i, a) in ids.iter().enumerate() {
        for b in &ids[i + 1..] {
            let (lo, hi) = if a < b { (a, b) } else { (b, a) };
            stmt.execute(params![lo, hi])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding;
    use crate::Db;

    /// A photo row: id, capture second, hash, size.
    struct Row {
        id: i64,
        at: Option<i64>,
        hash: u64,
        w: i64,
        h: i64,
    }
    const fn row(id: i64, at: i64, hash: u64) -> Row {
        Row { id, at: Some(at), hash, w: 4032, h: 3024 }
    }

    fn insert(c: &Connection, r: &Row) {
        c.execute(
            "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, width, height, orientation, captured_at, capture_source, imported_at, phash)
             VALUES (?1, ?2, 'p', 'f.jpg', 'jpg', 0, ?3, ?4, ?5, 1, ?6, ?7, 10, ?8)",
            params![r.id, format!("h{}", r.id), r.w * r.h / 4, r.w, r.h, r.at, r.at.map(|_| "original"), r.hash as i64],
        )
        .unwrap();
    }

    fn library(rows: &[Row]) -> Db {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            for r in rows {
                insert(c, r);
            }
            Ok(())
        })
        .unwrap();
        db
    }

    /// A unit vector along `axis`, bent towards `other` so that its cosine with a plain
    /// `axis` vector is about `cos`.
    fn vector(cos: f32, other: usize) -> Vec<i8> {
        let mut v = vec![0f32; embedding::DIM];
        v[0] = cos;
        v[other] = (1.0 - cos * cos).sqrt();
        embedding::quantize(&v)
    }

    fn embed(db: &Db, id: i64, v: &[i8]) {
        db.with_conn(|c| embedding::store(c, id, "clip", v, None)).unwrap();
    }

    fn sets(db: &Db) -> Vec<DuplicateSet> {
        db.with_conn(find_sets).unwrap()
    }

    fn ids(s: &DuplicateSet) -> Vec<i64> {
        let mut v: Vec<i64> = s.members.iter().map(|m| m.id).collect();
        v.sort();
        v
    }

    /// Flips the lowest `n` bits.
    const fn bits(hash: u64, n: u32) -> u64 {
        hash ^ ((1u64 << n) - 1)
    }

    const H: u64 = 0xA5A5_F00F_3C3C_9669;

    #[test]
    fn the_same_shot_twice_is_an_exact_copy() {
        let db = library(&[row(1, 1000, H), Row { w: 2016, h: 1512, ..row(2, 1000, bits(H, 1)) }]);
        let s = sets(&db);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].likeness, Likeness::Exact);
        assert_eq!(ids(&s[0]), vec![1, 2]);
        assert_eq!(s[0].keep, 1, "the larger");
        assert_eq!(s[0].members[0].id, 1, "the keeper first");
        assert_eq!(s[0].key, "1,2");
        assert_eq!(db.with_conn(count_sets).unwrap(), (1, 1));
    }

    #[test]
    fn a_burst_is_one_set() {
        let rows: Vec<Row> = (1..=30).map(|i| row(i, 1000 + i / 4, bits(H, (i % 5) as u32))).collect();
        let db = library(&rows);
        for i in 1..=30 {
            embed(&db, i, &vector(0.99, 1 + (i as usize % 3)));
        }
        db.with_conn(|c| Ok(c.execute("UPDATE assets SET sharpness = CASE id WHEN 17 THEN 0.9 ELSE 0.2 END", [])?)).unwrap();
        let s = sets(&db);
        assert_eq!(s.len(), 1, "one set, not pairs");
        assert_eq!(s[0].members.len(), 30);
        assert_eq!(s[0].likeness, Likeness::Similar);
        assert_eq!(s[0].keep, 17, "the sharpest");
    }

    /// Round 7: the app's path (read, judge with no connection, read the members) finds what
    /// the one-connection path finds.
    #[test]
    fn the_app_finds_the_same_sets_without_holding_a_connection() {
        let mut rows: Vec<Row> = (1..=12).map(|i| row(i, 1000 + i / 4, bits(H, (i % 5) as u32))).collect();
        rows.push(row(20, 5000, H));
        rows.push(Row { w: 2016, h: 1512, ..row(21, 5000, bits(H, 1)) });
        let db = library(&rows);
        for i in 1..=12 {
            embed(&db, i, &vector(0.99, 1 + (i as usize % 3)));
        }
        let held = sets(&db);
        assert_eq!(held.len(), 2);
        assert_eq!(find_sets_on(&db, None).unwrap(), held);
    }

    #[test]
    fn an_export_is_not_a_duplicate_of_its_source() {
        let db = library(&[row(1, 1000, H), row(2, 1000, H)]);
        db.with_conn(|c| Ok(c.execute("UPDATE assets SET derived_from = 1 WHERE id = 2", [])?)).unwrap();
        assert!(sets(&db).is_empty());
    }

    #[test]
    fn a_timelapse_is_not_similar() {
        let rows: Vec<Row> = (1..=6).map(|i| row(i, 1000 + i * 10, bits(H, 8))).collect();
        let db = library(&rows);
        embed(&db, 1, &vector(1.0, 1));
        for i in 2..=6 {
            embed(&db, i, &vector(0.93, i as usize));
        }
        // The first frame is the plain axis; the rest are 0.93 from it and ~0.86 from each other.
        assert!(sets(&db).is_empty());
    }

    #[test]
    fn without_embeddings_the_rules_are_stricter() {
        let db = library(&[row(1, 1000, H), row(2, 1002, bits(H, 8))]);
        assert!(sets(&db).is_empty(), "8 bits at 2 s is too far without embeddings");
        let db = library(&[row(1, 1000, H), row(2, 1002, bits(H, 3))]);
        let s = sets(&db);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].likeness, Likeness::Similar);
        let db = library(&[row(1, 1000, H), row(2, 1002, bits(H, 5))]);
        assert!(sets(&db).is_empty(), "5 bits is too far without embeddings");
        let db = library(&[row(1, 1000, H), row(2, 1005, bits(H, 3))]);
        assert!(sets(&db).is_empty(), "5 s is too far without embeddings");
    }

    #[test]
    fn a_dismissed_pair_stays_apart() {
        let db = library(&[row(1, 1000, H), row(2, 1001, bits(H, 3))]);
        assert_eq!(sets(&db).len(), 1);
        db.with_conn(|c| dismiss(c, &[2, 1], 5)).unwrap();
        assert!(sets(&db).is_empty());
        // A third photo like both: it may pair with one of them, never bring 1 and 2 together.
        db.with_conn(|c| {
            insert(c, &row(3, 1000, bits(H, 1)));
            Ok(())
        })
        .unwrap();
        let s = sets(&db);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].members.len(), 2);
        assert!(ids(&s[0]).contains(&3));
        db.with_conn(|c| undismiss(c, &[1, 2])).unwrap();
        assert_eq!(sets(&db)[0].members.len(), 3);
    }

    #[test]
    fn copies_with_lost_dates_are_found_by_hash() {
        let db = library(&[row(1, 1000, H), Row { at: None, ..row(2, 0, bits(H, 2)) }]);
        let s = sets(&db);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].likeness, Likeness::Exact);
        // The same when the copy's date is only the file's modified time, days later.
        let db = library(&[row(1, 1000, H), row(2, 900_000, H)]);
        db.with_conn(|c| Ok(c.execute("UPDATE assets SET capture_source = 'file_time' WHERE id = 2", [])?)).unwrap();
        assert_eq!(sets(&db).len(), 1);
        // But two real captures days apart that happen to look alike are not copies.
        let db = library(&[row(1, 1000, H), row(2, 900_000, H)]);
        assert!(sets(&db).is_empty());
    }

    #[test]
    fn a_different_shape_is_not_a_copy() {
        let db = library(&[row(1, 1000, H), Row { w: 3024, h: 3024, ..row(2, 1000, H) }]);
        let s = sets(&db);
        assert!(s.iter().all(|s| s.likeness != Likeness::Exact), "a square crop is not the same picture");
    }

    #[test]
    fn hidden_photos_are_not_paired_with_visible_ones() {
        let db = library(&[row(1, 1000, H), row(2, 1000, H)]);
        db.with_conn(|c| Ok(c.execute("UPDATE assets SET concealed = 1 WHERE id = 2", [])?)).unwrap();
        assert!(sets(&db).is_empty());
    }

    #[test]
    fn videos_trash_and_movie_halves_are_left_out() {
        let db = library(&[row(1, 1000, H), row(2, 1000, H), row(3, 1000, H), row(4, 1000, H)]);
        db.with_conn(|c| {
            c.execute("UPDATE assets SET media_type = 1 WHERE id = 2", [])?;
            c.execute("UPDATE assets SET state = 1, trashed_at = 5 WHERE id = 3", [])?;
            Ok(())
        })
        .unwrap();
        let s = sets(&db);
        assert_eq!(s.len(), 1);
        assert_eq!(ids(&s[0]), vec![1, 4]);
    }

    #[test]
    fn the_keeper_prefers_the_original_format_then_the_better_photo() {
        let db = library(&[row(1, 1000, H), row(2, 1000, H), row(3, 1000, H)]);
        db.with_conn(|c| {
            c.execute("UPDATE assets SET extension = 'heic' WHERE id = 2", [])?;
            Ok(())
        })
        .unwrap();
        assert_eq!(sets(&db)[0].keep, 2, "HEIC over JPEG");
        let db = library(&[row(1, 1000, H), row(2, 1000, H)]);
        db.with_conn(|c| Ok(c.execute("UPDATE assets SET rating = 4 WHERE id = 2", [])?)).unwrap();
        assert_eq!(sets(&db)[0].keep, 2, "rated");
        let db = library(&[row(1, 1000, H), row(2, 1000, H)]);
        assert_eq!(sets(&db)[0].keep, 1, "the lower id when nothing else decides");
    }

    #[test]
    fn an_edit_is_never_an_exact_copy() {
        let dir = tempfile::tempdir().unwrap();
        // 2 is the same picture as 1; 3 is 1 with its exposure raised.
        let pixels = |a: &str, b: &str| -> Option<f32> { Some(if a == "h3" || b == "h3" { 18.0 } else { 1.0 }) };
        let look = FileLook { originals: dir.path(), pixels: &pixels };
        let find = |rows: &[Row], look: &FileLook| library(rows).with_conn(|c| find_sets_in(c, Some(look))).unwrap();
        let copy = find(&[row(1, 1000, H), row(2, 1000, H)], &look);
        assert_eq!(copy[0].likeness, Likeness::Exact);
        let edit = find(&[row(1, 1000, H), row(3, 1000, H)], &look);
        assert_eq!(edit.len(), 1);
        assert_eq!(edit[0].likeness, Likeness::Similar, "an edit is at most similar");
        // Without the thumbnails nothing speaks against an exact copy.
        let blind = FileLook { originals: dir.path(), pixels: &|_, _| None };
        assert_eq!(find(&[row(1, 1000, H), row(3, 1000, H)], &blind)[0].likeness, Likeness::Exact);
    }

    #[test]
    fn a_raw_and_its_jpeg_are_not_duplicates() {
        let db = library(&[row(1, 1000, H), row(2, 1000, H)]);
        db.with_conn(|c| Ok(c.execute("UPDATE assets SET extension = 'nef' WHERE id = 2", [])?)).unwrap();
        assert!(sets(&db).is_empty());
    }

    #[test]
    fn an_original_and_its_lightroom_export_are_not_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("1.jpg"), b"JPEG from a Canon EOS").unwrap();
        std::fs::write(dir.path().join("2.jpg"), b"JPEG by Adobe Photoshop Lightroom Classic 13.0").unwrap();
        let db = library(&[row(1, 1000, H), row(2, 1000, H)]);
        db.with_conn(|c| {
            c.execute("UPDATE assets SET relative_path = id || '.jpg'", [])?;
            Ok(())
        })
        .unwrap();
        assert_eq!(db.with_conn(find_sets).unwrap().len(), 1, "without the files they look alike");
        let look = FileLook { originals: dir.path(), pixels: &|_, _| None };
        assert!(db.with_conn(|c| find_sets_in(c, Some(&look))).unwrap().is_empty());
    }

    #[test]
    fn newest_sets_come_first() {
        let db = library(&[row(1, 1000, H), row(2, 1000, H), row(3, 90_000, !H), row(4, 90_000, !H)]);
        let s = sets(&db);
        assert_eq!(s.iter().map(|s| s.key.as_str()).collect::<Vec<_>>(), vec!["3,4", "1,2"]);
    }

    // From the replay on Julian's library (2026-09-26): what the first rules got wrong.

    #[test]
    fn flat_frames_are_not_matched_by_their_hash_alone() {
        // Dark or blank photos hash to nearly nothing; two different ones must not be copies.
        let db = library(&[row(1, 1000, 0x0000_0000_0004_0000), Row { at: None, ..row(2, 0, 0) }, row(3, 1000, 0)]);
        embed(&db, 1, &vector(1.0, 1));
        embed(&db, 2, &vector(0.55, 2));
        embed(&db, 3, &vector(0.60, 3));
        assert!(sets(&db).is_empty());
    }

    #[test]
    fn a_same_second_burst_is_similar_not_exact() {
        let db = library(&[row(1, 1000, H), row(2, 1000, bits(H, 1)), row(3, 1000, H)]);
        db.with_conn(|c| {
            c.execute("UPDATE assets SET filename = 'IMG_3689.HEIC', extension = 'heic' WHERE id = 1", [])?;
            c.execute("UPDATE assets SET filename = 'IMG_3690.HEIC', extension = 'heic' WHERE id = 2", [])?;
            c.execute("UPDATE assets SET filename = 'IMG_3691.HEIC', extension = 'heic' WHERE id = 3", [])?;
            Ok(())
        })
        .unwrap();
        for i in 1..=3 {
            embed(&db, i, &vector(0.997, i as usize));
        }
        let s = sets(&db);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].likeness, Likeness::Similar, "frames of a burst are not copies");
        // A HEIC and a JPEG made from it, numbered far apart, are copies.
        let db = library(&[row(1, 1000, H), row(2, 1000, bits(H, 2))]);
        db.with_conn(|c| {
            c.execute("UPDATE assets SET filename = 'IMG_6053.HEIC', extension = 'heic' WHERE id = 1", [])?;
            c.execute("UPDATE assets SET filename = 'IMG_6340.JPG' WHERE id = 2", [])?;
            Ok(())
        })
        .unwrap();
        embed(&db, 1, &vector(1.0, 1));
        embed(&db, 2, &vector(0.99, 2));
        assert_eq!(sets(&db)[0].likeness, Likeness::Exact);
    }

    #[test]
    fn an_exact_match_needs_the_embeddings_to_agree() {
        let db = library(&[row(1, 1000, H), row(2, 1000, H)]);
        embed(&db, 1, &vector(1.0, 1));
        embed(&db, 2, &vector(0.9, 2));
        assert!(sets(&db).iter().all(|s| s.likeness != Likeness::Exact));
    }

    #[test]
    fn a_chain_does_not_join_unlike_photos() {
        // Each is like the next, but the ends are far apart: they never share a set.
        let db = library(&[
            row(1, 1000, H),
            row(2, 1001, bits(H, 1)),
            row(3, 1002, bits(H, 2)),
            row(4, 1003, bits(H, 3)),
            row(5, 1004, bits(H, 4)),
        ]);
        // Five directions in a plane, 8° apart.
        let at = |deg: f32| {
            let mut v = vec![0f32; embedding::DIM];
            v[0] = deg.to_radians().cos();
            v[1] = deg.to_radians().sin();
            embedding::quantize(&v)
        };
        let v: Vec<Vec<i8>> = (0..5).map(|k| at(8.0 * k as f32)).collect();
        assert!(v.windows(2).all(|w| embedding::cosine(&w[0], &w[1]) >= SIMILAR_COSINE), "neighbours are alike");
        assert!(embedding::cosine(&v[0], &v[4]) < SET_COSINE, "the ends are not");
        for (k, e) in v.iter().enumerate() {
            embed(&db, k as i64 + 1, e);
        }
        let s = sets(&db);
        assert!(!s.is_empty());
        assert!(s.iter().all(|s| !(ids(s).contains(&1) && ids(s).contains(&5))));
    }

    // Task 2: resolving a set.

    fn album(c: &Connection, id: i64, items: &[(i64, f64)]) {
        c.execute("INSERT INTO albums (id, name, kind, created_at, updated_at) VALUES (?1, ?2, 0, 1, 1)", params![id, format!("A{id}")]).unwrap();
        for (asset, pos) in items {
            c.execute("INSERT INTO album_items (album_id, asset_id, position, added_at) VALUES (?1, ?2, ?3, 1)", params![id, asset, pos]).unwrap();
        }
    }

    /// Every column of an asset row and its album memberships, to compare before and after.
    fn snapshot(c: &Connection, id: i64) -> (Vec<String>, Vec<(i64, String)>) {
        let mut stmt = c.prepare("SELECT * FROM assets WHERE id = ?1").unwrap();
        let n = stmt.column_count();
        let row = stmt
            .query_row([id], |r| Ok((0..n).map(|i| format!("{:?}", r.get_ref(i).unwrap())).collect::<Vec<_>>()))
            .unwrap();
        let mut stmt = c.prepare("SELECT album_id, position FROM album_items WHERE asset_id = ?1 ORDER BY album_id").unwrap();
        let albums = stmt.query_map([id], |r| Ok((r.get(0)?, format!("{:?}", r.get_ref(1)?)))).unwrap().map(|r| r.unwrap()).collect();
        (row, albums)
    }

    fn mark(db: &Db) {
        db.with_conn(|c| {
            c.execute("UPDATE assets SET rating = 2, keywords = 'beach' || char(10) || 'sea', title = NULL WHERE id = 1", [])?;
            c.execute("UPDATE assets SET is_favorite = 1, rating = 4, keywords = 'sea' || char(10) || 'anna', title = 'Swim', caption = 'Morning', color_label = 3 WHERE id = 2", [])?;
            c.execute("UPDATE assets SET flag = 1, keywords = 'dog' WHERE id = 3", [])?;
            album(c, 10, &[(2, 5.0), (3, 7.0)]);
            album(c, 11, &[(1, 1.0), (3, 2.0)]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn resolving_keeps_one_and_merges_what_the_others_carried() {
        let db = library(&[row(1, 1000, H), row(2, 1000, H), row(3, 1000, H)]);
        mark(&db);
        let undo = db.with_conn(|c| resolve(c, 1, &[2, 3], 50)).unwrap();
        db.with_conn(|c| {
            let (fav, rating, label, flag, kw, title, caption): (i64, i64, i64, i64, String, Option<String>, Option<String>) = c.query_row(
                "SELECT is_favorite, rating, color_label, flag, keywords, title, caption FROM assets WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
            )?;
            assert_eq!((fav, rating, label, flag), (1, 4, 3, 1), "favourite if any, the highest rating, the first label and flag");
            assert_eq!(kw.lines().collect::<Vec<_>>(), vec!["beach", "sea", "anna", "dog"]);
            assert_eq!((title.as_deref(), caption.as_deref()), (Some("Swim"), Some("Morning")));
            let in10: f64 = c.query_row("SELECT position FROM album_items WHERE album_id = 10 AND asset_id = 1", [], |r| r.get(0))?;
            assert_eq!(in10, 5.0, "at the removed photo's place");
            let in11: f64 = c.query_row("SELECT position FROM album_items WHERE album_id = 11 AND asset_id = 1", [], |r| r.get(0))?;
            assert_eq!(in11, 1.0, "already there: untouched");
            let trashed: i64 = c.query_row("SELECT COUNT(*) FROM assets WHERE state = 1 AND id IN (2, 3)", [], |r| r.get(0))?;
            assert_eq!(trashed, 2);
            Ok(())
        })
        .unwrap();
        assert_eq!(undo.removed, vec![2, 3]);
        assert_eq!(undo.albums_added, vec![10]);
    }

    #[test]
    fn undo_restores_every_field_and_membership_exactly() {
        let db = library(&[row(1, 1000, H), row(2, 1000, H), row(3, 1000, H), row(4, 5000, !H), row(5, 5000, !H)]);
        mark(&db);
        let before: Vec<_> = (1..=5).map(|i| db.with_conn(|c| Ok(snapshot(c, i))).unwrap()).collect();
        let undos = db.with_conn(|c| Ok(vec![resolve(c, 1, &[2, 3], 50)?, resolve(c, 5, &[4], 50)?])).unwrap();
        db.with_conn(|c| undo_resolve(c, &undos)).unwrap();
        let after: Vec<_> = (1..=5).map(|i| db.with_conn(|c| Ok(snapshot(c, i))).unwrap()).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn a_live_photo_goes_with_its_movie() {
        let db = library(&[row(1, 1000, H), row(2, 1000, H)]);
        db.with_conn(|c| {
            c.execute("UPDATE assets SET media_type = 2, live_photo_group = 'g' WHERE id = 2", [])?;
            c.execute(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, live_photo_group)
                 VALUES (3, 'h3', 'p', 'f.mov', 'mov', 1, 1, 1, 10, 'g')",
                [],
            )?;
            resolve(c, 1, &[2], 50)?;
            let movie: i64 = c.query_row("SELECT state FROM assets WHERE id = 3", [], |r| r.get(0))?;
            assert_eq!(movie, 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_keeper_in_the_trash_is_refused() {
        let db = library(&[row(1, 1000, H), row(2, 1000, H)]);
        db.with_conn(|c| Ok(c.execute("UPDATE assets SET state = 1, trashed_at = 5 WHERE id = 1", [])?)).unwrap();
        let err = db.with_conn(|c| resolve(c, 1, &[2], 50)).unwrap_err();
        assert!(matches!(err, crate::CoreError::Invalid(_)), "{err:?}");
        let err = db.with_conn(|c| resolve(c, 2, &[2], 50)).unwrap_err();
        assert!(matches!(err, crate::CoreError::Invalid(_)), "keeping and removing the same photo");
        let state: i64 = db.with_conn(|c| Ok(c.query_row("SELECT state FROM assets WHERE id = 2", [], |r| r.get(0))?)).unwrap();
        assert_eq!(state, 0, "nothing happened");
    }

    /// Review (2026-10-31): a photo already in the Trash (deleted in another window since the
    /// screen was drawn) is left out of the resolve, so Undo never brings back what the user
    /// had trashed on purpose.
    #[test]
    fn undo_never_restores_what_was_already_in_the_trash() {
        let db = library(&[row(1, 1000, H), row(2, 1000, H), row(3, 1000, H)]);
        db.with_conn(|c| Ok(c.execute("UPDATE assets SET state = 1, trashed_at = 5 WHERE id = 3", [])?)).unwrap();
        let undo = db.with_conn(|c| resolve(c, 1, &[2, 3, 99], 50)).unwrap();
        assert_eq!(undo.removed, vec![2], "only what this resolve moved");
        db.with_conn(|c| undo_resolve(c, &[undo])).unwrap();
        let state = |id: i64| db.with_conn(|c| Ok(c.query_row("SELECT state FROM assets WHERE id = ?1", [id], |r| r.get::<_, i64>(0))?)).unwrap();
        assert_eq!((state(2), state(3)), (0, 1), "2 is back, 3 stays in the Trash");
    }

    #[test]
    fn a_batch_with_a_refused_set_resolves_nothing() {
        let db = library(&[row(1, 1000, H), row(2, 1000, H), row(3, 5000, !H), row(4, 5000, !H)]);
        db.with_conn(|c| Ok(c.execute("UPDATE assets SET state = 1, trashed_at = 5 WHERE id = 3", [])?)).unwrap();
        let err = db.with_conn(|c| resolve_all(c, &[(1, vec![2]), (3, vec![4])], 50)).unwrap_err();
        assert!(matches!(err, crate::CoreError::Invalid(_)));
        let trashed: i64 = db.with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM assets WHERE state = 1", [], |r| r.get(0))?)).unwrap();
        assert_eq!(trashed, 1, "only the one that already was");
        let undos = db.with_conn(|c| resolve_all(c, &[(1, vec![2])], 50)).unwrap();
        assert_eq!(undos.len(), 1);
    }

    #[test]
    #[ignore]
    fn finds_sets_at_250k_in_under_a_second() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let tx = c.unchecked_transaction()?;
            let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
            let mut next = || {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x
            };
            let mut at = 1_500_000_000i64;
            let mut last = 0u64;
            for id in 1..=250_000i64 {
                // One photo in fifty is a copy of the one before, one in fifty a burst frame
                // a second later; the rest are minutes apart and unlike each other.
                let hash = match id % 50 {
                    0 => last,
                    1 => {
                        at += 1;
                        bits(last, 3)
                    }
                    _ => {
                        at += 60 + (next() % 600) as i64;
                        next()
                    }
                };
                last = hash;
                insert(&tx, &Row { id, at: Some(at), hash, w: 4032, h: 3024 });
            }
            tx.commit()?;
            Ok(())
        })
        .unwrap();
        let t = std::time::Instant::now();
        let n = sets(&db).len();
        let ms = t.elapsed().as_millis();
        println!("250k: {n} sets in {ms} ms");
        assert!(n >= 4000, "the planted copies and bursts are found: {n}");
        assert!(ms < 1000, "{ms} ms");
    }
}
