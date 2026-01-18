.PHONY: help iac-validate build-lambdas sam-local run-admin-frontend run-api-local-none run-api-local-google \
        deploy-dev deploy-prod deploy-frontend deploy-all-dev deploy-all-prod

## Show this help message
help:
	@echo "URL Shortener - Available targets:"
	@echo ""
	@echo "  Local Development:"
	@echo "    run-api-local-none    Start API server with auth disabled (uses X-Debug-User header)"
	@echo "    run-api-local-google  Start API server with Google Sign-In (requires env vars)"
	@echo "    run-admin-frontend    Serve admin UI on http://localhost:8000"
	@echo "    sam-local             Run Lambdas locally via SAM (requires Docker)"
	@echo ""
	@echo "  Build & Validate:"
	@echo "    build-lambdas         Build Lambda artifacts (add ARM=1 for Graviton2)"
	@echo "    iac-validate          Validate SAM template"
	@echo ""
	@echo "  Deployment:"
	@echo "    deploy-dev            Deploy backend to dev environment"
	@echo "    deploy-prod           Deploy backend to prod (with confirmation)"
	@echo "    deploy-frontend       Deploy admin UI to Cloudflare Pages"
	@echo "    deploy-all-dev        Deploy backend + frontend to dev"
	@echo "    deploy-all-prod       Deploy backend + frontend to prod"
	@echo ""
	@echo "  Quick Start (local dev):"
	@echo "    make run-api-local-none   # Terminal 1: API on :3001"
	@echo "    make run-admin-frontend   # Terminal 2: UI on :8000"
	@echo ""

.DEFAULT_GOAL := help

## Validate the SAM template (Phase 11 acceptance)
iac-validate:
	sam validate -t infra/sam/template.yaml

## Build Lambda custom runtime artifacts using cargo-lambda (AWS recommended)
## Produces:
##  - infra/sam/artifacts/lambda-redirect/bootstrap
##  - infra/sam/artifacts/lambda-admin/bootstrap
##
## For ARM64 (Graviton2): make build-lambdas ARM=1
ART = infra/sam/artifacts

build-lambdas:
	@set -e; \
	mkdir -p $(ART)/lambda-redirect $(ART)/lambda-admin; \
	if [ "$(ARM)" = "1" ]; then \
		cargo lambda build --release --arm64 -p lambda-redirect -p lambda-admin; \
	else \
		cargo lambda build --release -p lambda-redirect -p lambda-admin; \
	fi; \
	cp target/lambda/lambda-redirect/bootstrap $(ART)/lambda-redirect/bootstrap; \
	cp target/lambda/lambda-admin/bootstrap $(ART)/lambda-admin/bootstrap; \
	echo "Artifacts ready under $(ART)/"

## Run SAM local API using current artifacts (requires Docker)
sam-local: build-lambdas iac-validate
	sam local start-api -t infra/sam/template.yaml

## --- Local development helpers (Admin GUI) ---
## Serve the static admin frontend on http://localhost:8000
run-admin-frontend:
	@echo "Serving admin-frontend on http://localhost:8000 (Ctrl+C to stop)"
	cd admin-frontend && python3 -m http.server 8000

## Start the local API with auth disabled (uses X-Debug-User header)
## Optional: DB_PATH=./data/shortlinks.db (default)
run-api-local-none:
	@echo "Starting api-server (AUTH_PROVIDER=none, STORAGE_PROVIDER=sqlite) on :3001"
	AUTH_PROVIDER=none \
	STORAGE_PROVIDER=sqlite \
	CORS_ALLOW_ORIGIN=http://localhost:8000 \
	RUST_LOG=info \
	cargo run -p api-server

## Start the local API with Google Sign-In verification enabled
## Required env: ALLOWED_DOMAIN, GOOGLE_OAUTH_CLIENT_ID
## Optional: DB_PATH, CORS_ALLOW_ORIGIN (defaults to http://localhost:8000)
## ALLOWED_DOMAIN=jpro.no
## GOOGLE_OAUTH_CLIENT_ID="333449424444-bb173lfcpqurosj5o2b39lmkpovnceqi.apps.googleusercontent.com"
run-api-local-google:
	@echo "Starting api-server (AUTH_PROVIDER=google, STORAGE_PROVIDER=sqlite) on :3001"
	: $${ALLOWED_DOMAIN?"ALLOWED_DOMAIN is required"}; \
	: $${GOOGLE_OAUTH_CLIENT_ID?"GOOGLE_OAUTH_CLIENT_ID is required"}; \
	AUTH_PROVIDER=google \
	STORAGE_PROVIDER=sqlite \
	CORS_ALLOW_ORIGIN=$${CORS_ALLOW_ORIGIN:-http://localhost:8000} \
	ALLOWED_DOMAIN=$${ALLOWED_DOMAIN} \
	GOOGLE_OAUTH_CLIENT_ID=$${GOOGLE_OAUTH_CLIENT_ID} \
	RUST_LOG=info \
	cargo run -p api-server

## --- Deployment ---

## Deploy backend to dev environment (builds lambdas first)
deploy-dev: build-lambdas iac-validate
	@echo "Deploying to DEV environment..."
	cd infra/sam && sam deploy --no-confirm-changeset

## Deploy backend to prod environment (builds lambdas first)
## Requires confirmation before deploying
deploy-prod: build-lambdas iac-validate
	@echo "Deploying to PROD environment..."
	@echo "WARNING: This will deploy to production!"
	@read -p "Are you sure? [y/N] " confirm && [ "$$confirm" = "y" ] || exit 1
	cd infra/sam && sam deploy --config-env prod --no-confirm-changeset

## Deploy frontend (Cloudflare Pages)
## Single deployment serves both dev and prod via custom domains:
##   - dev-admin-sc.jpro.dev → uses dev backend
##   - admin-sc.jpro.dev → uses prod backend
## Frontend auto-detects environment from hostname
deploy-frontend:
	@echo "Deploying frontend (Cloudflare Pages)..."
	cd admin-frontend && npx wrangler pages deploy . --project-name=url-shortener-admin

## Full deploy to dev (backend + frontend)
deploy-all-dev: deploy-dev deploy-frontend
	@echo "DEV deployment complete!"

## Full deploy to prod (backend + frontend)
deploy-all-prod: deploy-prod deploy-frontend
	@echo "PROD deployment complete!"
