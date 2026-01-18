//! sqlite-adapter — SQLite implementation of the LinkRepository port for local/dev.
//!
//! Purpose
//! - Provide a lightweight, file-based repository to run the system locally
//!   without cloud dependencies.
//! - Implements the `LinkRepository` trait from the `domain` crate.
//! - Exposes `increment_global_counter()` helper mirroring the Dynamo adapter to
//!   support Base62 slug generation strategies when desired.
//!
//! Notes
//! - Uses `rusqlite` with the `bundled` feature for portability.
//! - Stores timestamps as seconds since UNIX_EPOCH (u64).

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH, Duration};

use domain::{
    AuditEntry, AuditRepository, ClickEvent, ClickRepository, CoreError, GroupMember,
    GroupRepository, GroupRole, LinkGroup, LinkRepository, ListOptions, ListResult, ShortLink,
    Slug, UserEmail,
};
use rusqlite::{params, Connection};

/// SQLite-backed repository for local development.
pub struct SqliteRepo {
    conn: std::sync::Mutex<Connection>,
}

impl SqliteRepo {
    /// Open (or create) a SQLite database at the given path and ensure schema.
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, CoreError> {
        let conn = Connection::open(path).map_err(map_sqerr)?;
        init_schema(&conn)?;
        Ok(Self { conn: std::sync::Mutex::new(conn) })
    }

    /// Construct from env var `DB_PATH` (defaults to `./data/shortlinks.db`).
    pub fn from_env() -> Result<Self, CoreError> {
        let path = std::env::var("DB_PATH").unwrap_or_else(|_| "./data/shortlinks.db".to_string());
        // Ensure directory exists
        if let Some(dir) = std::path::Path::new(&path).parent() { let _ = std::fs::create_dir_all(dir); }
        Self::new(path)
    }

    /// Atomically increment the global counter and return the new value.
    pub fn increment_global_counter(&self) -> Result<u64, CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let tx = conn.unchecked_transaction().map_err(map_sqerr)?;
        // Ensure counter row exists
        tx.execute(
            "INSERT OR IGNORE INTO counters(name, value) VALUES('global', 0)",
            [],
        ).map_err(map_sqerr)?;
        tx.execute(
            "UPDATE counters SET value = value + 1 WHERE name = 'global'",
            [],
        ).map_err(map_sqerr)?;
        let val: u64 = tx.query_row(
            "SELECT value FROM counters WHERE name = 'global'",
            [],
            |row| row.get::<_, i64>(0),
        ).map(|v| v as u64).map_err(map_sqerr)?;
        tx.commit().map_err(map_sqerr)?;
        Ok(val)
    }
}

