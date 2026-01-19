//! lambda-admin — AWS Lambda entrypoint for admin API (create/list links).
//!
//! Purpose
//! - Handle API Gateway HTTP API (v2) events for:
//!   - `POST /api/links` — create short link (requires Google Bearer auth).
//!   - `GET /api/links` — list links (requires Google Bearer auth).
//!   - `GET /api/me` — get current user info (email, is_admin).
//! - Use `LinkService` with `DynamoRepo` for persistence.
//! - Initialize structured logging compatible with Lambda.
//!
//! Security
//! - Auth is performed by verifying a Google ID token via `google_auth::verify`.
//! - If `GOOGLE_AUTH_INSECURE_SKIP_SIGNATURE` is set to a truthy value, the
//!   adapter will run without signature verification; this process emits a WARN
//!   at startup reminding not to use this mode in production.
//!
//! Authorization
//! - Regular users can only see/edit their own links.
//! - Admins (listed in `ADMIN_EMAILS` env var) can see/edit all links.
//! - `ADMIN_EMAILS` is a comma-separated list of email addresses.

use lambda_http::{run, service_fn, Body, Error, Request, Response};
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use aws_dynamo::DynamoRepo;
use domain::slug::Base62SlugGenerator;
use domain::LinkRepository;
use domain::SlugGenerator;
use domain::{
    Clock, CoreError, GroupMember, GroupRepository, GroupRole, LinkGroup, Slug, UserEmail,
};
use google_auth::{AuthError as GAuthError, VerifiedUser};
use http_common::lambda::{get_host, resp, resp_with_error, with_cors};

#[derive(Clone)]
struct AppState {
    repo: DynamoRepo,
    slugger: Base62SlugGenerator,
    clock: StdClock,
}

#[derive(Clone)]
struct StdClock;
impl Clock for StdClock {
    fn now(&self) -> std::time::SystemTime {
        std::time::SystemTime::now()
    }
}

#[derive(serde::Deserialize)]
struct CreateLinkReq {
    original_url: String,
    #[serde(default)]
    alias: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    activate_at: Option<String>,
    #[serde(default)]
    redirect_delay: Option<u32>,
    #[serde(default)]
    group_id: Option<String>,
}

#[derive(serde::Deserialize)]
struct UpdateLinkReq {
    #[serde(default)]
    original_url: Option<String>,
    #[serde(default)]
    is_active: Option<bool>,
    #[serde(default)]
    description: Option<Option<String>>,
    #[serde(default)]
    expires_at: Option<Option<String>>,
    #[serde(default)]
    activate_at: Option<Option<String>>,
    #[serde(default)]
    redirect_delay: Option<Option<u32>>,
    #[serde(default)]
    group_id: Option<Option<String>>,
}

#[derive(serde::Deserialize)]
struct BulkSlugsReq {
    slugs: Vec<String>,
}

#[derive(serde::Serialize)]
struct BulkResultOut {
    affected: usize,
}

// -------------------------
// Group API Types
// -------------------------

#[derive(serde::Deserialize)]
struct CreateGroupReq {
    name: String,
    #[serde(default)]
    description: Option<String>,
}

#[derive(serde::Deserialize)]
struct UpdateGroupReq {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<Option<String>>,
}

#[derive(serde::Deserialize)]
struct AddMemberReq {
    email: String,
    #[serde(default = "default_role")]
    role: String,
}

fn default_role() -> String {
    "editor".into()
}

#[derive(serde::Serialize)]
struct GroupOut {
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    created_at: String,
    created_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
}

#[derive(serde::Serialize)]
struct GroupListOut {
    groups: Vec<GroupOut>,
}

#[derive(serde::Serialize)]
struct MemberOut {
    email: String,
    role: String,
    added_at: String,
    added_by: String,
}

#[derive(serde::Serialize)]
struct MemberListOut {
    members: Vec<MemberOut>,
}

fn group_to_out(group: &LinkGroup, role: Option<GroupRole>) -> GroupOut {
    GroupOut {
        id: group.id.clone(),
        name: group.name.clone(),
        description: group.description.clone(),
        created_at: http_common::system_time_to_rfc3339(group.created_at),
        created_by: group.created_by.as_str().to_string(),
        role: role.map(|r| r.as_str().to_string()),
    }
}

fn member_to_out(member: &GroupMember) -> MemberOut {
    MemberOut {
        email: member.user_email.as_str().to_string(),
        role: member.role.as_str().to_string(),
        added_at: http_common::system_time_to_rfc3339(member.added_at),
        added_by: member.added_by.as_str().to_string(),
    }
}

#[derive(serde::Serialize)]
struct LinkOut {
    slug: String,
    short_url: String,
    original_url: String,
    created_at: String,
    created_by: String,
    click_count: u64,
    is_active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    activate_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    redirect_delay: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_id: Option<String>,
}

#[derive(serde::Serialize)]
struct ListOut {
    links: Vec<LinkOut>,
    total: usize,
    has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<UserInfo>,
}

#[derive(serde::Serialize, Clone)]
struct UserInfo {
    email: String,
    is_admin: bool,
}

fn link_to_out(link: domain::ShortLink, host: &str) -> LinkOut {
    LinkOut {
        slug: link.slug.as_str().to_string(),
        short_url: http_common::build_short_url_from_host(host, link.slug.as_str()),
        original_url: link.original_url,
        created_at: http_common::system_time_to_rfc3339(link.created_at),
        created_by: link.created_by.as_str().to_string(),
        click_count: link.click_count,
        is_active: link.is_active,
        updated_at: link.updated_at.map(http_common::system_time_to_rfc3339),
        expires_at: link.expires_at.map(http_common::system_time_to_rfc3339),
        description: link.description,
        activate_at: link.activate_at.map(http_common::system_time_to_rfc3339),
        redirect_delay: link.redirect_delay,
        group_id: link.group_id,
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    init_tracing();
    warn_if_insecure_skip_sig();

    let repo = DynamoRepo::from_env().map_err(|e| format!("dynamo init error: {e}"))?;
    let state = AppState {
        repo,
        slugger: Base62SlugGenerator::new(5),
        clock: StdClock,
    };

    let handler = service_fn(move |req: Request| {
        let st = state.clone();
        async move { route(st, req).await }
    });
    run(handler).await?;
    Ok(())
}

fn init_tracing() {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt::layer().with_target(true).with_writer(std::io::stdout))
        .init();
}

