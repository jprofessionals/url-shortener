# High-Performance Serverless URL Shortener

A cloud-agnostic, white-label URL shortening service engineered for the "scale-to-zero" era. Built with **Rust**, this project prioritizes minimal latency, near-zero operational costs, and portability between hyperscalers (AWS, GCP) and self-hosted environments.

## 🎯 Project Goals

*   **Cost Efficiency:** Designed to run within the "Always Free" tiers of AWS (Lambda/DynamoDB) and GCP (Cloud Run/Firestore).
*   **Performance:** Sub-millisecond startup times using Rust, minimizing "cold start" penalties common in serverless functions.
*   **Portability:** Implements a **Hexagonal Architecture** (Ports & Adapters), allowing the core/domain logic to remain identical whether deploying to a Raspberry Pi or an AWS Lambda function.
*   **White Label:** Fully configurable domains for serving redirects and hosting the admin interface.

## Design Documents

| Document                                                       | Description                                                                                                        |
|:---------------------------------------------------------------|:-------------------------------------------------------------------------------------------------------------------|
| [Design Plan](docs/design-plan.md)                             | High-level overview of project plan and expectations.                                                              |
| [Implementation Plan](docs/implementation-plan.md)             | Detailed design and implementation notes. Created up front                                                         |
| [Junie Implementation Plan](docs/junie-implementation-plan.md) | A more detailed version of the implementation plan, created by and for Junie. May have diverged from original plan |
| [AWS Implementation](docs/diagrams/aws-implementation.puml)    | AWS-specific notes and details.                                                                                    |

### Assorted task related breakdowns

| Task Name                                     | Breakout Document                 | Description               |
|:----------------------------------------------|:----------------------------------|:--------------------------|
| [Phase 11](docs/phase-11-iac-plan.md)         | docs/junie-implementation-plan.md | Define AWS infrastructure |
| [Phase 10](docs/spec_phase_10_lambda_apps.md) | docs/junie-implementation-plan.md | Define AWS Lambda apps    |

## ✨ Features

*   **URL Shortening:** Auto-generated slugs (Base62) or custom aliases.
*   **Smart Redirects:** Configurable HTTP status codes (301, 302, 307).
*   **Link Management:**
    *   Temporarily disable/enable links.
    *   Set validity windows (`valid_from`, `valid_until`).
*   **Zero-Cost Analytics:** Tracks clicks, country, and user-agent without expensive database writes (using log-based analytics).
*   **Secure Admin:** Google Sign-In (OIDC) integration with an allow-list for "Super Admins."

## 🏗️ Architecture

This project uses a **Rust Workspace** to enforce strict separation of concerns:

```
project
├── domain/         # Pure Business Logic (No Cloud SDKs)
├── adapters/       # Infrastructure plugins (DynamoDB, Firestore, SQLite)
└── apps/           # Entry points (Lambda Handler, Cloud Run Server, CLI)
```

## ☁️ Deploy on AWS with SAM

This repo includes an AWS SAM template for deploying two Lambda functions (`lambda-redirect`, `lambda-admin`), an HTTP API (API Gateway v2), and two DynamoDB tables (`shortlinks-<stage>`, `counters-<stage>`).

Prerequisites:
- AWS CLI configured with credentials for your target account
- AWS SAM CLI installed (`sam --version`)
- Rust toolchain (stable)

Build Lambda artifacts (x86_64):
```bash
make build-lambdas
```

Validate the template:
```bash
sam validate -t infra/sam/template.yaml
```

Deploy (guided) to eu-west-1, stack name `url-shortener-dev`:
```bash
sam deploy --guided \
  --region eu-west-1 \
  --stack-name url-shortener-dev \
  -t infra/sam/template.yaml
```
You will be prompted for required parameters (no defaults are set):
- `AllowedDomain` (e.g., `acme.com`)
- `GoogleOAuthClientId` (your Google OAuth client ID)
- `ShortlinkDomain` (e.g., `https://short.acme.com`) — used in admin responses
- `CorsAllowOrigin` (e.g., `https://admin.acme.com`)

The Lambda Admin function consumes these via env vars: `GOOGLE_OAUTH_CLIENT_ID`, `ALLOWED_DOMAIN`, `SHORTLINK_DOMAIN`, `CORS_ALLOW_ORIGIN`. Dynamo table names are injected as `DYNAMO_TABLE_SHORTLINKS` and `DYNAMO_TABLE_COUNTERS`.

