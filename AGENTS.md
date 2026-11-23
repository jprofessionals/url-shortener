# URL Shortener agent instructions

## System profile

- Stage: `production`. Keep changes small and reversible. Preserve existing links, stored data and access rules.
- Handles: `personal-data` including email addresses for link owners and group members. Keep real records out of test fixtures, new log output and unapproved services.
- Used by: JPro administers short links that recipients use. A redirect outage prevents recipients from reaching their destinations.

## Project rules

- Keep business logic in `domain/`, free of cloud SDKs and HTTP or storage implementations. Applications wire the domain to its adapters.
- Preserve public access to redirects. Enforce admin API access on the server and verify ownership, group membership and administrator privileges where applicable. Keep `X-Debug-User` and Google signature bypass in isolated development environments.
- Production data access for agents is `none`. Use synthetic data for development and verification. Refer to secrets by name and keep their values out of output, prompts and commits.
- Use `Makefile` targets for manual deployment. Use the repository's GitHub Actions workflows for automated deployment and production promotion. See [deployment](README.md#deployment) when working on releases.

## Verification

- Use the smallest check that exercises the changed behavior. Run `cargo test -p <crate>` for a targeted Rust check and the commands in [CI](.github/workflows/ci.yml) for workspace checks.
- Verify HTTP changes through the affected entry point. Follow the [local smoke check](README.md#local-smoke-check) for create, list and redirect. Verify changes to production group access through `lambda-admin`.
- Report which checks ran, their results and anything left unverified.

Guidelines: https://github.com/jprofessionals/agentic-dev (applied 2026-11-23)
