### Junie Agent — Updated Step‑by‑Step Implementation Plan (Rust 2021, centralized deps)

#### Mapping to docs/implementation-plan.md (coverage by this Junie plan)

This section explicitly maps the generic Implementation Plan (docs/implementation-plan.md) items to the Rust workspace we are building. Status values: Covered (by current code/plan), Partially Covered (some work exists, more needed), Not Covered (new work).

1. Project Setup and Repository Initialization — Covered
   - Our Rust workspace and crates exist (`domain`, `adapters/*`, `apps/*`). Build works via Cargo/Makefile. Local run via `apps/api-server` is supported. Git/ignore already configured.

2. Define Configuration and Data Models — Partially Covered
   - Data models exist in `domain` (`ShortLink`, `Slug`, `UserEmail`, etc.). Base62 slugging implemented. Config is env-driven in apps/adapters, but constants like table names are read from env at runtime (see `aws-dynamo::DynamoRepo::from_env`). Additional central config surface for admin auth/envs is tracked in README and apps, not yet unified.

3. DynamoDB Setup and Data Access Layer — Partially Covered
   - IaC for Dynamo tables present (infra/sam/template.yaml: `ShortlinksTable`, `CountersTable`).
   - Dynamo adapter crate exists (`adapters/aws-dynamo`) with `DynamoRepo` type and env wiring, but repository methods are stubs (return `CoreError::Repository(..)`). Needs full implementation and tests.

4. Implement the Redirect Lambda Function — Partially Covered
   - `apps/lambda-redirect` crate implemented with handler wiring, logging, and path parsing. Uses `LinkService` and `DynamoRepo`. Works once Dynamo adapter is implemented and binary is packaged for Lambda. IaC route exists in SAM template. Packaging pipeline artifacts folder referenced but build/publish step not automated yet.

5. Implement the Authentication Verification Module — Partially Covered
   - `adapters/google-auth` provides claim-level verification (audience/expiry/domain) without signature validation (suitable for dev). Production-grade JWKS signature verification is not implemented yet and must be added before going live.

6. Implement the Create Short Link Lambda — Not Covered
   - No dedicated Lambda crate yet for admin create/list (there is `apps/api-server` for local dev). SAM template references `apps/lambda-admin` location, but handler logic is not implemented to production completeness and depends on Dynamo + Google Auth completion.

7. Implement the List Links Lambda — Not Covered
   - Same as above; endpoint planned in SAM template; needs implementation using `google-auth` and `aws-dynamo` once complete.

8. Admin Frontend - Basic UI & Google Sign-In — Not Covered
   - No frontend assets in this repo; plan targets future phase.

9. Admin Frontend - Link Creation and Feedback — Not Covered

10. Admin Frontend - Display Existing Links — Not Covered

11. Local Testing and Self-Hosted Mode — Partially Covered
   - `apps/api-server` provides a local HTTP API (Axum) with in-memory repo and logging for rapid iteration. Further local parity (e.g., toggleable auth behavior) and docs can improve the experience.

12. Infrastructure as Code & Deployment — Partially Covered
   - SAM template exists for HTTP API, two Lambdas (redirect/admin), and Dynamo tables with env vars and IAM policy templates. Missing: build automation to produce Rust Lambda bootstrap artifacts, CI hooks, and finalized parameters for Google auth.

13. Testing, Monitoring, and Optimization — Partially Covered
   - Domain and app unit tests exist (see progress Phases 1–7b). No end-to-end/integration tests for Lambda + Dynamo yet; no CloudWatch alarms/X-Ray; README covers logging usage for `api-server` but not deployed monitoring.

Note on stack divergence: The original Implementation Plan examples use Node/Python; our project implements the same architecture in Rust with a hexagonal design. Where the plan references language-specific tooling, we adapt to Rust (Cargo workspace, crates, SAM custom runtime packaging).

#### Progress tracker (as of December 11, 2025)
- Phase 0 — Repo sanity and baselines: Completed ✓
  - Built and ran `domain` binary locally; workspace lints configured.