### Local testing with SAM

You can run the HTTP API locally using Docker:
```bash
make build-lambdas
sam local start-api -t infra/sam/template.yaml \
  --parameter-overrides \
    AllowedDomain=acme.com \
    GoogleOAuthClientId=your-client-id.apps.googleusercontent.com \
    ShortlinkDomain=https://short.acme.com \
    CorsAllowOrigin=http://localhost:5173
```
Notes:
- Local mode uses your AWS credentials for DynamoDB by default. Ensure the tables exist (created by a prior `sam deploy`) or adjust the adapter for purely local storage if needed.
- For local development you may set `GOOGLE_AUTH_INSECURE_SKIP_SIGNATURE=1` to bypass signature verification; the app logs a WARNING and still enforces audience/expiry/domain claims.

## Admin Frontend (Svelte + Vite)

We will ship the admin UI as a static, client-only SPA built with Svelte + Vite (no SSR). It calls the Admin API using a Google ID token in the `Authorization: Bearer <token>` header. This keeps costs near-zero and makes deployment trivial across AWS/GCP/Cloudflare/GitHub Pages.

### Local development

Prerequisites:
- Node.js 18+ and npm (or pnpm/yarn)

Scaffold and run a Svelte + Vite app (in a separate directory/repo is fine):
```bash
npm create vite@latest admin -- --template svelte
cd admin
npm i
```

Configure environment variables (Vite uses import-time envs):
Create a `.env.local` file with:
```
VITE_API_BASE=https://<your-api-id>.execute-api.<region>.amazonaws.com/dev
VITE_GOOGLE_CLIENT_ID=<your-client-id>.apps.googleusercontent.com
```

Run the dev server:
```bash
npm run dev -- --open
```

Backend CORS note: when running the backend locally with SAM, set the SAM parameter `CorsAllowOrigin` to your dev origin (e.g., `http://localhost:5173`) so the browser can call the API.

Minimal Svelte usage sketch:
- Use the Google Identity Services script to obtain a Google ID token.
- Store the token in memory and call `fetch("
${import.meta.env.VITE_API_BASE}/api/links", { headers: { Authorization: `Bearer ${idToken}` } })`.

### Build

Build static assets into `dist/`:
```bash
npm run build
```

### Deploy (AWS minimal-cost)

Option A — S3 static website (cheapest, add CloudFront later for HTTPS/custom domain):
1) Create an S3 bucket (unique name), enable static website hosting.
2) Upload the build output:
```bash
aws s3 sync dist/ s3://<your-admin-bucket>/ --delete
```
3) Set the SAM parameter `CorsAllowOrigin` to the website origin (or your CloudFront domain if you add one).
4) In Google Cloud Console, add your admin site origin to the OAuth client’s Authorized JavaScript origins.

Option B — CloudFront + S3 (HTTPS + custom domain):
- Put a CloudFront distribution in front of the S3 bucket, attach an ACM certificate and your custom domain. Point DNS to CloudFront. Set `CorsAllowOrigin` to your final admin origin (e.g., `https://admin.example.com`).

Zero-cost alternatives:
- Cloudflare Pages or GitHub Pages can host the `dist/` folder for free. Update `CorsAllowOrigin` to that origin.

Configuration checklist:
- Backend SAM Parameters: `AllowedDomain`, `GoogleOAuthClientId`, `ShortlinkDomain`, `CorsAllowOrigin`.
- Frontend Vite envs: `VITE_API_BASE`, `VITE_GOOGLE_CLIENT_ID`.

## 🔬 Test the Admin component locally

Two easy local modes are available. The backend runs as a local HTTP server (`apps/api-server`) and the frontend is served as static files from `admin-frontend/`.

Prerequisites:
- Rust stable toolchain
- Python 3 (for a tiny static file server)

Folders of interest:
- Backend API: `apps/api-server` (Axum)
- Frontend (static, phase 1): `admin-frontend/` (index.html, config.js, app.js)

Makefile targets provided:
- `make run-api-local-none` — Start API with auth disabled (uses X-Debug-User header)
- `make run-api-local-google` — Start API with Google Sign-In verification
- `make run-admin-frontend` — Serve the static admin UI on http://localhost:8000

### Option A — Fastest: Auth disabled (debug header)
This mode requires no Google setup. The UI will show a Debug email field and the backend accepts it via the `X-Debug-User` header.

