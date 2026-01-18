### Phase 10 — Lambda Apps: Technical Spec (Draft)

#### Goals
- Provide two AWS Lambda entrypoints for the URL Shortener:
  - `lambda-redirect`: public redirect function mapping a short slug to its target URL and returning an HTTP redirect (308).
  - `lambda-admin`: admin API for creating and listing links, protected by Google ID token auth (same rules as `api-server`).
- Use the existing `domain` types and `LinkService` with the DynamoDB adapter (`adapters/aws-dynamo`).
- Keep dependencies scoped to each Lambda crate. No changes to `domain`.
- Handlers should compile on stable and be testable locally (unit-style tests without AWS).

#### Crate layout to add
- `apps/lambda-redirect` (bin)
  - Purpose: Minimal HTTP redirector for API Gateway v2 (HTTP API).
  - Depends on: `domain`, `adapters/aws-dynamo`, `aws_lambda_events`, `lambda_runtime`, `serde`, `tracing`, `tracing-subscriber` (fmt/json), optionally `tower-http` not required here.

- `apps/lambda-admin` (bin)
  - Purpose: Admin API (POST/GET /api/links) for create/list operations.
  - Depends on: `domain`, `adapters/aws-dynamo`, `adapters/google-auth`, `aws_lambda_events`, `lambda_runtime`, `serde`, `serde_json`, `tracing`, `tracing-subscriber`.

Note: The pre-existing `apps/api-lambda` folder will not be modified in this phase. We will add two new crates whose names match the plan. A later housekeeping phase can remove or repurpose `apps/api-lambda`.

#### Event model and response
- API Gateway v2 (HTTP API) events using `aws_lambda_events::apigw::ApiGatewayV2httpRequest` and responses via `ApiGatewayV2httpResponse`.
- All responses must set appropriate status codes and headers. For redirect, include `Location` header.

#### Environment and configuration
- Shared:
  - `RUST_LOG` and `LOG_FORMAT` like `api-server` (pretty|json).
  - `DYNAMO_TABLE_SHORTLINKS`, `DYNAMO_TABLE_COUNTERS` for `DynamoRepo::from_env()`.
- Admin-only:
  - `GOOGLE_OAUTH_CLIENT_ID` and `ALLOWED_DOMAIN` for Bearer auth.
  - `GOOGLE_AUTH_INSECURE_SKIP_SIGNATURE`: if truthy, emit WARN banner indicating signature verification is disabled (local/dev only). Once JWKS verification exists, default will be verification ON.

#### Redirect Lambda (`lambda-redirect`) — handler behavior
1. Parse path parameter `slug` from the event (e.g., `/{slug}`).
2. Validate slug using `domain::Slug::new`.
3. Resolve via `LinkService<aws_dynamo::DynamoRepo, Base62SlugGenerator, StdClock>::resolve`.
4. Map results:
   - Ok(url) → 308 Permanent Redirect with `Location: <url>`.
   - NotFound → 404 with simple JSON `{error:"not_found"}`.
   - InvalidSlug → 400.
   - Other errors → 500.
5. Log at `info!/warn!/error!` like `api-server`.

#### Admin Lambda (`lambda-admin`) — handler behavior
Supported routes under API Gateway HTTP API:
- `POST /api/links`
  - Body: `{ "original_url": string, "custom_slug"?: string }`.
  - Auth: Read `Authorization: Bearer <id_token>`, verify via `google_auth::verify(token, aud, domain)` with env.
  - On success: create link via service and return `201` JSON `{ slug, original_url }`.
  - On failures: 400 (validation), 401 (auth), 409 (slug conflict), 500 (internal).

- `GET /api/links`
  - Optional auth for now: require the same Bearer token (consistent with `api-server`).
  - Return list (max 100) as JSON `[{ slug, original_url }]`.

Routing strategy:
- Use `request.raw_path()` and `request.request_context.http.method` to branch. No Axum; keep it dependency-light.

#### Construction of `LinkService`
- Both Lambdas will construct `DynamoRepo::from_env()` and wrap it in `LinkService<Repo, Base62SlugGenerator, StdClock>`.
- `Base62SlugGenerator` seed/width values can mirror `api-server` (e.g., `Base62SlugGenerator::new(1)`).
- `StdClock` as a local implementation of `domain::Clock` that returns `SystemTime::now()`.

#### Logging
- Initialize `tracing-subscriber` similarly to `api-server` with `EnvFilter` and `LOG_FORMAT`.
- On `GOOGLE_AUTH_INSECURE_SKIP_SIGNATURE` truthy, emit a single WARN banner at startup in `lambda-admin` and ensure the adapter also warns on first verify call.

#### Error mapping (HTTP)
- 308 Permanent Redirect on successful resolve (prefer 308 like `api-server`).
- 400 for bad slug or invalid request body.
- 401 for auth failures.
- 404 for missing slug.
- 409 for slug conflict.
- 500 for internal errors.

#### Testing strategy
- Unit tests per crate that:
  - Build a minimal `ApiGatewayV2httpRequest` with path/method/body and call the handler function directly (bypass the Lambda runtime loop).
  - Assert on `status_code`, headers, and body JSON where applicable.
  - For `lambda-admin` auth paths, generate payload-only tokens like in `adapters/google-auth` tests (no signature) and set `GOOGLE_OAUTH_CLIENT_ID` and `ALLOWED_DOMAIN` in the test environment.
  - Keep tests deterministic; avoid any network or real AWS calls.

#### Build and local run
- Compile:
  - `cargo build -p lambda-redirect --release`
  - `cargo build -p lambda-admin --release`
- Lambda packaging/cross-compilation to musl is out of scope for this phase; we only require release builds on the host.

#### Acceptance criteria
- Both crates compile on stable in release mode.
- Unit tests for both crates pass locally (`cargo test -p lambda-redirect`, `cargo test -p lambda-admin`).
- Redirect handler returns correct status/headers for success and 404.
- Admin handler supports POST and GET with auth, mapping errors to correct HTTP codes.
- No changes to `domain` API. No network calls in tests.

#### Implementation steps (next phase execution plan)
1. Add new workspace members in root `Cargo.toml` for `apps/lambda-redirect` and `apps/lambda-admin`.
2. Create crates with `main.rs` including a top-of-file crate doc explaining purpose and role.
3. Add minimal handler functions and Lambda runtime bootstrap (`lambda_runtime::run(service_fn(handler))`).
4. Wire `LinkService` with `DynamoRepo::from_env()` in both crates.
5. Implement auth extraction in `lambda-admin` using `google_auth` and env vars.
6. Implement tests using `aws_lambda_events` request/response structs.
7. Build with `--release` and run tests for both crates.
