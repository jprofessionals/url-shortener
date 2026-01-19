//! api-server — Local development HTTP API for the URL Shortener workspace.
//!
//! Provides redirect and admin endpoints and supports local dev with:
//! - Auth: Google ID token verification or disabled (debug) mode via X-Debug-User.
//! - Storage: In-memory (default) or SQLite (file) when the `sqlite` feature is enabled.
//! - CORS: Configurable via CORS_ALLOW_ORIGIN (origin string) for admin frontend.
//!
//! Contract follows docs/spec_admin_api.md for create/list fields and behavior where practical.
//!
//! Run:
//! ```bash
//! # pretty logs (default); PORT optional
//! cargo run -p api-server
//!
//! # with Dynamo adapter enabled (requires env vars)
//! DYNAMO_TABLE_SHORTLINKS=shortlinks \
//! DYNAMO_TABLE_COUNTERS=counters \
//!   cargo run -p api-server --features dynamo
//! ```
//!
//! Configuration: See `config.rs` for all environment variables.
//!

mod config;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::http::HeaderValue;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect},
    routing::{get, post},
    Json, Router,
};
use domain::adapters::memory_repo::InMemoryRepo;
use domain::slug::Base62SlugGenerator;
use domain::SlugGenerator;
use domain::{Clock, CoreError, LinkRepository, Slug, UserEmail};
use google_auth::{AuthError as GAuthError, VerifiedUser};
use serde::{Deserialize, Serialize};
use tower_http::{
    cors::{AllowOrigin, CorsLayer},
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

// Local repo abstraction supporting memory or sqlite (feature-gated).
enum RepoKind {
    Memory(InMemoryRepo),
    #[cfg(feature = "sqlite")]
    Sqlite(sqlite_adapter::SqliteRepo),
}

#[derive(Clone)]
struct AnyRepo {
    kind: Arc<RepoKind>,
    counter: Arc<Mutex<u64>>, // used when Memory; ignored when Sqlite which has its own counter
}

#[allow(dead_code)]
impl AnyRepo {
    fn memory() -> Self {
        Self {
            kind: Arc::new(RepoKind::Memory(InMemoryRepo::new())),
            counter: Arc::new(Mutex::new(0)),
        }
    }

    #[cfg(feature = "sqlite")]
    fn sqlite_from_env() -> Result<Self, CoreError> {
        Ok(Self {
            kind: Arc::new(RepoKind::Sqlite(sqlite_adapter::SqliteRepo::from_env()?)),
            counter: Arc::new(Mutex::new(0)),
        })
    }

    fn get(&self, slug: &Slug) -> Result<Option<domain::ShortLink>, CoreError> {
        match &*self.kind {
            RepoKind::Memory(r) => r.get(slug),
            #[cfg(feature = "sqlite")]
            RepoKind::Sqlite(r) => r.get(slug),
        }
    }

    fn put(&self, link: domain::ShortLink) -> Result<(), CoreError> {
        match &*self.kind {
            RepoKind::Memory(r) => r.put(link),
            #[cfg(feature = "sqlite")]
            RepoKind::Sqlite(r) => r.put(link),
        }
    }

    fn list(&self, limit: usize) -> Result<Vec<domain::ShortLink>, CoreError> {
        match &*self.kind {
            RepoKind::Memory(r) => r.list(limit),
            #[cfg(feature = "sqlite")]
            RepoKind::Sqlite(r) => r.list(limit),
        }
    }

    fn update(&self, link: &domain::ShortLink) -> Result<(), CoreError> {
        match &*self.kind {
            RepoKind::Memory(r) => r.update(link),
            #[cfg(feature = "sqlite")]
            RepoKind::Sqlite(r) => r.update(link),
        }
    }

    fn increment_click(&self, slug: &domain::Slug) -> Result<(), CoreError> {
        match &*self.kind {
            RepoKind::Memory(r) => r.increment_click(slug),
            #[cfg(feature = "sqlite")]
            RepoKind::Sqlite(r) => r.increment_click(slug),
        }
    }

    fn list_by_creator(
        &self,
        email: &domain::UserEmail,
        limit: usize,
    ) -> Result<Vec<domain::ShortLink>, CoreError> {
        match &*self.kind {
            RepoKind::Memory(r) => r.list_by_creator(email, limit),
            #[cfg(feature = "sqlite")]
            RepoKind::Sqlite(r) => r.list_by_creator(email, limit),
        }
    }

    fn increment_global_counter(&self) -> Result<u64, CoreError> {
        match &*self.kind {
            RepoKind::Memory(_) => {
                let mut g = self
                    .counter
                    .lock()
                    .map_err(|_| CoreError::Repository("counter mutex".into()))?;
                let id = *g;
                *g = id.saturating_add(1);
                Ok(id)
            }
            #[cfg(feature = "sqlite")]
            RepoKind::Sqlite(r) => r.increment_global_counter(),
        }
    }

    fn delete(
        &self,
        slug: &domain::Slug,
        deleted_at: std::time::SystemTime,
    ) -> Result<(), CoreError> {
        match &*self.kind {
            RepoKind::Memory(r) => r.delete(slug, deleted_at),
            #[cfg(feature = "sqlite")]
            RepoKind::Sqlite(r) => r.delete(slug, deleted_at),
        }
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<domain::ShortLink>, CoreError> {
        match &*self.kind {
            RepoKind::Memory(r) => r.search(query, limit),
            #[cfg(feature = "sqlite")]
            RepoKind::Sqlite(r) => r.search(query, limit),
        }
    }

    fn list_paginated(
        &self,
        options: &domain::ListOptions,
    ) -> Result<domain::ListResult<domain::ShortLink>, CoreError> {
        match &*self.kind {
            RepoKind::Memory(r) => r.list_paginated(options),
            #[cfg(feature = "sqlite")]
            RepoKind::Sqlite(r) => r.list_paginated(options),
        }
    }

    fn list_by_group(
        &self,
        group_id: &str,
        limit: usize,
    ) -> Result<Vec<domain::ShortLink>, CoreError> {
        match &*self.kind {
            RepoKind::Memory(r) => r.list_by_group(group_id, limit),
            #[cfg(feature = "sqlite")]
            RepoKind::Sqlite(r) => r.list_by_group(group_id, limit),
        }
    }

    fn bulk_delete(
        &self,
        slugs: &[domain::Slug],
        deleted_at: std::time::SystemTime,
    ) -> Result<usize, CoreError> {
        match &*self.kind {
            RepoKind::Memory(r) => r.bulk_delete(slugs, deleted_at),
            #[cfg(feature = "sqlite")]
            RepoKind::Sqlite(r) => r.bulk_delete(slugs, deleted_at),
        }
    }

    fn bulk_update_active(
        &self,
        slugs: &[domain::Slug],
        is_active: bool,
        updated_at: std::time::SystemTime,
    ) -> Result<usize, CoreError> {
        match &*self.kind {
            RepoKind::Memory(r) => r.bulk_update_active(slugs, is_active, updated_at),
            #[cfg(feature = "sqlite")]
            RepoKind::Sqlite(r) => r.bulk_update_active(slugs, is_active, updated_at),
        }
    }
}

#[derive(Clone)]
struct AppState {
    repo: AnyRepo,
    slugger: Base62SlugGenerator,
    clock: StdClock,
    auth_provider: config::AuthProvider,
    allowed_domain: Option<String>,
    google_oauth_client_id: Option<String>,
    shortlink_domain: Option<String>,
}

#[derive(Clone)]
struct StdClock;
impl Clock for StdClock {
    fn now(&self) -> std::time::SystemTime {
        std::time::SystemTime::now()
    }
}

#[tokio::main]
async fn main() {
    // Load and validate config first (fail fast on misconfiguration)
    let cfg = match config::Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Configuration error: {}", e);
            std::process::exit(1);
        }
    };

    init_tracing(&cfg);
    cfg.warn_if_insecure();

    let repo = build_repo_from_env(&cfg);
    let state = AppState {
        repo,
        slugger: Base62SlugGenerator::new(5),
        clock: StdClock,
        auth_provider: cfg.auth_provider.clone(),
        allowed_domain: cfg.allowed_domain.clone(),
        google_oauth_client_id: cfg.google_oauth_client_id.clone(),
        shortlink_domain: cfg.shortlink_domain.clone(),
    };

    // Request ID header name
    let x_request_id = axum::http::HeaderName::from_static("x-request-id");

    let mut app = Router::new()
        .route("/:slug", get(get_slug))
        .route(
            "/api/links",
            post(create_link).get(list_links).options(preflight_links),
        )
        .route(
            "/api/links/:slug",
            axum::routing::patch(update_link)
                .delete(delete_link)
                .options(preflight_link),
        )
        .route(
            "/api/links/bulk/delete",
            post(bulk_delete_links).options(preflight_links),
        )
        .route(
            "/api/links/bulk/activate",
            post(bulk_activate_links).options(preflight_links),
        )
        .route(
            "/api/links/bulk/deactivate",
            post(bulk_deactivate_links).options(preflight_links),
        )
        .route("/api/me", get(get_me).options(preflight_links))
        .layer(PropagateRequestIdLayer::new(x_request_id.clone()))
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &axum::http::Request<_>| {
                let request_id = request
                    .headers()
                    .get("x-request-id")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("-");
                tracing::info_span!(
                    "http_request",
                    method = %request.method(),
                    uri = %request.uri(),
                    request_id = %request_id,
                )
            }),
        )
        .layer(SetRequestIdLayer::new(x_request_id, MakeRequestUuid))
        .with_state(state);

    // CORS - already validated in Config::from_env()
    let cors = if cfg.cors_allow_origin == HeaderValue::from_static("*") {
        CorsLayer::permissive()
    } else {
        CorsLayer::new()
            .allow_origin(AllowOrigin::list([cfg.cors_allow_origin]))
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::PATCH,
                axum::http::Method::DELETE,
                axum::http::Method::OPTIONS,
            ])
            .allow_headers([
                axum::http::header::AUTHORIZATION,
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderName::from_static("x-debug-user"),
            ])
    };
    app = app.layer(cors);

    let addr: SocketAddr = ([0, 0, 0, 0], cfg.port).into();
    info!(%addr, "api-server listening");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind port");
    axum::serve(listener, app).await.expect("server error");
}