fn init_schema(conn: &Connection) -> Result<(), CoreError> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS shortlinks (
            slug TEXT PRIMARY KEY,
            original_url TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            created_by TEXT NOT NULL,
            click_count INTEGER NOT NULL DEFAULT 0,
            is_active INTEGER NOT NULL DEFAULT 1,
            updated_at INTEGER,
            expires_at INTEGER,
            description TEXT,
            activate_at INTEGER,
            redirect_delay INTEGER,
            deleted_at INTEGER,
            group_id TEXT
        );
        CREATE TABLE IF NOT EXISTS counters (
            name TEXT PRIMARY KEY,
            value INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS link_groups (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT,
            created_at INTEGER NOT NULL,
            created_by TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS group_members (
            group_id TEXT NOT NULL,
            user_email TEXT NOT NULL,
            role TEXT NOT NULL DEFAULT 'viewer',
            added_at INTEGER NOT NULL,
            added_by TEXT NOT NULL,
            PRIMARY KEY (group_id, user_email)
        );
        CREATE TABLE IF NOT EXISTS click_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            slug TEXT NOT NULL,
            clicked_at INTEGER NOT NULL,
            user_agent TEXT,
            referrer TEXT,
            country TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_click_events_slug ON click_events(slug);
        CREATE INDEX IF NOT EXISTS idx_click_events_clicked_at ON click_events(clicked_at);
        CREATE TABLE IF NOT EXISTS audit_log (
            id TEXT PRIMARY KEY,
            timestamp INTEGER NOT NULL,
            actor_email TEXT NOT NULL,
            action TEXT NOT NULL,
            target_type TEXT NOT NULL,
            target_id TEXT NOT NULL,
            changes TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_audit_log_target ON audit_log(target_type, target_id);
        CREATE INDEX IF NOT EXISTS idx_audit_log_actor ON audit_log(actor_email);
        "#
    ).map_err(map_sqerr)?;
    // Migration: add new columns if they don't exist (for existing databases)
    let _ = conn.execute("ALTER TABLE shortlinks ADD COLUMN click_count INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE shortlinks ADD COLUMN is_active INTEGER NOT NULL DEFAULT 1", []);
    let _ = conn.execute("ALTER TABLE shortlinks ADD COLUMN updated_at INTEGER", []);
    let _ = conn.execute("ALTER TABLE shortlinks ADD COLUMN expires_at INTEGER", []);
    let _ = conn.execute("ALTER TABLE shortlinks ADD COLUMN description TEXT", []);
    let _ = conn.execute("ALTER TABLE shortlinks ADD COLUMN activate_at INTEGER", []);
    let _ = conn.execute("ALTER TABLE shortlinks ADD COLUMN redirect_delay INTEGER", []);
    let _ = conn.execute("ALTER TABLE shortlinks ADD COLUMN deleted_at INTEGER", []);
    let _ = conn.execute("ALTER TABLE shortlinks ADD COLUMN group_id TEXT", []);
    Ok(())
}

fn map_sqerr<E: std::fmt::Display>(e: E) -> CoreError { CoreError::Repository(format!("sqlite error: {e}")) }

fn system_time_to_secs(t: SystemTime) -> u64 { t.duration_since(UNIX_EPOCH).unwrap_or(Duration::from_secs(0)).as_secs() }
fn secs_to_system_time(secs: u64) -> SystemTime { UNIX_EPOCH + Duration::from_secs(secs) }

fn row_to_shortlink(row: &rusqlite::Row) -> Result<ShortLink, CoreError> {
    let slug_str: String = row.get(0).map_err(map_sqerr)?;
    let orig: String = row.get(1).map_err(map_sqerr)?;
    let ts: i64 = row.get(2).map_err(map_sqerr)?;
    let by: String = row.get(3).map_err(map_sqerr)?;
    let click_count: i64 = row.get(4).map_err(map_sqerr)?;
    let is_active: i64 = row.get(5).map_err(map_sqerr)?;
    let updated_at: Option<i64> = row.get(6).map_err(map_sqerr)?;
    let expires_at: Option<i64> = row.get(7).map_err(map_sqerr)?;
    let description: Option<String> = row.get(8).map_err(map_sqerr)?;
    let activate_at: Option<i64> = row.get(9).map_err(map_sqerr)?;
    let redirect_delay: Option<i64> = row.get(10).map_err(map_sqerr)?;
    let deleted_at: Option<i64> = row.get(11).map_err(map_sqerr)?;
    let group_id: Option<String> = row.get(12).map_err(map_sqerr)?;

    let s = Slug::new(slug_str).map_err(|e| CoreError::Repository(format!("bad slug in db: {e}")))?;
    let u = UserEmail::new(by).map_err(|_| CoreError::Repository("bad created_by".into()))?;
    Ok(ShortLink {
        slug: s,
        original_url: orig,
        created_at: secs_to_system_time(ts as u64),
        created_by: u,
        click_count: click_count as u64,
        is_active: is_active != 0,
        updated_at: updated_at.map(|t| secs_to_system_time(t as u64)),
        expires_at: expires_at.map(|t| secs_to_system_time(t as u64)),
        description,
        activate_at: activate_at.map(|t| secs_to_system_time(t as u64)),
        redirect_delay: redirect_delay.map(|t| t as u32),
        deleted_at: deleted_at.map(|t| secs_to_system_time(t as u64)),
        group_id,
    })
}

impl LinkRepository for SqliteRepo {
    fn get(&self, slug: &Slug) -> Result<Option<ShortLink>, CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let mut stmt = conn.prepare("SELECT slug, original_url, created_at, created_by, click_count, is_active, updated_at, expires_at, description, activate_at, redirect_delay, deleted_at, group_id FROM shortlinks WHERE slug = ?1")
            .map_err(map_sqerr)?;
        let mut rows = stmt.query(params![slug.as_str()]).map_err(map_sqerr)?;
        if let Some(row) = rows.next().map_err(map_sqerr)? {
            Ok(Some(row_to_shortlink(row)?))
        } else {
            Ok(None)
        }
    }

    fn put(&self, link: ShortLink) -> Result<(), CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let updated_at_secs: Option<i64> = link.updated_at.map(|t| system_time_to_secs(t) as i64);
        let expires_at_secs: Option<i64> = link.expires_at.map(|t| system_time_to_secs(t) as i64);
        let activate_at_secs: Option<i64> = link.activate_at.map(|t| system_time_to_secs(t) as i64);
        let deleted_at_secs: Option<i64> = link.deleted_at.map(|t| system_time_to_secs(t) as i64);
        let redirect_delay: Option<i64> = link.redirect_delay.map(|t| t as i64);
        let res = conn.execute(
            "INSERT INTO shortlinks(slug, original_url, created_at, created_by, click_count, is_active, updated_at, expires_at, description, activate_at, redirect_delay, deleted_at, group_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                link.slug.as_str(),
                link.original_url,
                system_time_to_secs(link.created_at) as i64,
                link.created_by.as_str(),
                link.click_count as i64,
                link.is_active as i64,
                updated_at_secs,
                expires_at_secs,
                link.description,
                activate_at_secs,
                redirect_delay,
                deleted_at_secs,
                link.group_id,
            ],
        );
        match res {
            Ok(_) => Ok(()),
            Err(e) => {
                if let rusqlite::Error::SqliteFailure(err, _) = &e { if err.code == rusqlite::ErrorCode::ConstraintViolation { return Err(CoreError::AlreadyExists); } }
                Err(map_sqerr(e))
            }
        }
    }

    fn list(&self, limit: usize) -> Result<Vec<ShortLink>, CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let mut stmt = conn.prepare("SELECT slug, original_url, created_at, created_by, click_count, is_active, updated_at, expires_at, description, activate_at, redirect_delay, deleted_at, group_id FROM shortlinks WHERE deleted_at IS NULL ORDER BY created_at DESC LIMIT ?1")
            .map_err(map_sqerr)?;
        let mut rows = stmt.query(params![limit as i64]).map_err(map_sqerr)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(map_sqerr)? {
            out.push(row_to_shortlink(row)?);
        }
        Ok(out)
    }

    fn update(&self, link: &ShortLink) -> Result<(), CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let updated_at_secs: Option<i64> = link.updated_at.map(|t| system_time_to_secs(t) as i64);
        let expires_at_secs: Option<i64> = link.expires_at.map(|t| system_time_to_secs(t) as i64);
        let activate_at_secs: Option<i64> = link.activate_at.map(|t| system_time_to_secs(t) as i64);
        let redirect_delay: Option<i64> = link.redirect_delay.map(|t| t as i64);
        let changed = conn.execute(
            "UPDATE shortlinks SET original_url = ?1, is_active = ?2, updated_at = ?3, expires_at = ?4, description = ?5, activate_at = ?6, redirect_delay = ?7, group_id = ?8 WHERE slug = ?9",
            params![link.original_url, link.is_active as i64, updated_at_secs, expires_at_secs, link.description, activate_at_secs, redirect_delay, link.group_id, link.slug.as_str()],
        ).map_err(map_sqerr)?;
        if changed == 0 {
            Err(CoreError::NotFound)
        } else {
            Ok(())
        }
    }

    fn increment_click(&self, slug: &Slug) -> Result<(), CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let changed = conn.execute(
            "UPDATE shortlinks SET click_count = click_count + 1 WHERE slug = ?1",
            params![slug.as_str()],
        ).map_err(map_sqerr)?;
        if changed == 0 {
            Err(CoreError::NotFound)
        } else {
            Ok(())
        }
    }

    fn list_by_creator(&self, email: &UserEmail, limit: usize) -> Result<Vec<ShortLink>, CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let mut stmt = conn.prepare("SELECT slug, original_url, created_at, created_by, click_count, is_active, updated_at, expires_at, description, activate_at, redirect_delay, deleted_at, group_id FROM shortlinks WHERE created_by = ?1 AND deleted_at IS NULL ORDER BY created_at DESC LIMIT ?2")
            .map_err(map_sqerr)?;
        let mut rows = stmt.query(params![email.as_str(), limit as i64]).map_err(map_sqerr)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(map_sqerr)? {
            out.push(row_to_shortlink(row)?);
        }
        Ok(out)
    }

    fn delete(&self, slug: &Slug, deleted_at: SystemTime) -> Result<(), CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let deleted_at_secs = system_time_to_secs(deleted_at) as i64;
        let changed = conn.execute(
            "UPDATE shortlinks SET deleted_at = ?1 WHERE slug = ?2 AND deleted_at IS NULL",
            params![deleted_at_secs, slug.as_str()],
        ).map_err(map_sqerr)?;
        if changed == 0 {
            Err(CoreError::NotFound)
        } else {
            Ok(())
        }
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<ShortLink>, CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let pattern = format!("%{}%", query.to_lowercase());
        let mut stmt = conn.prepare("SELECT slug, original_url, created_at, created_by, click_count, is_active, updated_at, expires_at, description, activate_at, redirect_delay, deleted_at, group_id FROM shortlinks WHERE deleted_at IS NULL AND (LOWER(slug) LIKE ?1 OR LOWER(original_url) LIKE ?1 OR LOWER(description) LIKE ?1) ORDER BY created_at DESC LIMIT ?2")
            .map_err(map_sqerr)?;
        let mut rows = stmt.query(params![pattern, limit as i64]).map_err(map_sqerr)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(map_sqerr)? {
            out.push(row_to_shortlink(row)?);
        }
        Ok(out)
    }

    fn list_paginated(&self, options: &ListOptions) -> Result<ListResult<ShortLink>, CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;

        // Build WHERE clause dynamically
        let mut conditions = Vec::new();
        let mut params_values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if !options.include_deleted {
            conditions.push("deleted_at IS NULL".to_string());
        }
        if let Some(ref email) = options.created_by {
            conditions.push(format!("created_by = ?{}", params_values.len() + 1));
            params_values.push(Box::new(email.as_str().to_string()));
        }
        if let Some(ref gid) = options.group_id {
            conditions.push(format!("group_id = ?{}", params_values.len() + 1));
            params_values.push(Box::new(gid.clone()));
        }
        if let Some(ref q) = options.search {
            let pattern = format!("%{}%", q.to_lowercase());
            let idx = params_values.len() + 1;
            conditions.push(format!("(LOWER(slug) LIKE ?{} OR LOWER(original_url) LIKE ?{} OR LOWER(description) LIKE ?{})", idx, idx, idx));
            params_values.push(Box::new(pattern));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        // Count total
        let count_sql = format!("SELECT COUNT(*) FROM shortlinks {}", where_clause);
        let total: i64 = {
            let mut stmt = conn.prepare(&count_sql).map_err(map_sqerr)?;
            let params_refs: Vec<&dyn rusqlite::ToSql> = params_values.iter().map(|b| b.as_ref()).collect();
            stmt.query_row(params_refs.as_slice(), |r| r.get(0)).map_err(map_sqerr)?
        };

        // Fetch items
        let select_sql = format!(
            "SELECT slug, original_url, created_at, created_by, click_count, is_active, updated_at, expires_at, description, activate_at, redirect_delay, deleted_at, group_id FROM shortlinks {} ORDER BY created_at DESC LIMIT ?{} OFFSET ?{}",
            where_clause,
            params_values.len() + 1,
            params_values.len() + 2
        );
        params_values.push(Box::new(options.limit as i64));
        params_values.push(Box::new(options.offset as i64));

        let mut stmt = conn.prepare(&select_sql).map_err(map_sqerr)?;
        let params_refs: Vec<&dyn rusqlite::ToSql> = params_values.iter().map(|b| b.as_ref()).collect();
        let mut rows = stmt.query(params_refs.as_slice()).map_err(map_sqerr)?;
        let mut items = Vec::new();
        while let Some(row) = rows.next().map_err(map_sqerr)? {
            items.push(row_to_shortlink(row)?);
        }

        let has_more = options.offset + items.len() < total as usize;
        Ok(ListResult { items, total: total as usize, has_more })
    }

    fn list_by_group(&self, group_id: &str, limit: usize) -> Result<Vec<ShortLink>, CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let mut stmt = conn.prepare("SELECT slug, original_url, created_at, created_by, click_count, is_active, updated_at, expires_at, description, activate_at, redirect_delay, deleted_at, group_id FROM shortlinks WHERE group_id = ?1 AND deleted_at IS NULL ORDER BY created_at DESC LIMIT ?2")
            .map_err(map_sqerr)?;
        let mut rows = stmt.query(params![group_id, limit as i64]).map_err(map_sqerr)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(map_sqerr)? {
            out.push(row_to_shortlink(row)?);
        }
        Ok(out)
    }

    fn bulk_delete(&self, slugs: &[Slug], deleted_at: SystemTime) -> Result<usize, CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let deleted_at_secs = system_time_to_secs(deleted_at) as i64;
        let mut count = 0;
        for slug in slugs {
            let changed = conn.execute(
                "UPDATE shortlinks SET deleted_at = ?1 WHERE slug = ?2 AND deleted_at IS NULL",
                params![deleted_at_secs, slug.as_str()],
            ).map_err(map_sqerr)?;
            count += changed;
        }
        Ok(count)
    }

    fn bulk_update_active(&self, slugs: &[Slug], is_active: bool, updated_at: SystemTime) -> Result<usize, CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let updated_at_secs = system_time_to_secs(updated_at) as i64;
        let mut count = 0;
        for slug in slugs {
            let changed = conn.execute(
                "UPDATE shortlinks SET is_active = ?1, updated_at = ?2 WHERE slug = ?3",
                params![is_active as i64, updated_at_secs, slug.as_str()],
            ).map_err(map_sqerr)?;
            count += changed;
        }
        Ok(count)
    }
}