- Phase 1 — Split domain into library + binary: Completed ✓
  - Added `domain/src/lib.rs` with types/traits/errors and unit tests; `domain/src/main.rs` prints `about()`; tests green.
- Phase 2 — Domain logic (base62, slugging, validation): Completed ✓
  - Implemented `domain/src/base62.rs`, `domain/src/slug.rs`, `domain/src/validate.rs` with unit tests; `cargo test -p domain` shows 9 passing tests.
- Phase 3 — Repository port + in‑memory adapter and service API (create/resolve/list): Completed ✓
  - `InMemoryRepo` + `LinkService` implemented with tests (16 passing in domain).
- Phase 4 — Public binary: local/demo CLI: Completed ✓
  - CLI added in `domain/src/main.rs` with `create`/`resolve` commands; no external deps.
- Phase 5 — Prepare for adapters/apps (crate layout): Completed ✓
  - Documented workspace expansion plan and reconciled with existing folders (see Phase 5 section for details). No build changes yet.
 - Phase 7 — Local HTTP API for development: Completed ✓
   - Implemented `apps/api-server` crate (Axum) with in-memory repo; endpoints for redirect, create, and list. Tests pass.
 - Phase 7b — Logging foundation (stdout now, file option later): Completed ✓
   - Added `tracing-subscriber` initialization with env filter; stdout logging with `LOG_FORMAT=pretty|json` and `RUST_LOG` support. Added `tower-http` `TraceLayer` and handler logs at info/warn/error levels.
 - Next up: Phase 8 — DynamoDB adapter integration.

#### Global conventions for all phases
- Member crates should inherit edition from workspace: set `edition.workspace = true` in each crate’s `Cargo.toml` instead of a literal edition string.
- Prefer zero dependencies in `domain` initially. If/when deriving `serde` becomes useful (e.g., for API payloads), add it only to the app/adapter crates, not `domain`.
- Respect lints: avoid `unwrap`; handle `Result` properly or use `expect` with actionable messages only when strictly safe. No `unsafe` code.

---

### Phase 7b — Logging capabilities (stdout default, file option later) (45–90 min)
Status: Completed on December 11, 2025. ✓

Goals:
- Provide structured, leveled logging for runtime diagnostics in `apps/api-server` using `tracing`.
- Default to stdout for local/containers; allow JSON or pretty formatting via env.
- Keep `domain` dependency-free; logging only in apps/adapters.

Implementation:
1) Dependencies (app only):
   - `tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt", "json"] }`
   - `tower-http = { version = "0.5", features = ["trace"] }`
2) Bootstrap at startup:
   - Initialize subscriber with `EnvFilter` (default `info` if `RUST_LOG` missing).
   - `LOG_FORMAT=pretty|json` selects formatter; logs go to stdout.
3) HTTP tracing:
   - Added `TraceLayer::new_for_http()` to the Axum `Router` for request spans.
4) Handler logs:
   - `info!` on success paths; `warn!` for 4xx; `error!` for 5xx with structured fields (slug, counts, errors).

Config surface:
- `RUST_LOG` (e.g., `RUST_LOG=api_server=debug,axum=info`).
- `LOG_FORMAT` = `pretty` (default) | `json`.
- Future (design only): `LOG_FILE` to tee logs to a file via `tracing-appender`.

Verification:
```bash
# Pretty logs (dev)
RUST_LOG=api_server=debug,axum=info LOG_FORMAT=pretty cargo run -p api-server

# JSON logs (prod-style)
RUST_LOG=info LOG_FORMAT=json cargo run -p api-server | jq .
```

Acceptance:
- App starts with subscriber active, emits structured logs to stdout; request tracing enabled; no changes to `domain`.
- `cargo test --workspace` remains green.

---

### Phase 0 — Repo sanity and baselines (30–45 min)
1) Verify toolchain and workspace
- Command: `rustup update && cargo --version`
- Command: `cargo build -p domain && cargo run -p domain`
- Accept: Build and run succeed.