fn init_tracing(cfg: &config::Config) {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let registry = tracing_subscriber::registry().with(env_filter);
    match cfg.log_format {
        config::LogFormat::Json => {
            registry
                .with(
                    fmt::layer()
                        .json()
                        .with_target(true)
                        .with_timer(fmt::time::SystemTime)
                        .with_writer(std::io::stdout),
                )
                .init();
        }
        config::LogFormat::Pretty => {
            registry
                .with(
                    fmt::layer()
                        .pretty()
                        .with_target(true)
                        .with_writer(std::io::stdout),
                )
                .init();
        }
    }
}

// Construct a repository instance based on config and feature flags.
fn build_repo_from_env(cfg: &config::Config) -> AnyRepo {
    match cfg.storage_provider {
        #[cfg(feature = "sqlite")]
        config::StorageProvider::Sqlite => match AnyRepo::sqlite_from_env() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("failed to init SqliteRepo from env: {e}");
                AnyRepo::memory()
            }
        },
        _ => AnyRepo::memory(),
    }
}

#[derive(Deserialize)]
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

#[derive(Deserialize)]
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

#[derive(Deserialize)]
struct BulkSlugsReq {
    slugs: Vec<String>,
}

#[derive(Serialize)]
struct BulkResultOut {
    affected: usize,
}