// ============ GroupRepository ============

impl GroupRepository for SqliteRepo {
    fn create_group(&self, group: LinkGroup) -> Result<(), CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let res = conn.execute(
            "INSERT INTO link_groups(id, name, description, created_at, created_by) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                group.id,
                group.name,
                group.description,
                system_time_to_secs(group.created_at) as i64,
                group.created_by.as_str(),
            ],
        );
        match res {
            Ok(_) => Ok(()),
            Err(e) => {
                if let rusqlite::Error::SqliteFailure(err, _) = &e {
                    if err.code == rusqlite::ErrorCode::ConstraintViolation {
                        return Err(CoreError::AlreadyExists);
                    }
                }
                Err(map_sqerr(e))
            }
        }
    }

    fn get_group(&self, id: &str) -> Result<Option<LinkGroup>, CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let mut stmt = conn.prepare("SELECT id, name, description, created_at, created_by FROM link_groups WHERE id = ?1")
            .map_err(map_sqerr)?;
        let mut rows = stmt.query(params![id]).map_err(map_sqerr)?;
        if let Some(row) = rows.next().map_err(map_sqerr)? {
            Ok(Some(row_to_group(row)?))
        } else {
            Ok(None)
        }
    }

    fn list_groups(&self, user_email: &UserEmail) -> Result<Vec<LinkGroup>, CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT g.id, g.name, g.description, g.created_at, g.created_by FROM link_groups g
             LEFT JOIN group_members m ON g.id = m.group_id
             WHERE g.created_by = ?1 OR m.user_email = ?1
             ORDER BY g.name"
        ).map_err(map_sqerr)?;
        let mut rows = stmt.query(params![user_email.as_str()]).map_err(map_sqerr)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(map_sqerr)? {
            out.push(row_to_group(row)?);
        }
        Ok(out)
    }

    fn update_group(&self, group: &LinkGroup) -> Result<(), CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let changed = conn.execute(
            "UPDATE link_groups SET name = ?1, description = ?2 WHERE id = ?3",
            params![group.name, group.description, group.id],
        ).map_err(map_sqerr)?;
        if changed == 0 { Err(CoreError::NotFound) } else { Ok(()) }
    }

    fn delete_group(&self, id: &str) -> Result<(), CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        // Delete members first
        conn.execute("DELETE FROM group_members WHERE group_id = ?1", params![id]).map_err(map_sqerr)?;
        let changed = conn.execute("DELETE FROM link_groups WHERE id = ?1", params![id]).map_err(map_sqerr)?;
        if changed == 0 { Err(CoreError::NotFound) } else { Ok(()) }
    }

    fn add_member(&self, member: GroupMember) -> Result<(), CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let role_str = match member.role {
            GroupRole::Viewer => "viewer",
            GroupRole::Editor => "editor",
            GroupRole::Admin => "admin",
        };
        let res = conn.execute(
            "INSERT INTO group_members(group_id, user_email, role, added_at, added_by) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                member.group_id,
                member.user_email.as_str(),
                role_str,
                system_time_to_secs(member.added_at) as i64,
                member.added_by.as_str(),
            ],
        );
        match res {
            Ok(_) => Ok(()),
            Err(e) => {
                if let rusqlite::Error::SqliteFailure(err, _) = &e {
                    if err.code == rusqlite::ErrorCode::ConstraintViolation {
                        return Err(CoreError::AlreadyExists);
                    }
                }
                Err(map_sqerr(e))
            }
        }
    }

    fn remove_member(&self, group_id: &str, user_email: &UserEmail) -> Result<(), CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let changed = conn.execute(
            "DELETE FROM group_members WHERE group_id = ?1 AND user_email = ?2",
            params![group_id, user_email.as_str()],
        ).map_err(map_sqerr)?;
        if changed == 0 { Err(CoreError::NotFound) } else { Ok(()) }
    }

    fn list_members(&self, group_id: &str) -> Result<Vec<GroupMember>, CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let mut stmt = conn.prepare(
            "SELECT group_id, user_email, role, added_at, added_by FROM group_members WHERE group_id = ?1"
        ).map_err(map_sqerr)?;
        let mut rows = stmt.query(params![group_id]).map_err(map_sqerr)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(map_sqerr)? {
            out.push(row_to_member(row)?);
        }
        Ok(out)
    }

    fn get_member(&self, group_id: &str, user_email: &UserEmail) -> Result<Option<GroupMember>, CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let mut stmt = conn.prepare(
            "SELECT group_id, user_email, role, added_at, added_by FROM group_members WHERE group_id = ?1 AND user_email = ?2"
        ).map_err(map_sqerr)?;
        let mut rows = stmt.query(params![group_id, user_email.as_str()]).map_err(map_sqerr)?;
        if let Some(row) = rows.next().map_err(map_sqerr)? {
            Ok(Some(row_to_member(row)?))
        } else {
            Ok(None)
        }
    }

    fn get_user_groups(&self, user_email: &UserEmail) -> Result<Vec<(LinkGroup, GroupRole)>, CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;

        let mut result = Vec::new();

        // Groups where user is creator (Admin role)
        {
            let mut stmt = conn.prepare(
                "SELECT id, name, description, created_at, created_by FROM link_groups WHERE created_by = ?1"
            ).map_err(map_sqerr)?;
            let mut rows = stmt.query(params![user_email.as_str()]).map_err(map_sqerr)?;
            while let Some(row) = rows.next().map_err(map_sqerr)? {
                result.push((row_to_group(row)?, GroupRole::Admin));
            }
        }

        // Groups where user is a member (not creator)
        {
            let mut stmt = conn.prepare(
                "SELECT g.id, g.name, g.description, g.created_at, g.created_by, m.role
                 FROM link_groups g
                 JOIN group_members m ON g.id = m.group_id
                 WHERE m.user_email = ?1 AND g.created_by != ?1"
            ).map_err(map_sqerr)?;
            let mut rows = stmt.query(params![user_email.as_str()]).map_err(map_sqerr)?;
            while let Some(row) = rows.next().map_err(map_sqerr)? {
                let group = row_to_group(row)?;
                let role_str: String = row.get(5).map_err(map_sqerr)?;
                let role = str_to_role(&role_str);
                result.push((group, role));
            }
        }

        Ok(result)
    }
}

