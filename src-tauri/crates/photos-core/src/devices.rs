use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{CoreError, Result};

/// Device kinds stored in `devices.kind`.
pub const KIND_UNKNOWN: i64 = 0;
pub const KIND_PHONE: i64 = 1;
pub const KIND_CAMERA: i64 = 2;

/// The `(make, model, body_serial)` row *is* the library's device section.
/// Returns `None` when there is nothing to identify the device by.
///
/// A missing serial means "unknown", not "a different camera". Not every format from one
/// body records it — a Nikon D7100 writes it into DNG but not into NEF — so matching on
/// the serial as if it were part of the identity listed the same camera twice, once per
/// file format. An absent serial therefore matches an existing body of the same make and
/// model, and a serial that turns up later is adopted by the row that had none.
pub fn upsert_device(
    conn: &Connection,
    make: Option<&str>,
    model: Option<&str>,
    serial: Option<&str>,
    now: i64,
) -> Result<Option<i64>> {
    let make = make.map(str::trim).filter(|s| !s.is_empty());
    let model = model.map(str::trim).filter(|s| !s.is_empty());
    let serial = serial.map(str::trim).filter(|s| !s.is_empty());
    if make.is_none() && model.is_none() {
        return Ok(None);
    }
    // Exact match, serial included: unambiguous.
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM devices WHERE make IS ?1 AND model IS ?2 AND body_serial IS ?3",
            params![make, model, serial],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        return Ok(Some(id));
    }
    match serial {
        // No serial to go on: join the oldest body of this make and model rather than
        // opening a second section for it.
        None => {
            let any: Option<i64> = conn
                .query_row(
                    "SELECT id FROM devices WHERE make IS ?1 AND model IS ?2 ORDER BY first_seen_at, id LIMIT 1",
                    params![make, model],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(id) = any {
                return Ok(Some(id));
            }
        }
        // A serial for a body first seen without one: adopt it, so the section keeps its
        // name, its photos and any rename the user has given it.
        Some(s) => {
            let unserialed: Option<i64> = conn
                .query_row(
                    "SELECT id FROM devices WHERE make IS ?1 AND model IS ?2 AND body_serial IS NULL ORDER BY first_seen_at, id LIMIT 1",
                    params![make, model],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(id) = unserialed {
                conn.execute("UPDATE devices SET body_serial = ?2 WHERE id = ?1", params![id, s])?;
                return Ok(Some(id));
            }
        }
    }
    let display_name = display_name(make, model);
    let kind = guess_kind(make, model);
    conn.execute(
        "INSERT INTO devices (make, model, body_serial, display_name, kind, first_seen_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![make, model, serial, display_name, kind, now],
    )?;
    Ok(Some(conn.last_insert_rowid()))
}

/// What kind of thing took the photos, for grouping devices in the sidebar: every iPhone
/// together, every camera together. Worked out from make and model, never stored, so a
/// better rule regroups old devices too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DeviceGroup {
    Iphone,
    Ipad,
    /// Android and Windows tablets, by model (Round 4H).
    Tablet,
    Phone,
    Camera,
    Drone,
    ActionCamera,
    Computer,
    Other,
}

/// Tablets by model: Samsung's SM-T/SM-X/SM-P series, Lenovo's TB- models, and anything
/// that calls itself a Tab, a Pad or a Surface. `text` is make and model, lowercased.
fn is_tablet(text: &str) -> bool {
    let words = ["sm-t", "sm-x", "sm-p", "tb-", "surface"];
    words.iter().any(|w| text.contains(w))
        || text.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| w == "tab" || w == "pad")
}