#[derive(Serialize)]
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

#[derive(Serialize)]
struct ListOut {
    links: Vec<LinkOut>,
    total: usize,
    has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<UserInfo>,
}

#[derive(Serialize)]
struct UserInfo {
    email: String,
    is_admin: bool,
}

fn link_to_out(
    link: domain::ShortLink,
    headers: &HeaderMap,
    shortlink_domain: &Option<String>,
) -> LinkOut {
    LinkOut {
        slug: link.slug.as_str().to_string(),
        short_url: build_short_url(headers, link.slug.as_str(), shortlink_domain),
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

fn is_admin(email: &str) -> bool {
    let admins = std::env::var("ADMIN_EMAILS").unwrap_or_default();
    admins
        .split(',')
        .map(|s| s.trim())
        .any(|a| a.eq_ignore_ascii_case(email))
}

async fn get_slug(State(state): State<AppState>, Path(slug): Path<String>) -> impl IntoResponse {
    match Slug::new(slug.clone()) {
        Ok(s) => match state.repo.get(&s) {
            Ok(Some(link)) => {
                info!(slug = %s.as_str(), redirect_to = %link.original_url, "resolve ok");
                Redirect::permanent(&link.original_url).into_response()
            }
            Ok(None) => {
                warn!(slug = %s.as_str(), "resolve 404");
                (
                    StatusCode::NOT_FOUND,
                    Json(http_common::json_err("not_found")),
                )
                    .into_response()
            }
            Err(e) => {
                error!(slug = %s.as_str(), err = ?e, "resolve error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(http_common::json_err("error")),
                )
                    .into_response()
            }
        },
        Err(_) => {
            warn!("bad slug in path");
            (
                StatusCode::BAD_REQUEST,
                Json(http_common::json_err("invalid_slug")),
            )
                .into_response()
        }
    }
}

async fn create_link(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateLinkReq>,
) -> impl IntoResponse {
    // Auth
    let verified = match verify_request_user(
        &headers,
        &state.auth_provider,
        &state.allowed_domain,
        &state.google_oauth_client_id,
    )
    .await
    {
        Ok(v) => v,
        Err(AuthHttp::Unauthorized) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(http_common::json_error_with_message(
                    "unauthorized",
                    "missing or invalid token",
                )),
            )
                .into_response()
        }
        Err(AuthHttp::Forbidden) => {
            return (
                StatusCode::FORBIDDEN,
                Json(http_common::json_error_with_message(
                    "forbidden",
                    "domain not allowed",
                )),
            )
                .into_response()
        }
    };
    let user_email = match UserEmail::new(verified.email.clone()) {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(http_common::json_error_with_message(
                    "unauthorized",
                    "invalid user email in token",
                )),
            )
                .into_response()
        }
    };

    // Validate URL
    if let Err(e) = domain::validate::validate_original_url(&body.original_url) {
        return (
            StatusCode::BAD_REQUEST,
            Json(http_common::json_error_with_message(
                "invalid_request",
                &format!("{}", e),
            )),
        )
            .into_response();
    }

    // Determine slug
    let slug = if let Some(alias) = &body.alias {
        if !http_common::is_valid_alias(alias) {
            return (
                StatusCode::BAD_REQUEST,
                Json(http_common::json_error_with_message(
                    "invalid_request",
                    "alias must be 3-32 characters, alphanumeric with hyphens/underscores",
                )),
            )
                .into_response();
        }
        match Slug::new(alias.clone()) {
            Ok(s) => s,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(http_common::json_error_with_message(
                        "invalid_request",
                        "invalid alias",
                    )),
                )
                    .into_response()
            }
        }
    } else {
        let id = match state.repo.increment_global_counter() {
            Ok(v) => v,
            Err(e) => {
                error!(err=?e, "counter error");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(http_common::json_error_with_message(
                        "internal",
                        "counter failure",
                    )),
                )
                    .into_response();
            }
        };
        state.slugger.next_slug(id)
    };

    // Persist
    let created_at = state.clock.now();
    let mut link = domain::ShortLink::new(slug, body.original_url.clone(), created_at, user_email);

    // Apply optional fields
    link.description = body.description;
    link.activate_at = body
        .activate_at
        .and_then(|s| http_common::parse_rfc3339(&s).ok());
    link.redirect_delay = body.redirect_delay;
    link.group_id = body.group_id;

    match state.repo.put(link.clone()) {
        Ok(()) => {
            info!(slug = %link.slug.as_str(), "create ok");
            (
                StatusCode::CREATED,
                Json(link_to_out(link, &headers, &state.shortlink_domain)),
            )
                .into_response()
        }
        Err(CoreError::AlreadyExists) => (
            StatusCode::CONFLICT,
            Json(http_common::json_error_with_message(
                "conflict",
                "alias already exists",
            )),
        )
            .into_response(),
        Err(CoreError::InvalidUrl(_)) | Err(CoreError::InvalidSlug(_)) => (
            StatusCode::BAD_REQUEST,
            Json(http_common::json_error_with_message(
                "invalid_request",
                "invalid input",
            )),
        )
            .into_response(),
        Err(e) => {
            error!(err=?e, "create error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(http_common::json_error_with_message(
                    "internal",
                    "server error",
                )),
            )
                .into_response()
        }
    }
}

