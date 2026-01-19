use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::SystemTime;

use crate::{
    AuditEntry, AuditRepository, ClickEvent, ClickRepository, CoreError, GroupMember,
    GroupRepository, GroupRole, LinkGroup, LinkRepository, ListOptions, ListResult, ShortLink,
    Slug, UserEmail,
};

/// Simple in-memory repository for tests. Not thread-safe for high concurrency
/// beyond the internal mutex guarding the map.
pub struct InMemoryRepo {
    inner: Mutex<BTreeMap<String, ShortLink>>,
}

/// In-memory group repository for tests.
pub struct InMemoryGroupRepo {
    groups: Mutex<BTreeMap<String, LinkGroup>>,
    members: Mutex<Vec<GroupMember>>,
}

/// In-memory click repository for tests.
pub struct InMemoryClickRepo {
    clicks: Mutex<Vec<ClickEvent>>,
}

/// In-memory audit repository for tests.
pub struct InMemoryAuditRepo {
    entries: Mutex<Vec<AuditEntry>>,
}

impl InMemoryRepo {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(BTreeMap::new()),
        }
    }

    fn key(slug: &Slug) -> String {
        slug.as_str().to_string()
    }
}

impl Default for InMemoryRepo {
    fn default() -> Self {
        Self::new()
    }
}

impl LinkRepository for InMemoryRepo {
    fn get(&self, slug: &Slug) -> Result<Option<ShortLink>, CoreError> {
        let map = self
            .inner
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        Ok(map.get(&Self::key(slug)).cloned())
    }

    fn put(&self, link: ShortLink) -> Result<(), CoreError> {
        let mut map = self
            .inner
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let key = Self::key(&link.slug);
        if map.contains_key(&key) {
            return Err(CoreError::AlreadyExists);
        }
        map.insert(key, link);
        Ok(())
    }

    fn list(&self, limit: usize) -> Result<Vec<ShortLink>, CoreError> {
        let map = self
            .inner
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        Ok(map
            .values()
            .filter(|l| l.deleted_at.is_none())
            .take(limit)
            .cloned()
            .collect())
    }

    fn update(&self, link: &ShortLink) -> Result<(), CoreError> {
        let mut map = self
            .inner
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let key = Self::key(&link.slug);
        if !map.contains_key(&key) {
            return Err(CoreError::NotFound);
        }
        map.insert(key, link.clone());
        Ok(())
    }

    fn increment_click(&self, slug: &Slug) -> Result<(), CoreError> {
        let mut map = self
            .inner
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let key = Self::key(slug);
        match map.get_mut(&key) {
            Some(link) => {
                link.click_count += 1;
                Ok(())
            }
            None => Err(CoreError::NotFound),
        }
    }

    fn list_by_creator(
        &self,
        email: &UserEmail,
        limit: usize,
    ) -> Result<Vec<ShortLink>, CoreError> {
        let map = self
            .inner
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        Ok(map
            .values()
            .filter(|link| link.created_by.as_str() == email.as_str() && link.deleted_at.is_none())
            .take(limit)
            .cloned()
            .collect())
    }