pub fn group_of(make: Option<&str>, model: Option<&str>, kind: i64) -> DeviceGroup {
    let make_l = make.unwrap_or("").to_ascii_lowercase();
    let text = format!("{make_l} {}", model.unwrap_or("")).to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| text.contains(w));
    if has(&["iphone"]) {
        DeviceGroup::Iphone
    } else if has(&["ipad"]) {
        DeviceGroup::Ipad
    } else if is_tablet(&text) {
        DeviceGroup::Tablet
    } else if kind == KIND_PHONE || has(&["pixel", "galaxy", "sm-", "oneplus", "xiaomi", "huawei", "motorola"]) {
        DeviceGroup::Phone
    } else if make_l.contains("dji") {
        DeviceGroup::Drone
    } else if has(&["gopro", "insta360"]) {
        DeviceGroup::ActionCamera
    } else if has(&["macbook", "imac", "mac mini", "screenshot", "windows"]) {
        DeviceGroup::Computer
    } else if make.is_some_and(|m| !m.trim().is_empty()) {
        DeviceGroup::Camera
    } else {
        DeviceGroup::Other
    }
}

/// A device section as the sidebar, Home and Settings show it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeviceSummary {
    #[ts(type = "number")]
    pub id: i64,
    pub display_name: String,
    /// `KIND_UNKNOWN` / `KIND_PHONE` / `KIND_CAMERA`
    #[ts(type = "number")]
    pub kind: i64,
    pub icon: Option<String>,
    #[ts(type = "number")]
    pub item_count: i64,
    #[ts(type = "number | null")]
    pub last_import_at: Option<i64>,
    pub hidden: bool,
    pub group: DeviceGroup,
}

/// Every device, most recently imported first. Counts exclude trashed items; devices
/// whose items were all deleted still appear (count 0) so they can be hidden.
pub fn list_devices(conn: &Connection) -> Result<Vec<DeviceSummary>> {
    // Only what the library shows counts: trashed photos and the movie halves of Live
    // Photos do not, and a device with nothing left has no section.
    let visible = format!("a.device_id = d.id AND a.state = 0 AND a.concealed = 0 AND NOT {}", crate::assets::MOVIE_HALF);
    let mut stmt = conn.prepare(&format!(
        "SELECT * FROM (
           SELECT d.id, d.display_name, d.kind, d.icon, d.hidden, d.make, d.model,
                  (SELECT COUNT(*) FROM assets a WHERE {visible}) AS n,
                  (SELECT MAX(a.imported_at) FROM assets a WHERE {visible}) AS last
           FROM devices d
         )
         WHERE n > 0
         ORDER BY last DESC NULLS LAST, display_name COLLATE NOCASE"
    ))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(DeviceSummary {
                id: r.get(0)?,
                display_name: r.get(1)?,
                kind: r.get(2)?,
                icon: r.get(3)?,
                hidden: r.get::<_, i64>(4)? != 0,
                group: group_of(r.get::<_, Option<String>>(5)?.as_deref(), r.get::<_, Option<String>>(6)?.as_deref(), r.get(2)?),
                item_count: r.get(7)?,
                last_import_at: r.get(8)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn rename_device(conn: &Connection, id: i64, name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CoreError::InvalidPath("device name is empty".into()));
    }
    let n = conn.execute("UPDATE devices SET display_name = ?2 WHERE id = ?1", params![id, name])?;
    if n == 0 {
        return Err(CoreError::NotFound(format!("device {id}")));
    }
    // The device's name is searchable, so every asset it took needs re-indexing.
    crate::fts::reindex_device(conn, id)?;
    Ok(())
}

pub fn set_device_hidden(conn: &Connection, id: i64, hidden: bool) -> Result<()> {
    let n = conn.execute("UPDATE devices SET hidden = ?2 WHERE id = ?1", params![id, hidden as i64])?;
    if n == 0 {
        return Err(CoreError::NotFound(format!("device {id}")));
    }
    Ok(())
}