1) Start the backend (SQLite by default):
```bash
make run-api-local-none
# Uses: AUTH_PROVIDER=none STORAGE_PROVIDER=sqlite PORT=3001 CORS_ALLOW_ORIGIN=http://localhost:8000
# Optional: override DB path
# DB_PATH=./data/shortlinks.db make run-api-local-none
```

2) Serve the admin UI:
```bash
make run-admin-frontend
# Opens http://localhost:8000
```

3) In the UI, enter a debug email (e.g., user@acme.com) and test Create/List.

API smoke via curl (optional):
```bash
curl -i -H 'X-Debug-User: you@acme.com' \
     -H 'content-type: application/json' \
     -d '{"original_url":"https://example.com"}' \
     http://localhost:3001/api/links
```

### Option B — Google Sign-In enabled
This mode uses real Google ID tokens from the browser. Configure your OAuth client and allowed domain.

1) Configure frontend:
- Edit `admin-frontend/config.js` and set:
  - `API_BASE: "http://localhost:3001"`
  - `GOOGLE_CLIENT_ID: "<your-client-id>.apps.googleusercontent.com"`
  - `AUTH_DISABLED: false`

2) Start the backend with Google verification:
```bash
export ALLOWED_DOMAIN=acme.com
export GOOGLE_OAUTH_CLIENT_ID=<your-client-id>.apps.googleusercontent.com
# Optional: DB_PATH=./data/shortlinks.db
make run-api-local-google
# Uses: AUTH_PROVIDER=google STORAGE_PROVIDER=sqlite CORS_ALLOW_ORIGIN=http://localhost:8000
```

3) Serve the admin UI:
```bash
make run-admin-frontend
# Then sign in with Google; only emails at $ALLOWED_DOMAIN are allowed.
```

Notes:
- CORS: Backend allows `http://localhost:8000` by default in these targets.
- Signature bypass (dev only): You can speed up auth locally by setting `GOOGLE_AUTH_INSECURE_SKIP_SIGNATURE=1` in the backend environment. Audience/expiry/domain checks still apply, and a WARNING is logged. Do NOT use in production.
- SQLite location: Set `DB_PATH=/absolute/or/relative/path.db` before the `make run-api-*` command to change where data is stored. Default is `./data/shortlinks.db`.

## 🚀 Getting Started (Local Development)

You can run the entire system locally using a file-based SQLite database. No cloud account required.

### Prerequisites
*   Rust (latest stable)
*   Make (optional, for convenience commands)

### Running Locally
1.  Clone the repository.
2.  Create a `.env` file (see Configuration below).
3.  Run the server:
    ```bash
    cargo run -p api-server
    ```
4.  Access the admin UI at `http://localhost:3000/admin`.

## ⚙️ Configuration (White Labeling)

The application is stateless and configured 100% via Environment Variables. This makes it white-label ready for any domain.

| Variable           | Description                                              | Default / Example                    |
|:-------------------|:---------------------------------------------------------|:-------------------------------------|
| `BASE_URL`         | **Required.** The public domain used for short links.    | `https://s.jpro.dev`                 |
| `PORT`             | Port to listen on (for `api-server` and `api-cloudrun`). | `3001`                               |
| `STORAGE_PROVIDER` | Which database adapter to use.                           | `local`, `aws`, or `gcp`             |
| `AUTH_PROVIDER`    | OIDC Provider for Admin Login.                           | `google`                             |
| `GOOGLE_CLIENT_ID` | OAuth2 Client ID from Google Cloud Console.              | `123...apps.googleusercontent.com`   |
| `ADMIN_EMAILS`     | Comma-separated list of allowed admin emails.            | `user@example.com,admin@company.com` |
| `RUST_LOG`         | Log level.                                               | `info`                               |

## 🪵 Logging Tutorial

This project uses the `tracing` ecosystem for fast, structured, and leveled logs. The `apps/api-server` app initializes a `tracing-subscriber` at startup and applies `tower-http::TraceLayer` so each HTTP request is traced with useful context.

### Quick start

- Pretty/dev logs (recommended locally):

```bash
RUST_LOG=api_server=debug,axum=info LOG_FORMAT=pretty cargo run -p api-server
```

- JSON/prod-style logs (pipe to jq for readability):

```bash
RUST_LOG=info LOG_FORMAT=json cargo run -p api-server | jq .
```