fn row_to_group(row: &rusqlite::Row) -> Result<LinkGroup, CoreError> {
    let id: String = row.get(0).map_err(map_sqerr)?;
    let name: String = row.get(1).map_err(map_sqerr)?;
    let description: Option<String> = row.get(2).map_err(map_sqerr)?;
    let created_at: i64 = row.get(3).map_err(map_sqerr)?;
    let created_by: String = row.get(4).map_err(map_sqerr)?;
    Ok(LinkGroup {
        id,
        name,
        description,
        created_at: secs_to_system_time(created_at as u64),
        created_by: UserEmail::new(created_by).map_err(|_| CoreError::Repository("bad email".into()))?,
    })
}

fn row_to_member(row: &rusqlite::Row) -> Result<GroupMember, CoreError> {
    let group_id: String = row.get(0).map_err(map_sqerr)?;
    let user_email: String = row.get(1).map_err(map_sqerr)?;
    let role_str: String = row.get(2).map_err(map_sqerr)?;
    let added_at: i64 = row.get(3).map_err(map_sqerr)?;
    let added_by: String = row.get(4).map_err(map_sqerr)?;
    Ok(GroupMember {
        group_id,
        user_email: UserEmail::new(user_email).map_err(|_| CoreError::Repository("bad email".into()))?,
        role: str_to_role(&role_str),
        added_at: secs_to_system_time(added_at as u64),
        added_by: UserEmail::new(added_by).map_err(|_| CoreError::Repository("bad email".into()))?,
    })
}

