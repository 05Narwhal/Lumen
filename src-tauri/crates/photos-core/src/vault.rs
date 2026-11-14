//! The Hidden folder (round 3): photos moved here appear nowhere else in Lumen, and the
//! folder opens with a password. The password is stored as an Argon2id PHC string, never in
//! clear. It keeps the photos out of sight in the app; the files themselves stay ordinary
//! files in the library folder.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{CoreError, Result};

/// The shortest password accepted.
pub const MIN_PASSWORD: usize = 4;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct VaultStatus {
    /// A password has been chosen.
    pub has_password: bool,
    /// How many items are in the folder.
    #[ts(type = "number")]
    pub count: i64,
}

pub fn status(conn: &Connection) -> Result<VaultStatus> {
    let has_password = conn.query_row("SELECT COUNT(*) FROM vault", [], |r| r.get::<_, i64>(0))? > 0;
    let count = conn.query_row(
        &format!("SELECT COUNT(*) FROM assets a WHERE a.in_vault = 1 AND a.unreadable = 0 AND a.state = 0 AND NOT {}", crate::assets::MOVIE_HALF),
        [],
        |r| r.get(0),
    )?;
    Ok(VaultStatus { has_password, count })
}

fn hash(password: &str) -> Result<String> {
    use argon2::password_hash::{PasswordHasher, SaltString};
    if password.chars().count() < MIN_PASSWORD {
        return Err(CoreError::Invalid(format!("the password needs at least {MIN_PASSWORD} characters")));
    }
    // Sixteen random bytes from a v4 UUID (the OS generator), so no second RNG is needed.
    let salt = SaltString::encode_b64(uuid::Uuid::new_v4().as_bytes()).map_err(|e| CoreError::Invalid(e.to_string()))?;
    let phc = argon2::Argon2::default().hash_password(password.as_bytes(), &salt).map_err(|e| CoreError::Invalid(e.to_string()))?;
    Ok(phc.to_string())
}

/// Sets (or replaces) the password.
pub fn set_password(conn: &Connection, password: &str, now: i64) -> Result<()> {
    let phc = hash(password)?;
    conn.execute(
        "INSERT INTO vault (id, phc, created_at) VALUES (1, ?1, ?2) ON CONFLICT(id) DO UPDATE SET phc = excluded.phc",
        params![phc, now],
    )?;
    Ok(())
}

/// The forgotten-password path: a new password, with the moment recorded.
pub fn reset_password(conn: &Connection, password: &str, now: i64) -> Result<()> {
    set_password(conn, password, now)?;
    conn.execute("UPDATE vault SET reset_at = ?1 WHERE id = 1", [now])?;
    Ok(())
}

pub fn verify(conn: &Connection, password: &str) -> Result<bool> {
    use argon2::password_hash::{PasswordHash, PasswordVerifier};
    let Some(phc) = conn.query_row("SELECT phc FROM vault WHERE id = 1", [], |r| r.get::<_, String>(0)).optional()? else {
        return Ok(false);
    };
    let Ok(parsed) = PasswordHash::new(&phc) else { return Ok(false) };
    Ok(argon2::Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok())
}

fn set_in_vault(conn: &Connection, ids: &[i64], value: bool) -> Result<Vec<i64>> {
    let all = crate::assets::with_movie_halves(conn, ids)?;
    let tx = conn.unchecked_transaction()?;
    {
        // Row by row, so the trigger recomputes `concealed` for each.
        let mut stmt = tx.prepare_cached("UPDATE assets SET in_vault = ?2 WHERE id = ?1")?;
        for &id in &all {
            stmt.execute(params![id, value as i64])?;
        }
    }
    tx.commit()?;
    Ok(all)
}

/// Moves photos into the folder (a Live Photo's movie half goes with its still). Returns
/// every id moved, for *Undo*.
pub fn move_in(conn: &Connection, ids: &[i64]) -> Result<Vec<i64>> {
    set_in_vault(conn, ids, true)
}

/// Moves photos back into the library. A photo also in a hiding album stays hidden.
pub fn move_out(conn: &Connection, ids: &[i64]) -> Result<Vec<i64>> {
    set_in_vault(conn, ids, false)
}

/// Whether `id` is in the folder (the protocol refuses its bytes while it is locked).
pub fn contains(conn: &Connection, id: i64) -> Result<bool> {
    Ok(conn.query_row("SELECT in_vault FROM assets WHERE id = ?1", [id], |r| r.get::<_, i64>(0)).optional()?.is_some_and(|v| v != 0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::{insert_assets, NewAsset};
    use crate::Db;

    fn photo(hash: &str, media_type: i64) -> NewAsset {
        NewAsset {
            content_hash: hash.into(),
            relative_path: format!("x/{hash}"),
            filename: hash.into(),
            extension: "heic".into(),
            media_type,
            byte_size: 1,
            imported_at: 1,
            ..Default::default()
        }
    }

    #[test]
    fn a_password_is_checked_and_never_stored() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            assert!(!status(c)?.has_password);
            assert!(!verify(c, "anything")?, "no password yet opens nothing");
            assert!(matches!(set_password(c, "abc", 1), Err(CoreError::Invalid(_))), "too short");
            set_password(c, "hunter22", 1)?;
            assert!(status(c)?.has_password);
            assert!(verify(c, "hunter22")?);
            assert!(!verify(c, "hunter23")?);
            let stored: String = c.query_row("SELECT phc FROM vault", [], |r| r.get(0))?;
            assert!(stored.starts_with("$argon2id$") && !stored.contains("hunter22"));
            reset_password(c, "new-one", 2)?;
            assert!(verify(c, "new-one")? && !verify(c, "hunter22")?);
            let reset: Option<i64> = c.query_row("SELECT reset_at FROM vault", [], |r| r.get(0))?;
            assert_eq!(reset, Some(2));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn moving_in_and_out_hides_and_shows_with_the_live_photo_half() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let ids = insert_assets(c, &[photo("still", 0), photo("mov", 1), photo("other", 0)])?;
            c.execute("UPDATE assets SET media_type = 2, live_photo_group = 'IMG_1' WHERE id = ?1", [ids[0]])?;
            c.execute("UPDATE assets SET live_photo_group = 'IMG_1' WHERE id = ?1", [ids[1]])?;
            let moved = move_in(c, &[ids[0]])?;
            assert_eq!(moved.len(), 2, "the movie half goes with its still");
            assert!(contains(c, ids[0])? && contains(c, ids[1])? && !contains(c, ids[2])?);
            let concealed: i64 = c.query_row("SELECT concealed FROM assets WHERE id = ?1", [ids[0]], |r| r.get(0))?;
            assert_eq!(concealed, 1);
            assert_eq!(status(c)?.count, 1, "one item the user sees");
            // Out again; in a hiding album it stays hidden.
            let album = crate::albums::create_album(c, "School", None, 1)?;
            crate::albums::add_to_album(c, album, &[ids[0]], 1)?;
            crate::concealed::set_album_hides(c, album, true, 1)?;
            move_out(c, &moved)?;
            assert!(!contains(c, ids[0])?);
            let concealed: i64 = c.query_row("SELECT concealed FROM assets WHERE id = ?1", [ids[0]], |r| r.get(0))?;
            assert_eq!(concealed, 1, "still in a hiding album");
            Ok(())
        })
        .unwrap();
    }
}
