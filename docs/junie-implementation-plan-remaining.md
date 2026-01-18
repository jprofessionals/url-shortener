### Junie Agent — Remaining Work Plan (mapping gaps → concrete steps)

This document lists the remaining tasks not fully covered by the existing `docs/junie-implementation-plan.md`, mapped to the original `docs/implementation-plan.md`. It focuses on Rust + Hexagonal Architecture and AWS SAM custom runtime.

#### Summary of design adherence (from README/design docs)
- Architecture: Current workspace matches the hexagonal design in README/design-plan (domain, adapters, apps). ✓
- Redirect path on serverless: `apps/lambda-redirect` + SAM route implemented; returns 308 and reads from Dynamo via adapter. ✓
- Admin endpoints: Implemented in `apps/lambda-admin` (`/api/links` GET/POST) with auth, validation, and CORS. ✓
- Google auth: `adapters/google-auth` now verifies JWKS signatures (RS256) + claims; dev bypass via env for local. ✓
- IaC: SAM template updated with parameters/env, CORS, least-privilege IAM; Makefile builds Rust artifacts; CI added. ✓
- Frontend admin UI: Stack chosen — Svelte + Vite (client-only SPA). Not implemented yet. ✗

---

### Gap-driven tasks (actionable)

1. Phase 8 — DynamoDB adapter: Implement `DynamoRepo` fully — Status: Done ✓ (2025-12-15)
   - Files: `adapters/aws-dynamo/src/lib.rs`
   - Steps:
     - Add dependency: `aws-sdk-dynamodb` (smithy) with minimal features.
     - Config/env: read `DYNAMO_TABLE_SHORTLINKS`, `DYNAMO_TABLE_COUNTERS` (already wired) and create a `Client` via `aws_config::load_from_env`.
     - Implement methods:
       - `get(&Slug) -> Option<ShortLink>` via `GetItem`.
       - `put(ShortLink)` via `PutItem` with conditional expression to avoid overwrites on custom slug (map ConditionalCheckFailed to `CoreError::AlreadyExists`).
       - `list(limit) -> Vec<ShortLink>` via `Scan` with limit and basic item mapping.
     - Add helper: `increment_global_counter() -> u64` using `UpdateItem` on `Counters` (`name = "global"`, `ADD value :one`, return updated).
     - Map SDK errors to `CoreError::Repository` except `ConditionalCheckFailed` → `AlreadyExists`, `ResourceNotFound` → `Repository("missing table")`.
     - Tests:
       - Unit tests for item <-> domain mapping (pure functions).
       - Optional integration test behind env flag `AWS_INTEGRATION=1` using a real/sandbox table.

   - Implementation notes (by Junie):
     - Implemented `DynamoRepo` with real AWS SDK client, bridging sync trait via a small internal Tokio runtime.
     - Added `get`, `put` (with conditional expression), `list`, and `increment_global_counter` with proper error mapping.
     - Added unit test covering item <-> domain mapping; crate compiles and tests pass: `cargo test -p aws-dynamo`.
     - Verified `apps/lambda-redirect` builds against the adapter.

