//! What Lumen needs from an Apple Photos library's Core Data store (`Photos.sqlite`).
//!
//! Core Data numbers its entities per schema version, and the numbers leak into table and
//! column names (`Z_28ASSETS.Z_3ASSETS` on one macOS, something else on the next), so they
//! are read from `Z_PRIMARYKEY` rather than written down. Any piece this library's schema
//! lacks (keywords, captions, albums) comes back empty instead of failing the import.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension};

use crate::ImportError;

/// Seconds between 1970-01-01 and Core Data's epoch, 2001-01-01.
const CORE_DATA_EPOCH: i64 = 978_307_200;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AppleAsset {
    pub uuid: String,
    /// One hex character for a file inside the library; a path for a referenced file.
    pub directory: String,
    pub filename: String,
    pub original_filename: Option<String>,
    pub favorite: bool,
    pub hidden: bool,
    pub trashed: bool,
    /// False for burst frames the user did not pick.
    pub visible: bool,
    pub video: bool,
    /// UTC seconds.
    pub created: Option<i64>,
    pub tz_offset_minutes: Option<i32>,
    pub location: Option<(f64, f64)>,
    pub title: Option<String>,
    pub caption: Option<String>,
    pub keywords: Vec<String>,
    pub albums: Vec<String>,
}

impl AppleAsset {
    /// Kept outside the library ("referenced"): `directory` is a path, not a letter.
    pub fn referenced(&self) -> bool {
        self.directory.len() > 1
    }
}

fn db_err(e: rusqlite::Error) -> ImportError {
    ImportError::Source(format!("Photos.sqlite: {e}"))
}

fn table_exists(c: &Connection, name: &str) -> bool {
    c.query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1", [name], |_| Ok(()))
        .optional()
        .ok()
        .flatten()
        .is_some()
}

fn columns(c: &Connection, table: &str) -> Vec<String> {
    let Ok(mut stmt) = c.prepare(&format!("PRAGMA table_info({table})")) else { return Vec::new() };
    stmt.query_map([], |r| r.get::<_, String>(1)).map(|rows| rows.flatten().collect()).unwrap_or_default()
}

fn entity(c: &Connection, name: &str) -> Option<i64> {
    c.query_row("SELECT Z_ENT FROM Z_PRIMARYKEY WHERE Z_NAME = ?1", [name], |r| r.get(0)).optional().ok().flatten()
}

/// `SELECT <key>, <text>` into lists per key, sorted; empty when the query cannot run on
/// this schema.
fn grouped(c: &Connection, sql: &str) -> HashMap<i64, Vec<String>> {
    let mut out: HashMap<i64, Vec<String>> = HashMap::new();
    let Ok(mut stmt) = c.prepare(sql) else { return out };
    if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))) {
        for (k, v) in rows.flatten() {
            out.entry(k).or_default().push(v);
        }
    }
    for v in out.values_mut() {
        v.sort();
        v.dedup();
    }
    out
}

