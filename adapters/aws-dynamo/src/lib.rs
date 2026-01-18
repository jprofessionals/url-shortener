//! DynamoDB adapter implementing the `LinkRepository` port.
//!
//! Production-ready implementation backed by `aws-sdk-dynamodb`.
//! - Stores shortlinks in the Shortlinks table with primary key `slug`.
//! - Maintains a monotonic counter item (`name = "global"`) in the Counters table
//!   to support Base62 slug generation in higher layers.
//! - Provides `from_env()` wiring for Lambda/apps using env vars:
//!   `DYNAMO_TABLE_SHORTLINKS`, `DYNAMO_TABLE_COUNTERS`.
//!
//! Notes:
//! - The domain `LinkRepository` trait is synchronous. We bridge to the async AWS
//!   SDK using an internal `tokio::runtime::Runtime` and `block_on`.

use aws_sdk_dynamodb::{Client, types::AttributeValue};
use aws_smithy_types::error::metadata::ProvideErrorMetadata;
use domain::{
    AuditAction, AuditEntry, AuditRepository, ClickEvent, ClickRepository, CoreError,
    GroupMember, GroupRepository, GroupRole, LinkGroup, LinkRepository, ListOptions, ListResult,
    ShortLink, Slug, UserEmail,
};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Configuration for DynamoDB table names.
#[derive(Clone, Debug)]
pub struct DynamoTables {
    pub shortlinks: String,
    pub counters: String,
    pub groups: String,
    pub group_members: String,
    pub clicks: String,
    pub audit: String,
}

impl DynamoTables {
    /// Create with explicit table names.
    pub fn new(shortlinks: impl Into<String>, counters: impl Into<String>) -> Self {
        Self {
            shortlinks: shortlinks.into(),
            counters: counters.into(),
            groups: "Groups".into(),
            group_members: "GroupMembers".into(),
            clicks: "Clicks".into(),
            audit: "AuditLog".into(),
        }
    }

    /// Build from environment variables.
    pub fn from_env() -> Result<Self, CoreError> {
        let shortlinks = std::env::var("DYNAMO_TABLE_SHORTLINKS")
            .map_err(|_| CoreError::Repository("missing DYNAMO_TABLE_SHORTLINKS".into()))?;
        let counters = std::env::var("DYNAMO_TABLE_COUNTERS")
            .map_err(|_| CoreError::Repository("missing DYNAMO_TABLE_COUNTERS".into()))?;
        let groups = std::env::var("DYNAMO_TABLE_GROUPS")
            .unwrap_or_else(|_| "Groups".into());
        let group_members = std::env::var("DYNAMO_TABLE_GROUP_MEMBERS")
            .unwrap_or_else(|_| "GroupMembers".into());
        let clicks = std::env::var("DYNAMO_TABLE_CLICKS")
            .unwrap_or_else(|_| "Clicks".into());
        let audit = std::env::var("DYNAMO_TABLE_AUDIT")
            .unwrap_or_else(|_| "AuditLog".into());
        Ok(Self { shortlinks, counters, groups, group_members, clicks, audit })
    }
}

/// Repository backed by AWS DynamoDB.
///
/// Supports both standalone mode (creates its own Tokio runtime) and Lambda mode
/// (reuses the existing runtime via `Handle::current()`).
#[derive(Clone)]
pub struct DynamoRepo {
    table_shortlinks: String,
    table_counters: String,
    table_groups: String,
    table_group_members: String,
    table_clicks: String,
    table_audit: String,
    client: Client,
    // Optional runtime - None when running inside Lambda (reuses existing runtime)
    rt: Option<std::sync::Arc<tokio::runtime::Runtime>>,
}

impl DynamoRepo {
    /// Create a new repo from explicit table names and an AWS SDK client.
    ///
    /// If called from within a Tokio runtime (e.g., Lambda), reuses the existing runtime.
    /// Otherwise creates a new runtime.
    pub fn with_client(tables: DynamoTables, client: Client) -> Result<Self, CoreError> {
        let rt = Self::maybe_create_runtime()?;
        Ok(Self {
            table_shortlinks: tables.shortlinks,
            table_counters: tables.counters,
            table_groups: tables.groups,
            table_group_members: tables.group_members,
            table_clicks: tables.clicks,
            table_audit: tables.audit,
            client,
            rt,
        })
    }

    /// Construct with table names but create a default AWS SDK client using env/IMDS.
    pub fn new(tables: DynamoTables) -> Result<Self, CoreError> {
        let rt = Self::maybe_create_runtime()?;
        let conf = Self::block_on_with_rt(&rt, aws_config::load_from_env());
        let client = Client::new(&conf);
        Ok(Self {
            table_shortlinks: tables.shortlinks,
            table_counters: tables.counters,
            table_groups: tables.groups,
            table_group_members: tables.group_members,
            table_clicks: tables.clicks,
            table_audit: tables.audit,
            client,
            rt,
        })
    }

    /// Construct from environment variables expected by the server:
    /// - `DYNAMO_TABLE_SHORTLINKS`
    /// - `DYNAMO_TABLE_COUNTERS`
    /// - `DYNAMO_TABLE_GROUPS` (optional, defaults to "Groups")
    /// - `DYNAMO_TABLE_GROUP_MEMBERS` (optional, defaults to "GroupMembers")
    /// - `DYNAMO_TABLE_CLICKS` (optional, defaults to "Clicks")
    /// - `DYNAMO_TABLE_AUDIT` (optional, defaults to "AuditLog")
    pub fn from_env() -> Result<Self, CoreError> {
        let tables = DynamoTables::from_env()?;
        Self::new(tables)
    }