/// The section name a device gets until the user renames it.
///
/// EXIF carries the maker's legal name, often in capitals ("NIKON CORPORATION", "OLYMPUS
/// IMAGING CORP."), and models frequently repeat the brand ("NIKON D7100"). Both are
/// cleaned up so the section reads like the product: "Nikon D7100".
pub fn display_name(make: Option<&str>, model: Option<&str>) -> String {
    let make = make.map(clean_make).filter(|m| !m.is_empty());
    let model = model.map(str::trim).filter(|m| !m.is_empty());
    match (make, model) {
        (Some(mk), Some(md)) => {
            // "NIKON D7100" under make "Nikon": the model already names the brand.
            let brand = mk.split_whitespace().next().unwrap_or(&mk).to_ascii_lowercase();
            let lower = md.to_ascii_lowercase();
            match lower.strip_prefix(&brand) {
                Some(rest) if rest.is_empty() || rest.starts_with(' ') => format!("{mk}{}", &md[brand.len()..]),
                _ => format!("{mk} {md}"),
            }
        }
        (Some(mk), None) => mk,
        (None, Some(md)) => md.to_string(),
        (None, None) => "Unknown device".to_string(),
    }
}

/// "NIKON CORPORATION" → "Nikon", "RICOH IMAGING COMPANY, LTD." → "Ricoh", "SONY" → "Sony".
/// Short capitalised names are acronyms ("DJI", "LG") and stay as they are.
fn clean_make(make: &str) -> String {
    const NOISE: &[&str] = &["corporation", "corp", "co", "ltd", "inc", "company", "imaging", "camera", "ag", "gmbh", "electronics"];
    let words: Vec<&str> = make
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|w| !w.is_empty())
        .collect();
    let keep: Vec<&str> = words
        .iter()
        .enumerate()
        .filter(|(i, w)| *i == 0 || !NOISE.contains(&w.trim_end_matches('.').to_ascii_lowercase().as_str()))
        .map(|(_, w)| *w)
        .collect();
    keep.iter()
        .map(|w| {
            let letters = w.chars().filter(|c| c.is_alphabetic()).count();
            let shouting = w.chars().all(|c| !c.is_lowercase());
            let quiet = w.chars().all(|c| !c.is_uppercase());
            if (shouting && letters > 3) || quiet {
                let mut cs = w.chars();
                cs.next().map(|f| f.to_uppercase().chain(cs.flat_map(char::to_lowercase)).collect()).unwrap_or_default()
            } else {
                w.to_string()
            }
        })
        .collect::<Vec<String>>()
        .join(" ")
}

/// The name `display_name` produced before it cleaned up makes. A row still carrying it
/// was never renamed by the user, so it is safe to replace.
fn legacy_display_name(make: Option<&str>, model: Option<&str>) -> String {
    match (make, model) {
        (Some(mk), Some(md)) if md.to_ascii_lowercase().starts_with(&mk.to_ascii_lowercase()) => md.to_string(),
        (Some(mk), Some(md)) => format!("{mk} {md}"),
        (Some(mk), None) => mk.to_string(),
        (None, Some(md)) => md.to_string(),
        (None, None) => "Unknown device".to_string(),
    }
}

/// Rename devices still carrying the old default name to the current one, re-indexing
/// their photos for search. Names the user chose are left alone. Returns how many changed.
pub fn refresh_default_names(conn: &Connection) -> Result<usize> {
    let rows: Vec<(i64, Option<String>, Option<String>, String)> = conn
        .prepare("SELECT id, make, model, display_name FROM devices")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let mut n = 0;
    for (id, make, model, current) in rows {
        let (make, model) = (make.as_deref(), model.as_deref());
        let wanted = display_name(make, model);
        if current != wanted && current == legacy_display_name(make, model) {
            conn.execute("UPDATE devices SET display_name = ?2 WHERE id = ?1", params![id, wanted])?;
            crate::fts::reindex_device(conn, id)?;
            n += 1;
        }
    }
    Ok(n)
}