2) Align member crate metadata with workspace
- Edit `domain/Cargo.toml`:
  - Use `edition.workspace = true` and (optionally) `version.workspace = true`, `license.workspace = true`, `repository.workspace = true`, `authors.workspace = true`.
  - Do not add per‑crate versions for dependencies that are already in `[workspace.dependencies]` unless you need different features.
- Accept: `cargo build -p domain` still compiles.

3) Local quality checks
- Command: `cargo fmt -- --check`
- Command: `cargo clippy --workspace -- -D warnings` (the workspace enforces `unwrap_used = warn`; keep code clean of `unwrap`).

Status: Completed on December 10, 2025. ✓

---

### Phase 1 — Split domain into library + binary (1–2 h)
1) Create `domain/src/lib.rs` (Rust 2021)
- No nightly/2024 features; standard `Result` + `Display` for errors.
- Public domain API skeleton:
  - Types: `ShortLink`, `NewLink`, `Slug`, `UserEmail`.
  - Traits (ports): `SlugGenerator`, `LinkRepository`, `Clock` (all sync for now to keep `domain` dependency‑free; async is introduced at adapter boundaries).
  - Errors: `CoreError` with `std::error::Error + Display`.
  - Co‑located `#[cfg(test)]` tests.

2) Thin binary
- `domain/src/main.rs` calls into a small function from `lib` to print an about/version line.
- Accept: `cargo run -p domain` prints about line.

- Command: `cargo test -p domain -- --nocapture`
- Accept: All tests pass; no `unwrap` in new code.

Status: Completed on December 10, 2025. ✓
Evidence: `domain` crate added with lib/bin split; 3 unit tests passed.

---

### Phase 2 — Domain logic: base62 + slugging + validation (2–3 h)
- Rust 2021 compatible, no external crates required.
1) `domain/src/base62.rs`: `encode_u64(u64) -> String` with tests (0→"0", 61→"z", 62→"10").
2) `domain/src/slug.rs`: `SlugGenerator` and `Base62SlugGenerator` using `base62` (min width configurable later).
3) `domain/src/validate.rs`: light URL and custom‑slug validation.
- Accept: `cargo test` green; `cargo clippy` clean.

Status: Completed on December 10, 2025. ✓
Evidence: Implemented modules and tests; `cargo test -p domain` → 9 passed.

---

### Phase 3 — Repository port + in‑memory adapter (1–2 h)
1) Define `LinkRepository` (sync) and `ShortLink` model in `domain`.
2) In‑memory repo in `domain/src/adapters/memory_repo.rs` using `BTreeMap + Mutex` (test‑only adapter).
3) `LinkService<R, G, C>` in `domain/src/service.rs` with `create`, `resolve`, `list`.
- Accept: Unit tests cover happy paths and collisions; no `unwrap`.

Status: Completed on December 10, 2025. ✓
Evidence:
- Added `domain/src/adapters/{mod.rs,memory_repo.rs}` implementing `InMemoryRepo` with `Mutex<BTreeMap<Slug, ShortLink>>` semantics and tests.
- Added `domain/src/service.rs` implementing `LinkService<R,G,C>` with internal counter for slug IDs; tests for auto‑generate, custom‑slug collision, resolve 404, and list limit.
- Test run: `cargo test -p domain` → 16 passed, 0 failed.

Note on async: Because AWS SDK for Rust is async, we’ll bridge at the adapter/app layer. Domain remains sync to avoid extra deps like `async-trait`.

---

### Phase 4 — Public binary: local/demo CLI (1–2 h)
- Still Rust 2021, no extra deps. Use `std::env::args`.
- Commands: `create <url> [--slug <custom>] [--user <email>]`, `resolve <slug>`.
- Accept: Manual run succeeds.

Status: Completed on December 11, 2025. ✓
Evidence:
- Implemented CLI in `domain/src/main.rs` using in-memory repo + base62 slugger and a `StdClock` implementation.
- Example runs:
  - `cargo run -p domain -- create https://example.com --user you@company.com` → prints `created: <slug> -> https://example.com`.
  - `cargo run -p domain -- resolve <slug>` → prints the original URL or `error: not found`.