2. Phase 6/7 — Lambda Admin app: Create `apps/lambda-admin` — Status: Done ✓
   - Handlers:
     - `POST /api/links` (create):
       - Parse `Authorization: Bearer <id_token>`; verify using `adapters/google-auth`.
       - JSON body `{ original_url: String, alias?: String }`.
       - If alias present: validate -> attempt conditional `put` (on exists → 409).
       - Else: call `increment_global_counter` then derive slug via `Base62SlugGenerator`.
       - Store `ShortLink` with `created_at` and `created_by`.
       - Return 201 JSON `{ slug, short_url?, original_url }`.
     - `GET /api/links` (list):
       - Verify auth; call `list(limit=200)`; sort by `created_at` desc; return JSON `{ links: [...] }`.
   - Logging: initialize `tracing_subscriber` similarly to `lambda-redirect`.
   - CORS: ensure `Access-Control-Allow-Origin` and `-Headers` in responses if not handled at API Gateway.
   - Wire env: `GOOGLE_OAUTH_CLIENT_ID`, `ALLOWED_DOMAIN`, Dynamo table names.

   - Progress (by Junie):
     - Block A Spec finalized and saved: `docs/spec_admin_api.md` (API contract for create/list, auth, CORS, error model).
     - Implemented `POST /api/links` and `GET /api/links` per Spec with Google Bearer auth (JWKS by default, env bypass for local), Base62 alias validation, generated slugs with min length=5 via Dynamo counter, RFC3339 timestamps, and full response shape including `short_url` and `created_by`.
     - Added CORS headers from `CORS_ALLOW_ORIGIN` and preflight `OPTIONS /api/links` handler.
     - Sorting by `created_at` desc and `limit` query param (default 200). Placeholder pagination token.
     - Unit tests added for alias validation and `limit` parsing; workspace builds and tests pass: `cargo build --workspace && cargo test -p lambda-admin`.
     - Next: verify end-to-end with SAM local once packaging artifacts are in place (see Phase 12).

3. Phase 5 — Google Auth: Add production JWKS signature verification — Status: Done ✓
   - Extend `adapters/google-auth`:
    - Implemented: JWKS-based RS256 signature verification using `jsonwebtoken`; default enabled.
    - Implemented: In-memory JWKS cache with TTL; fetches from `https://www.googleapis.com/oauth2/v3/certs`.
    - Implemented: Dev bypass via `GOOGLE_AUTH_INSECURE_SKIP_SIGNATURE` env (emits WARN in apps).
    - Added unit tests using a synthetic RSA keypair and JWKS override to validate signature success and audience mismatch; kept existing claims-only tests for insecure mode.

4. Phase 12 — SAM packaging/build & deploy pipeline — Status: Done ✓
   - Build script:
     - Implemented: `Makefile` target `build-lambdas` to cross-compile `apps/lambda-redirect` and `apps/lambda-admin` for `x86_64-unknown-linux-gnu` and place as `infra/sam/artifacts/<crate>/bootstrap`.
     - Ensure `strip` binaries; keep `RUSTFLAGS='-C target-cpu=x86-64-v3'` optional.
   - SAM template updated (`infra/sam/template.yaml`):
     - Parameters required (no defaults): `AllowedDomain`, `GoogleOAuthClientId`, `ShortlinkDomain`, `CorsAllowOrigin`.
     - HttpApi (v2) with routes wired; CORS configured from `CorsAllowOrigin` (methods: OPTIONS, GET, POST; headers: Authorization, Content-Type).
     - Env wired for both Lambdas: `GOOGLE_OAUTH_CLIENT_ID`, `ALLOWED_DOMAIN`, `SHORTLINK_DOMAIN`, `CORS_ALLOW_ORIGIN`, `DYNAMO_TABLE_*`.
     - DynamoDB tables provisioned (on-demand): Shortlinks (pk slug S) and Counters (pk name S).
     - IAM tightened to least-privilege; removed Counters write from Redirect; Admin scoped to needed actions.
   - README updated with `make build-lambdas`, `sam validate`, and `sam deploy --guided` (region eu-west-1, stack `url-shortener-dev`).
   - GitHub Actions CI added (fmt, clippy -D warnings, test, make build-lambdas).

