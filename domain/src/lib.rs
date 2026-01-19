//! Domain library for the URL Shortener.
//!
//! This crate is dependency-free (inherits workspace metadata only) and holds
//! the domain types, ports (traits), and error definitions. Keep adapters and
//! IO concerns out of this crate.

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::time::SystemTime;

/// A URL-safe slug identifying a short link.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Slug(String);

impl Slug {
    pub fn new<S: Into<String>>(s: S) -> Result<Self, CoreError> {
        let val = s.into();
        // Very light validation for now: non-empty and ascii
        if val.is_empty() {
            return Err(CoreError::InvalidSlug("empty".into()));
        }
        if !val
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(CoreError::InvalidSlug("invalid characters".into()));
        }
        Ok(Self(val))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Email address of the user creating links.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UserEmail(String);

impl UserEmail {
    pub fn new<S: Into<String>>(s: S) -> Result<Self, CoreError> {
        let val = s.into();
        // Lightweight check; full RFC compliance not required here
        if val.is_empty() || !val.contains('@') {
            return Err(CoreError::InvalidUserEmail);
        }
        Ok(Self(val))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Input data for creating a new short link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewLink {
    pub original_url: String,
    pub custom_slug: Option<Slug>,
    pub user_email: UserEmail,
}

/// Stored short link mapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortLink {
    pub slug: Slug,
    pub original_url: String,
    pub created_at: SystemTime,
    pub created_by: UserEmail,
    /// Number of times this link has been visited (redirected).
    pub click_count: u64,
    /// Whether the link is active. Inactive links return 404.
    pub is_active: bool,
    /// Last time the link was updated (target URL or status changed).
    pub updated_at: Option<SystemTime>,
    /// Optional expiration time. Links return 410 Gone after this time.
    pub expires_at: Option<SystemTime>,
    /// Optional description/notes for the link.
    pub description: Option<String>,
    /// Optional scheduled activation time. Links are not active before this time.
    pub activate_at: Option<SystemTime>,
    /// Optional redirect delay in seconds for interstitial countdown page.
    /// None = immediate redirect, Some(n) = show countdown for n seconds.
    pub redirect_delay: Option<u32>,
    /// Soft delete timestamp. If set, link is considered deleted.
    pub deleted_at: Option<SystemTime>,
    /// Optional group ID for organizing links.
    pub group_id: Option<String>,
}

impl ShortLink {
    /// Create a new ShortLink with default values for click_count (0) and is_active (true).
    pub fn new(
        slug: Slug,
        original_url: String,
        created_at: SystemTime,
        created_by: UserEmail,
    ) -> Self {
        Self {
            slug,
            original_url,
            created_at,
            created_by,
            click_count: 0,
            is_active: true,
            updated_at: None,
            expires_at: None,
            description: None,
            activate_at: None,
            redirect_delay: None,
            deleted_at: None,
            group_id: None,
        }
    }

    /// Check if the link has expired based on the given current time.
    pub fn is_expired(&self, now: SystemTime) -> bool {
        self.expires_at.is_some_and(|exp| now >= exp)
    }

    /// Check if the link is scheduled for future activation.
    pub fn is_scheduled(&self, now: SystemTime) -> bool {
        self.activate_at.is_some_and(|act| now < act)
    }

    /// Check if the link has been soft-deleted.
    pub fn is_deleted(&self) -> bool {
        self.deleted_at.is_some()
    }

    /// Check if the link is available for redirect (active, not expired, not scheduled, not deleted).
    pub fn is_available(&self, now: SystemTime) -> bool {
        self.is_active && !self.is_expired(now) && !self.is_scheduled(now) && !self.is_deleted()
    }
}

/// A link group for organizing links and sharing access.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkGroup {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: SystemTime,
    pub created_by: UserEmail,
}

/// A member of a link group with their access level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupMember {
    pub group_id: String,
    pub user_email: UserEmail,
    pub role: GroupRole,
    pub added_at: SystemTime,
    pub added_by: UserEmail,
}

/// Role/permission level for group members.
#[derive(Clone, Debug, PartialEq, Eq, Copy)]
pub enum GroupRole {
    /// Can view links in the group.
    Viewer,
    /// Can view and create/edit links in the group.
    Editor,
    /// Can view, edit, and manage group members.
    Admin,
}

