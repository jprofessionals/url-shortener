# URL Shortener

A serverless URL shortener written in Rust. The AWS deployment uses Lambda and
DynamoDB, and Cloudflare Pages serves the static admin UI. Short links use generated
Base62 slugs or custom aliases and can be disabled, scheduled or expired.
The redirect service also supports link previews and QR codes.

## Run locally

You need stable Rust and Make. For the static admin UI, also install Python 3.
The local Axum server uses SQLite by default and needs no AWS credentials.

Start the API with a fresh SQLite database for development or verification.
Use debug mode on a network isolated from untrusted clients because `api-server`
listens on all interfaces.

```bash
local_data=$(mktemp -d)
PORT=3001 ALLOWED_DOMAIN=example.com DB_PATH="$local_data/shortlinks.db" make run-api-local-none
```

For the admin UI, temporarily set `AUTH_DISABLED: true` in
[`admin-frontend/config.js`](admin-frontend/config.js), then run this in a second terminal.

```bash
make run-admin-frontend
```

Open `http://localhost:8000` and enter `user@example.com` in the debug email field.
The UI selects `http://localhost:3001` as the API for localhost. Restore
`AUTH_DISABLED: false` before committing frontend changes.

### Local smoke check

With the API running in debug mode, run these commands in a second terminal.
The create request should return `201`. The list should contain the new alias.
The last request should return `308` with `Location: https://example.com`.

```bash
api=http://localhost:3001
debug_user=user@example.com
slug="smoke-$(date +%s)"

curl -fsS -w '\nHTTP %{http_code}\n' \
  -H "X-Debug-User: $debug_user" \
  -H 'Content-Type: application/json' \
  -d "{\"original_url\":\"https://example.com\",\"alias\":\"$slug\"}" \
  "$api/api/links"

curl -fsS -w '\n' -H "X-Debug-User: $debug_user" "$api/api/links?search=$slug"

curl -sS -D - -o /dev/null "$api/$slug"
```

To check the same flow in the admin UI, create a short link to
`https://example.com`, find it in the list and open it. Group and membership
operations are handled by `lambda-admin`, so verify those changes through that
entry point using synthetic data.

### Google Sign-In

Keep `AUTH_DISABLED: false` in `admin-frontend/config.js`. Use a Google OAuth
client ID for development in `GOOGLE_CLIENT_ID` and add `http://localhost:8000`
to that client's authorized JavaScript origins. Start the API with the same
client ID and the allowed Google Workspace domain. The API verifies the Google
signature and requires a verified email and a matching hosted domain (`hd`).

```bash
export ALLOWED_DOMAIN=example.com
export GOOGLE_OAUTH_CLIENT_ID=your-client-id.apps.googleusercontent.com
make run-api-local-google
```

Serve the UI with `make run-admin-frontend`. The local API defaults to port
`3001`, the default SQLite file is `./data/shortlinks.db`, and `DB_PATH` can
override it. Set `STORAGE_PROVIDER=memory` when running `cargo run -p api-server`
directly for an in-memory backend. Use `RUST_LOG` to change log levels and
`LOG_FORMAT=json` for structured logs from `api-server`.

## Architecture

| Path | Responsibility |
| --- | --- |
| [`domain/`](domain/) | Link rules, types and ports, with no cloud SDK dependencies |
| [`adapters/`](adapters/) | DynamoDB and SQLite storage and Google ID token verification |
| [`apps/api-server/`](apps/api-server/) | Local Axum HTTP API |
| [`apps/lambda-redirect/`](apps/lambda-redirect/) | Public redirects, previews and QR codes |
| [`apps/lambda-admin/`](apps/lambda-admin/) | Authenticated link and group management |
| [`shared/http-common/`](shared/http-common/) | Shared HTTP helpers |
| [`admin-frontend/`](admin-frontend/) | Static HTML, CSS and JavaScript |
| [`infra/sam/`](infra/sam/) | AWS SAM infrastructure |

Google ID tokens identify users of the production admin API. It checks access
to links and groups on the server. The redirect Lambda updates each link's click
count in DynamoDB for ordinary redirects. Use synthetic links and accounts when
testing.

## Tests and CI

Run `cargo test -p domain` for a fast check of domain logic. The
[CI workflow](.github/workflows/ci.yml) runs workspace formatting, Clippy, tests
with all features and a Lambda artifact build.

## Deployment

[Deploy AWS Lambdas](.github/workflows/deploy-lambdas.yml) builds and deploys
the backend to dev. [Promote to Production](.github/workflows/promote-to-prod.yml)
uses Lambda artifacts from a successful dev deployment. The
[admin frontend workflow](.github/workflows/deploy-frontend.yml) deploys the
static files to Cloudflare Pages. Use `make help` for manual deployment targets.

See the [AWS deployment guide](docs/deployment-guide-aws.md) for AWS setup and
troubleshooting. `make build-lambdas` requires `cargo-lambda` and
`make iac-validate` requires the SAM CLI. `make sam-local` runs the Lambda API
with Docker and the DynamoDB configuration from the SAM template. Use the Axum
quick start above for local verification with SQLite.

## Design history

- [Design plan](docs/design-plan.md)
- [Implementation plan](docs/implementation-plan.md)
- [Junie implementation plan](docs/junie-implementation-plan.md)
- [AWS implementation diagram](docs/diagrams/aws-implementation.puml)
- [Lambda phase specification](docs/spec_phase_10_lambda_apps.md)
- [Infrastructure phase plan](docs/phase-11-iac-plan.md)
- [Roadmap](ROADMAP.md)

## License

MIT