fn str_to_role(s: &str) -> GroupRole {
    match s {
        "admin" => GroupRole::Admin,
        "editor" => GroupRole::Editor,
        _ => GroupRole::Viewer,
    }
}

// ============ ClickRepository ============

impl ClickRepository for SqliteRepo {
    fn record_click(&self, event: ClickEvent) -> Result<(), CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        conn.execute(
            "INSERT INTO click_events(slug, clicked_at, user_agent, referrer, country) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                event.slug.as_str(),
                system_time_to_secs(event.clicked_at) as i64,
                event.user_agent,
                event.referrer,
                event.country,
            ],
        ).map_err(map_sqerr)?;
        Ok(())
    }

    fn get_clicks(&self, slug: &Slug, limit: usize) -> Result<Vec<ClickEvent>, CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let mut stmt = conn.prepare(
            "SELECT slug, clicked_at, user_agent, referrer, country FROM click_events WHERE slug = ?1 ORDER BY clicked_at DESC LIMIT ?2"
        ).map_err(map_sqerr)?;
        let mut rows = stmt.query(params![slug.as_str(), limit as i64]).map_err(map_sqerr)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(map_sqerr)? {
            out.push(row_to_click(row)?);
        }
        Ok(out)
    }

    fn get_click_count_since(&self, slug: &Slug, since: SystemTime) -> Result<u64, CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let since_secs = system_time_to_secs(since) as i64;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM click_events WHERE slug = ?1 AND clicked_at >= ?2",
            params![slug.as_str(), since_secs],
            |r| r.get(0),
        ).map_err(map_sqerr)?;
        Ok(count as u64)
    }

    fn get_clicks_by_day(&self, slug: &Slug, days: usize) -> Result<Vec<(String, u64)>, CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let cutoff = SystemTime::now()
            .checked_sub(Duration::from_secs(days as u64 * 24 * 60 * 60))
            .unwrap_or(UNIX_EPOCH);
        let cutoff_secs = system_time_to_secs(cutoff) as i64;

        let mut stmt = conn.prepare(
            "SELECT date(clicked_at, 'unixepoch') as day, COUNT(*) as cnt
             FROM click_events
             WHERE slug = ?1 AND clicked_at >= ?2
             GROUP BY day
             ORDER BY day"
        ).map_err(map_sqerr)?;
        let mut rows = stmt.query(params![slug.as_str(), cutoff_secs]).map_err(map_sqerr)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(map_sqerr)? {
            let day: String = row.get(0).map_err(map_sqerr)?;
            let count: i64 = row.get(1).map_err(map_sqerr)?;
            out.push((day, count as u64));
        }
        Ok(out)
    }
}