    fn delete(&self, slug: &Slug, deleted_at: SystemTime) -> Result<(), CoreError> {
        let mut map = self
            .inner
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let key = Self::key(slug);
        match map.get_mut(&key) {
            Some(link) => {
                link.deleted_at = Some(deleted_at);
                Ok(())
            }
            None => Err(CoreError::NotFound),
        }
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<ShortLink>, CoreError> {
        let map = self
            .inner
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let q = query.to_lowercase();
        Ok(map
            .values()
            .filter(|link| {
                link.deleted_at.is_none()
                    && (link.slug.as_str().to_lowercase().contains(&q)
                        || link.original_url.to_lowercase().contains(&q)
                        || link
                            .description
                            .as_ref()
                            .is_some_and(|d| d.to_lowercase().contains(&q)))
            })
            .take(limit)
            .cloned()
            .collect())
    }

    fn list_paginated(&self, options: &ListOptions) -> Result<ListResult<ShortLink>, CoreError> {
        let map = self
            .inner
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let mut items: Vec<_> = map
            .values()
            .filter(|link| {
                // Filter deleted
                if !options.include_deleted && link.deleted_at.is_some() {
                    return false;
                }
                // Filter by creator
                if let Some(ref email) = options.created_by {
                    if link.created_by.as_str() != email.as_str() {
                        return false;
                    }
                }
                // Filter by group
                if let Some(ref gid) = options.group_id {
                    if link.group_id.as_ref() != Some(gid) {
                        return false;
                    }
                }
                // Filter by search
                if let Some(ref q) = options.search {
                    let ql = q.to_lowercase();
                    if !link.slug.as_str().to_lowercase().contains(&ql)
                        && !link.original_url.to_lowercase().contains(&ql)
                        && !link
                            .description
                            .as_ref()
                            .is_some_and(|d| d.to_lowercase().contains(&ql))
                    {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();

        // Sort by created_at desc
        items.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        let total = items.len();
        let has_more = options.offset + options.limit < total;
        let items: Vec<_> = items
            .into_iter()
            .skip(options.offset)
            .take(options.limit)
            .collect();

        Ok(ListResult {
            items,
            total,
            has_more,
        })
    }

    fn list_by_group(&self, group_id: &str, limit: usize) -> Result<Vec<ShortLink>, CoreError> {
        let map = self
            .inner
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        Ok(map
            .values()
            .filter(|link| {
                link.group_id.as_ref() == Some(&group_id.to_string()) && link.deleted_at.is_none()
            })
            .take(limit)
            .cloned()
            .collect())
    }

    fn bulk_delete(&self, slugs: &[Slug], deleted_at: SystemTime) -> Result<usize, CoreError> {
        let mut map = self
            .inner
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let mut count = 0;
        for slug in slugs {
            let key = Self::key(slug);
            if let Some(link) = map.get_mut(&key) {
                link.deleted_at = Some(deleted_at);
                count += 1;
            }
        }
        Ok(count)
    }

    fn bulk_update_active(
        &self,
        slugs: &[Slug],
        is_active: bool,
        updated_at: SystemTime,
    ) -> Result<usize, CoreError> {
        let mut map = self
            .inner
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let mut count = 0;
        for slug in slugs {
            let key = Self::key(slug);
            if let Some(link) = map.get_mut(&key) {
                link.is_active = is_active;
                link.updated_at = Some(updated_at);
                count += 1;
            }
        }
        Ok(count)
    }
}

// ============ InMemoryGroupRepo ============

impl InMemoryGroupRepo {
    pub fn new() -> Self {
        Self {
            groups: Mutex::new(BTreeMap::new()),
            members: Mutex::new(Vec::new()),
        }
    }
}

impl Default for InMemoryGroupRepo {
    fn default() -> Self {
        Self::new()
    }
}

impl GroupRepository for InMemoryGroupRepo {
    fn create_group(&self, group: LinkGroup) -> Result<(), CoreError> {
        let mut groups = self
            .groups
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        if groups.contains_key(&group.id) {
            return Err(CoreError::AlreadyExists);
        }
        groups.insert(group.id.clone(), group);
        Ok(())
    }

    fn get_group(&self, id: &str) -> Result<Option<LinkGroup>, CoreError> {
        let groups = self
            .groups
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        Ok(groups.get(id).cloned())
    }

    fn list_groups(&self, user_email: &UserEmail) -> Result<Vec<LinkGroup>, CoreError> {
        let groups = self
            .groups
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let members = self
            .members
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;

        // Find groups where user is a member or creator
        let user_group_ids: Vec<_> = members
            .iter()
            .filter(|m| m.user_email.as_str() == user_email.as_str())
            .map(|m| m.group_id.clone())
            .collect();

        Ok(groups
            .values()
            .filter(|g| {
                g.created_by.as_str() == user_email.as_str() || user_group_ids.contains(&g.id)
            })
            .cloned()
            .collect())
    }

    fn update_group(&self, group: &LinkGroup) -> Result<(), CoreError> {
        let mut groups = self
            .groups
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        if !groups.contains_key(&group.id) {
            return Err(CoreError::NotFound);
        }
        groups.insert(group.id.clone(), group.clone());
        Ok(())
    }

    fn delete_group(&self, id: &str) -> Result<(), CoreError> {
        let mut groups = self
            .groups
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let mut members = self
            .members
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        if groups.remove(id).is_none() {
            return Err(CoreError::NotFound);
        }
        members.retain(|m| m.group_id != id);
        Ok(())
    }

    fn add_member(&self, member: GroupMember) -> Result<(), CoreError> {
        let groups = self
            .groups
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        if !groups.contains_key(&member.group_id) {
            return Err(CoreError::NotFound);
        }
        drop(groups);

        let mut members = self
            .members
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        // Check if already a member
        if members.iter().any(|m| {
            m.group_id == member.group_id && m.user_email.as_str() == member.user_email.as_str()
        }) {
            return Err(CoreError::AlreadyExists);
        }
        members.push(member);
        Ok(())
    }

    fn remove_member(&self, group_id: &str, user_email: &UserEmail) -> Result<(), CoreError> {
        let mut members = self
            .members
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let initial_len = members.len();
        members
            .retain(|m| !(m.group_id == group_id && m.user_email.as_str() == user_email.as_str()));
        if members.len() == initial_len {
            return Err(CoreError::NotFound);
        }
        Ok(())
    }

    fn list_members(&self, group_id: &str) -> Result<Vec<GroupMember>, CoreError> {
        let members = self
            .members
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        Ok(members
            .iter()
            .filter(|m| m.group_id == group_id)
            .cloned()
            .collect())
    }

    fn get_member(
        &self,
        group_id: &str,
        user_email: &UserEmail,
    ) -> Result<Option<GroupMember>, CoreError> {
        let members = self
            .members
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        Ok(members
            .iter()
            .find(|m| m.group_id == group_id && m.user_email.as_str() == user_email.as_str())
            .cloned())
    }

    fn get_user_groups(
        &self,
        user_email: &UserEmail,
    ) -> Result<Vec<(LinkGroup, GroupRole)>, CoreError> {
        let groups = self
            .groups
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let members = self
            .members
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;

        let mut result = Vec::new();

        // Groups where user is creator (Admin role)
        for group in groups.values() {
            if group.created_by.as_str() == user_email.as_str() {
                result.push((group.clone(), GroupRole::Admin));
            }
        }

        // Groups where user is a member
        for member in members.iter() {
            if member.user_email.as_str() == user_email.as_str() {
                if let Some(group) = groups.get(&member.group_id) {
                    // Don't duplicate if already added as creator
                    if group.created_by.as_str() != user_email.as_str() {
                        result.push((group.clone(), member.role));
                    }
                }
            }
        }

        Ok(result)
    }
}

// ============ InMemoryClickRepo ============

impl InMemoryClickRepo {
    pub fn new() -> Self {
        Self {
            clicks: Mutex::new(Vec::new()),
        }
    }
}

impl Default for InMemoryClickRepo {
    fn default() -> Self {
        Self::new()
    }
}

impl ClickRepository for InMemoryClickRepo {
    fn record_click(&self, event: ClickEvent) -> Result<(), CoreError> {
        let mut clicks = self
            .clicks
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        clicks.push(event);
        Ok(())
    }

    fn get_clicks(&self, slug: &Slug, limit: usize) -> Result<Vec<ClickEvent>, CoreError> {
        let clicks = self
            .clicks
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let mut matching: Vec<_> = clicks
            .iter()
            .filter(|c| c.slug.as_str() == slug.as_str())
            .cloned()
            .collect();
        matching.sort_by(|a, b| b.clicked_at.cmp(&a.clicked_at));
        Ok(matching.into_iter().take(limit).collect())
    }

    fn get_click_count_since(&self, slug: &Slug, since: SystemTime) -> Result<u64, CoreError> {
        let clicks = self
            .clicks
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        Ok(clicks
            .iter()
            .filter(|c| c.slug.as_str() == slug.as_str() && c.clicked_at >= since)
            .count() as u64)
    }

    fn get_clicks_by_day(&self, slug: &Slug, days: usize) -> Result<Vec<(String, u64)>, CoreError> {
        use std::collections::HashMap;
        let clicks = self
            .clicks
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;

        let now = SystemTime::now();
        let cutoff = now
            .checked_sub(std::time::Duration::from_secs(days as u64 * 24 * 60 * 60))
            .unwrap_or(SystemTime::UNIX_EPOCH);

        let mut by_day: HashMap<String, u64> = HashMap::new();

        for click in clicks.iter() {
            if click.slug.as_str() != slug.as_str() || click.clicked_at < cutoff {
                continue;
            }
            // Format as YYYY-MM-DD using duration since UNIX_EPOCH
            let secs = click
                .clicked_at
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let days_since_epoch = secs / 86400;
            // Simple day key based on days since epoch (good enough for grouping)
            let day_key = format!("day-{}", days_since_epoch);
            *by_day.entry(day_key).or_insert(0) += 1;
        }

        let mut result: Vec<_> = by_day.into_iter().collect();
        result.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(result)
    }
}

// ============ InMemoryAuditRepo ============

impl InMemoryAuditRepo {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
        }
    }
}

impl Default for InMemoryAuditRepo {
    fn default() -> Self {
        Self::new()
    }
}

impl AuditRepository for InMemoryAuditRepo {
    fn log(&self, entry: AuditEntry) -> Result<(), CoreError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        entries.push(entry);
        Ok(())
    }

    fn list_for_target(
        &self,
        target_type: &str,
        target_id: &str,
        limit: usize,
    ) -> Result<Vec<AuditEntry>, CoreError> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let mut matching: Vec<_> = entries
            .iter()
            .filter(|e| e.target_type == target_type && e.target_id == target_id)
            .cloned()
            .collect();
        matching.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        Ok(matching.into_iter().take(limit).collect())
    }

    fn list_by_actor(
        &self,
        actor_email: &UserEmail,
        limit: usize,
    ) -> Result<Vec<AuditEntry>, CoreError> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let mut matching: Vec<_> = entries
            .iter()
            .filter(|e| e.actor_email.as_str() == actor_email.as_str())
            .cloned()
            .collect();
        matching.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        Ok(matching.into_iter().take(limit).collect())
    }