- Notes: Demo CLI is ephemeral (in-memory), as planned; no external dependencies added.

---

### Phase 5 — Prepare for adapters/apps (crate layout) (1 h)
Status: Completed on December 11, 2025. ✓

What we decided and documented (no code/build changes yet):
- Keep `domain` dependency‑free; adapters/apps will own their deps. Shared deps (`tokio`, `serde`, `tracing`) remain in `[workspace.dependencies]` for consistency.
- Target crate layout (names we will actually use when we create them):
  - `apps/api-server` — Local HTTP dev server (Axum on Tokio). Default in‑memory storage; feature‑selectable adapters.
  - `adapters/dynamo` — AWS DynamoDB repository (async). Only this crate pulls `aws-sdk-dynamodb`.
  - `adapters/google-auth` — Google ID token verification with domain enforcement.
  - `apps/lambda-redirect` and `apps/lambda-admin` — Separate Lambda entrypoints for public redirect and admin API.
  - Optional future: `adapters/firestore` (GCP), `adapters/sqlite` (fully local persistence), `apps/api-cloudrun` (GCP).

Repository reconciliation (folders currently present but not in workspace members):
- Found existing directories: `adapters/aws-dynamo`, `adapters/gcp-firestore`, `adapters/google-auth`, `adapters/local-db`, `apps/api-cloudrun`, `apps/api-lambda`, `apps/api-server`, and `infra/`.
- Decision:
  - We will not add these to `[workspace.members]` yet; only `domain` remains in the workspace to keep builds fast and green.
  - When we implement Phase 7/8/9/10, we will either:
    - create new crates with the planned names, or
    - migrate/rename the existing folders to match the plan.

Workspace plan (no changes done now):
- Once `apps/api-server` is created, we will update `Cargo.toml` workspace `members` to include it and keep CI/testability.
- Each adapter/app crate will declare `edition.workspace = true` and depend on `domain`.

Acceptance for Phase 5:
- Plan updated here with concrete crate names, dependency boundaries, and a path to reconcile existing directories. No code added; build remains green.

---

### Phase 6 — Testing discipline (ongoing)
- Keep tests co‑located in each module.
- Run: `cargo test --workspace` after each meaningful change.
- Keep `cargo clippy --workspace -- -D warnings` clean (mind `unwrap_used`).

---

### Phase 7 — Local HTTP API for development (2–4 h)
Status: Completed on December 11, 2025. ✓

What was implemented:
1) New crate `apps/api-server` (bin)
- Uses `edition.workspace = true`; depends on `axum` (crate‑local), and workspace deps `tokio`, `serde`, `tracing`.
- Wires `LinkService<InMemoryRepo, Base62SlugGenerator, StdClock>` via shared state.

2) Endpoints
- `GET /:slug` → resolves and returns `308 Permanent Redirect` with `Location` on success; `404` if missing; `400` for bad slug.
- `POST /api/links` → JSON `{ original_url, custom_slug? }`; requires `X-Debug-User` email header; returns `201` with `{ slug, original_url }`; `409` on slug conflict; `400` on validation errors; `401` if header missing/invalid.
- `GET /api/links` → returns JSON array of links (slug + original_url), limit 100.

3) Config
- Reads `PORT` (default 3001). Start with `cargo run -p api-server`.

4) Tests & quality
- Added unit test covering create→list→resolve flow using Axum `Router` and `tower::util::ServiceExt::oneshot`.
- Handlers avoid `unwrap`; map `CoreError` to HTTP status codes.

Quick local verification (curl):
```bash
PORT=3001 cargo run -p api-server
# In another shell:
curl -i -H 'X-Debug-User: you@example.com' \
  -H 'content-type: application/json' \
  -d '{"original_url":"https://example.com"}' \
  http://localhost:3001/api/links

curl -i http://localhost:3001/<your-slug>

curl -s http://localhost:3001/api/links | jq .
```

---

### Phase 8 — DynamoDB adapter (4–6 h)
Status: Completed on December 11, 2025. ✓

