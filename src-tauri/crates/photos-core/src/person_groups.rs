//! Manual groups of people (Family, Friends, …). A person may be in any number of groups.
//! Groups list only people still on `/people`, so a merged or rejected person drops out
//! without a cleanup pass.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use ts_rs::TS;

use crate::{CoreError, Result};

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PersonGroup {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    /// Members still listed on `/people`.
    #[ts(type = "number[]")]
    pub person_ids: Vec<i64>,
}

fn clean(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CoreError::Invalid("A group needs a name.".into()));
    }
    Ok(name)
}

fn taken(conn: &Connection, name: &str, except: i64) -> Result<bool> {
    Ok(conn
        .query_row("SELECT id FROM person_groups WHERE name = ?1 AND id != ?2", params![name, except], |r| r.get::<_, i64>(0))
        .optional()?
        .is_some())
}

pub fn list(conn: &Connection) -> Result<Vec<PersonGroup>> {
    let mut groups: Vec<PersonGroup> = conn
        .prepare("SELECT id, name FROM person_groups ORDER BY position, name COLLATE NOCASE")?
        .query_map([], |r| Ok(PersonGroup { id: r.get(0)?, name: r.get(1)?, person_ids: Vec::new() }))?
        .collect::<std::result::Result<_, _>>()?;
    let mut stmt = conn.prepare(
        "SELECT m.group_id, m.person_id FROM person_group_members m
         JOIN people p ON p.id = m.person_id AND p.status = 'listed'
         ORDER BY m.group_id, m.person_id",
    )?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (g, p) = row?;
        if let Some(group) = groups.iter_mut().find(|x| x.id == g) {
            group.person_ids.push(p);
        }
    }
    Ok(groups)
}

pub fn create(conn: &Connection, name: &str) -> Result<i64> {
    let name = clean(name)?;
    if taken(conn, name, 0)? {
        return Err(CoreError::Invalid(format!("There is already a group called {name}.")));
    }
    conn.execute(
        "INSERT INTO person_groups (name, position) VALUES (?1, (SELECT COALESCE(MAX(position), 0) + 1 FROM person_groups))",
        [name],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn rename(conn: &Connection, id: i64, name: &str) -> Result<()> {
    let name = clean(name)?;
    if taken(conn, name, id)? {
        return Err(CoreError::Invalid(format!("There is already a group called {name}.")));
    }
    conn.execute("UPDATE person_groups SET name = ?2 WHERE id = ?1", params![id, name])?;
    Ok(())
}

/// Deletes the group, never its people. Returns its members, for Undo.
pub fn delete(conn: &Connection, id: i64) -> Result<Vec<i64>> {
    let members: Vec<i64> = conn
        .prepare("SELECT person_id FROM person_group_members WHERE group_id = ?1")?
        .query_map([id], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    conn.execute("DELETE FROM person_group_members WHERE group_id = ?1", [id])?;
    conn.execute("DELETE FROM person_groups WHERE id = ?1", [id])?;
    Ok(members)
}

/// Adds or removes people. Idempotent.
pub fn set_members(conn: &Connection, group: i64, people: &[i64], member: bool) -> Result<()> {
    for p in people {
        if member {
            conn.execute("INSERT OR IGNORE INTO person_group_members (group_id, person_id) VALUES (?1, ?2)", params![group, p])?;
        } else {
            conn.execute("DELETE FROM person_group_members WHERE group_id = ?1 AND person_id = ?2", params![group, p])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn person(c: &Connection, status: &str) -> i64 {
        c.execute("INSERT INTO people (created_at, status) VALUES (0, ?1)", [status]).unwrap();
        c.last_insert_rowid()
    }

    #[test]
    fn groups_hold_listed_people_and_survive_their_own_deletion_as_undo() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let (anna, ben, gone) = (person(c, "listed"), person(c, "listed"), person(c, "removed"));
            let family = create(c, " Family ")?;
            assert!(create(c, "family").is_err(), "names are unique, ignoring case");
            assert!(create(c, "  ").is_err());
            set_members(c, family, &[anna, ben, gone], true)?;
            set_members(c, family, &[anna], true)?;
            set_members(c, family, &[ben], false)?;
            let groups = list(c)?;
            assert_eq!(groups[0].name, "Family");
            assert_eq!(groups[0].person_ids, vec![anna], "a removed person is not listed");
            rename(c, family, "Kin")?;
            let undo = delete(c, family)?;
            assert_eq!(undo.len(), 2);
            assert!(list(c)?.is_empty());
            let again = create(c, "Kin")?;
            set_members(c, again, &undo, true)?;
            assert_eq!(list(c)?[0].person_ids, vec![anna]);
            Ok(())
        })
        .unwrap();
    }
}