fn warn_if_insecure_skip_sig() {
    let val = std::env::var("GOOGLE_AUTH_INSECURE_SKIP_SIGNATURE").unwrap_or_default();
    if matches_ignore_case(&val, &["1", "true", "yes"]) {
        warn!("ID token signature verification is DISABLED (GOOGLE_AUTH_INSECURE_SKIP_SIGNATURE=1). DO NOT USE IN PRODUCTION.");
    }
}

fn matches_ignore_case(s: &str, any: &[&str]) -> bool {
    any.iter().any(|t| s.eq_ignore_ascii_case(t))
}

/// Check if the given email is in the ADMIN_EMAILS list.
fn is_admin(email: &str) -> bool {
    let admins = std::env::var("ADMIN_EMAILS").unwrap_or_default();
    admins
        .split(',')
        .map(|s| s.trim())
        .any(|admin| admin.eq_ignore_ascii_case(email))
}

async fn route(state: AppState, req: Request) -> Result<Response<Body>, Error> {
    let raw_path = req.uri().path().to_string();
    let method = req.method().as_str().to_string();

    // API Gateway HTTP API includes stage prefix in rawPath (e.g., /dev/api/links)
    // Strip the stage prefix by finding /api/ and keeping from there
    let path = if let Some(idx) = raw_path.find("/api/") {
        raw_path[idx..].to_string()
    } else {
        raw_path
    };

    // Bulk operations (must check before single link patterns)
    if path == "/api/links/bulk/delete" {
        return match method.as_str() {
            "OPTIONS" => Ok(with_cors(resp(204, None, None))),
            "POST" => bulk_delete_links(state, req).await,
            _ => Ok(with_cors(resp(
                405,
                None,
                Some(http_common::json_err("method_not_allowed")),
            ))),
        };
    }
    if path == "/api/links/bulk/activate" {
        return match method.as_str() {
            "OPTIONS" => Ok(with_cors(resp(204, None, None))),
            "POST" => bulk_activate_links(state, req).await,
            _ => Ok(with_cors(resp(
                405,
                None,
                Some(http_common::json_err("method_not_allowed")),
            ))),
        };
    }
    if path == "/api/links/bulk/deactivate" {
        return match method.as_str() {
            "OPTIONS" => Ok(with_cors(resp(204, None, None))),
            "POST" => bulk_deactivate_links(state, req).await,
            _ => Ok(with_cors(resp(
                405,
                None,
                Some(http_common::json_err("method_not_allowed")),
            ))),
        };
    }

    // Check if path is /api/links/{slug}
    let slug_path_prefix = "/api/links/";
    if path.starts_with(slug_path_prefix) && path.len() > slug_path_prefix.len() {
        let slug = path[slug_path_prefix.len()..].to_string();
        return match method.as_str() {
            "OPTIONS" => Ok(with_cors(resp(204, None, None))),
            "PATCH" => update_link(state, req, slug).await,
            "DELETE" => delete_link(state, req, slug).await,
            _ => Ok(with_cors(resp(
                405,
                None,
                Some(http_common::json_err("method_not_allowed")),
            ))),
        };
    }

    // Group member routes: /api/groups/{id}/members and /api/groups/{id}/members/{email}
    let group_members_prefix = "/api/groups/";
    if path.starts_with(group_members_prefix) && path.contains("/members") {
        // Parse: /api/groups/{group_id}/members or /api/groups/{group_id}/members/{email}
        let rest = &path[group_members_prefix.len()..];
        if let Some(members_idx) = rest.find("/members") {
            let group_id = rest[..members_idx].to_string();
            let after_members = &rest[members_idx + 8..]; // Skip "/members"

            if after_members.is_empty() {
                // /api/groups/{id}/members
                return match method.as_str() {
                    "OPTIONS" => Ok(with_cors(resp(204, None, None))),
                    "GET" => list_group_members(state, req, group_id).await,
                    "POST" => add_group_member(state, req, group_id).await,
                    _ => Ok(with_cors(resp(
                        405,
                        None,
                        Some(http_common::json_err("method_not_allowed")),
                    ))),
                };
            } else if after_members.starts_with('/') && after_members.len() > 1 {
                // /api/groups/{id}/members/{email}
                let member_email = after_members[1..].to_string();
                return match method.as_str() {
                    "OPTIONS" => Ok(with_cors(resp(204, None, None))),
                    "DELETE" => remove_group_member(state, req, group_id, member_email).await,
                    _ => Ok(with_cors(resp(
                        405,
                        None,
                        Some(http_common::json_err("method_not_allowed")),
                    ))),
                };
            }
        }
    }

    // Group routes: /api/groups/{id}
    let groups_prefix = "/api/groups/";
    if path.starts_with(groups_prefix)
        && path.len() > groups_prefix.len()
        && !path.contains("/members")
    {
        let group_id = path[groups_prefix.len()..].to_string();
        return match method.as_str() {
            "OPTIONS" => Ok(with_cors(resp(204, None, None))),
            "GET" => get_group(state, req, group_id).await,
            "PATCH" => update_group(state, req, group_id).await,
            "DELETE" => delete_group(state, req, group_id).await,
            _ => Ok(with_cors(resp(
                405,
                None,
                Some(http_common::json_err("method_not_allowed")),
            ))),
        };
    }

    match (method.as_str(), path.as_str()) {
        ("OPTIONS", "/api/links") | ("OPTIONS", "/api/me") | ("OPTIONS", "/api/groups") => {
            Ok(with_cors(resp(204, None, None)))
        }
        ("POST", "/api/links") => create_link(state, req).await,
        ("GET", "/api/links") => list_links(state, req).await,
        ("GET", "/api/me") => get_me(req).await,
        ("GET", "/api/groups") => list_groups(state, req).await,
        ("POST", "/api/groups") => create_group(state, req).await,
        _ => Ok(with_cors(resp(
            404,
            None,
            Some(http_common::json_err("not_found")),
        ))),
    }
}

