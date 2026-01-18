High-Performance Serverless URL Shortener — Developer Guidelines

Scope: This file captures project-specific knowledge to accelerate development, testing, and debugging for advanced Rust developers working on this repository.

1. Workspace and Build
- Workspace: `Cargo.toml` declares a Rust workspace named `url-shortener` with `core` as the only member (and default-member). Edition is `2024`.
  - root `Cargo.toml`:
    - `[workspace] members = ["core"]`
  - `core/Cargo.toml`:
    - Minimal package manifest; no external dependencies yet.
- Targets (current):
  - `core` is a binary crate (`src/main.rs`) printing "Hello, world!". No library target exists yet.
- Build commands:
  - Build all: `cargo build`
  - Build the `core` crate only: `cargo build -p core`
- Edition/Toolchain assumptions:
  - Uses Rust 2024 edition features. Ensure your toolchain is up to date: `rustup update`.

2. Configuration and Run (Local)
- The README documents the intended larger system (adapters/apps, env-driven config). In this repo state, only the `core` binary exists and does not read environment variables yet.
- Run the current binary: `cargo run -p core`

3. Testing: How we do it here
- Policy
  - Prefer placing unit tests near the code under test inside `src/` using `#[cfg(test)]` modules for white-box tests.
  - Use `tests/` (integration tests) for black-box testing of the public API (eventually of a `lib` target) or binary behavior via `assert_cmd`/`escargot` once introduced.
  - Keep tests deterministic; avoid network and real cloud calls. For future adapters, gate integration tests behind feature flags or env vars (e.g., `AWS_INTEGRATION=1`).
- Commands
  - Run all tests in workspace: `cargo test`
  - Run tests for `core` only: `cargo test -p core`
  - Show test output: `cargo test -- --nocapture`
- Coverage (optional)
  - Suggested: `cargo llvm-cov` or `grcov` once a library target exists. Not configured yet.

4. Demonstration: Creating and Running a Simple Test
This section shows an ephemeral example you can reproduce to verify the test harness works without permanently adding files. The demonstration was executed and validated on December 8, 2025.

Steps to reproduce:
1) Create a temporary test file:
   - Path: `core/tests/demo_smoke.rs`
   - Contents:
     ```rust
     #[test]
     fn demo_smoke_passes() {
         // Basic arithmetic to prove the harness runs
         assert_eq!(2 + 2, 4);
     }
     ```
2) Run tests for the `core` crate:
   ```bash
   cargo test -p core
   ```
   Expected: the `demo_smoke_passes` test passes.
3) Remove the temporary file after verification to keep the repo clean:
   ```bash
   git rm --cached -r --ignore-unmatch core/tests/demo_smoke.rs 2>/dev/null || true
   rm -f core/tests/demo_smoke.rs
   ```

Note: In this session we created, executed, and removed the example test to satisfy the "demonstrate the process" requirement without leaving extra artifacts.

5. Evolving the Codebase (recommended patterns)
- Transition `core` into a library + binary split:
  - Create `core/src/lib.rs` for testable domain logic; make `src/main.rs` a thin `fn main()` that calls into the library.
  - This enables rich unit/integration testing and future adapters/apps to depend on the library.
- Feature flags for adapters and cloud SDKs:
  - Keep `core` dependency-free and platform-agnostic; implement AWS/GCP/local adapters as separate crates with optional features.
- Error handling:
  - Use `thiserror` for domain errors and `anyhow` for application seams (binaries). Keep boundary conversions explicit.
- Logging and tracing:
  - Adopt `tracing` + `tracing-subscriber` with json layer for prod, pretty for local.

6. Debugging Tips
- Rebuild with backtraces and logs enabled:
  - `RUST_BACKTRACE=1 RUST_LOG=debug cargo run -p core`
- Use `cargo expand` (via `cargo install cargo-expand`) when macros are added later.
- For perf-sensitive paths (once they exist), rely on `cargo bench` and `criterion`.

7. CI/CD Considerations (future)
- Minimal CI job should run `cargo fmt -- --check`, `cargo clippy -- -D warnings`, `cargo test --workspace`.
- For multi-crate expansion, matrix by feature set to keep adapters optional.

Appendix — Commands quick ref
- Build: `cargo build -p core`
- Run: `cargo run -p core`
- Test: `cargo test -p core`