impl GroupRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            GroupRole::Viewer => "viewer",
            GroupRole::Editor => "editor",
            GroupRole::Admin => "admin",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "viewer" => Some(GroupRole::Viewer),
            "editor" => Some(GroupRole::Editor),
            "admin" => Some(GroupRole::Admin),
            _ => None,
        }
    }

    pub fn can_edit(&self) -> bool {
        matches!(self, GroupRole::Editor | GroupRole::Admin)
    }

    pub fn can_manage(&self) -> bool {
        matches!(self, GroupRole::Admin)
    }
}

/// A click event for analytics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClickEvent {
    pub slug: Slug,
    pub clicked_at: SystemTime,
    pub user_agent: Option<String>,
    pub referrer: Option<String>,
    pub country: Option<String>,
}

/// An audit log entry tracking changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditEntry {
    pub id: String,
    pub timestamp: SystemTime,
    pub actor_email: UserEmail,
    pub action: AuditAction,
    pub target_type: String,
    pub target_id: String,
    pub changes: Option<String>, // JSON string of changes
}

/// Types of auditable actions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuditAction {
    Create,
    Update,
    Delete,
    Restore,
    Activate,
    Deactivate,
    AddMember,
    RemoveMember,
}

impl AuditAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuditAction::Create => "create",
            AuditAction::Update => "update",
            AuditAction::Delete => "delete",
            AuditAction::Restore => "restore",
            AuditAction::Activate => "activate",
            AuditAction::Deactivate => "deactivate",
            AuditAction::AddMember => "add_member",
            AuditAction::RemoveMember => "remove_member",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "create" => Some(AuditAction::Create),
            "update" => Some(AuditAction::Update),
            "delete" => Some(AuditAction::Delete),
            "restore" => Some(AuditAction::Restore),
            "activate" => Some(AuditAction::Activate),
            "deactivate" => Some(AuditAction::Deactivate),
            "add_member" => Some(AuditAction::AddMember),
            "remove_member" => Some(AuditAction::RemoveMember),
            _ => None,
        }
    }
}

/// Time source abstraction to make code testable.
pub trait Clock: Send + Sync {
    fn now(&self) -> SystemTime;
}

/// Slug generator interface; deterministic by input id in some strategies.
pub trait SlugGenerator: Send + Sync {
    fn next_slug(&self, next_id: u64) -> Slug;
}

/// Pagination parameters for list queries.
#[derive(Clone, Debug, Default)]
pub struct ListOptions {
    pub limit: usize,
    pub offset: usize,
    pub created_by: Option<UserEmail>,
    pub group_id: Option<String>,
    pub search: Option<String>,
    pub include_deleted: bool,
}

/// Paginated list result.
#[derive(Clone, Debug)]
pub struct ListResult<T> {
    pub items: Vec<T>,
    pub total: usize,
    pub has_more: bool,
}

/// Repository port for persisting and loading links.
pub trait LinkRepository: Send + Sync {
    fn get(&self, slug: &Slug) -> Result<Option<ShortLink>, CoreError>;
    fn put(&self, link: ShortLink) -> Result<(), CoreError>;
    fn list(&self, limit: usize) -> Result<Vec<ShortLink>, CoreError>;
    /// Update an existing link (original_url, is_active, updated_at).
    fn update(&self, link: &ShortLink) -> Result<(), CoreError>;
    /// Atomically increment the click count for a link.
    fn increment_click(&self, slug: &Slug) -> Result<(), CoreError>;
    /// List links created by a specific user.
    fn list_by_creator(&self, email: &UserEmail, limit: usize)
        -> Result<Vec<ShortLink>, CoreError>;
    /// Delete a link (soft delete by default).
    fn delete(&self, slug: &Slug, deleted_at: SystemTime) -> Result<(), CoreError>;
    /// Search links by slug or URL.
    fn search(&self, query: &str, limit: usize) -> Result<Vec<ShortLink>, CoreError>;
    /// List links with pagination and filters.
    fn list_paginated(&self, options: &ListOptions) -> Result<ListResult<ShortLink>, CoreError>;
    /// List links by group ID.
    fn list_by_group(&self, group_id: &str, limit: usize) -> Result<Vec<ShortLink>, CoreError>;
    /// Bulk delete links (soft delete).
    fn bulk_delete(&self, slugs: &[Slug], deleted_at: SystemTime) -> Result<usize, CoreError>;
    /// Bulk update is_active status.
    fn bulk_update_active(
        &self,
        slugs: &[Slug],
        is_active: bool,
        updated_at: SystemTime,
    ) -> Result<usize, CoreError>;
}