enum AuthHttp {
    Unauthorized,
    Forbidden,
}

async fn verify_request_user(
    headers: &HeaderMap,
    auth_provider: &config::AuthProvider,
    allowed_domain: &Option<String>,
    google_oauth_client_id: &Option<String>,
) -> Result<VerifiedUser, AuthHttp> {
    if *auth_provider == config::AuthProvider::None {
        let email = headers
            .get("X-Debug-User")
            .and_then(|v| v.to_str().ok())
            .ok_or(AuthHttp::Unauthorized)?;
        // Optional domain enforcement even in none-mode
        if let Some(dom) = allowed_domain {
            if !email
                .rsplit_once('@')
                .map(|(_, d)| d.eq_ignore_ascii_case(dom))
                .unwrap_or(false)
            {
                return Err(AuthHttp::Forbidden);
            }
        }
        return Ok(VerifiedUser {
            email: email.to_string(),
            sub: "debug".into(),
        });
    }

    // Google mode
    let auth = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(AuthHttp::Unauthorized)?;
    let token = auth.strip_prefix("Bearer ").ok_or(AuthHttp::Unauthorized)?;
    // These are validated at startup when auth_provider=Google, so unwrap is safe here
    let aud = google_oauth_client_id
        .as_ref()
        .ok_or(AuthHttp::Unauthorized)?;
    let allowed = allowed_domain.as_ref().ok_or(AuthHttp::Unauthorized)?;
    match google_auth::verify_async(token, aud, allowed).await {
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

#[derive(Deserialize)]
struct ListQuery {
    limit: Option<usize>,
    offset: Option<usize>,
    search: Option<String>,
    created_by: Option<String>,
    group_id: Option<String>,
    include_deleted: Option<bool>,
}

async fn list_links(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    // Auth
    let verified = match verify_request_user(
        &headers,
        &state.auth_provider,
        &state.allowed_domain,
        &state.google_oauth_client_id,
    )
    .await
    {
        Ok(v) => v,
        Err(AuthHttp::Unauthorized) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(http_common::json_error_with_message(
                    "unauthorized",
                    "missing or invalid token",
                )),
            )
                .into_response()
        }
        Err(AuthHttp::Forbidden) => {
            return (
                StatusCode::FORBIDDEN,
                Json(http_common::json_error_with_message(
                    "forbidden",
                    "domain not allowed",
                )),
            )
                .into_response()
        }
    };

    let limit = match q.limit {
        Some(n) if (1..=500).contains(&n) => n,
        Some(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(http_common::json_error_with_message(
                    "invalid_request",
                    "limit must be between 1 and 500",
                )),
            )
                .into_response()
        }
        None => 50, // default
    };
    let offset = q.offset.unwrap_or(0);

    // Build list options for paginated query
    let created_by = q
        .created_by
        .as_ref()
        .and_then(|e| UserEmail::new(e.clone()).ok());
    let options = domain::ListOptions {
        limit,
        offset,
        search: q.search.clone(),
        created_by,
        group_id: q.group_id.clone(),
        include_deleted: q.include_deleted.unwrap_or(false),
    };

    match state.repo.list_paginated(&options) {
        Ok(result) => {
            let links: Vec<LinkOut> = result
                .items
                .into_iter()
                .map(|l| link_to_out(l, &headers, &state.shortlink_domain))
                .collect();
            let user_info = UserInfo {
                email: verified.email.clone(),
                is_admin: is_admin(&verified.email),
            };
            (
                StatusCode::OK,
                Json(ListOut {
                    links,
                    total: result.total,
                    has_more: result.has_more,
                    user: Some(user_info),
                }),
            )
                .into_response()
        }
        Err(e) => {
            error!(err=?e, "list error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(http_common::json_error_with_message(
                    "internal",
                    "server error",
                )),
            )
                .into_response()
        }
    }
}