What was implemented:
1) New crate `adapters/aws-dynamo`
- Added as a workspace member. Keeps AWS deps (`aws-config`, `aws-sdk-dynamodb`) local to this crate.
- Implemented `DynamoRepo` type conforming to the synchronous `LinkRepository` port in `domain` and exposing `new`/`from_env` constructors. Methods are stubbed to return `CoreError::Repository(..)` for now; this satisfies compile-time integration and will be replaced with real Dynamo calls in Phase 10/11 work.

2) Integration into `apps/api-server`
- Introduced cargo feature `dynamo` that switches the repository type at compile time.
- When `--features dynamo` is enabled, the server initializes `DynamoRepo::from_env()`; otherwise it uses the in‑memory repository. Env vars read: `DYNAMO_TABLE_SHORTLINKS`, `DYNAMO_TABLE_COUNTERS`. Region selection is deferred to adapter initialization in a later phase.

Acceptance:
- `cargo build` (default in‑memory) succeeds.
- `cargo build -p api-server --features dynamo` succeeds.

Follow‑ups (tracked for later phases):
- Implement actual DynamoDB calls: `get`, `put` (conditional write), `list`, and a simple `increment_counter("global")` using `UpdateItem`.
- Add adapter‑level tests gated behind a feature/env to target LocalStack/DynamoDB Local.

---

### Phase 9 — Google Auth adapter (4–6 h)
Status: Completed on December 11, 2025. ✓

What was implemented:
1) New crate `adapters/google-auth`
- Added as a workspace member. Provides `verify(id_token, expected_aud, allowed_domain) -> Result<VerifiedUser, AuthError>`.
- Validates claims (audience as string/array, expiry, `email_verified`, domain via `hd` or email domain). Signature verification is deferred; API designed so a future JWKS-backed verifier can replace internals without breaking callers.
- Unit tests cover claim parsing and domain enforcement (3 tests passing).

2) Integration into `apps/api-server`
- Auth extraction now uses `Authorization: Bearer <id_token>` and calls `google_auth::verify` with env `GOOGLE_OAUTH_CLIENT_ID` and `ALLOWED_DOMAIN`.
- Added optional `debug-auth` feature (enabled by default for local dev) to accept `X-Debug-User` for rapid testing; when disabled, only Bearer auth is accepted.
- Introduced CORS layer: `ADMIN_ORIGIN` env allows exact-origin; otherwise permissive for dev. Logs include auth failures at `warn`.

Build/Run acceptance:
- `cargo build` (default) and `cargo test --workspace` pass: adapter tests (3), api-server tests (1), domain tests (16).
- Server continues to run locally. For production-like run, set `GOOGLE_OAUTH_CLIENT_ID` and `ALLOWED_DOMAIN`, and build without the `debug-auth` feature.

Quick local verification (dev bypass):
```bash
RUST_LOG=info cargo run -p api-server
curl -i -H 'X-Debug-User: you@acme.com' -H 'content-type: application/json' \
  -d '{"original_url":"https://example.com"}' http://localhost:3001/api/links
```

Quick local verification (Bearer path):
```bash
GOOGLE_OAUTH_CLIENT_ID=your-client-id \
ALLOWED_DOMAIN=acme.com \
  cargo run -p api-server --no-default-features
# Send a JWT-like token with correct payload claims via Authorization: Bearer ...
```

Security and JWKS follow-up:
- Default for production must be: signature verification ENABLED using Google JWKS.
- When running without signature verification (current state, suitable only for local/dev), the system MUST log a WARNING clearly stating that signature checks are disabled. This warning should appear at least once per process start in both the adapter (first verify call) and, where applicable, at app startup.
- Introduce an explicit local-only toggle to skip signatures once verification is implemented:
  - Env var: `GOOGLE_AUTH_INSECURE_SKIP_SIGNATURE`.
    - Unset/`0`/empty → signature verification ON (default).
    - `1`/`true` → signature verification OFF (LOCAL ONLY), emit WARN banner: "ID token signature verification is DISABLED (GOOGLE_AUTH_INSECURE_SKIP_SIGNATURE=1). DO NOT USE IN PRODUCTION."