async fn create_link(state: AppState, req: Request) -> Result<Response<Body>, Error> {
    let verified = match verify_request_user(&req).await {
        Ok(v) => v,
        Err(AuthHttp::Unauthorized) => {
            return Ok(with_cors(resp_with_error(
                401,
                "unauthorized",
                "missing or invalid token",
            )))
        }
        Err(AuthHttp::Forbidden) => {
            return Ok(with_cors(resp_with_error(
                403,
                "forbidden",
                "domain not allowed",
            )))
        }
    };
    let user_email = match UserEmail::new(verified.email.clone()) {
        Ok(u) => u,
        Err(_) => {
            return Ok(with_cors(resp_with_error(
                401,
                "unauthorized",
                "invalid user email in token",
            )))
        }
    };

    let body_str = match req.body() {
        Body::Empty => {
            return Ok(with_cors(resp_with_error(
                400,
                "invalid_request",
                "missing body",
            )))
        }
        Body::Text(s) => s.clone(),
        Body::Binary(b) => String::from_utf8(b.clone()).unwrap_or_default(),
        _ => String::new(),
    };

    let payload: CreateLinkReq = match serde_json::from_str(&body_str) {
        Ok(p) => p,
        Err(_) => {
            return Ok(with_cors(resp_with_error(
                400,
                "invalid_request",
                "bad json",
            )))
        }
    };

    // Validate URL
    if let Err(e) = domain::validate::validate_original_url(&payload.original_url) {
        return Ok(with_cors(resp_with_error(
            400,
            "invalid_request",
            &format!("{}", e),
        )));
    }

    // Prepare created_at
    let created_at = state.clock.now();

    // Determine slug
    let slug = if let Some(alias) = &payload.alias {
        if !http_common::is_valid_alias(alias) {
            return Ok(with_cors(resp_with_error(
                400,
                "invalid_request",
                "alias must be base62 length 3..32",
            )));
        }
        match Slug::new(alias.clone()) {
            Ok(s) => s,
            Err(_) => {
                return Ok(with_cors(resp_with_error(
                    400,
                    "invalid_request",
                    "invalid alias",
                )))
            }
        }
    } else {
        let id = match state.repo.increment_global_counter() {
            Ok(v) => v,
            Err(e) => {
                error!(err=?e, "counter error");
                return Ok(with_cors(resp_with_error(
                    500,
                    "internal",
                    "counter failure",
                )));
            }
        };
        state.slugger.next_slug(id)
    };

    // Persist
    let mut link =
        domain::ShortLink::new(slug, payload.original_url.clone(), created_at, user_email);

    // Apply optional fields
    link.description = payload.description;
    link.activate_at = payload
        .activate_at
        .and_then(|s| http_common::parse_rfc3339(&s).ok());
    link.redirect_delay = payload.redirect_delay;
    link.group_id = payload.group_id;

    match state.repo.put(link.clone()) {
        Ok(()) => {
            let host = get_host(&req);
            info!(slug = %link.slug.as_str(), "create ok");
            Ok(with_cors(resp(
                201,
                None,
                Some(serde_json::to_value(link_to_out(link, host)).expect("LinkOut serialization")),
            )))
        }
        Err(CoreError::AlreadyExists) => Ok(with_cors(resp_with_error(
            409,
            "conflict",
            "alias already exists",
        ))),
        Err(CoreError::InvalidUrl(_)) | Err(CoreError::InvalidSlug(_)) => Ok(with_cors(
            resp_with_error(400, "invalid_request", "invalid input"),
        )),
        Err(e) => {
            error!(err=?e, "create error");
            Ok(with_cors(resp_with_error(500, "internal", "server error")))
        }
    }
}

async fn update_link(
    state: AppState,
    req: Request,
    slug_str: String,
) -> Result<Response<Body>, Error> {
    // Auth required
    let verified = match verify_request_user(&req).await {
        Ok(v) => v,
        Err(AuthHttp::Unauthorized) => {
            return Ok(with_cors(resp_with_error(
                401,
                "unauthorized",
                "missing or invalid token",
            )))
        }
        Err(AuthHttp::Forbidden) => {
            return Ok(with_cors(resp_with_error(
                403,
                "forbidden",
                "domain not allowed",
            )))
        }
    };

    let user_is_admin = is_admin(&verified.email);

    // Parse slug
    let slug = match Slug::new(slug_str.clone()) {
        Ok(s) => s,
        Err(_) => {
            return Ok(with_cors(resp_with_error(
                400,
                "invalid_request",
                "invalid slug",
            )))
        }
    };

    // Parse body
    let body_str = match req.body() {
        Body::Empty => {
            return Ok(with_cors(resp_with_error(
                400,
                "invalid_request",
                "missing body",
            )))
        }
        Body::Text(s) => s.clone(),
        Body::Binary(b) => String::from_utf8(b.clone()).unwrap_or_default(),
        _ => String::new(),
    };

    let payload: UpdateLinkReq = match serde_json::from_str(&body_str) {
        Ok(p) => p,
        Err(_) => {
            return Ok(with_cors(resp_with_error(
                400,
                "invalid_request",
                "bad json",
            )))
        }
    };

    // Get existing link
    let mut link = match state.repo.get(&slug) {
        Ok(Some(l)) => l,
        Ok(None) => {
            return Ok(with_cors(resp_with_error(
                404,
                "not_found",
                "link not found",
            )))
        }
        Err(e) => {
            error!(err=?e, "get error");
            return Ok(with_cors(resp_with_error(500, "internal", "server error")));
        }
    };

    // Authorization: check if user can edit this link
    // - System admins can edit any link
    // - Link creator can edit their own link
    // - Group editors/admins can edit links in their group
    if !user_is_admin && link.created_by.as_str() != verified.email {
        // Check if link belongs to a group and user has edit access
        let user_email = UserEmail::new(verified.email.clone()).unwrap();
        let can_edit_via_group = if let Some(ref gid) = link.group_id {
            match state.repo.get_member(gid, &user_email) {
                Ok(Some(member)) => member.role.can_edit(),
                _ => false,
            }
        } else {
            false
        };

        if !can_edit_via_group {
            warn!(user = %verified.email, link_owner = %link.created_by.as_str(), "unauthorized edit attempt");
            return Ok(with_cors(resp_with_error(
                403,
                "forbidden",
                "you can only edit your own links or links in groups you have editor access to",
            )));
        }
    }

    // Apply updates
    if let Some(new_url) = payload.original_url {
        if let Err(e) = domain::validate::validate_original_url(&new_url) {
            return Ok(with_cors(resp_with_error(
                400,
                "invalid_request",
                &format!("{}", e),
            )));
        }
        link.original_url = new_url;
    }
    if let Some(new_active) = payload.is_active {
        link.is_active = new_active;
    }
    if let Some(desc) = payload.description {
        link.description = desc;
    }
    // Handle expires_at: Some(Some(str)) = set, Some(None) = clear, None = no change
    if let Some(expires_opt) = payload.expires_at {
        link.expires_at = match expires_opt {
            Some(ts_str) => match http_common::rfc3339_to_system_time(&ts_str) {
                Ok(t) => Some(t),
                Err(_) => {
                    return Ok(with_cors(resp_with_error(
                        400,
                        "invalid_request",
                        "invalid expires_at format, use ISO 8601",
                    )))
                }
            },
            None => None, // Clear expiration
        };
    }
    if let Some(activate_opt) = payload.activate_at {
        link.activate_at = activate_opt.and_then(|s| http_common::parse_rfc3339(&s).ok());
    }
    if let Some(delay) = payload.redirect_delay {
        link.redirect_delay = delay;
    }
    if let Some(gid) = payload.group_id {
        link.group_id = gid;
    }
    link.updated_at = Some(state.clock.now());

    // Persist update
    match state.repo.update(&link) {
        Ok(()) => {
            let host = get_host(&req);
            info!(slug = %link.slug.as_str(), "update ok");
            Ok(with_cors(resp(
                200,
                None,
                Some(serde_json::to_value(link_to_out(link, host)).expect("LinkOut serialization")),
            )))
        }
        Err(CoreError::NotFound) => Ok(with_cors(resp_with_error(
            404,
            "not_found",
            "link not found",
        ))),
        Err(e) => {
            error!(err=?e, "update error");
            Ok(with_cors(resp_with_error(500, "internal", "server error")))
        }
    }
}