async fn preflight_links() -> impl IntoResponse {
    StatusCode::NO_CONTENT
}

async fn preflight_link() -> impl IntoResponse {
    StatusCode::NO_CONTENT
}

async fn update_link(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug_str): Path<String>,
    Json(body): Json<UpdateLinkReq>,
) -> impl IntoResponse {
    // Auth
    let verified = match verify_request_user(
        &headers,
        &state.auth_provider,
        &state.allowed_domain,
        &state.google_oauth_client_id,
    )
    .await
    {
        Ok(v) => v,
        Err(AuthHttp::Unauthorized) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(http_common::json_error_with_message(
                    "unauthorized",
                    "missing or invalid token",
                )),
            )
                .into_response()
        }
        Err(AuthHttp::Forbidden) => {
            return (
                StatusCode::FORBIDDEN,
                Json(http_common::json_error_with_message(
                    "forbidden",
                    "domain not allowed",
                )),
            )
                .into_response()
        }
    };

    // Parse slug
    let slug = match Slug::new(slug_str.clone()) {
        Ok(s) => s,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(http_common::json_error_with_message(
                    "invalid_request",
                    "invalid slug",
                )),
            )
                .into_response()
        }
    };

    // Get existing link
    let mut link = match state.repo.get(&slug) {
        Ok(Some(l)) => l,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(http_common::json_err("not_found")),
            )
                .into_response()
        }
        Err(e) => {
            error!(err=?e, "get error");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(http_common::json_error_with_message(
                    "internal",
                    "server error",
                )),
            )
                .into_response();
        }
    };

    // Check ownership or admin
    let user_is_admin = is_admin(&verified.email);
    if link.created_by.as_str() != verified.email && !user_is_admin {
        return (
            StatusCode::FORBIDDEN,
            Json(http_common::json_error_with_message(
                "forbidden",
                "not link owner",
            )),
        )
            .into_response();
    }

    // Apply updates
    if let Some(url) = body.original_url {
        if let Err(e) = domain::validate::validate_original_url(&url) {
            return (
                StatusCode::BAD_REQUEST,
                Json(http_common::json_error_with_message(
                    "invalid_request",
                    &format!("{}", e),
                )),
            )
                .into_response();
        }
        link.original_url = url;
    }
    if let Some(active) = body.is_active {
        link.is_active = active;
    }
    if let Some(desc) = body.description {
        link.description = desc;
    }
    if let Some(expires) = body.expires_at {
        link.expires_at = expires.and_then(|s| http_common::parse_rfc3339(&s).ok());
    }
    if let Some(activate) = body.activate_at {
        link.activate_at = activate.and_then(|s| http_common::parse_rfc3339(&s).ok());
    }
    if let Some(delay) = body.redirect_delay {
        link.redirect_delay = delay;
    }
    if let Some(gid) = body.group_id {
        link.group_id = gid;
    }
    link.updated_at = Some(state.clock.now());

    // Save
    match state.repo.update(&link) {
        Ok(()) => {
            info!(slug = %slug_str, "update ok");
            (
                StatusCode::OK,
                Json(link_to_out(link, &headers, &state.shortlink_domain)),
            )
                .into_response()
        }
        Err(CoreError::NotFound) => (
            StatusCode::NOT_FOUND,
            Json(http_common::json_err("not_found")),
        )
            .into_response(),
        Err(e) => {
            error!(err=?e, "update error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(http_common::json_error_with_message(
                    "internal",
                    "server error",
                )),
            )
                .into_response()
        }
    }
}

