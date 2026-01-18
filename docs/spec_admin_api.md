### Spec — Admin API (Block A)

Scope: This document defines the Admin API contract and authentication behavior for creating and listing short links. It reflects decisions agreed in Block A and is implementation‑ready for the Lambda Admin app and SAM configuration.

#### 1. Authentication & Authorization
- Transport: Authorization header only — `Authorization: Bearer <id_token>`.
- Verification (default, production):
  - Verify JWT signature via Google JWKS (RS256).
  - Validate `aud` equals `GOOGLE_OAUTH_CLIENT_ID`.
  - Validate `iss` is one of Google issuers (`https://accounts.google.com`, `accounts.google.com`).
  - Validate `exp` (token not expired) and `iat` sanity.
  - Extract `email` and require it ends with `@<ALLOWED_DOMAIN>` (case‑insensitive). Domain source = email suffix; do not rely solely on `hd` claim.
- Local/dev override: Signature verification may be disabled only if `GOOGLE_AUTH_INSECURE_SKIP_SIGNATURE=1` is set. Apps MUST log a WARN when disabled. All other claim checks still apply.
- Authorization model: Any authenticated user from the allowed domain is permitted to create and list links (single role: admin).

HTTP failures related to auth:
- 401 Unauthorized — missing/invalid token, signature failure, bad audience/issuer/expiry.
- 403 Forbidden — token valid but email domain is not allowed.

#### 2. CORS
- Header `Access-Control-Allow-Origin`: value taken from env `CORS_ALLOW_ORIGIN`; if unset, default to `*` in dev. For production, set explicit admin origin.
- Header `Access-Control-Allow-Headers`: `Authorization, Content-Type`.
- Header `Access-Control-Allow-Methods`: `OPTIONS, GET, POST`.
- Preflight: Handle `OPTIONS /api/links` returning 204 with the above headers.

#### 3. Data Model (response surface)
- Timestamp format: RFC3339 in UTC, e.g., `2025-12-15T13:45:00Z`.
- Creator identity: `created_by` is the full email (e.g., `alice@yourcompany.com`).

Link object (as returned by API):
```json
{
  "slug": "aZ19B",
  "short_url": "https://short.example.com/aZ19B",
  "original_url": "https://example.com/very/long/path",
  "created_at": "2025-12-15T13:45:00Z",
  "created_by": "alice@yourcompany.com"
}
```

#### 4. Slug policy
- Custom alias validation: Base62 only `[0-9A-Za-z]`, length 3..32. Reject others (400 invalid_request).
- Generated slugs: Base62 derived from a monotonically increasing counter (e.g., DynamoDB atomic counter → Base62). Minimal length is 5; codes grow in length only as needed to represent the counter value.

#### 5. Endpoints

##### 5.1 Create short link — `POST /api/links`
- Auth: required (see Section 1).
- Request body:
```json
{
  "original_url": "https://example.com/path?x=1",
  "alias": "optCustom"
}
```
- Validation:
  - `original_url` is required, non-empty, absolute `http` or `https` URL; length ≤ 2048.
  - If `alias` present, it must satisfy the slug policy (Section 4).
- Behavior:
  - If `alias` present: attempt conditional insert; if slug exists → 409 conflict.
  - Else: obtain next counter value → derive Base62 slug with min length 5 → insert.
  - `created_at` set to current UTC time (RFC3339). `created_by` from token email.
  - `short_url` domain resolution: use env `SHORTLINK_DOMAIN` if set (e.g., `https://short.company.com`); otherwise construct from the incoming request Host header and `https://` if host is not explicitly http.
- Responses:
  - 201 Created with JSON body:
```json
{
  "slug": "aZ19B",
  "short_url": "https://short.company.com/aZ19B",
  "original_url": "https://example.com/path?x=1",
  "created_at": "2025-12-15T13:45:00Z",
  "created_by": "alice@yourcompany.com"
}
```
  - Errors (JSON error envelope; see Section 6): 400, 401, 403, 409, 500.

##### 5.2 List links — `GET /api/links`
- Auth: required.
- Query params:
  - `limit` (optional, int, 1..500). Default 200.
  - `page_token` (optional, string) — opaque pagination cursor.
- Behavior:
  - Return up to `limit` links, sorted by `created_at` descending (most recent first).
  - If more results are available, include `next_token` (opaque cursor) for subsequent calls.
- Response 200:
```json
{
  "links": [
    {
      "slug": "aZ19B",
      "short_url": "https://short.company.com/aZ19B",
      "original_url": "https://example.com/path?x=1",
      "created_at": "2025-12-15T13:45:00Z",
      "created_by": "alice@yourcompany.com"
    }
  ],
  "next_token": "eyJjIjo..."
}
```

#### 6. Error semantics
All errors return JSON with a consistent envelope and appropriate HTTP status code.

Envelope:
```json
{
  "error": {
    "code": "invalid_request",
    "message": "Human-readable description"
  }
}
```

Status mapping:
- 400 Bad Request → `invalid_request` (malformed JSON, failed validation, unsupported alias characters/lengths, invalid URL scheme).
- 401 Unauthorized → `unauthorized` (missing/invalid token, signature/audience/issuer/expiry failure).
- 403 Forbidden → `forbidden` (email domain not allowed).
- 409 Conflict → `conflict` (alias already exists).
- 500 Internal Server Error → `internal` (unexpected server failure).

#### 7. Redirect status (reference)
- Public redirect endpoint MUST use `308 Permanent Redirect` for consistency and method safety.

#### 8. Security notes
- Always prefer explicit origins for CORS in production; avoid `*` when feasible.
- Log authentication outcomes with structured fields (no sensitive token contents), including `sub` hash and `email` domain outcome.
- When signature verification is disabled by env (local/dev only), emit a clear WARNING at startup and on first auth pass.

#### 9. Non‑normative examples

Create (success):
```
POST /api/links
Authorization: Bearer eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9...
Content-Type: application/json

{
  "original_url": "https://www.rust-lang.org/learn",
  "alias": "Rust101"
}
```
→ 201 Created
```
{
  "slug": "Rust101",
  "short_url": "https://short.company.com/Rust101",
  "original_url": "https://www.rust-lang.org/learn",
  "created_at": "2025-12-15T13:45:00Z",
  "created_by": "dev@yourcompany.com"
}
```

Create (alias conflict): 409
```
{
  "error": { "code": "conflict", "message": "Alias already exists" }
}
```

List (first page): 200
```
{
  "links": [
    {
      "slug": "aZ19B",
      "short_url": "https://short.company.com/aZ19B",
      "original_url": "https://example.com/path?x=1",
      "created_at": "2025-12-15T13:45:00Z",
      "created_by": "alice@yourcompany.com"
    }
  ],
  "next_token": "opaque-cursor-if-more"
}
```

Preflight:
```
OPTIONS /api/links
→ 204 No Content
Access-Control-Allow-Origin: https://admin.company.com
Access-Control-Allow-Headers: Authorization, Content-Type
Access-Control-Allow-Methods: OPTIONS, GET, POST
```