fn row_to_click(row: &rusqlite::Row) -> Result<ClickEvent, CoreError> {
    let slug_str: String = row.get(0).map_err(map_sqerr)?;
    let clicked_at: i64 = row.get(1).map_err(map_sqerr)?;
    let user_agent: Option<String> = row.get(2).map_err(map_sqerr)?;
    let referrer: Option<String> = row.get(3).map_err(map_sqerr)?;
    let country: Option<String> = row.get(4).map_err(map_sqerr)?;
    Ok(ClickEvent {
        slug: Slug::new(slug_str).map_err(|e| CoreError::Repository(format!("bad slug: {e}")))?,
        clicked_at: secs_to_system_time(clicked_at as u64),
        user_agent,
        referrer,
        country,
    })
}

// ============ AuditRepository ============

impl AuditRepository for SqliteRepo {
    fn log(&self, entry: AuditEntry) -> Result<(), CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let action_str = format!("{:?}", entry.action);
        conn.execute(
            "INSERT INTO audit_log(id, timestamp, actor_email, action, target_type, target_id, changes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                entry.id,
                system_time_to_secs(entry.timestamp) as i64,
                entry.actor_email.as_str(),
                action_str,
                entry.target_type,
                entry.target_id,
                entry.changes,
            ],
        ).map_err(map_sqerr)?;
        Ok(())
    }

    fn list_for_target(&self, target_type: &str, target_id: &str, limit: usize) -> Result<Vec<AuditEntry>, CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let mut stmt = conn.prepare(
            "SELECT id, timestamp, actor_email, action, target_type, target_id, changes
             FROM audit_log WHERE target_type = ?1 AND target_id = ?2
             ORDER BY timestamp DESC LIMIT ?3"
        ).map_err(map_sqerr)?;
        let mut rows = stmt.query(params![target_type, target_id, limit as i64]).map_err(map_sqerr)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(map_sqerr)? {
            out.push(row_to_audit(row)?);
        }
        Ok(out)
    }

    fn list_by_actor(&self, actor_email: &UserEmail, limit: usize) -> Result<Vec<AuditEntry>, CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let mut stmt = conn.prepare(
            "SELECT id, timestamp, actor_email, action, target_type, target_id, changes
             FROM audit_log WHERE actor_email = ?1
             ORDER BY timestamp DESC LIMIT ?2"
        ).map_err(map_sqerr)?;
        let mut rows = stmt.query(params![actor_email.as_str(), limit as i64]).map_err(map_sqerr)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(map_sqerr)? {
            out.push(row_to_audit(row)?);
        }
        Ok(out)
    }

    fn list_recent(&self, limit: usize) -> Result<Vec<AuditEntry>, CoreError> {
        let conn = self.conn.lock().map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let mut stmt = conn.prepare(
            "SELECT id, timestamp, actor_email, action, target_type, target_id, changes
             FROM audit_log ORDER BY timestamp DESC LIMIT ?1"
        ).map_err(map_sqerr)?;
        let mut rows = stmt.query(params![limit as i64]).map_err(map_sqerr)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(map_sqerr)? {
            out.push(row_to_audit(row)?);
        }
        Ok(out)
    }
}