fn guess_kind(make: Option<&str>, model: Option<&str>) -> i64 {
    let text = format!("{} {}", make.unwrap_or(""), model.unwrap_or("")).to_ascii_lowercase();
    if text.contains("iphone") || text.contains("pixel") || text.contains("galaxy") || text.contains("samsung") {
        KIND_PHONE
    } else if make.is_some() {
        KIND_CAMERA
    } else {
        KIND_UNKNOWN
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;

    #[test]
    fn devices_fall_into_groups_by_make_and_model() {
        use DeviceGroup::*;
        let g = |make: &str, model: &str| group_of(Some(make), Some(model), guess_kind(Some(make), Some(model)));
        assert_eq!(g("Apple", "iPhone 13 Pro"), Iphone);
        assert_eq!(g("Apple", "iPad Pro (11-inch)"), Ipad);
        assert_eq!(g("samsung", "SM-S918B"), Phone);
        assert_eq!(g("Google", "Pixel 8"), Phone);
        assert_eq!(g("NIKON CORPORATION", "NIKON Z 6_2"), Camera);
        assert_eq!(g("Canon", "Canon EOS R6"), Camera);
        assert_eq!(g("SONY", "ILCE-7M4"), Camera);
        assert_eq!(g("DJI", "FC3582"), Drone);
        assert_eq!(g("GoPro", "HERO11 Black"), ActionCamera);
        assert_eq!(g("Apple", "MacBook Pro"), Computer);
        assert_eq!(group_of(None, None, KIND_UNKNOWN), Other);
        assert_eq!(group_of(None, Some("iPhone 15"), KIND_PHONE), Iphone);
    }

    #[test]
    fn tablets_are_tablets() {
        use DeviceGroup::*;
        let g = |make: &str, model: &str| group_of(Some(make), Some(model), guess_kind(Some(make), Some(model)));
        assert_eq!(g("Apple", "iPad Air"), Ipad);
        assert_eq!(g("samsung", "SM-X710"), Tablet); // Galaxy Tab S9
        assert_eq!(g("samsung", "SM-T510"), Tablet); // Galaxy Tab A
        assert_eq!(g("samsung", "SM-P610"), Tablet); // Galaxy Tab S6 Lite
        assert_eq!(g("samsung", "Galaxy Tab S8"), Tablet);
        assert_eq!(g("LENOVO", "Lenovo TB-X606F"), Tablet);
        assert_eq!(g("Xiaomi", "Xiaomi Pad 6"), Tablet);
        assert_eq!(g("Microsoft", "Surface Pro 9"), Tablet);
        // Known miss: a Xiaomi Pad 6 that writes only its code name looks like a phone.
        assert_eq!(g("Xiaomi", "23043RP34G"), Phone);
        // Phones whose names are near a tablet's stay phones.
        assert_eq!(g("samsung", "SM-S918B"), Phone);
        assert_eq!(g("samsung", "SM-A546B"), Phone);
    }

    #[test]
    fn the_summary_carries_the_group() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let id = upsert_device(c, Some("Apple"), Some("iPad Air"), None, 1)?.unwrap();
            c.execute(
                "INSERT INTO assets (content_hash, relative_path, filename, extension, media_type, byte_size, imported_at, device_id)
                 VALUES ('h', 'p', 'f.jpg', 'jpg', 0, 1, 1, ?1)",
                [id],
            )?;
            let list = list_devices(c)?;
            assert_eq!(list[0].group, DeviceGroup::Ipad);
            Ok(())
        })
        .unwrap();
    }

    /// A Nikon D7100 writes its body serial into DNG but not into NEF. Treating the
    /// serial as part of the identity listed the same camera twice, one section per file
    /// format, which is what happened to a real library.
    #[test]
    fn one_body_stays_one_section_when_only_some_files_carry_the_serial() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            // NEF first: no serial recorded.
            let from_nef = upsert_device(c, Some("NIKON CORPORATION"), Some("NIKON D7100"), None, 1)?.unwrap();
            // Then DNG from the same body, which does carry one.
            let from_dng = upsert_device(c, Some("NIKON CORPORATION"), Some("NIKON D7100"), Some("4541648"), 2)?.unwrap();
            assert_eq!(from_nef, from_dng, "the same camera must not open a second section");

            // The serial is adopted, so it is known from here on.
            let serial: Option<String> =
                c.query_row("SELECT body_serial FROM devices WHERE id = ?1", [from_nef], |r| r.get(0))?;
            assert_eq!(serial.as_deref(), Some("4541648"));

            // And a later NEF, still serial-less, lands on the same row.
            let later_nef = upsert_device(c, Some("NIKON CORPORATION"), Some("NIKON D7100"), None, 3)?.unwrap();
            assert_eq!(later_nef, from_nef);

            let n: i64 = c.query_row("SELECT COUNT(*) FROM devices", [], |r| r.get(0))?;
            assert_eq!(n, 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn two_bodies_of_the_same_model_stay_apart() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let a = upsert_device(c, Some("NIKON CORPORATION"), Some("NIKON D7100"), Some("111"), 1)?.unwrap();
            let b = upsert_device(c, Some("NIKON CORPORATION"), Some("NIKON D7100"), Some("222"), 2)?.unwrap();
            assert_ne!(a, b, "different serials are different cameras");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn upsert_is_idempotent_and_names_devices() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let a = upsert_device(c, Some("Apple"), Some("iPhone 17 Pro"), Some("S1"), 1)?.unwrap();
            let b = upsert_device(c, Some("Apple"), Some("iPhone 17 Pro"), Some("S1"), 2)?.unwrap();
            assert_eq!(a, b);
            let (name, kind): (String, i64) =
                c.query_row("SELECT display_name, kind FROM devices WHERE id = ?1", [a], |r| Ok((r.get(0)?, r.get(1)?)))?;
            assert_eq!((name.as_str(), kind), ("Apple iPhone 17 Pro", KIND_PHONE));
            let sony = upsert_device(c, Some("SONY"), Some("ILCE-7M4"), None, 3)?.unwrap();
            assert_ne!(sony, a);
            let canon = upsert_device(c, Some("Canon"), Some("Canon EOS R6"), None, 3)?.unwrap();
            let name: String = c.query_row("SELECT display_name FROM devices WHERE id = ?1", [canon], |r| r.get(0))?;
            assert_eq!(name, "Canon EOS R6", "no doubled make");
            assert_eq!(upsert_device(c, None, None, Some("x"), 4)?, None);
            Ok(())
        })
        .unwrap();
    }

    /// Makers write their legal name into EXIF, often in capitals, and often repeat it
    /// in the model. A section called "NIKON CORPORATION NIKON D7100" is what a real
    /// library showed.
    #[test]
    fn names_read_like_the_product_not_the_exif_field() {
        let cases = [
            (Some("NIKON CORPORATION"), Some("NIKON D7100"), "Nikon D7100"),
            (Some("SONY"), Some("ILCE-7M4"), "Sony ILCE-7M4"),
            (Some("Canon"), Some("Canon EOS R6"), "Canon EOS R6"),
            (Some("OLYMPUS IMAGING CORP."), Some("E-M5MarkII"), "Olympus E-M5MarkII"),
            (Some("RICOH IMAGING COMPANY, LTD."), Some("PENTAX K-1"), "Ricoh PENTAX K-1"),
            (Some("FUJIFILM"), Some("X-T5"), "Fujifilm X-T5"),
            (Some("Apple"), Some("iPhone 12"), "Apple iPhone 12"),
            (Some("DJI"), Some("FC3582"), "DJI FC3582"),
            (Some("LEICA CAMERA AG"), Some("LEICA Q2"), "Leica Q2"),
            (Some("samsung"), Some("SM-S918B"), "Samsung SM-S918B"),
            (None, Some("NIKON D7100"), "NIKON D7100"),
            (Some("NIKON CORPORATION"), None, "Nikon"),
        ];
        for (make, model, want) in cases {
            assert_eq!(display_name(make, model), want, "{make:?} {model:?}");
        }
    }

    /// Rows named by the old rule are renamed; a name the user chose is left alone.
    #[test]
    fn refreshes_default_names_but_keeps_renames() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            c.execute_batch(
                "INSERT INTO devices (make, model, display_name, kind, first_seen_at) VALUES
                   ('NIKON CORPORATION', 'NIKON D7100', 'NIKON CORPORATION NIKON D7100', 2, 1),
                   ('SONY', 'ILCE-7M4', 'Studio body', 2, 1);",
            )?;
            assert_eq!(refresh_default_names(c)?, 1);
            let names: Vec<String> = c
                .prepare("SELECT display_name FROM devices ORDER BY id")?
                .query_map([], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?;
            assert_eq!(names, ["Nikon D7100", "Studio body"]);
            assert_eq!(refresh_default_names(c)?, 0, "idempotent");
            Ok(())
        })
        .unwrap();
    }

    /// A device section is a view of the photos in the library. Once they are all in the
    /// Trash — or all that is left is the hidden movie half of a Live Photo — there is
    /// nothing to show, so the section goes too; restoring brings it back.
    #[test]
    fn devices_with_nothing_active_are_not_listed() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let phone = upsert_device(c, Some("Apple"), Some("iPhone 12"), None, 1)?.unwrap();
            let cam = upsert_device(c, Some("NIKON CORPORATION"), Some("NIKON D7100"), None, 1)?.unwrap();
            let add = |dev: i64, name: &str, media_type: i64, state: i64, group: Option<&str>| {
                c.execute(
                    "INSERT INTO assets (content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, device_id, state, live_photo_group)
                     VALUES (?1, ?1, ?1, 'x', ?2, 1, 1, 1, ?3, ?4, ?5)",
                    params![name, media_type, dev, state, group],
                )
                .unwrap();
            };
            add(cam, "trashed.nef", 0, 1, None);
            add(phone, "live.heic", crate::assets::MEDIA_TYPE_LIVE, 1, Some("live"));
            add(phone, "live.mov", 1, 0, Some("live")); // a stray active movie half
            assert!(list_devices(c)?.is_empty(), "{:?}", list_devices(c)?);

            add(phone, "kept.heic", 0, 0, None);
            let listed = list_devices(c)?;
            assert_eq!(listed.len(), 1);
            assert_eq!((listed[0].id, listed[0].item_count), (phone, 1), "the movie half is not an item");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn lists_counts_renames_and_hides() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let phone = upsert_device(c, Some("Apple"), Some("iPhone 17 Pro"), None, 1)?.unwrap();
            let cam = upsert_device(c, Some("SONY"), Some("ILCE-7M4"), None, 1)?.unwrap();
            for (i, (dev, state, at)) in [(phone, 0, 10), (phone, 1, 50), (cam, 0, 30), (cam, 0, 40)].iter().enumerate() {
                c.execute(
                    "INSERT INTO assets (content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, device_id, state)
                     VALUES (?1, ?2, ?3, 'jpg', 0, 1, 1, ?4, ?5, ?6)",
                    params![format!("h{i}"), format!("p{i}"), format!("f{i}"), at, dev, state],
                )?;
            }
            let list = list_devices(c)?;
            assert_eq!(list.len(), 2);
            assert_eq!((list[0].id, list[0].item_count, list[0].last_import_at), (cam, 2, Some(40)), "most recent import first");
            assert_eq!((list[1].id, list[1].item_count), (phone, 1), "trashed items do not count");
            rename_device(c, phone, "  Julian's iPhone ")?;
            set_device_hidden(c, cam, true)?;
            let list = list_devices(c)?;
            assert_eq!(list[1].display_name, "Julian's iPhone");
            assert!(list[0].hidden);
            assert!(rename_device(c, phone, " ").is_err());
            assert!(matches!(set_device_hidden(c, 999, true), Err(CoreError::NotFound(_))));
            Ok(())
        })
        .unwrap();
    }
}