Now exercise the API in another shell (examples):

```bash
curl -i -H 'X-Debug-User: you@example.com' \
     -H 'content-type: application/json' \
     -d '{"original_url":"https://example.com"}' \
     http://localhost:3001/api/links

curl -i http://localhost:3001/<your-slug>
```

### Configuration knobs

- RUST_LOG: standard env filter controlling log levels per target.
  - Examples:
    - `RUST_LOG=info`
    - `RUST_LOG=api_server=debug,axum=info,tower_http=warn`
- LOG_FORMAT: selects formatter. Options: `pretty` (default) | `json`.
- PORT: sets the HTTP port (defaults to 3001). Does not affect logging.

### What gets logged

- Request/response tracing via `TraceLayer` creates spans per HTTP request.
- Handlers emit structured fields:
  - On resolve success: `slug`, `redirect_to` (level: info)
  - On list: `count` (info)
  - On 4xx: warnings (e.g., `invalid user email`, `invalid custom slug`)
  - On 5xx: errors with `err` details

Pretty example (abridged):

```
INFO  api_server: resolve ok slug=abc123 redirect_to=https://example.com
WARN  api_server: validation error on create
ERROR api_server: internal error on create err=repository error: ...
```

JSON example (abridged):

```json
{"timestamp":"...","level":"INFO","target":"api_server","fields":{"message":"resolve ok","slug":"abc123","redirect_to":"https://example.com"}}
```

### Troubleshooting

- No logs? Ensure `RUST_LOG` is not overly restrictive and that the process outputs to your terminal (no service manager swallowing stdout).
- Seeing too much noise from dependencies? Tweak filters, e.g. `RUST_LOG=api_server=debug,axum=info,tower_http=warn`.
- Prefer visual scanning? Use `LOG_FORMAT=pretty`. For machine parsing or shipping to aggregators, use `LOG_FORMAT=json`.

### Future: file logging option

The design allows adding an optional file sink (e.g., via `tracing-appender`) controlled by an env var such as `LOG_FILE`. Stdout remains the default and recommended target for containers and serverless.

## ☁️ Deployment Guides

### Option A: AWS (Recommended for Lowest Cost)
*   **Compute:** AWS Lambda (ARM64/Graviton).
*   **Storage:** Amazon DynamoDB (On-Demand).
*   **CDN/SSL:** Amazon CloudFront.
*   **Cost:** ~$0.00 for the first 1M requests/month.

**Deploy via Terraform:**
```bash
cd infra/aws
terraform init
terraform apply -var="domain_name=s.jpro.dev"
```

### Option B: Google Cloud Platform
*   **Compute:** Cloud Run (Scale to zero).
*   **Storage:** Firestore (Native mode).
*   **CDN/SSL:** Firebase Hosting (for free SSL & custom domain).

**Deploy via CLI:**
```bash
# Build container
gcloud builds submit --tag gcr.io/PROJECT/shortener
# Deploy service
gcloud run deploy shortener --image gcr.io/PROJECT/shortener --set-env-vars BASE_URL=[https://link.mydomain.com](https://link.mydomain.com)
```

### Option C: Self-Hosted / Docker
Run anywhere Docker runs (VPS, Raspberry Pi, K8s).

```bash
docker run -d \
  -p 80:3000 \
  -v $(pwd)/data:/data \
  -e STORAGE_PROVIDER=local \
  -e SQLITE_PATH=/data/db.sqlite \
  -e BASE_URL=[https://my-private-link.com](https://my-private-link.com) \
  my-shortener-image
```

## 🔒 Security

*   **Authentication:** The service does not store passwords. It relies on verifying OIDC ID Tokens (JWTs) from Google.
*   **Authorization:** Only users whose email addresses match the `ADMIN_EMAILS` env var are granted write access.
*   **Public Access:** The generic redirection endpoint `GET /{slug}` is public. All other API endpoints (`/api/admin/*`) are protected.

## 📊 Analytics Strategy

To keep costs low, we generally avoid writing to the database on every click (which costs Write Units).
*   **AWS:** Logs are flushed to CloudWatch. Use **CloudWatch Logs Insights** to query stats.
*   **GCP:** Logs are sent to Cloud Logging. Sink to **BigQuery** for analysis.
*   **Local:** Writes to a local `analytics.csv` or separate SQLite table.

## License

MIT