pub fn read_library(c: &Connection) -> Result<Vec<AppleAsset>, ImportError> {
    let assets_table = if table_exists(c, "ZASSET") { "ZASSET" } else { "ZGENERICASSET" };
    let asset_cols = columns(c, assets_table);
    if asset_cols.is_empty() {
        return Err(ImportError::Source("Photos.sqlite has no asset table; is this a Photos library from macOS 10.15 or later?".into()));
    }
    // `a.<name>` when this schema has the column, else a literal standing in for it.
    let col = |name: &str, fallback: &str| if asset_cols.iter().any(|c| c == name) { format!("a.{name}") } else { fallback.to_string() };

    // Albums and keywords in two passes, keyed by the asset's Z_PK (and the attributes' Z_PK).
    let (asset_ent, album_ent, aaa_ent, keyword_ent) =
        (entity(c, "Asset"), entity(c, "Album"), entity(c, "AdditionalAssetAttributes"), entity(c, "Keyword"));
    let albums = match (asset_ent, album_ent) {
        (Some(asset), Some(album)) => grouped(
            c,
            &format!(
                "SELECT j.Z_{asset}ASSETS, g.ZTITLE FROM Z_{album}ASSETS j JOIN ZGENERICALBUM g ON g.Z_PK = j.Z_{album}ALBUMS
                 WHERE g.ZKIND = 2 AND COALESCE(g.ZTRASHEDSTATE, 0) = 0 AND g.ZTITLE IS NOT NULL AND g.ZTITLE <> ''"
            ),
        ),
        _ => HashMap::new(),
    };
    let keywords = match (aaa_ent, keyword_ent) {
        (Some(aaa), Some(kw)) => grouped(
            c,
            &format!("SELECT j.Z_{aaa}ASSETATTRIBUTES, k.ZTITLE FROM Z_{aaa}KEYWORDS j JOIN ZKEYWORD k ON k.Z_PK = j.Z_{kw}KEYWORDS WHERE k.ZTITLE IS NOT NULL"),
        ),
        _ => HashMap::new(),
    };
    let has_aaa = table_exists(c, "ZADDITIONALASSETATTRIBUTES");
    let aaa_cols = if has_aaa { columns(c, "ZADDITIONALASSETATTRIBUTES") } else { Vec::new() };
    let aaa = |name: &str| if aaa_cols.iter().any(|c| c == name) { format!("x.{name}") } else { "NULL".into() };
    let caption = if table_exists(c, "ZASSETDESCRIPTION") {
        "(SELECT d.ZLONGDESCRIPTION FROM ZASSETDESCRIPTION d WHERE d.ZASSETATTRIBUTES = x.Z_PK LIMIT 1)".to_string()
    } else {
        "NULL".to_string()
    };
    let join = if has_aaa { "LEFT JOIN ZADDITIONALASSETATTRIBUTES x ON x.ZASSET = a.Z_PK" } else { "" };
    let (x_pk, original, title, tz) = if has_aaa {
        ("x.Z_PK".to_string(), aaa("ZORIGINALFILENAME"), aaa("ZTITLE"), aaa("ZTIMEZONEOFFSET"))
    } else {
        ("NULL".to_string(), "NULL".to_string(), "NULL".to_string(), "NULL".to_string())
    };
    let caption = if has_aaa { caption } else { "NULL".into() };
    let sql = format!(
        "SELECT a.Z_PK, a.ZUUID, a.ZDIRECTORY, a.ZFILENAME, {fav}, {hidden}, {trashed}, {visibility}, a.ZKIND,
                {created}, {lat}, {lon}, {x_pk}, {original}, {title}, {tz}, {caption}
         FROM {assets_table} a {join} ORDER BY a.Z_PK",
        fav = col("ZFAVORITE", "0"),
        hidden = col("ZHIDDEN", "0"),
        trashed = col("ZTRASHEDSTATE", "0"),
        visibility = col("ZVISIBILITYSTATE", "0"),
        created = col("ZDATECREATED", "NULL"),
        lat = col("ZLATITUDE", "NULL"),
        lon = col("ZLONGITUDE", "NULL"),
    );
    let mut stmt = c.prepare(&sql).map_err(db_err)?;
    let rows = stmt
        .query_map([], |r| {
            let pk: i64 = r.get(0)?;
            let x_pk: Option<i64> = r.get(12)?;
            let lat: Option<f64> = r.get(10)?;
            let lon: Option<f64> = r.get(11)?;
            Ok(AppleAsset {
                uuid: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                directory: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                filename: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                favorite: r.get::<_, Option<i64>>(4)?.unwrap_or(0) != 0,
                hidden: r.get::<_, Option<i64>>(5)?.unwrap_or(0) != 0,
                trashed: r.get::<_, Option<i64>>(6)?.unwrap_or(0) != 0,
                visible: r.get::<_, Option<i64>>(7)?.unwrap_or(0) == 0,
                video: r.get::<_, Option<i64>>(8)?.unwrap_or(0) == 1,
                created: r.get::<_, Option<f64>>(9)?.map(|t| t as i64 + CORE_DATA_EPOCH),
                // Photos writes -180 for "no location", which the latitude range rules out;
                // 0,0 is as good as none for a photo.
                location: match (lat, lon) {
                    (Some(la), Some(lo)) if (-90.0..=90.0).contains(&la) && (-180.0..=180.0).contains(&lo) && (la, lo) != (0.0, 0.0) => Some((la, lo)),
                    _ => None,
                },
                original_filename: r.get::<_, Option<String>>(13)?.filter(|s| !s.is_empty()),
                title: r.get::<_, Option<String>>(14)?.filter(|s| !s.trim().is_empty()),
                tz_offset_minutes: r.get::<_, Option<i64>>(15)?.map(|s| (s / 60) as i32),
                caption: r.get::<_, Option<String>>(16)?.filter(|s| !s.trim().is_empty()),
                keywords: x_pk.and_then(|k| keywords.get(&k).cloned()).unwrap_or_default(),
                albums: albums.get(&pk).cloned().unwrap_or_default(),
            })
        })
        .map_err(db_err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(db_err)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A Photos 5+ schema in miniature. Entity numbers deliberately differ from a real
    /// library's (Asset 3, Album 28, AdditionalAssetAttributes 1, Keyword 40), so reading
    /// them from `Z_PRIMARYKEY` is what makes this pass.
    pub(crate) const FIXTURE: &str = r#"
        CREATE TABLE Z_PRIMARYKEY (Z_ENT INTEGER, Z_NAME TEXT);
        INSERT INTO Z_PRIMARYKEY VALUES (5,'Asset'),(31,'Album'),(2,'AdditionalAssetAttributes'),(44,'Keyword');
        CREATE TABLE ZASSET (Z_PK INTEGER PRIMARY KEY, ZUUID TEXT, ZDIRECTORY TEXT, ZFILENAME TEXT, ZFAVORITE INTEGER,
            ZHIDDEN INTEGER, ZTRASHEDSTATE INTEGER, ZVISIBILITYSTATE INTEGER, ZKIND INTEGER, ZDATECREATED REAL,
            ZLATITUDE REAL, ZLONGITUDE REAL);
        INSERT INTO ZASSET VALUES
            (1,'AAA','A','AAA.heic',1,0,0,0,0, 721000000.0, 64.04,-16.18),
            (2,'BBB','B','BBB.mov',0,1,0,0,1, 721000100.0, -180.0,-180.0),
            (3,'CCC','C','CCC.jpeg',0,0,1,0,0, NULL, -180.0,-180.0),
            (4,'DDD','D','DDD.jpeg',0,0,0,2,0, NULL, -180.0,-180.0),
            (5,'EEE','/Volumes/Old/Pictures/2019','IMG_9.JPG',0,0,0,0,0, NULL, -180.0,-180.0),
            (6,'FFF','F','FFF.jpeg',0,0,0,0,0, 700000000.0, -180.0,-180.0);
        CREATE TABLE ZADDITIONALASSETATTRIBUTES (Z_PK INTEGER PRIMARY KEY, ZASSET INTEGER, ZORIGINALFILENAME TEXT,
            ZTITLE TEXT, ZTIMEZONEOFFSET INTEGER);
        INSERT INTO ZADDITIONALASSETATTRIBUTES VALUES (10,1,'IMG_0001.HEIC','Skógafoss',7200),(11,2,'IMG_0002.MOV',NULL,NULL);
        CREATE TABLE ZASSETDESCRIPTION (Z_PK INTEGER PRIMARY KEY, ZASSETATTRIBUTES INTEGER, ZLONGDESCRIPTION TEXT);
        INSERT INTO ZASSETDESCRIPTION VALUES (1,10,'Rainbow over the falls');
        CREATE TABLE ZKEYWORD (Z_PK INTEGER PRIMARY KEY, ZTITLE TEXT);
        INSERT INTO ZKEYWORD VALUES (7,'waterfall'),(8,'iceland');
        CREATE TABLE Z_2KEYWORDS (Z_2ASSETATTRIBUTES INTEGER, Z_44KEYWORDS INTEGER);
        INSERT INTO Z_2KEYWORDS VALUES (10,7),(10,8);
        CREATE TABLE ZGENERICALBUM (Z_PK INTEGER PRIMARY KEY, ZKIND INTEGER, ZTITLE TEXT, ZTRASHEDSTATE INTEGER);
        INSERT INTO ZGENERICALBUM VALUES (20,2,'Iceland',0),(21,2,'Deleted album',1),(22,1505,'Smart',0);
        CREATE TABLE Z_31ASSETS (Z_31ALBUMS INTEGER, Z_5ASSETS INTEGER);
        INSERT INTO Z_31ASSETS VALUES (20,1),(21,1),(22,1),(20,2);
    "#;

    #[test]
    fn reads_what_lumen_needs_from_any_schema_numbering() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(FIXTURE).unwrap();
        let assets = read_library(&c).unwrap();
        assert_eq!(assets.len(), 6);
        let a = &assets[0];
        assert_eq!(a.original_filename.as_deref(), Some("IMG_0001.HEIC"));
        assert!(a.favorite && a.visible && !a.video && !a.referenced());
        assert_eq!(a.created, Some(721000000 + CORE_DATA_EPOCH));
        assert_eq!(a.tz_offset_minutes, Some(120));
        assert_eq!(a.location, Some((64.04, -16.18)));
        assert_eq!(a.title.as_deref(), Some("Skógafoss"));
        assert_eq!(a.caption.as_deref(), Some("Rainbow over the falls"));
        assert_eq!(a.keywords, ["iceland", "waterfall"]);
        assert_eq!(a.albums, ["Iceland"], "trashed and smart albums are not albums");
        assert!(assets[1].hidden && assets[1].video && assets[1].location.is_none());
        assert_eq!(assets[1].albums, ["Iceland"]);
        assert!(assets[2].trashed);
        assert!(!assets[3].visible);
        assert!(assets[4].referenced());
        assert_eq!(assets[5].original_filename, None);
    }

    #[test]
    fn a_library_without_the_optional_tables_still_reads() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE ZGENERICASSET (Z_PK INTEGER PRIMARY KEY, ZUUID TEXT, ZDIRECTORY TEXT, ZFILENAME TEXT, ZKIND INTEGER);
             INSERT INTO ZGENERICASSET VALUES (1,'AAA','A','AAA.jpeg',0);",
        )
        .unwrap();
        let assets = read_library(&c).unwrap();
        assert_eq!(assets.len(), 1);
        assert!(assets[0].visible && !assets[0].favorite && assets[0].albums.is_empty());
    }

    #[test]
    fn something_else_is_refused() {
        let c = Connection::open_in_memory().unwrap();
        assert!(read_library(&c).is_err());
    }
}