async fn delete_link(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug_str): Path<String>,
) -> impl IntoResponse {
    // Auth
    let verified = match verify_request_user(
        &headers,
        &state.auth_provider,
        &state.allowed_domain,
        &state.google_oauth_client_id,
    )
    .await
    {
        Ok(v) => v,
        Err(AuthHttp::Unauthorized) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(http_common::json_error_with_message(
                    "unauthorized",
                    "missing or invalid token",
                )),
            )
                .into_response()
        }
        Err(AuthHttp::Forbidden) => {
            return (
                StatusCode::FORBIDDEN,
                Json(http_common::json_error_with_message(
                    "forbidden",
                    "domain not allowed",
                )),
            )
                .into_response()
        }
    };

    // Parse slug
    let slug = match Slug::new(slug_str.clone()) {
        Ok(s) => s,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(http_common::json_error_with_message(
                    "invalid_request",
                    "invalid slug",
                )),
            )
                .into_response()
        }
    };

    // Get existing link to check ownership
    let link = match state.repo.get(&slug) {
        Ok(Some(l)) => l,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(http_common::json_err("not_found")),
            )
                .into_response()
        }
        Err(e) => {
            error!(err=?e, "get error");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(http_common::json_error_with_message(
                    "internal",
                    "server error",
                )),
            )
                .into_response();
        }
    };

    // Check ownership or admin
    let user_is_admin = is_admin(&verified.email);
    if link.created_by.as_str() != verified.email && !user_is_admin {
        return (
            StatusCode::FORBIDDEN,
            Json(http_common::json_error_with_message(
                "forbidden",
                "not link owner",
            )),
        )
            .into_response();
    }

    // Soft delete
    let deleted_at = state.clock.now();
    match state.repo.delete(&slug, deleted_at) {
        Ok(()) => {
            info!(slug = %slug_str, "delete ok");
            (StatusCode::NO_CONTENT, ()).into_response()
        }
        Err(CoreError::NotFound) => (
            StatusCode::NOT_FOUND,
            Json(http_common::json_err("not_found")),
        )
            .into_response(),
        Err(e) => {
            error!(err=?e, "delete error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(http_common::json_error_with_message(
                    "internal",
                    "server error",
                )),
            )
                .into_response()
        }
    }
}