    /// Check if we're inside a Tokio runtime. If yes, return None (reuse existing).
    /// If no, create a new runtime.
    fn maybe_create_runtime() -> Result<Option<std::sync::Arc<tokio::runtime::Runtime>>, CoreError> {
        if tokio::runtime::Handle::try_current().is_ok() {
            // Already inside a runtime (e.g., Lambda) - don't create another
            Ok(None)
        } else {
            // Standalone mode - create our own runtime
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(|e| CoreError::Repository(format!("tokio runtime init: {e}")))?;
            Ok(Some(std::sync::Arc::new(rt)))
        }
    }

    /// Run an async future, using either our owned runtime or the current runtime.
    fn block_on<F: std::future::Future>(&self, fut: F) -> F::Output {
        Self::block_on_with_rt(&self.rt, fut)
    }

    fn block_on_with_rt<F: std::future::Future>(rt: &Option<std::sync::Arc<tokio::runtime::Runtime>>, fut: F) -> F::Output {
        match rt {
            Some(rt) => rt.block_on(fut),
            None => {
                // We're inside an existing runtime - use block_in_place + Handle::current()
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(fut)
                })
            }
        }
    }

    /// Atomically increment the global counter and return the new value.
    pub fn increment_global_counter(&self) -> Result<u64, CoreError> {
        use aws_sdk_dynamodb::types::ReturnValue;
        let table = self.table_counters.clone();
        let fut = async {
            self.client.update_item()
                .table_name(table)
                .key("name", AttributeValue::S("global".into()))
                .update_expression("ADD #v :one")
                .expression_attribute_names("#v", "value")
                .expression_attribute_values(":one", AttributeValue::N("1".into()))
                .return_values(ReturnValue::UpdatedNew)
                .send()
                .await
        };
        let out = self.block_on(fut).map_err(map_sdk_err)?;
        let attrs = out.attributes().ok_or_else(|| CoreError::Repository("update returned no attributes".into()))?;
        let v = attrs.get("value").and_then(|av| av.as_n().ok())
            .ok_or_else(|| CoreError::Repository("counter missing value".into()))?;
        v.parse::<u64>().map_err(|e| CoreError::Repository(format!("parse counter: {e}")))
    }
}

impl LinkRepository for DynamoRepo {
    fn get(&self, slug: &Slug) -> Result<Option<ShortLink>, CoreError> {
        let table = self.table_shortlinks.clone();
        let key_slug = slug.as_str().to_string();
        let fut = async {
            self.client.get_item()
                .table_name(table)
                .key("slug", AttributeValue::S(key_slug))
                .send().await
        };
        let out = self.block_on(fut).map_err(map_sdk_err)?;
        if let Some(item) = out.item() { Ok(Some(item_to_domain(item)?)) } else { Ok(None) }
    }

    fn put(&self, link: ShortLink) -> Result<(), CoreError> {
        // Always use a conditional put to avoid accidental overwrite
        let table = self.table_shortlinks.clone();
        let item = domain_to_item(&link);
        let fut = async {
            self.client.put_item()
                .table_name(table)
                .set_item(Some(item))
                .condition_expression("attribute_not_exists(#s)")
                .expression_attribute_names("#s", "slug")
                .send().await
        };
        self.block_on(fut).map_err(|e| match e.as_service_error() {
            Some(se) if se.code() == Some("ConditionalCheckFailedException") => CoreError::AlreadyExists,
            _ => map_sdk_err(e),
        })?;
        Ok(())
    }

    fn list(&self, limit: usize) -> Result<Vec<ShortLink>, CoreError> {
        let table = self.table_shortlinks.clone();
        let lim = limit as i32;
        let fut = async {
            self.client.scan().table_name(table).limit(lim).send().await
        };
        let out = self.block_on(fut).map_err(map_sdk_err)?;
        let mut res = Vec::new();
        for it in out.items().iter() {
            if let Ok(sl) = item_to_domain(it) { res.push(sl); }
        }
        Ok(res)
    }

    fn update(&self, link: &ShortLink) -> Result<(), CoreError> {
        let table = self.table_shortlinks.clone();
        let slug = link.slug.as_str().to_string();
        let original_url = link.original_url.clone();
        let is_active = link.is_active;
        let updated_at = link.updated_at.map(system_time_to_secs);
        let expires_at = link.expires_at.map(system_time_to_secs);

        let fut = async {
            let mut req = self.client.update_item()
                .table_name(table)
                .key("slug", AttributeValue::S(slug))
                .update_expression("SET original_url = :url, is_active = :active, updated_at = :ts, expires_at = :exp")
                .expression_attribute_values(":url", AttributeValue::S(original_url))
                .expression_attribute_values(":active", AttributeValue::Bool(is_active))
                .condition_expression("attribute_exists(slug)");

            if let Some(ts) = updated_at {
                req = req.expression_attribute_values(":ts", AttributeValue::N(ts.to_string()));
            } else {
                req = req.expression_attribute_values(":ts", AttributeValue::Null(true));
            }

            if let Some(exp) = expires_at {
                req = req.expression_attribute_values(":exp", AttributeValue::N(exp.to_string()));
            } else {
                req = req.expression_attribute_values(":exp", AttributeValue::Null(true));
            }

            req.send().await
        };
        self.block_on(fut).map_err(|e| match e.as_service_error() {
            Some(se) if se.code() == Some("ConditionalCheckFailedException") => CoreError::NotFound,
            _ => map_sdk_err(e),
        })?;
        Ok(())
    }