fn row_to_audit(row: &rusqlite::Row) -> Result<AuditEntry, CoreError> {
    use domain::AuditAction;
    let id: String = row.get(0).map_err(map_sqerr)?;
    let timestamp: i64 = row.get(1).map_err(map_sqerr)?;
    let actor_email: String = row.get(2).map_err(map_sqerr)?;
    let action_str: String = row.get(3).map_err(map_sqerr)?;
    let target_type: String = row.get(4).map_err(map_sqerr)?;
    let target_id: String = row.get(5).map_err(map_sqerr)?;
    let changes: Option<String> = row.get(6).map_err(map_sqerr)?;

    let action = match action_str.as_str() {
        "Create" => AuditAction::Create,
        "Update" => AuditAction::Update,
        "Delete" => AuditAction::Delete,
        "Activate" => AuditAction::Activate,
        "Deactivate" => AuditAction::Deactivate,
        _ => AuditAction::Create, // fallback
    };

    Ok(AuditEntry {
        id,
        timestamp: secs_to_system_time(timestamp as u64),
        actor_email: UserEmail::new(actor_email).map_err(|_| CoreError::Repository("bad email".into()))?,
        action,
        target_type,
        target_id,
        changes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_db() -> (SqliteRepo, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let repo = SqliteRepo::new(path).unwrap();
        (repo, dir)
    }

    #[test]
    fn put_get_roundtrip() {
        let (repo, _dir) = tmp_db();
        let link = ShortLink::new(
            Slug::new("abc123").unwrap(),
            "https://example.com".into(),
            SystemTime::UNIX_EPOCH,
            UserEmail::new("u@acme.com").unwrap(),
        );
        repo.put(link.clone()).unwrap();
        let got = repo.get(&link.slug).unwrap().unwrap();
        assert_eq!(got.original_url, "https://example.com");
        assert_eq!(got.click_count, 0);
        assert!(got.is_active);
    }

    #[test]
    fn put_duplicate_conflict() {
        let (repo, _dir) = tmp_db();
        let link = ShortLink::new(
            Slug::new("dup").unwrap(),
            "https://e".into(),
            SystemTime::UNIX_EPOCH,
            UserEmail::new("u@acme.com").unwrap(),
        );
        repo.put(link.clone()).unwrap();
        let err = repo.put(link).unwrap_err();
        assert!(matches!(err, CoreError::AlreadyExists));
    }

    #[test]
    fn list_orders_and_limits() {
        let (repo, _dir) = tmp_db();
        for i in 0..5u64 {
            let mut l = ShortLink::new(
                Slug::new(format!("k{i}")).unwrap(),
                format!("https://e/{i}"),
                UNIX_EPOCH + Duration::from_secs(i),
                UserEmail::new("u@acme.com").unwrap(),
            );
            l.click_count = 0;
            repo.put(l).unwrap();
        }
        let items = repo.list(3).unwrap();
        assert_eq!(items.len(), 3);
        // First item should be the latest (i=4)
        assert_eq!(items[0].slug.as_str(), "k4");
    }

    #[test]
    fn increment_click_works() {
        let (repo, _dir) = tmp_db();
        let link = ShortLink::new(
            Slug::new("clickme").unwrap(),
            "https://example.com".into(),
            SystemTime::UNIX_EPOCH,
            UserEmail::new("u@acme.com").unwrap(),
        );
        repo.put(link.clone()).unwrap();

        // Increment 3 times
        repo.increment_click(&link.slug).unwrap();
        repo.increment_click(&link.slug).unwrap();
        repo.increment_click(&link.slug).unwrap();

        let got = repo.get(&link.slug).unwrap().unwrap();
        assert_eq!(got.click_count, 3);
    }

    #[test]
    fn update_link_works() {
        let (repo, _dir) = tmp_db();
        let mut link = ShortLink::new(
            Slug::new("updateme").unwrap(),
            "https://old.com".into(),
            SystemTime::UNIX_EPOCH,
            UserEmail::new("u@acme.com").unwrap(),
        );
        repo.put(link.clone()).unwrap();

        // Update original_url and is_active
        link.original_url = "https://new.com".into();
        link.is_active = false;
        link.updated_at = Some(UNIX_EPOCH + Duration::from_secs(100));
        repo.update(&link).unwrap();

        let got = repo.get(&link.slug).unwrap().unwrap();
        assert_eq!(got.original_url, "https://new.com");
        assert!(!got.is_active);
    }

    #[test]
    fn list_by_creator_works() {
        let (repo, _dir) = tmp_db();
        let user1 = UserEmail::new("user1@acme.com").unwrap();
        let user2 = UserEmail::new("user2@acme.com").unwrap();

        // Create links for user1
        for i in 0..3 {
            let l = ShortLink::new(
                Slug::new(format!("u1-{i}")).unwrap(),
                format!("https://u1/{i}"),
                UNIX_EPOCH + Duration::from_secs(i),
                user1.clone(),
            );
            repo.put(l).unwrap();
        }

        // Create links for user2
        for i in 0..2 {
            let l = ShortLink::new(
                Slug::new(format!("u2-{i}")).unwrap(),
                format!("https://u2/{i}"),
                UNIX_EPOCH + Duration::from_secs(i),
                user2.clone(),
            );
            repo.put(l).unwrap();
        }

        let user1_links = repo.list_by_creator(&user1, 10).unwrap();
        assert_eq!(user1_links.len(), 3);

        let user2_links = repo.list_by_creator(&user2, 10).unwrap();
        assert_eq!(user2_links.len(), 2);
    }
}