/// Repository port for link groups.
pub trait GroupRepository: Send + Sync {
    fn create_group(&self, group: LinkGroup) -> Result<(), CoreError>;
    fn get_group(&self, id: &str) -> Result<Option<LinkGroup>, CoreError>;
    fn list_groups(&self, user_email: &UserEmail) -> Result<Vec<LinkGroup>, CoreError>;
    fn update_group(&self, group: &LinkGroup) -> Result<(), CoreError>;
    fn delete_group(&self, id: &str) -> Result<(), CoreError>;
    fn add_member(&self, member: GroupMember) -> Result<(), CoreError>;
    fn remove_member(&self, group_id: &str, user_email: &UserEmail) -> Result<(), CoreError>;
    fn list_members(&self, group_id: &str) -> Result<Vec<GroupMember>, CoreError>;
    fn get_member(
        &self,
        group_id: &str,
        user_email: &UserEmail,
    ) -> Result<Option<GroupMember>, CoreError>;
    fn get_user_groups(
        &self,
        user_email: &UserEmail,
    ) -> Result<Vec<(LinkGroup, GroupRole)>, CoreError>;
}

/// Repository port for click analytics.
pub trait ClickRepository: Send + Sync {
    fn record_click(&self, event: ClickEvent) -> Result<(), CoreError>;
    fn get_clicks(&self, slug: &Slug, limit: usize) -> Result<Vec<ClickEvent>, CoreError>;
    fn get_click_count_since(&self, slug: &Slug, since: SystemTime) -> Result<u64, CoreError>;
    fn get_clicks_by_day(&self, slug: &Slug, days: usize) -> Result<Vec<(String, u64)>, CoreError>;
}

/// Repository port for audit log.
pub trait AuditRepository: Send + Sync {
    fn log(&self, entry: AuditEntry) -> Result<(), CoreError>;
    fn list_for_target(
        &self,
        target_type: &str,
        target_id: &str,
        limit: usize,
    ) -> Result<Vec<AuditEntry>, CoreError>;
    fn list_by_actor(
        &self,
        actor_email: &UserEmail,
        limit: usize,
    ) -> Result<Vec<AuditEntry>, CoreError>;
    fn list_recent(&self, limit: usize) -> Result<Vec<AuditEntry>, CoreError>;
}

/// Core domain errors (no external error crates to keep deps at zero).
#[derive(Debug)]
pub enum CoreError {
    InvalidUrl(String),
    InvalidSlug(String),
    InvalidUserEmail,
    AlreadyExists,
    NotFound,
    Repository(String),
}

impl Display for CoreError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            CoreError::InvalidUrl(msg) => write!(f, "invalid url: {}", msg),
            CoreError::InvalidSlug(msg) => write!(f, "invalid slug: {}", msg),
            CoreError::InvalidUserEmail => write!(f, "invalid user email"),
            CoreError::AlreadyExists => write!(f, "resource already exists"),
            CoreError::NotFound => write!(f, "not found"),
            CoreError::Repository(msg) => write!(f, "repository error: {}", msg),
        }
    }
}

impl Error for CoreError {}

/// Return a short about/version line for the binary to print.
pub fn about() -> String {
    // Use env! at compile time; fallback literals kept minimal.
    let pkg = env!("CARGO_PKG_NAME");
    let ver = env!("CARGO_PKG_VERSION");
    format!("{} v{} — domain library loaded", pkg, ver)
}

// Re-export modules when added
pub mod adapters;
pub mod base62;
pub mod service;
pub mod slug;
pub mod validate;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_new_accepts_simple_values() {
        let s = Slug::new("abc123").expect("valid slug");
        assert_eq!(s.as_str(), "abc123");
    }

    #[test]
    fn slug_rejects_empty() {
        let err = Slug::new("").unwrap_err();
        match err {
            CoreError::InvalidSlug(_) => {}
            _ => panic!("expected InvalidSlug"),
        }
    }

    #[test]
    fn user_email_basic_validation() {
        let ok = UserEmail::new("user@example.com");
        assert!(ok.is_ok());

        let bad = UserEmail::new("not-an-email");
        assert!(matches!(bad, Err(CoreError::InvalidUserEmail)));
    }
}
