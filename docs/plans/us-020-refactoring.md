# US-020: Codebase Refactoring & Best Practices

This document outlines improvements identified during a comprehensive code review, organized by priority and effort.

## Overview

The codebase follows hexagonal architecture well, but has accumulated technical debt in error handling, code duplication, and configuration management. This plan addresses these issues systematically.

## Phase 1: Quick Wins (Low effort, High impact)

### 1.1 Fix JSON Serialization Panics
**Files:** `lambda-admin/src/main.rs`, `lambda-redirect/src/main.rs`

Replace `.unwrap()` on `serde_json` calls with proper error handling:
```rust
// Before
serde_json::to_value(out).unwrap()

// After
serde_json::to_value(out).map_err(|e| /* return 500 error */)?
```

**Locations:**
- `lambda-admin/src/main.rs:160, 190`
- `lambda-redirect/src/main.rs:92`

### 1.2 Standardize lambda_http Version
**Files:** `apps/lambda-redirect/Cargo.toml`

Update from `0.13` to `1.0.1` to match `lambda-admin`.

### 1.3 Use AtomicU64 for Counter
**File:** `domain/src/service.rs`

Replace `Mutex<u64>` with `AtomicU64` for better concurrency:
```rust
use std::sync::atomic::{AtomicU64, Ordering};

pub struct LinkService<R, S, C> {
    next_id: AtomicU64,
    // ...
}

fn reserve_id(&self) -> u64 {
    self.next_id.fetch_add(1, Ordering::Relaxed)
}
```

## Phase 2: Security Hardening (Medium effort, High impact)

### 2.1 CORS Configuration Validation
**File:** `apps/api-server/src/main.rs:127`

Fail at startup if `CORS_ALLOW_ORIGIN` is invalid instead of silently falling back to `*`.

### 2.2 Auth Mode Hardening
**File:** `apps/api-server/src/main.rs:275-284`

When `AUTH_PROVIDER=none`, require `ALLOWED_DOMAIN` to be set. Log warning about insecure mode.

### 2.3 Add Startup Warning for Signature Skip
**File:** `apps/api-server/src/main.rs`

Call `google_auth::warn_if_insecure_skip_sig()` at startup like lambda-admin does.

### 2.4 Tokio Runtime Error Propagation
**File:** `adapters/aws-dynamo/src/lib.rs:38`

Replace `.expect()` with proper error propagation in `DynamoRepo::with_client()`.

## Phase 3: Code Deduplication (Medium effort, Medium impact)

### 3.1 Extract Shared HTTP Utilities
Create new crate `crates/http-common` with:
- Response builders (`resp()`, `json_err()`, `resp_with_error()`)
- `build_short_url()` helper
- `is_valid_alias()` validation
- `parse_limit()` query parsing

**Affected files:**
- `apps/api-server/src/main.rs`
- `apps/lambda-admin/src/main.rs`
- `apps/lambda-redirect/src/main.rs`

### 3.2 Extract Auth Verification
Move `verify_request_user()` logic to shared location, potentially in `google-auth` crate or new `auth-common` crate.

## Phase 4: Configuration Management (Medium effort, High impact)

### 4.1 Centralized Config Struct
Create `Config` struct that validates all environment variables at startup:

```rust
pub struct Config {
    pub port: u16,
    pub auth_provider: AuthProvider,
    pub cors_allow_origin: HeaderValue,
    pub allowed_domain: Option<String>,
    pub storage_provider: StorageProvider,
    pub google_oauth_client_id: Option<String>,
    pub db_path: Option<PathBuf>,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        // Validate all at once, fail fast
    }
}
```

### 4.2 Validate Google OAuth Client ID at Startup
Currently fails at request time. Should validate in `build_repo_from_env()`.

## Phase 5: Observability Improvements (Low effort, Medium impact)

### 5.1 Request ID Correlation
Add X-Request-ID generation and logging for production debugging.

### 5.2 Consistent Logging Levels
Standardize: client errors (4xx) → `warn!`, server errors (5xx) → `error!`.

### 5.3 Tracing Spans
Add `tracing::info_span!()` to handlers for async correlation.

## Phase 6: Dependency Cleanup (Low effort, Low impact)

### 6.1 Remove Unused Dependencies
- Check if `thiserror` is actually used in `sqlite-adapter` and `google-auth`

### 6.2 Migrate from once_cell
Replace `once_cell::sync::Lazy` with `std::sync::OnceLock` (requires Rust 1.80+).

### 6.3 Pin Workspace Dependencies
Consider exact versions for reproducible builds.

## Phase 7: Testing Improvements (High effort, High impact)

### 7.1 Integration Tests
Add end-to-end tests: HTTP request → LinkService → Repository → Response.

### 7.2 Auth Negative Path Tests
- Expired token handling
- Invalid signature rejection
- Missing email_verified rejection

### 7.3 Edge Case Tests
- Maximum URL length (2048 chars)
- Concurrent slug generation
- DynamoDB timeout handling
- Malformed JSON payloads

### 7.4 Remove Dead Code
Remove unused `parse_limit()` in `api-server/src/main.rs:404`.

## Phase 8: API Consistency (Low effort, Medium impact)

### 8.1 Standardize Error Response Format
```json
{
  "error": {
    "code": "error_code",
    "message": "Human readable description"
  }
}
```

### 8.2 Alias Validation Alignment
Update `is_valid_alias()` to match `Slug::new()` validation (allow `-` and `_`).

### 8.3 Query Parameter Validation
Return 400 Bad Request for out-of-range `limit` instead of using default silently.

## Implementation Order

| Phase | Priority | Effort | Dependencies |
|-------|----------|--------|--------------|
| 1     | Critical | Low    | None         |
| 2     | High     | Medium | None         |
| 4     | High     | Medium | None         |
| 3     | Medium   | Medium | Phase 1      |
| 5     | Medium   | Low    | None         |
| 6     | Low      | Low    | None         |
| 7     | Medium   | High   | Phase 1-4    |
| 8     | Low      | Low    | Phase 3      |

## Acceptance Criteria

- [x] All `.unwrap()` on JSON serialization replaced with error handling (Phase 1)
- [x] lambda_http version consistent across workspace (Phase 1)
- [x] AtomicU64 used for counter in LinkService (Phase 1)
- [x] CORS configuration validated at startup (Phase 2)
- [x] Auth mode warns when AUTH_PROVIDER=none without ALLOWED_DOMAIN (Phase 2)
- [x] Startup warning for GOOGLE_AUTH_INSECURE_SKIP_SIGNATURE (Phase 2)
- [x] DynamoRepo::with_client returns Result instead of panicking (Phase 2)
- [x] Removed dead code (parse_limit, unused imports) (Phase 2)
- [x] Shared HTTP utilities extracted to http-common crate (Phase 3)
- [x] Config struct validates all env vars at startup (Phase 4)
- [x] X-Request-ID generation and logging added (Phase 5)
- [x] Logging levels standardized (4xx=warn, 5xx=error) (Phase 5)
- [x] Removed unused thiserror from sqlite-adapter (Phase 6)
- [x] Migrated from once_cell to std::sync::LazyLock (Phase 6)
- [x] Error response format standardized (Phase 8)
- [x] Alias validation aligned with Slug::new (allows hyphen/underscore) (Phase 8)
- [x] Query parameter validation returns 400 for invalid limit (Phase 8)
- [ ] Integration test suite added (Phase 7 - deferred)