    fn increment_click(&self, slug: &Slug) -> Result<(), CoreError> {
        let table = self.table_shortlinks.clone();
        let slug_str = slug.as_str().to_string();

        let fut = async {
            self.client.update_item()
                .table_name(table)
                .key("slug", AttributeValue::S(slug_str))
                .update_expression("SET click_count = if_not_exists(click_count, :zero) + :inc")
                .expression_attribute_values(":zero", AttributeValue::N("0".into()))
                .expression_attribute_values(":inc", AttributeValue::N("1".into()))
                .condition_expression("attribute_exists(slug)")
                .send()
                .await
        };
        self.block_on(fut).map_err(|e| match e.as_service_error() {
            Some(se) if se.code() == Some("ConditionalCheckFailedException") => CoreError::NotFound,
            _ => map_sdk_err(e),
        })?;
        Ok(())
    }

    fn list_by_creator(&self, email: &UserEmail, limit: usize) -> Result<Vec<ShortLink>, CoreError> {
        let table = self.table_shortlinks.clone();
        let lim = limit as i32;
        let email_str = email.as_str().to_string();

        let fut = async {
            self.client.scan()
                .table_name(table)
                .limit(lim)
                .filter_expression("created_by = :email AND attribute_not_exists(deleted_at)")
                .expression_attribute_values(":email", AttributeValue::S(email_str))
                .send()
                .await
        };
        let out = self.block_on(fut).map_err(map_sdk_err)?;
        let mut res = Vec::new();
        for it in out.items().iter() {
            if let Ok(sl) = item_to_domain(it) { res.push(sl); }
        }
        Ok(res)
    }