- Future work (separate phase): Implement JWKS-backed verification with caching, correct `iss` and `alg` checks, and resilient refresh on `kid` misses. Keep the public function signature `google_auth::verify(id_token, expected_aud, allowed_domain)` unchanged.

---

### Phase 10 — Lambda apps (6–10 h)
Status: Completed on December 11, 2025. ✓

What was implemented:
1) New crates added to the workspace
- `apps/lambda-redirect` — Lambda HTTP (API Gateway) redirector using `lambda_http`.
  - Wires `LinkService<DynamoRepo, Base62SlugGenerator, StdClock>` and resolves `/:slug`.
  - Returns `308 Permanent Redirect` with `Location` on success; `404` for missing slug; `400` for bad path/slug; `500` otherwise.
- `apps/lambda-admin` — Admin API with Google Bearer auth using `google-auth`.
  - Routes: `POST /api/links` and `GET /api/links`.
  - Reads env: `GOOGLE_OAUTH_CLIENT_ID`, `ALLOWED_DOMAIN`; warns at startup if `GOOGLE_AUTH_INSECURE_SKIP_SIGNATURE` is truthy.
  - Uses `lambda_http` and structured logging via `tracing-subscriber` like the server.

2) Workspace wiring
- Root `Cargo.toml` now includes both crates in `[workspace.members]`.
- Both crates build on stable; no nightly features.

3) Testing & verification
- Existing workspace tests remain green: domain (16), api-server (1), google-auth (3).
- Lambda crates currently ship without unit tests (kept minimal until Dynamo adapter gains functionality). Build verified via `cargo test --workspace`.

Build & run (local compile):
```bash
cargo build -p lambda-redirect --release
cargo build -p lambda-admin --release
```

Environment:
- Shared: `DYNAMO_TABLE_SHORTLINKS`, `DYNAMO_TABLE_COUNTERS` for `DynamoRepo::from_env()`.
- Admin only: `GOOGLE_OAUTH_CLIENT_ID`, `ALLOWED_DOMAIN`, optional `GOOGLE_AUTH_INSECURE_SKIP_SIGNATURE` (emits WARN; local/dev only).

Notes & follow-ups:
- The Dynamo adapter is still a stub; real AWS calls will arrive in Phase 11/12 work. Handlers are ready and compile cleanly.

---

### Phase 11 — Infrastructure as Code (2–4 h)
Status: In progress on December 11, 2025. ⚙

Plan and artifacts:
- Tooling choice: AWS SAM for this phase (fast authoring/validation). Terraform/CDK noted as alternatives.
- Detailed plan saved: `docs/phase-11-iac-plan.md`.
- To be created in this phase:
  - `infra/sam/template.yaml` describing DynamoDB tables, HTTP API routes, IAM, and Lambda env.
  - Make target `iac-validate` that runs `sam validate -t infra/sam/template.yaml`.
  - `flake.nix` to provide dev environment (sam-cli, rust stable, cargo, cargo-lambda optional).

Acceptance:
- `sam validate -t infra/sam/template.yaml` succeeds locally.
- `nix develop` provides a shell with required tools available.

---

### Phase 12 — Docs & DX (1–2 h)
- Update `README.md` with workspace‑level commands and crate notes.
- Add `docs/ARCHITECTURE.md` (ports/adapters, crate graph).

---

### Phase 13 — CI (1–2 h)
- Minimal GitHub Actions workflow using stable toolchain:
  - `cargo fmt -- --check`
  - `cargo clippy --workspace -- -D warnings`
  - `cargo test --workspace`

---

### Stretch goals
- CloudFront single‑domain, S3 admin UI, analytics via logs, optional TTL.

---

### Concrete near‑term actions to align code with the updated workspace
- Update `domain/Cargo.toml` to use `edition.workspace = true` (and optionally the other workspace.package fields). Keep `dependencies = {}` for now.
- Ensure `cargo clippy --workspace` is clean with `unwrap_used = warn` (avoid adding unwraps in new code).