async fn bulk_delete_links(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BulkSlugsReq>,
) -> impl IntoResponse {
    // Auth
    let verified = match verify_request_user(
        &headers,
        &state.auth_provider,
        &state.allowed_domain,
        &state.google_oauth_client_id,
    )
    .await
    {
        Ok(v) => v,
        Err(AuthHttp::Unauthorized) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(http_common::json_error_with_message(
                    "unauthorized",
                    "missing or invalid token",
                )),
            )
                .into_response()
        }
        Err(AuthHttp::Forbidden) => {
            return (
                StatusCode::FORBIDDEN,
                Json(http_common::json_error_with_message(
                    "forbidden",
                    "domain not allowed",
                )),
            )
                .into_response()
        }
    };

    // Only admins can bulk delete
    if !is_admin(&verified.email) {
        return (
            StatusCode::FORBIDDEN,
            Json(http_common::json_error_with_message(
                "forbidden",
                "admin required for bulk operations",
            )),
        )
            .into_response();
    }

    // Parse slugs
    let slugs: Vec<Slug> = body
        .slugs
        .iter()
        .filter_map(|s| Slug::new(s.clone()).ok())
        .collect();

    if slugs.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(http_common::json_error_with_message(
                "invalid_request",
                "no valid slugs provided",
            )),
        )
            .into_response();
    }

    let deleted_at = state.clock.now();
    match state.repo.bulk_delete(&slugs, deleted_at) {
        Ok(affected) => {
            info!(count = affected, "bulk delete ok");
            (StatusCode::OK, Json(BulkResultOut { affected })).into_response()
        }
        Err(e) => {
            error!(err=?e, "bulk delete error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(http_common::json_error_with_message(
                    "internal",
                    "server error",
                )),
            )
                .into_response()
        }
    }
}

async fn bulk_activate_links(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BulkSlugsReq>,
) -> impl IntoResponse {
    bulk_update_active_impl(state, headers, body, true).await
}

async fn bulk_deactivate_links(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BulkSlugsReq>,
) -> impl IntoResponse {
    bulk_update_active_impl(state, headers, body, false).await
}