    fn delete(&self, slug: &Slug, deleted_at: SystemTime) -> Result<(), CoreError> {
        let table = self.table_shortlinks.clone();
        let slug_str = slug.as_str().to_string();
        let deleted_at_secs = system_time_to_secs(deleted_at);

        let fut = async {
            self.client.update_item()
                .table_name(table)
                .key("slug", AttributeValue::S(slug_str))
                .update_expression("SET deleted_at = :ts")
                .expression_attribute_values(":ts", AttributeValue::N(deleted_at_secs.to_string()))
                .condition_expression("attribute_exists(slug) AND attribute_not_exists(deleted_at)")
                .send()
                .await
        };
        self.block_on(fut).map_err(|e| match e.as_service_error() {
            Some(se) if se.code() == Some("ConditionalCheckFailedException") => CoreError::NotFound,
            _ => map_sdk_err(e),
        })?;
        Ok(())
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<ShortLink>, CoreError> {
        let table = self.table_shortlinks.clone();
        let lim = limit as i32;
        let q = query.to_lowercase();

        let fut = async {
            self.client.scan()
                .table_name(table)
                .limit(lim)
                .filter_expression("attribute_not_exists(deleted_at) AND (contains(#slug, :q) OR contains(original_url, :q) OR contains(description, :q))")
                .expression_attribute_names("#slug", "slug")
                .expression_attribute_values(":q", AttributeValue::S(q))
                .send()
                .await
        };
        let out = self.block_on(fut).map_err(map_sdk_err)?;
        let mut res = Vec::new();
        for it in out.items().iter() {
            if let Ok(sl) = item_to_domain(it) { res.push(sl); }
        }
        Ok(res)
    }

    fn list_paginated(&self, options: &ListOptions) -> Result<ListResult<ShortLink>, CoreError> {
        // DynamoDB doesn't support offset-based pagination well, so we fetch all and filter
        // For production at scale, consider using a GSI or different pagination strategy
        let table = self.table_shortlinks.clone();
        let mut filter_parts = Vec::new();
        let mut expr_values: HashMap<String, AttributeValue> = HashMap::new();
        let mut expr_names: HashMap<String, String> = HashMap::new();

        if !options.include_deleted {
            filter_parts.push("attribute_not_exists(deleted_at)".to_string());
        }
        if let Some(ref email) = options.created_by {
            filter_parts.push("created_by = :email".to_string());
            expr_values.insert(":email".into(), AttributeValue::S(email.as_str().to_string()));
        }
        if let Some(ref gid) = options.group_id {
            filter_parts.push("group_id = :gid".to_string());
            expr_values.insert(":gid".into(), AttributeValue::S(gid.clone()));
        }
        if let Some(ref q) = options.search {
            filter_parts.push("(contains(#slug, :q) OR contains(original_url, :q) OR contains(description, :q))".to_string());
            expr_names.insert("#slug".into(), "slug".into());
            expr_values.insert(":q".into(), AttributeValue::S(q.to_lowercase()));
        }

        let filter_expr = if filter_parts.is_empty() {
            None
        } else {
            Some(filter_parts.join(" AND "))
        };

        let fut = async {
            let mut req = self.client.scan().table_name(table);
            if let Some(expr) = filter_expr {
                req = req.filter_expression(expr);
            }
            for (k, v) in expr_values {
                req = req.expression_attribute_values(k, v);
            }
            for (k, v) in expr_names {
                req = req.expression_attribute_names(k, v);
            }
            req.send().await
        };
        let out = self.block_on(fut).map_err(map_sdk_err)?;
        let mut all_items: Vec<ShortLink> = out.items().iter()
            .filter_map(|it| item_to_domain(it).ok())
            .collect();

        // Sort by created_at desc
        all_items.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        let total = all_items.len();
        let has_more = options.offset + options.limit < total;
        let items: Vec<_> = all_items.into_iter()
            .skip(options.offset)
            .take(options.limit)
            .collect();

        Ok(ListResult { items, total, has_more })
    }

    fn list_by_group(&self, group_id: &str, limit: usize) -> Result<Vec<ShortLink>, CoreError> {
        let table = self.table_shortlinks.clone();
        let lim = limit as i32;

        let fut = async {
            self.client.scan()
                .table_name(table)
                .limit(lim)
                .filter_expression("group_id = :gid AND attribute_not_exists(deleted_at)")
                .expression_attribute_values(":gid", AttributeValue::S(group_id.to_string()))
                .send()
                .await
        };
        let out = self.block_on(fut).map_err(map_sdk_err)?;
        let mut res = Vec::new();
        for it in out.items().iter() {
            if let Ok(sl) = item_to_domain(it) { res.push(sl); }
        }
        Ok(res)
    }

    fn bulk_delete(&self, slugs: &[Slug], deleted_at: SystemTime) -> Result<usize, CoreError> {
        let deleted_at_secs = system_time_to_secs(deleted_at);
        let mut count = 0;
        for slug in slugs {
            let table = self.table_shortlinks.clone();
            let slug_str = slug.as_str().to_string();
            let fut = async {
                self.client.update_item()
                    .table_name(table)
                    .key("slug", AttributeValue::S(slug_str))
                    .update_expression("SET deleted_at = :ts")
                    .expression_attribute_values(":ts", AttributeValue::N(deleted_at_secs.to_string()))
                    .condition_expression("attribute_exists(slug) AND attribute_not_exists(deleted_at)")
                    .send()
                    .await
            };
            if self.block_on(fut).is_ok() {
                count += 1;
            }
        }
        Ok(count)
    }

    fn bulk_update_active(&self, slugs: &[Slug], is_active: bool, updated_at: SystemTime) -> Result<usize, CoreError> {
        let updated_at_secs = system_time_to_secs(updated_at);
        let mut count = 0;
        for slug in slugs {
            let table = self.table_shortlinks.clone();
            let slug_str = slug.as_str().to_string();
            let fut = async {
                self.client.update_item()
                    .table_name(table)
                    .key("slug", AttributeValue::S(slug_str))
                    .update_expression("SET is_active = :active, updated_at = :ts")
                    .expression_attribute_values(":active", AttributeValue::Bool(is_active))
                    .expression_attribute_values(":ts", AttributeValue::N(updated_at_secs.to_string()))
                    .condition_expression("attribute_exists(slug)")
                    .send()
                    .await
            };
            if self.block_on(fut).is_ok() {
                count += 1;
            }
        }
        Ok(count)
    }
}

fn map_sdk_err<E: ProvideErrorMetadata + std::fmt::Display>(e: E) -> CoreError {
    if let Some(code) = e.code() {
        if code == "ResourceNotFoundException" { return CoreError::Repository("missing table".into()); }
    }
    CoreError::Repository(format!("dynamo error: {e}"))
}

fn system_time_to_secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn secs_to_system_time(secs: u64) -> SystemTime {
    UNIX_EPOCH + std::time::Duration::from_secs(secs)
}

fn domain_to_item(link: &ShortLink) -> HashMap<String, AttributeValue> {
    let mut m = HashMap::new();
    m.insert("slug".into(), AttributeValue::S(link.slug.as_str().to_string()));
    m.insert("original_url".into(), AttributeValue::S(link.original_url.clone()));
    m.insert("created_at".into(), AttributeValue::N(system_time_to_secs(link.created_at).to_string()));
    m.insert("created_by".into(), AttributeValue::S(link.created_by.as_str().to_string()));
    m.insert("click_count".into(), AttributeValue::N(link.click_count.to_string()));
    m.insert("is_active".into(), AttributeValue::Bool(link.is_active));
    if let Some(updated_at) = link.updated_at {
        m.insert("updated_at".into(), AttributeValue::N(system_time_to_secs(updated_at).to_string()));
    }
    if let Some(expires_at) = link.expires_at {
        m.insert("expires_at".into(), AttributeValue::N(system_time_to_secs(expires_at).to_string()));
    }
    if let Some(ref description) = link.description {
        m.insert("description".into(), AttributeValue::S(description.clone()));
    }
    if let Some(activate_at) = link.activate_at {
        m.insert("activate_at".into(), AttributeValue::N(system_time_to_secs(activate_at).to_string()));
    }
    if let Some(redirect_delay) = link.redirect_delay {
        m.insert("redirect_delay".into(), AttributeValue::N(redirect_delay.to_string()));
    }
    if let Some(deleted_at) = link.deleted_at {
        m.insert("deleted_at".into(), AttributeValue::N(system_time_to_secs(deleted_at).to_string()));
    }
    if let Some(ref group_id) = link.group_id {
        m.insert("group_id".into(), AttributeValue::S(group_id.clone()));
    }
    m
}

fn item_to_domain(item: &HashMap<String, AttributeValue>) -> Result<ShortLink, CoreError> {
    let slug = item.get("slug").and_then(|v| v.as_s().ok()).ok_or_else(|| CoreError::Repository("item missing slug".into()))?;
    let original_url = item.get("original_url").and_then(|v| v.as_s().ok()).ok_or_else(|| CoreError::Repository("item missing original_url".into()))?.to_string();
    let created_at = item.get("created_at").and_then(|v| v.as_n().ok()).ok_or_else(|| CoreError::Repository("item missing created_at".into()))?;
    let created_by = item.get("created_by").and_then(|v| v.as_s().ok()).ok_or_else(|| CoreError::Repository("item missing created_by".into()))?;

    // Fields with backward-compatible defaults
    let click_count = item.get("click_count")
        .and_then(|v| v.as_n().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let is_active = item.get("is_active")
        .and_then(|v| v.as_bool().ok())
        .copied()
        .unwrap_or(true);
    let updated_at = item.get("updated_at")
        .and_then(|v| v.as_n().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .map(secs_to_system_time);
    let expires_at = item.get("expires_at")
        .and_then(|v| v.as_n().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .map(secs_to_system_time);
    let description = item.get("description")
        .and_then(|v| v.as_s().ok())
        .map(|s| s.to_string());
    let activate_at = item.get("activate_at")
        .and_then(|v| v.as_n().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .map(secs_to_system_time);
    let redirect_delay = item.get("redirect_delay")
        .and_then(|v| v.as_n().ok())
        .and_then(|s| s.parse::<u32>().ok());
    let deleted_at = item.get("deleted_at")
        .and_then(|v| v.as_n().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .map(secs_to_system_time);
    let group_id = item.get("group_id")
        .and_then(|v| v.as_s().ok())
        .map(|s| s.to_string());

    let slug = Slug::new(slug.to_string()).map_err(|e| CoreError::Repository(format!("bad slug in item: {e}")))?;
    let created_by = UserEmail::new(created_by.to_string()).map_err(|_| CoreError::Repository("bad created_by".into()))?;
    let created_at = created_at.parse::<u64>().map(secs_to_system_time)
        .map_err(|e| CoreError::Repository(format!("bad created_at: {e}")))?;

    Ok(ShortLink { slug, original_url, created_at, created_by, click_count, is_active, updated_at, expires_at, description, activate_at, redirect_delay, deleted_at, group_id })
}

// -------------------------
// Group Repository
// -------------------------

fn group_to_item(group: &LinkGroup) -> HashMap<String, AttributeValue> {
    let mut m = HashMap::new();
    m.insert("id".into(), AttributeValue::S(group.id.clone()));
    m.insert("name".into(), AttributeValue::S(group.name.clone()));
    m.insert("created_at".into(), AttributeValue::N(system_time_to_secs(group.created_at).to_string()));
    m.insert("created_by".into(), AttributeValue::S(group.created_by.as_str().to_string()));
    if let Some(ref desc) = group.description {
        m.insert("description".into(), AttributeValue::S(desc.clone()));
    }
    m
}

fn item_to_group(item: &HashMap<String, AttributeValue>) -> Result<LinkGroup, CoreError> {
    let id = item.get("id").and_then(|v| v.as_s().ok())
        .ok_or_else(|| CoreError::Repository("group missing id".into()))?.to_string();
    let name = item.get("name").and_then(|v| v.as_s().ok())
        .ok_or_else(|| CoreError::Repository("group missing name".into()))?.to_string();
    let created_at = item.get("created_at").and_then(|v| v.as_n().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or_else(|| CoreError::Repository("group missing created_at".into()))?;
    let created_by = item.get("created_by").and_then(|v| v.as_s().ok())
        .ok_or_else(|| CoreError::Repository("group missing created_by".into()))?;
    let description = item.get("description")
        .and_then(|v| v.as_s().ok())
        .map(|s| s.to_string());

    let created_by = UserEmail::new(created_by.to_string())
        .map_err(|_| CoreError::Repository("bad created_by".into()))?;

    Ok(LinkGroup {
        id,
        name,
        description,
        created_at: secs_to_system_time(created_at),
        created_by,
    })
}

fn member_to_item(member: &GroupMember) -> HashMap<String, AttributeValue> {
    let mut m = HashMap::new();
    m.insert("group_id".into(), AttributeValue::S(member.group_id.clone()));
    m.insert("user_email".into(), AttributeValue::S(member.user_email.as_str().to_string()));
    m.insert("role".into(), AttributeValue::S(member.role.as_str().to_string()));
    m.insert("added_at".into(), AttributeValue::N(system_time_to_secs(member.added_at).to_string()));
    m.insert("added_by".into(), AttributeValue::S(member.added_by.as_str().to_string()));
    m
}

fn item_to_member(item: &HashMap<String, AttributeValue>) -> Result<GroupMember, CoreError> {
    let group_id = item.get("group_id").and_then(|v| v.as_s().ok())
        .ok_or_else(|| CoreError::Repository("member missing group_id".into()))?.to_string();
    let user_email = item.get("user_email").and_then(|v| v.as_s().ok())
        .ok_or_else(|| CoreError::Repository("member missing user_email".into()))?;
    let role_str = item.get("role").and_then(|v| v.as_s().ok())
        .ok_or_else(|| CoreError::Repository("member missing role".into()))?;
    let added_at = item.get("added_at").and_then(|v| v.as_n().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or_else(|| CoreError::Repository("member missing added_at".into()))?;
    let added_by = item.get("added_by").and_then(|v| v.as_s().ok())
        .ok_or_else(|| CoreError::Repository("member missing added_by".into()))?;

    let user_email = UserEmail::new(user_email.to_string())
        .map_err(|_| CoreError::Repository("bad user_email".into()))?;
    let added_by = UserEmail::new(added_by.to_string())
        .map_err(|_| CoreError::Repository("bad added_by".into()))?;
    let role = GroupRole::from_str(role_str)
        .ok_or_else(|| CoreError::Repository("bad role".into()))?;

    Ok(GroupMember {
        group_id,
        user_email,
        role,
        added_at: secs_to_system_time(added_at),
        added_by,
    })
}

impl GroupRepository for DynamoRepo {
    fn create_group(&self, group: LinkGroup) -> Result<(), CoreError> {
        let table = self.table_groups.clone();
        let item = group_to_item(&group);
        let fut = async {
            self.client.put_item()
                .table_name(table)
                .set_item(Some(item))
                .condition_expression("attribute_not_exists(id)")
                .send().await
        };
        self.block_on(fut).map_err(|e| match e.as_service_error() {
            Some(se) if se.code() == Some("ConditionalCheckFailedException") => CoreError::AlreadyExists,
            _ => map_sdk_err(e),
        })?;
        Ok(())
    }

    fn get_group(&self, id: &str) -> Result<Option<LinkGroup>, CoreError> {
        let table = self.table_groups.clone();
        let id_str = id.to_string();
        let fut = async {
            self.client.get_item()
                .table_name(table)
                .key("id", AttributeValue::S(id_str))
                .send().await
        };
        let out = self.block_on(fut).map_err(map_sdk_err)?;
        if let Some(item) = out.item() {
            Ok(Some(item_to_group(item)?))
        } else {
            Ok(None)
        }
    }

    fn list_groups(&self, _user_email: &UserEmail) -> Result<Vec<LinkGroup>, CoreError> {
        // List all groups (filtering by membership is done via get_user_groups)
        let table = self.table_groups.clone();
        let fut = async {
            self.client.scan()
                .table_name(table)
                .send().await
        };
        let out = self.block_on(fut).map_err(map_sdk_err)?;
        let mut res = Vec::new();
        for it in out.items().iter() {
            if let Ok(g) = item_to_group(it) {
                res.push(g);
            }
        }
        Ok(res)
    }

    fn update_group(&self, group: &LinkGroup) -> Result<(), CoreError> {
        let table = self.table_groups.clone();
        let id = group.id.clone();
        let name = group.name.clone();
        let desc = group.description.clone();

        let fut = async {
            let mut req = self.client.update_item()
                .table_name(table)
                .key("id", AttributeValue::S(id))
                .update_expression("SET #n = :name, description = :desc")
                .expression_attribute_names("#n", "name")
                .expression_attribute_values(":name", AttributeValue::S(name))
                .condition_expression("attribute_exists(id)");

            if let Some(d) = desc {
                req = req.expression_attribute_values(":desc", AttributeValue::S(d));
            } else {
                req = req.expression_attribute_values(":desc", AttributeValue::Null(true));
            }

            req.send().await
        };
        self.block_on(fut).map_err(|e| match e.as_service_error() {
            Some(se) if se.code() == Some("ConditionalCheckFailedException") => CoreError::NotFound,
            _ => map_sdk_err(e),
        })?;
        Ok(())
    }

    fn delete_group(&self, id: &str) -> Result<(), CoreError> {
        let table = self.table_groups.clone();
        let id_str = id.to_string();
        let fut = async {
            self.client.delete_item()
                .table_name(table)
                .key("id", AttributeValue::S(id_str))
                .condition_expression("attribute_exists(id)")
                .send().await
        };
        self.block_on(fut).map_err(|e| match e.as_service_error() {
            Some(se) if se.code() == Some("ConditionalCheckFailedException") => CoreError::NotFound,
            _ => map_sdk_err(e),
        })?;
        Ok(())
    }

    fn add_member(&self, member: GroupMember) -> Result<(), CoreError> {
        let table = self.table_group_members.clone();
        let item = member_to_item(&member);
        let fut = async {
            self.client.put_item()
                .table_name(table)
                .set_item(Some(item))
                .send().await
        };
        self.block_on(fut).map_err(map_sdk_err)?;
        Ok(())
    }

    fn remove_member(&self, group_id: &str, user_email: &UserEmail) -> Result<(), CoreError> {
        let table = self.table_group_members.clone();
        let gid = group_id.to_string();
        let email = user_email.as_str().to_string();
        let fut = async {
            self.client.delete_item()
                .table_name(table)
                .key("group_id", AttributeValue::S(gid))
                .key("user_email", AttributeValue::S(email))
                .send().await
        };
        self.block_on(fut).map_err(map_sdk_err)?;
        Ok(())
    }

    fn list_members(&self, group_id: &str) -> Result<Vec<GroupMember>, CoreError> {
        let table = self.table_group_members.clone();
        let gid = group_id.to_string();
        let fut = async {
            self.client.query()
                .table_name(table)
                .key_condition_expression("group_id = :gid")
                .expression_attribute_values(":gid", AttributeValue::S(gid))
                .send().await
        };
        let out = self.block_on(fut).map_err(map_sdk_err)?;
        let mut res = Vec::new();
        for it in out.items().iter() {
            if let Ok(m) = item_to_member(it) {
                res.push(m);
            }
        }
        Ok(res)
    }

    fn get_member(&self, group_id: &str, user_email: &UserEmail) -> Result<Option<GroupMember>, CoreError> {
        let table = self.table_group_members.clone();
        let gid = group_id.to_string();
        let email = user_email.as_str().to_string();
        let fut = async {
            self.client.get_item()
                .table_name(table)
                .key("group_id", AttributeValue::S(gid))
                .key("user_email", AttributeValue::S(email))
                .send().await
        };
        let out = self.block_on(fut).map_err(map_sdk_err)?;
        if let Some(item) = out.item() {
            Ok(Some(item_to_member(item)?))
        } else {
            Ok(None)
        }
    }

    fn get_user_groups(&self, user_email: &UserEmail) -> Result<Vec<(LinkGroup, GroupRole)>, CoreError> {
        // First, get all memberships for this user via scan (GSI on user_email would be better)
        let table = self.table_group_members.clone();
        let email = user_email.as_str().to_string();
        let fut = async {
            self.client.scan()
                .table_name(table)
                .filter_expression("user_email = :email")
                .expression_attribute_values(":email", AttributeValue::S(email))
                .send().await
        };
        let out = self.block_on(fut).map_err(map_sdk_err)?;

        let mut results = Vec::new();
        for item in out.items().iter() {
            if let Ok(member) = item_to_member(item) {
                if let Ok(Some(group)) = self.get_group(&member.group_id) {
                    results.push((group, member.role));
                }
            }
        }
        Ok(results)
    }
}

// -------------------------
// Click Repository
// -------------------------

fn click_to_item(event: &ClickEvent) -> HashMap<String, AttributeValue> {
    let mut m = HashMap::new();
    m.insert("slug".into(), AttributeValue::S(event.slug.as_str().to_string()));
    m.insert("clicked_at".into(), AttributeValue::N(system_time_to_secs(event.clicked_at).to_string()));
    if let Some(ref ua) = event.user_agent {
        m.insert("user_agent".into(), AttributeValue::S(ua.clone()));
    }
    if let Some(ref referrer) = event.referrer {
        m.insert("referrer".into(), AttributeValue::S(referrer.clone()));
    }
    if let Some(ref country) = event.country {
        m.insert("country".into(), AttributeValue::S(country.clone()));
    }
    m
}

fn item_to_click(item: &HashMap<String, AttributeValue>) -> Result<ClickEvent, CoreError> {
    let slug = item.get("slug").and_then(|v| v.as_s().ok())
        .ok_or_else(|| CoreError::Repository("click missing slug".into()))?;
    let clicked_at = item.get("clicked_at").and_then(|v| v.as_n().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or_else(|| CoreError::Repository("click missing clicked_at".into()))?;
    let user_agent = item.get("user_agent")
        .and_then(|v| v.as_s().ok())
        .map(|s| s.to_string());
    let referrer = item.get("referrer")
        .and_then(|v| v.as_s().ok())
        .map(|s| s.to_string());
    let country = item.get("country")
        .and_then(|v| v.as_s().ok())
        .map(|s| s.to_string());

    let slug = Slug::new(slug.to_string())
        .map_err(|e| CoreError::Repository(format!("bad slug: {e}")))?;

    Ok(ClickEvent {
        slug,
        clicked_at: secs_to_system_time(clicked_at),
        user_agent,
        referrer,
        country,
    })
}

impl ClickRepository for DynamoRepo {
    fn record_click(&self, event: ClickEvent) -> Result<(), CoreError> {
        let table = self.table_clicks.clone();
        let item = click_to_item(&event);
        let fut = async {
            self.client.put_item()
                .table_name(table)
                .set_item(Some(item))
                .send().await
        };
        self.block_on(fut).map_err(map_sdk_err)?;
        Ok(())
    }

    fn get_clicks(&self, slug: &Slug, limit: usize) -> Result<Vec<ClickEvent>, CoreError> {
        let table = self.table_clicks.clone();
        let slug_str = slug.as_str().to_string();
        let lim = limit as i32;
        let fut = async {
            self.client.query()
                .table_name(table)
                .key_condition_expression("#slug = :slug")
                .expression_attribute_names("#slug", "slug")
                .expression_attribute_values(":slug", AttributeValue::S(slug_str))
                .scan_index_forward(false) // Most recent first
                .limit(lim)
                .send().await
        };
        let out = self.block_on(fut).map_err(map_sdk_err)?;
        let mut res = Vec::new();
        for it in out.items().iter() {
            if let Ok(c) = item_to_click(it) {
                res.push(c);
            }
        }
        Ok(res)
    }

    fn get_click_count_since(&self, slug: &Slug, since: SystemTime) -> Result<u64, CoreError> {
        let table = self.table_clicks.clone();
        let slug_str = slug.as_str().to_string();
        let since_secs = system_time_to_secs(since);
        let fut = async {
            self.client.query()
                .table_name(table)
                .key_condition_expression("#slug = :slug AND clicked_at >= :since")
                .expression_attribute_names("#slug", "slug")
                .expression_attribute_values(":slug", AttributeValue::S(slug_str))
                .expression_attribute_values(":since", AttributeValue::N(since_secs.to_string()))
                .select(aws_sdk_dynamodb::types::Select::Count)
                .send().await
        };
        let out = self.block_on(fut).map_err(map_sdk_err)?;
        Ok(out.count() as u64)
    }

    fn get_clicks_by_day(&self, slug: &Slug, days: usize) -> Result<Vec<(String, u64)>, CoreError> {
        // Get all clicks for this slug, then aggregate by day
        let table = self.table_clicks.clone();
        let slug_str = slug.as_str().to_string();
        let fut = async {
            self.client.query()
                .table_name(table)
                .key_condition_expression("#slug = :slug")
                .expression_attribute_names("#slug", "slug")
                .expression_attribute_values(":slug", AttributeValue::S(slug_str))
                .send().await
        };
        let out = self.block_on(fut).map_err(map_sdk_err)?;

        // Group by day
        let mut day_counts: HashMap<String, u64> = HashMap::new();
        for item in out.items().iter() {
            if let Some(clicked_at) = item.get("clicked_at")
                .and_then(|v| v.as_n().ok())
                .and_then(|s| s.parse::<u64>().ok())
            {
                // Convert to date string (YYYY-MM-DD)
                let secs = clicked_at;
                let days_since_epoch = secs / 86400;
                let day_key = format!("day-{}", days_since_epoch);
                *day_counts.entry(day_key).or_insert(0) += 1;
            }
        }

        // Sort by day and take last N days
        let mut results: Vec<_> = day_counts.into_iter().collect();
        results.sort_by(|a, b| b.0.cmp(&a.0)); // Descending
        results.truncate(days);

        Ok(results)
    }
}

// -------------------------
// Audit Repository
// -------------------------

fn audit_to_item(entry: &AuditEntry) -> HashMap<String, AttributeValue> {
    let mut m = HashMap::new();
    m.insert("id".into(), AttributeValue::S(entry.id.clone()));
    m.insert("timestamp".into(), AttributeValue::N(system_time_to_secs(entry.timestamp).to_string()));
    m.insert("actor_email".into(), AttributeValue::S(entry.actor_email.as_str().to_string()));
    m.insert("action".into(), AttributeValue::S(entry.action.as_str().to_string()));
    m.insert("target_type".into(), AttributeValue::S(entry.target_type.clone()));
    m.insert("target_id".into(), AttributeValue::S(entry.target_id.clone()));
    if let Some(ref changes) = entry.changes {
        m.insert("changes".into(), AttributeValue::S(changes.clone()));
    }
    m
}

fn item_to_audit(item: &HashMap<String, AttributeValue>) -> Result<AuditEntry, CoreError> {
    let id = item.get("id").and_then(|v| v.as_s().ok())
        .ok_or_else(|| CoreError::Repository("audit missing id".into()))?.to_string();
    let timestamp = item.get("timestamp").and_then(|v| v.as_n().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or_else(|| CoreError::Repository("audit missing timestamp".into()))?;
    let actor_email = item.get("actor_email").and_then(|v| v.as_s().ok())
        .ok_or_else(|| CoreError::Repository("audit missing actor_email".into()))?;
    let action_str = item.get("action").and_then(|v| v.as_s().ok())
        .ok_or_else(|| CoreError::Repository("audit missing action".into()))?;
    let target_type = item.get("target_type").and_then(|v| v.as_s().ok())
        .ok_or_else(|| CoreError::Repository("audit missing target_type".into()))?.to_string();
    let target_id = item.get("target_id").and_then(|v| v.as_s().ok())
        .ok_or_else(|| CoreError::Repository("audit missing target_id".into()))?.to_string();
    let changes = item.get("changes")
        .and_then(|v| v.as_s().ok())
        .map(|s| s.to_string());

    let actor_email = UserEmail::new(actor_email.to_string())
        .map_err(|_| CoreError::Repository("bad actor_email".into()))?;
    let action = AuditAction::from_str(action_str)
        .ok_or_else(|| CoreError::Repository("bad action".into()))?;

    Ok(AuditEntry {
        id,
        timestamp: secs_to_system_time(timestamp),
        actor_email,
        action,
        target_type,
        target_id,
        changes,
    })
}

impl AuditRepository for DynamoRepo {
    fn log(&self, entry: AuditEntry) -> Result<(), CoreError> {
        let table = self.table_audit.clone();
        let item = audit_to_item(&entry);
        let fut = async {
            self.client.put_item()
                .table_name(table)
                .set_item(Some(item))
                .send().await
        };
        self.block_on(fut).map_err(map_sdk_err)?;
        Ok(())
    }

    fn list_for_target(&self, target_type: &str, target_id: &str, limit: usize) -> Result<Vec<AuditEntry>, CoreError> {
        let table = self.table_audit.clone();
        let tt = target_type.to_string();
        let ti = target_id.to_string();
        let lim = limit as i32;
        let fut = async {
            self.client.scan()
                .table_name(table)
                .filter_expression("target_type = :tt AND target_id = :ti")
                .expression_attribute_values(":tt", AttributeValue::S(tt))
                .expression_attribute_values(":ti", AttributeValue::S(ti))
                .limit(lim)
                .send().await
        };
        let out = self.block_on(fut).map_err(map_sdk_err)?;
        let mut res: Vec<_> = out.items().iter()
            .filter_map(|it| item_to_audit(it).ok())
            .collect();
        res.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        res.truncate(limit);
        Ok(res)
    }

    fn list_by_actor(&self, actor_email: &UserEmail, limit: usize) -> Result<Vec<AuditEntry>, CoreError> {
        let table = self.table_audit.clone();
        let email = actor_email.as_str().to_string();
        let lim = limit as i32;
        let fut = async {
            self.client.scan()
                .table_name(table)
                .filter_expression("actor_email = :email")
                .expression_attribute_values(":email", AttributeValue::S(email))
                .limit(lim)
                .send().await
        };
        let out = self.block_on(fut).map_err(map_sdk_err)?;
        let mut res: Vec<_> = out.items().iter()
            .filter_map(|it| item_to_audit(it).ok())
            .collect();
        res.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        res.truncate(limit);
        Ok(res)
    }

    fn list_recent(&self, limit: usize) -> Result<Vec<AuditEntry>, CoreError> {
        let table = self.table_audit.clone();
        let fut = async {
            self.client.scan()
                .table_name(table)
                .send().await
        };
        let out = self.block_on(fut).map_err(map_sdk_err)?;
        let mut res: Vec<_> = out.items().iter()
            .filter_map(|it| item_to_audit(it).ok())
            .collect();
        res.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        res.truncate(limit);
        Ok(res)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_link() -> ShortLink {
        ShortLink::new(
            Slug::new("abc123").unwrap(),
            "https://example.com".into(),
            secs_to_system_time(1_700_000_000),
            UserEmail::new("user@acme.com").unwrap(),
        )
    }

    #[test]
    fn roundtrip_item_mapping() {
        let link = sample_link();
        let item = domain_to_item(&link);
        let link2 = item_to_domain(&item).unwrap();
        assert_eq!(link.slug, link2.slug);
        assert_eq!(link.original_url, link2.original_url);
        assert_eq!(system_time_to_secs(link.created_at), system_time_to_secs(link2.created_at));
        assert_eq!(link.created_by.as_str(), link2.created_by.as_str());
        assert_eq!(link.click_count, link2.click_count);
        assert_eq!(link.is_active, link2.is_active);
        assert_eq!(link.updated_at, link2.updated_at);
        assert_eq!(link.expires_at, link2.expires_at);
    }

    #[test]
    fn backward_compatible_item_mapping() {
        // Simulate an old item without new fields
        let mut item = HashMap::new();
        item.insert("slug".into(), AttributeValue::S("old-link".into()));
        item.insert("original_url".into(), AttributeValue::S("https://old.com".into()));
        item.insert("created_at".into(), AttributeValue::N("1700000000".into()));
        item.insert("created_by".into(), AttributeValue::S("user@example.com".into()));

        let link = item_to_domain(&item).unwrap();
        assert_eq!(link.slug.as_str(), "old-link");
        assert_eq!(link.click_count, 0); // default
        assert!(link.is_active); // default
        assert!(link.updated_at.is_none()); // default
        assert!(link.expires_at.is_none()); // default
    }
}