5. Phases 8–10 — Admin Frontend (Svelte + Vite SPA) — Status: Pending
   - Stack: Svelte + Vite (no SSR), client-only; hosted as static assets. 
   - Local run (DX):
     - Scaffold: `npm create vite@latest admin -- --template svelte`. ✓
     - Configure `.env.local` with:
       - `VITE_API_BASE=https://<api-id>.execute-api.<region>.amazonaws.com/dev` ✓
       - `VITE_GOOGLE_CLIENT_ID=<client-id>.apps.googleusercontent.com` ✓
     - Run: `npm run dev -- --open` (defaults to http://localhost:5173). ✓
     - Backend CORS: set SAM parameter `CorsAllowOrigin=http://localhost:5173` when running `sam local start-api`. ✓
   - Minimal-cost AWS deploy:
     - Build: `npm run build` → `dist/`. ✓
     - Option A (cheapest): S3 static website — `aws s3 sync dist/ s3://<admin-bucket>/ --delete`. ✓
     - Option B: Cloudflare Pages or GitHub Pages (zero-cost). ✓
     - Set SAM parameter `CorsAllowOrigin` to the admin site origin (S3/CloudFront/Pages). ✓
     - In Google Cloud Console, add the admin origin to OAuth Authorized JavaScript origins. ✓
   - Auth flow: Use Google Identity Services (GIS) to obtain an ID token; send to Admin API via `Authorization: Bearer <token>`; backend verifies JWKS signature + claims + domain. ✓
   - Deliverables: Link to README “Admin Frontend (Svelte + Vite)” for full instructions; initial minimal UI: sign-in, create, list. 

6. Phase 11 — Local testing parity and DX — Status: Pending
   - `apps/api-server` improvements:
     - Add env-driven selection of repo: `STORAGE_PROVIDER=local|aws|gcp` with existing `local-db` adapter; allow `aws-dynamo` once implemented.
     - Auth toggle: `AUTH_PROVIDER=none|google` and for google use `adapters/google-auth` with `insecure_no_sig` for local.
     - README quickstart commands and `.env.example`.

7. Phase 12 — IaC polishing — Status: Pending
   - SAM template tweaks:
     - Tighten IAM policies (remove counters write from redirect if not used).
     - Add CORS config on HttpApi for `/api/*` routes.
     - Outputs for Lambda ARNs and table names; parameters validation.
     - Optional: Custom domain + ACM + Route53 (future).

8. Phase 13 — Tests, monitoring, and ops — Status: Pending
   - Tests:
     - Add black-box tests for `apps/api-server` endpoints (Reqwest + axum testing), covering create/list/redirect happy-path and error cases.
     - Adapter tests (unit) for item mapping and error mapping; integration tests behind env flag for real AWS.
   - Monitoring:
     - Confirm CloudWatch Logs; add structured error logs.
     - Optional: CloudWatch Alarms on Lambda errors.

---

### Acceptance checklist by original plan items
- 3. Dynamo data access layer: CRUD + counter via Dynamo SDK; unit tests; optional integration tests.
- 4. Redirect Lambda: Works end-to-end against Dynamo; SAM route functional.
- 5. Auth module: JWKS signature verification enabled by default; dev bypass via feature/env.
- 6–7. Admin Lambdas: Implemented, tested locally (`sam local start-api`) and via unit tests.
- 8–10. Frontend: Minimal UI capable of sign-in, create, list; calls deployed API.
- 11. Local mode: `api-server` supports adapters and optional auth.
- 12. IaC & Deploy: `make build-lambdas` + `sam deploy` documented; parameters passed.
- 13. Tests/Monitoring: Core tests in place; logs visible; optional alarms.

---

### Next immediate steps (recommended sequence)
1) Implement `adapters/aws-dynamo` with SDK and unit tests. — Completed ✓
2) Build `apps/lambda-admin` (create/list) and wire SAM env/permissions. — Completed ✓
3) Add JWKS verification to `adapters/google-auth` (feature-gated), update apps to use it. — Completed ✓
4) Add Makefile build for Lambda artifacts; verify `sam local start-api` works; deploy to `StageName=dev`. — Completed ✓
5) Implement Admin Frontend using Svelte + Vite (client-only). Include README instructions for local dev and minimal-cost AWS deploy (S3 or free static hosts) and wire CORS. — Pending