async fn bulk_update_active_impl(
    state: AppState,
    headers: HeaderMap,
    body: BulkSlugsReq,
    is_active: bool,
) -> impl IntoResponse {
    // Auth
    let verified = match verify_request_user(
        &headers,
        &state.auth_provider,
        &state.allowed_domain,
        &state.google_oauth_client_id,
    )
    .await
    {
        Ok(v) => v,
        Err(AuthHttp::Unauthorized) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(http_common::json_error_with_message(
                    "unauthorized",
                    "missing or invalid token",
                )),
            )
                .into_response()
        }
        Err(AuthHttp::Forbidden) => {
            return (
                StatusCode::FORBIDDEN,
                Json(http_common::json_error_with_message(
                    "forbidden",
                    "domain not allowed",
                )),
            )
                .into_response()
        }
    };

    // Only admins can bulk update
    if !is_admin(&verified.email) {
        return (
            StatusCode::FORBIDDEN,
            Json(http_common::json_error_with_message(
                "forbidden",
                "admin required for bulk operations",
            )),
        )
            .into_response();
    }

    // Parse slugs
    let slugs: Vec<Slug> = body
        .slugs
        .iter()
        .filter_map(|s| Slug::new(s.clone()).ok())
        .collect();

    if slugs.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(http_common::json_error_with_message(
                "invalid_request",
                "no valid slugs provided",
            )),
        )
            .into_response();
    }

    let updated_at = state.clock.now();
    match state.repo.bulk_update_active(&slugs, is_active, updated_at) {
        Ok(affected) => {
            info!(
                count = affected,
                is_active = is_active,
                "bulk update active ok"
            );
            (StatusCode::OK, Json(BulkResultOut { affected })).into_response()
        }
        Err(e) => {
            error!(err=?e, "bulk update error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(http_common::json_error_with_message(
                    "internal",
                    "server error",
                )),
            )
                .into_response()
        }
    }
}

async fn get_me(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    // Auth
    let verified = match verify_request_user(
        &headers,
        &state.auth_provider,
        &state.allowed_domain,
        &state.google_oauth_client_id,
    )
    .await
    {
        Ok(v) => v,
        Err(AuthHttp::Unauthorized) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(http_common::json_error_with_message(
                    "unauthorized",
                    "missing or invalid token",
                )),
            )
                .into_response()
        }
        Err(AuthHttp::Forbidden) => {
            return (
                StatusCode::FORBIDDEN,
                Json(http_common::json_error_with_message(
                    "forbidden",
                    "domain not allowed",
                )),
            )
                .into_response()
        }
    };

    let user_info = UserInfo {
        email: verified.email.clone(),
        is_admin: is_admin(&verified.email),
    };
    (StatusCode::OK, Json(user_info)).into_response()
}

/// Build short URL using shortlink_domain from config, or Host header as fallback.
fn build_short_url(headers: &HeaderMap, slug: &str, shortlink_domain: &Option<String>) -> String {
    let host = shortlink_domain
        .as_deref()
        .or_else(|| headers.get("host").and_then(|v| v.to_str().ok()))
        .unwrap_or("");
    http_common::build_short_url_from_host(host, slug)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, Request};
    use tower::util::ServiceExt;

    fn app() -> Router {
        let state = AppState {
            repo: AnyRepo::memory(),
            slugger: Base62SlugGenerator::new(5),
            clock: StdClock,
            auth_provider: config::AuthProvider::None,
            allowed_domain: None,
            google_oauth_client_id: None,
            shortlink_domain: None,
        };
        Router::new()
            .route("/:slug", get(get_slug))
            .route(
                "/api/links",
                post(create_link).get(list_links).options(preflight_links),
            )
            .with_state(state)
    }

    #[tokio::test]
    async fn create_and_resolve_flow() {
        let router = app();

        // Create
        let req = Request::builder()
            .method("POST")
            .uri("/api/links")
            .header("content-type", "application/json")
            .header("X-Debug-User", "user@example.com")
            .body(Body::from("{\"original_url\":\"https://example.com\"}"))
            .unwrap();

        let resp = router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        // List
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/links")
                    .header("X-Debug-User", "user@example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Resolve should redirect (308)
        // We don't know slug from response easily here without parsing; call service directly for a known slug path.
        // Instead, create with custom slug and then resolve it
        let req = Request::builder()
            .method("POST")
            .uri("/api/links")
            .header("content-type", "application/json")
            .header("X-Debug-User", "user@example.com")
            .body(Body::from(
                "{\"original_url\":\"https://e2.com\",\"alias\":\"custom1\"}",
            ))
            .unwrap();
        let resp = router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/custom1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::PERMANENT_REDIRECT);
        assert_eq!(
            resp.headers().get(header::LOCATION).unwrap(),
            "https://e2.com"
        );
    }
}

// Note: json_err, json_error_with_message, is_valid_alias, and system_time_to_rfc3339
// are now provided by the http-common crate.