    fn list_recent(&self, limit: usize) -> Result<Vec<AuditEntry>, CoreError> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| CoreError::Repository("mutex poisoned".into()))?;
        let mut all: Vec<_> = entries.iter().cloned().collect();
        all.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        Ok(all.into_iter().take(limit).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Slug, UserEmail};
    use std::time::SystemTime;

    fn mk_link(slug: &str) -> ShortLink {
        ShortLink::new(
            Slug::new(slug).unwrap(),
            "https://example.com".to_string(),
            SystemTime::UNIX_EPOCH,
            UserEmail::new("user@example.com").unwrap(),
        )
    }

    #[test]
    fn put_get_roundtrip() {
        let repo = InMemoryRepo::new();
        let link = mk_link("abc");
        repo.put(link.clone()).unwrap();
        let got = repo.get(&link.slug).unwrap().unwrap();
        assert_eq!(got.original_url, "https://example.com");
    }

    #[test]
    fn put_rejects_duplicate() {
        let repo = InMemoryRepo::new();
        let link = mk_link("dup");
        repo.put(link.clone()).unwrap();
        let err = repo.put(link).unwrap_err();
        assert!(matches!(err, CoreError::AlreadyExists));
    }

    #[test]
    fn list_honors_limit() {
        let repo = InMemoryRepo::new();
        for i in 0..10 {
            let s = format!("k{}", i);
            let _ = repo.put(mk_link(&s));
        }
        let v = repo.list(5).unwrap();
        assert_eq!(v.len(), 5);
    }
}