async fn get_me(req: Request) -> Result<Response<Body>, Error> {
    let verified = match verify_request_user(&req).await {
        Ok(v) => v,
        Err(AuthHttp::Unauthorized) => {
            return Ok(with_cors(resp_with_error(
                401,
                "unauthorized",
                "missing or invalid token",
            )))
        }
        Err(AuthHttp::Forbidden) => {
            return Ok(with_cors(resp_with_error(
                403,
                "forbidden",
                "domain not allowed",
            )))
        }
    };

    let user_info = UserInfo {
        email: verified.email.clone(),
        is_admin: is_admin(&verified.email),
    };
    Ok(with_cors(resp(
        200,
        None,
        Some(serde_json::to_value(user_info).expect("UserInfo serialization")),
    )))
}

async fn list_links(state: AppState, req: Request) -> Result<Response<Body>, Error> {
    // Auth required
    let verified = match verify_request_user(&req).await {
        Ok(v) => v,
        Err(AuthHttp::Unauthorized) => {
            return Ok(with_cors(resp_with_error(
                401,
                "unauthorized",
                "missing or invalid token",
            )))
        }
        Err(AuthHttp::Forbidden) => {
            return Ok(with_cors(resp_with_error(
                403,
                "forbidden",
                "domain not allowed",
            )))
        }
    };

    let user_email_str = verified.email.clone();
    let user_email = UserEmail::new(user_email_str.clone()).unwrap();
    let user_is_admin = is_admin(&user_email_str);

    // Parse query parameters
    let query = req.uri().query();
    let limit = http_common::parse_limit_query(query).unwrap_or(50);
    let offset: usize = http_common::parse_query_param(query, "offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let search = http_common::parse_query_param(query, "search");
    let group_id = http_common::parse_query_param(query, "group_id");
    let include_deleted = http_common::parse_query_param(query, "include_deleted")
        .map(|s| s == "true" || s == "1")
        .unwrap_or(false);
    let created_by_filter = http_common::parse_query_param(query, "created_by");

    // Determine effective filter:
    // - Admins can see all links or filter by any creator
    // - Non-admins filtering by group_id: if they're a member, show all group links
    // - Non-admins without group_id: only see their own links
    let created_by = if user_is_admin {
        created_by_filter.and_then(|e| UserEmail::new(e).ok())
    } else if let Some(ref gid) = group_id {
        // Check if user is a member of this group
        match state.repo.get_member(gid, &user_email) {
            Ok(Some(_)) => {
                // User is a member - show all links in the group (no created_by filter)
                None
            }
            Ok(None) => {
                // User is NOT a member - deny access to this group's links
                return Ok(with_cors(resp_with_error(
                    403,
                    "forbidden",
                    "you are not a member of this group",
                )));
            }
            Err(e) => {
                error!(err=?e, "get member error");
                return Ok(with_cors(resp_with_error(500, "internal", "server error")));
            }
        }
    } else {
        // No group filter - show only user's own links
        Some(user_email.clone())
    };

    let options = domain::ListOptions {
        limit,
        offset,
        search,
        created_by,
        group_id,
        include_deleted,
    };

    match state.repo.list_paginated(&options) {
        Ok(result) => {
            let host = get_host(&req);
            let links: Vec<LinkOut> = result
                .items
                .into_iter()
                .map(|l| link_to_out(l, host))
                .collect();
            let user = UserInfo {
                email: user_email_str,
                is_admin: user_is_admin,
            };
            let out = ListOut {
                links,
                total: result.total,
                has_more: result.has_more,
                user: Some(user),
            };
            Ok(with_cors(resp(
                200,
                None,
                Some(serde_json::to_value(out).expect("ListOut serialization")),
            )))
        }
        Err(e) => {
            error!(err=?e, "list error");
            Ok(with_cors(resp_with_error(500, "internal", "server error")))
        }
    }
}

async fn delete_link(
    state: AppState,
    req: Request,
    slug_str: String,
) -> Result<Response<Body>, Error> {
    // Auth required
    let verified = match verify_request_user(&req).await {
        Ok(v) => v,
        Err(AuthHttp::Unauthorized) => {
            return Ok(with_cors(resp_with_error(
                401,
                "unauthorized",
                "missing or invalid token",
            )))
        }
        Err(AuthHttp::Forbidden) => {
            return Ok(with_cors(resp_with_error(
                403,
                "forbidden",
                "domain not allowed",
            )))
        }
    };

    let user_is_admin = is_admin(&verified.email);

    // Parse slug
    let slug = match Slug::new(slug_str.clone()) {
        Ok(s) => s,
        Err(_) => {
            return Ok(with_cors(resp_with_error(
                400,
                "invalid_request",
                "invalid slug",
            )))
        }
    };

    // Get existing link to check ownership
    let link = match state.repo.get(&slug) {
        Ok(Some(l)) => l,
        Ok(None) => {
            return Ok(with_cors(resp_with_error(
                404,
                "not_found",
                "link not found",
            )))
        }
        Err(e) => {
            error!(err=?e, "get error");
            return Ok(with_cors(resp_with_error(500, "internal", "server error")));
        }
    };

    // Authorization: check if user can delete this link
    // - System admins can delete any link
    // - Link creator can delete their own link
    // - Group editors/admins can delete links in their group
    if !user_is_admin && link.created_by.as_str() != verified.email {
        // Check if link belongs to a group and user has edit access
        let user_email = UserEmail::new(verified.email.clone()).unwrap();
        let can_delete_via_group = if let Some(ref gid) = link.group_id {
            match state.repo.get_member(gid, &user_email) {
                Ok(Some(member)) => member.role.can_edit(),
                _ => false,
            }
        } else {
            false
        };

        if !can_delete_via_group {
            warn!(user = %verified.email, link_owner = %link.created_by.as_str(), "unauthorized delete attempt");
            return Ok(with_cors(resp_with_error(
                403,
                "forbidden",
                "you can only delete your own links or links in groups you have editor access to",
            )));
        }
    }

    // Soft delete
    let deleted_at = state.clock.now();
    match state.repo.delete(&slug, deleted_at) {
        Ok(()) => {
            info!(slug = %slug_str, "delete ok");
            Ok(with_cors(resp(204, None, None)))
        }
        Err(CoreError::NotFound) => Ok(with_cors(resp_with_error(
            404,
            "not_found",
            "link not found",
        ))),
        Err(e) => {
            error!(err=?e, "delete error");
            Ok(with_cors(resp_with_error(500, "internal", "server error")))
        }
    }
}

async fn bulk_delete_links(state: AppState, req: Request) -> Result<Response<Body>, Error> {
    // Auth required
    let verified = match verify_request_user(&req).await {
        Ok(v) => v,
        Err(AuthHttp::Unauthorized) => {
            return Ok(with_cors(resp_with_error(
                401,
                "unauthorized",
                "missing or invalid token",
            )))
        }
        Err(AuthHttp::Forbidden) => {
            return Ok(with_cors(resp_with_error(
                403,
                "forbidden",
                "domain not allowed",
            )))
        }
    };

    // Only admins can bulk delete
    if !is_admin(&verified.email) {
        return Ok(with_cors(resp_with_error(
            403,
            "forbidden",
            "admin required for bulk operations",
        )));
    }

    // Parse body
    let body_str = match req.body() {
        Body::Empty => {
            return Ok(with_cors(resp_with_error(
                400,
                "invalid_request",
                "missing body",
            )))
        }
        Body::Text(s) => s.clone(),
        Body::Binary(b) => String::from_utf8(b.clone()).unwrap_or_default(),
        _ => String::new(),
    };

    let payload: BulkSlugsReq = match serde_json::from_str(&body_str) {
        Ok(p) => p,
        Err(_) => {
            return Ok(with_cors(resp_with_error(
                400,
                "invalid_request",
                "bad json",
            )))
        }
    };

    // Parse slugs
    let slugs: Vec<Slug> = payload
        .slugs
        .iter()
        .filter_map(|s| Slug::new(s.clone()).ok())
        .collect();

    if slugs.is_empty() {
        return Ok(with_cors(resp_with_error(
            400,
            "invalid_request",
            "no valid slugs provided",
        )));
    }

    let deleted_at = state.clock.now();
    match state.repo.bulk_delete(&slugs, deleted_at) {
        Ok(affected) => {
            info!(count = affected, "bulk delete ok");
            Ok(with_cors(resp(
                200,
                None,
                Some(serde_json::to_value(BulkResultOut { affected }).expect("serialize")),
            )))
        }
        Err(e) => {
            error!(err=?e, "bulk delete error");
            Ok(with_cors(resp_with_error(500, "internal", "server error")))
        }
    }
}

async fn bulk_activate_links(state: AppState, req: Request) -> Result<Response<Body>, Error> {
    bulk_update_active_impl(state, req, true).await
}

async fn bulk_deactivate_links(state: AppState, req: Request) -> Result<Response<Body>, Error> {
    bulk_update_active_impl(state, req, false).await
}

async fn bulk_update_active_impl(
    state: AppState,
    req: Request,
    is_active: bool,
) -> Result<Response<Body>, Error> {
    // Auth required
    let verified = match verify_request_user(&req).await {
        Ok(v) => v,
        Err(AuthHttp::Unauthorized) => {
            return Ok(with_cors(resp_with_error(
                401,
                "unauthorized",
                "missing or invalid token",
            )))
        }
        Err(AuthHttp::Forbidden) => {
            return Ok(with_cors(resp_with_error(
                403,
                "forbidden",
                "domain not allowed",
            )))
        }
    };

    // Only admins can bulk update
    if !is_admin(&verified.email) {
        return Ok(with_cors(resp_with_error(
            403,
            "forbidden",
            "admin required for bulk operations",
        )));
    }

    // Parse body
    let body_str = match req.body() {
        Body::Empty => {
            return Ok(with_cors(resp_with_error(
                400,
                "invalid_request",
                "missing body",
            )))
        }
        Body::Text(s) => s.clone(),
        Body::Binary(b) => String::from_utf8(b.clone()).unwrap_or_default(),
        _ => String::new(),
    };

    let payload: BulkSlugsReq = match serde_json::from_str(&body_str) {
        Ok(p) => p,
        Err(_) => {
            return Ok(with_cors(resp_with_error(
                400,
                "invalid_request",
                "bad json",
            )))
        }
    };

    // Parse slugs
    let slugs: Vec<Slug> = payload
        .slugs
        .iter()
        .filter_map(|s| Slug::new(s.clone()).ok())
        .collect();

    if slugs.is_empty() {
        return Ok(with_cors(resp_with_error(
            400,
            "invalid_request",
            "no valid slugs provided",
        )));
    }

    let updated_at = state.clock.now();
    match state.repo.bulk_update_active(&slugs, is_active, updated_at) {
        Ok(affected) => {
            info!(
                count = affected,
                is_active = is_active,
                "bulk update active ok"
            );
            Ok(with_cors(resp(
                200,
                None,
                Some(serde_json::to_value(BulkResultOut { affected }).expect("serialize")),
            )))
        }
        Err(e) => {
            error!(err=?e, "bulk update error");
            Ok(with_cors(resp_with_error(500, "internal", "server error")))
        }
    }
}

// -------------------------
// Group API Handlers
// -------------------------

async fn list_groups(state: AppState, req: Request) -> Result<Response<Body>, Error> {
    let verified = match verify_request_user(&req).await {
        Ok(v) => v,
        Err(AuthHttp::Unauthorized) => {
            return Ok(with_cors(resp_with_error(
                401,
                "unauthorized",
                "missing or invalid token",
            )))
        }
        Err(AuthHttp::Forbidden) => {
            return Ok(with_cors(resp_with_error(
                403,
                "forbidden",
                "domain not allowed",
            )))
        }
    };

    let user_email = match UserEmail::new(verified.email.clone()) {
        Ok(u) => u,
        Err(_) => {
            return Ok(with_cors(resp_with_error(
                401,
                "unauthorized",
                "invalid user email",
            )))
        }
    };

    match state.repo.get_user_groups(&user_email) {
        Ok(groups_with_roles) => {
            let groups: Vec<GroupOut> = groups_with_roles
                .iter()
                .map(|(g, r)| group_to_out(g, Some(*r)))
                .collect();
            let out = GroupListOut { groups };
            Ok(with_cors(resp(
                200,
                None,
                Some(serde_json::to_value(out).expect("serialize")),
            )))
        }
        Err(e) => {
            error!(err=?e, "list groups error");
            Ok(with_cors(resp_with_error(500, "internal", "server error")))
        }
    }
}

async fn create_group(state: AppState, req: Request) -> Result<Response<Body>, Error> {
    let verified = match verify_request_user(&req).await {
        Ok(v) => v,
        Err(AuthHttp::Unauthorized) => {
            return Ok(with_cors(resp_with_error(
                401,
                "unauthorized",
                "missing or invalid token",
            )))
        }
        Err(AuthHttp::Forbidden) => {
            return Ok(with_cors(resp_with_error(
                403,
                "forbidden",
                "domain not allowed",
            )))
        }
    };

    let user_email = match UserEmail::new(verified.email.clone()) {
        Ok(u) => u,
        Err(_) => {
            return Ok(with_cors(resp_with_error(
                401,
                "unauthorized",
                "invalid user email",
            )))
        }
    };

    let body_str = match req.body() {
        Body::Empty => {
            return Ok(with_cors(resp_with_error(
                400,
                "invalid_request",
                "missing body",
            )))
        }
        Body::Text(s) => s.clone(),
        Body::Binary(b) => String::from_utf8(b.clone()).unwrap_or_default(),
        _ => String::new(),
    };

    let payload: CreateGroupReq = match serde_json::from_str(&body_str) {
        Ok(p) => p,
        Err(_) => {
            return Ok(with_cors(resp_with_error(
                400,
                "invalid_request",
                "bad json",
            )))
        }
    };

    if payload.name.is_empty() || payload.name.len() > 100 {
        return Ok(with_cors(resp_with_error(
            400,
            "invalid_request",
            "name must be 1-100 characters",
        )));
    }

    // Generate a unique group ID
    let group_id = format!("grp_{}", http_common::generate_id());
    let now = state.clock.now();

    let group = LinkGroup {
        id: group_id.clone(),
        name: payload.name,
        description: payload.description,
        created_at: now,
        created_by: user_email.clone(),
    };

    // Create the group
    if let Err(e) = state.repo.create_group(group.clone()) {
        error!(err=?e, "create group error");
        return Ok(with_cors(resp_with_error(500, "internal", "server error")));
    }

    // Add creator as admin member
    let member = GroupMember {
        group_id: group_id.clone(),
        user_email: user_email.clone(),
        role: GroupRole::Admin,
        added_at: now,
        added_by: user_email,
    };
    if let Err(e) = state.repo.add_member(member) {
        error!(err=?e, "add creator as member error");
        // Group created but member add failed - not ideal but continue
    }

    info!(group_id = %group_id, "group created");
    Ok(with_cors(resp(
        201,
        None,
        Some(
            serde_json::to_value(group_to_out(&group, Some(GroupRole::Admin))).expect("serialize"),
        ),
    )))
}

async fn get_group(
    state: AppState,
    req: Request,
    group_id: String,
) -> Result<Response<Body>, Error> {
    let verified = match verify_request_user(&req).await {
        Ok(v) => v,
        Err(AuthHttp::Unauthorized) => {
            return Ok(with_cors(resp_with_error(
                401,
                "unauthorized",
                "missing or invalid token",
            )))
        }
        Err(AuthHttp::Forbidden) => {
            return Ok(with_cors(resp_with_error(
                403,
                "forbidden",
                "domain not allowed",
            )))
        }
    };

    let user_email = match UserEmail::new(verified.email.clone()) {
        Ok(u) => u,
        Err(_) => {
            return Ok(with_cors(resp_with_error(
                401,
                "unauthorized",
                "invalid user email",
            )))
        }
    };

    // Get group
    let group = match state.repo.get_group(&group_id) {
        Ok(Some(g)) => g,
        Ok(None) => {
            return Ok(with_cors(resp_with_error(
                404,
                "not_found",
                "group not found",
            )))
        }
        Err(e) => {
            error!(err=?e, "get group error");
            return Ok(with_cors(resp_with_error(500, "internal", "server error")));
        }
    };

    // Check membership (unless admin)
    let is_system_admin = is_admin(&verified.email);
    let member_role = if is_system_admin {
        Some(GroupRole::Admin)
    } else {
        match state.repo.get_member(&group_id, &user_email) {
            Ok(Some(m)) => Some(m.role),
            Ok(None) => {
                return Ok(with_cors(resp_with_error(
                    403,
                    "forbidden",
                    "you are not a member of this group",
                )))
            }
            Err(e) => {
                error!(err=?e, "get member error");
                return Ok(with_cors(resp_with_error(500, "internal", "server error")));
            }
        }
    };

    Ok(with_cors(resp(
        200,
        None,
        Some(serde_json::to_value(group_to_out(&group, member_role)).expect("serialize")),
    )))
}

async fn update_group(
    state: AppState,
    req: Request,
    group_id: String,
) -> Result<Response<Body>, Error> {
    let verified = match verify_request_user(&req).await {
        Ok(v) => v,
        Err(AuthHttp::Unauthorized) => {
            return Ok(with_cors(resp_with_error(
                401,
                "unauthorized",
                "missing or invalid token",
            )))
        }
        Err(AuthHttp::Forbidden) => {
            return Ok(with_cors(resp_with_error(
                403,
                "forbidden",
                "domain not allowed",
            )))
        }
    };

    let user_email = match UserEmail::new(verified.email.clone()) {
        Ok(u) => u,
        Err(_) => {
            return Ok(with_cors(resp_with_error(
                401,
                "unauthorized",
                "invalid user email",
            )))
        }
    };

    // Get existing group
    let mut group = match state.repo.get_group(&group_id) {
        Ok(Some(g)) => g,
        Ok(None) => {
            return Ok(with_cors(resp_with_error(
                404,
                "not_found",
                "group not found",
            )))
        }
        Err(e) => {
            error!(err=?e, "get group error");
            return Ok(with_cors(resp_with_error(500, "internal", "server error")));
        }
    };

    // Check if user has admin access
    let is_system_admin = is_admin(&verified.email);
    if !is_system_admin {
        match state.repo.get_member(&group_id, &user_email) {
            Ok(Some(m)) if m.role.can_manage() => {}
            Ok(_) => {
                return Ok(with_cors(resp_with_error(
                    403,
                    "forbidden",
                    "admin role required to update group",
                )))
            }
            Err(e) => {
                error!(err=?e, "get member error");
                return Ok(with_cors(resp_with_error(500, "internal", "server error")));
            }
        }
    }

    // Parse body
    let body_str = match req.body() {
        Body::Empty => {
            return Ok(with_cors(resp_with_error(
                400,
                "invalid_request",
                "missing body",
            )))
        }
        Body::Text(s) => s.clone(),
        Body::Binary(b) => String::from_utf8(b.clone()).unwrap_or_default(),
        _ => String::new(),
    };

    let payload: UpdateGroupReq = match serde_json::from_str(&body_str) {
        Ok(p) => p,
        Err(_) => {
            return Ok(with_cors(resp_with_error(
                400,
                "invalid_request",
                "bad json",
            )))
        }
    };

    // Apply updates
    if let Some(name) = payload.name {
        if name.is_empty() || name.len() > 100 {
            return Ok(with_cors(resp_with_error(
                400,
                "invalid_request",
                "name must be 1-100 characters",
            )));
        }
        group.name = name;
    }
    if let Some(desc) = payload.description {
        group.description = desc;
    }

    match state.repo.update_group(&group) {
        Ok(()) => {
            info!(group_id = %group_id, "group updated");
            Ok(with_cors(resp(
                200,
                None,
                Some(serde_json::to_value(group_to_out(&group, None)).expect("serialize")),
            )))
        }
        Err(CoreError::NotFound) => Ok(with_cors(resp_with_error(
            404,
            "not_found",
            "group not found",
        ))),
        Err(e) => {
            error!(err=?e, "update group error");
            Ok(with_cors(resp_with_error(500, "internal", "server error")))
        }
    }
}

async fn delete_group(
    state: AppState,
    req: Request,
    group_id: String,
) -> Result<Response<Body>, Error> {
    let verified = match verify_request_user(&req).await {
        Ok(v) => v,
        Err(AuthHttp::Unauthorized) => {
            return Ok(with_cors(resp_with_error(
                401,
                "unauthorized",
                "missing or invalid token",
            )))
        }
        Err(AuthHttp::Forbidden) => {
            return Ok(with_cors(resp_with_error(
                403,
                "forbidden",
                "domain not allowed",
            )))
        }
    };

    let user_email = match UserEmail::new(verified.email.clone()) {
        Ok(u) => u,
        Err(_) => {
            return Ok(with_cors(resp_with_error(
                401,
                "unauthorized",
                "invalid user email",
            )))
        }
    };

    // Check if group exists
    if let Err(e) = state.repo.get_group(&group_id) {
        error!(err=?e, "get group error");
        return Ok(with_cors(resp_with_error(500, "internal", "server error")));
    }

    // Check if user has admin access
    let is_system_admin = is_admin(&verified.email);
    if !is_system_admin {
        match state.repo.get_member(&group_id, &user_email) {
            Ok(Some(m)) if m.role.can_manage() => {}
            Ok(_) => {
                return Ok(with_cors(resp_with_error(
                    403,
                    "forbidden",
                    "admin role required to delete group",
                )))
            }
            Err(e) => {
                error!(err=?e, "get member error");
                return Ok(with_cors(resp_with_error(500, "internal", "server error")));
            }
        }
    }

    match state.repo.delete_group(&group_id) {
        Ok(()) => {
            info!(group_id = %group_id, "group deleted");
            Ok(with_cors(resp(204, None, None)))
        }
        Err(CoreError::NotFound) => Ok(with_cors(resp_with_error(
            404,
            "not_found",
            "group not found",
        ))),
        Err(e) => {
            error!(err=?e, "delete group error");
            Ok(with_cors(resp_with_error(500, "internal", "server error")))
        }
    }
}

async fn list_group_members(
    state: AppState,
    req: Request,
    group_id: String,
) -> Result<Response<Body>, Error> {
    let verified = match verify_request_user(&req).await {
        Ok(v) => v,
        Err(AuthHttp::Unauthorized) => {
            return Ok(with_cors(resp_with_error(
                401,
                "unauthorized",
                "missing or invalid token",
            )))
        }
        Err(AuthHttp::Forbidden) => {
            return Ok(with_cors(resp_with_error(
                403,
                "forbidden",
                "domain not allowed",
            )))
        }
    };

    let user_email = match UserEmail::new(verified.email.clone()) {
        Ok(u) => u,
        Err(_) => {
            return Ok(with_cors(resp_with_error(
                401,
                "unauthorized",
                "invalid user email",
            )))
        }
    };

    // Check membership (unless system admin)
    let is_system_admin = is_admin(&verified.email);
    if !is_system_admin {
        match state.repo.get_member(&group_id, &user_email) {
            Ok(Some(_)) => {}
            Ok(None) => {
                return Ok(with_cors(resp_with_error(
                    403,
                    "forbidden",
                    "you are not a member of this group",
                )))
            }
            Err(e) => {
                error!(err=?e, "get member error");
                return Ok(with_cors(resp_with_error(500, "internal", "server error")));
            }
        }
    }

    match state.repo.list_members(&group_id) {
        Ok(members) => {
            let out = MemberListOut {
                members: members.iter().map(member_to_out).collect(),
            };
            Ok(with_cors(resp(
                200,
                None,
                Some(serde_json::to_value(out).expect("serialize")),
            )))
        }
        Err(e) => {
            error!(err=?e, "list members error");
            Ok(with_cors(resp_with_error(500, "internal", "server error")))
        }
    }
}

async fn add_group_member(
    state: AppState,
    req: Request,
    group_id: String,
) -> Result<Response<Body>, Error> {
    let verified = match verify_request_user(&req).await {
        Ok(v) => v,
        Err(AuthHttp::Unauthorized) => {
            return Ok(with_cors(resp_with_error(
                401,
                "unauthorized",
                "missing or invalid token",
            )))
        }
        Err(AuthHttp::Forbidden) => {
            return Ok(with_cors(resp_with_error(
                403,
                "forbidden",
                "domain not allowed",
            )))
        }
    };

    let user_email = match UserEmail::new(verified.email.clone()) {
        Ok(u) => u,
        Err(_) => {
            return Ok(with_cors(resp_with_error(
                401,
                "unauthorized",
                "invalid user email",
            )))
        }
    };

    // Check if user has admin access to the group
    let is_system_admin = is_admin(&verified.email);
    if !is_system_admin {
        match state.repo.get_member(&group_id, &user_email) {
            Ok(Some(m)) if m.role.can_manage() => {}
            Ok(_) => {
                return Ok(with_cors(resp_with_error(
                    403,
                    "forbidden",
                    "admin role required to add members",
                )))
            }
            Err(e) => {
                error!(err=?e, "get member error");
                return Ok(with_cors(resp_with_error(500, "internal", "server error")));
            }
        }
    }

    // Parse body
    let body_str = match req.body() {
        Body::Empty => {
            return Ok(with_cors(resp_with_error(
                400,
                "invalid_request",
                "missing body",
            )))
        }
        Body::Text(s) => s.clone(),
        Body::Binary(b) => String::from_utf8(b.clone()).unwrap_or_default(),
        _ => String::new(),
    };

    let payload: AddMemberReq = match serde_json::from_str(&body_str) {
        Ok(p) => p,
        Err(_) => {
            return Ok(with_cors(resp_with_error(
                400,
                "invalid_request",
                "bad json",
            )))
        }
    };

    let new_member_email = match UserEmail::new(payload.email.clone()) {
        Ok(e) => e,
        Err(_) => {
            return Ok(with_cors(resp_with_error(
                400,
                "invalid_request",
                "invalid email",
            )))
        }
    };

    let role = match GroupRole::parse(&payload.role) {
        Some(r) => r,
        None => {
            return Ok(with_cors(resp_with_error(
                400,
                "invalid_request",
                "invalid role, use: viewer, editor, or admin",
            )))
        }
    };

    let member = GroupMember {
        group_id: group_id.clone(),
        user_email: new_member_email,
        role,
        added_at: state.clock.now(),
        added_by: user_email,
    };

    match state.repo.add_member(member.clone()) {
        Ok(()) => {
            info!(group_id = %group_id, member = %payload.email, "member added");
            Ok(with_cors(resp(
                201,
                None,
                Some(serde_json::to_value(member_to_out(&member)).expect("serialize")),
            )))
        }
        Err(e) => {
            error!(err=?e, "add member error");
            Ok(with_cors(resp_with_error(500, "internal", "server error")))
        }
    }
}

async fn remove_group_member(
    state: AppState,
    req: Request,
    group_id: String,
    member_email_str: String,
) -> Result<Response<Body>, Error> {
    let verified = match verify_request_user(&req).await {
        Ok(v) => v,
        Err(AuthHttp::Unauthorized) => {
            return Ok(with_cors(resp_with_error(
                401,
                "unauthorized",
                "missing or invalid token",
            )))
        }
        Err(AuthHttp::Forbidden) => {
            return Ok(with_cors(resp_with_error(
                403,
                "forbidden",
                "domain not allowed",
            )))
        }
    };

    let user_email = match UserEmail::new(verified.email.clone()) {
        Ok(u) => u,
        Err(_) => {
            return Ok(with_cors(resp_with_error(
                401,
                "unauthorized",
                "invalid user email",
            )))
        }
    };

    // URL decode the email (it may contain @)
    let member_email_decoded =
        urlencoding::decode(&member_email_str).unwrap_or_else(|_| member_email_str.clone().into());
    let member_email = match UserEmail::new(member_email_decoded.to_string()) {
        Ok(e) => e,
        Err(_) => {
            return Ok(with_cors(resp_with_error(
                400,
                "invalid_request",
                "invalid email",
            )))
        }
    };

    // Check if user has admin access to the group
    let is_system_admin = is_admin(&verified.email);
    if !is_system_admin {
        match state.repo.get_member(&group_id, &user_email) {
            Ok(Some(m)) if m.role.can_manage() => {}
            Ok(_) => {
                return Ok(with_cors(resp_with_error(
                    403,
                    "forbidden",
                    "admin role required to remove members",
                )))
            }
            Err(e) => {
                error!(err=?e, "get member error");
                return Ok(with_cors(resp_with_error(500, "internal", "server error")));
            }
        }
    }

    match state.repo.remove_member(&group_id, &member_email) {
        Ok(()) => {
            info!(group_id = %group_id, member = %member_email_decoded, "member removed");
            Ok(with_cors(resp(204, None, None)))
        }
        Err(e) => {
            error!(err=?e, "remove member error");
            Ok(with_cors(resp_with_error(500, "internal", "server error")))
        }
    }
}

enum AuthHttp {
    Unauthorized,
    Forbidden,
}

async fn verify_request_user(req: &Request) -> Result<VerifiedUser, AuthHttp> {
    // Find Authorization header
    let auth = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or(AuthHttp::Unauthorized)?;
    let token = auth.strip_prefix("Bearer ").ok_or(AuthHttp::Unauthorized)?;
    let aud = std::env::var("GOOGLE_OAUTH_CLIENT_ID").map_err(|_| AuthHttp::Unauthorized)?;
    let allowed = std::env::var("ALLOWED_DOMAIN").map_err(|_| AuthHttp::Unauthorized)?;
    match google_auth::verify_async(token, &aud, &allowed).await {
        Ok(u) => Ok(u),
        Err(GAuthError::DomainNotAllowed) => {
            warn!("auth failed: domain not allowed");
            Err(AuthHttp::Forbidden)
        }
        Err(e) => {
            warn!(err=?e, "auth failed");
            Err(AuthHttp::Unauthorized)
        }
    }
}

// Note: Response builders (resp, resp_with_error, with_cors), validation (is_valid_alias),
// time utilities (system_time_to_rfc3339), URL building (build_short_url_from_host),
// and query parsing (parse_limit_query) are now provided by the http-common crate.

// Tests for shared utilities are in http-common crate.
