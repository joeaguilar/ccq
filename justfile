# ccq — Claude Code transcript query CLI
# Run `just` or `just --list` to see all recipes

default:
    @just --list

# ─── Build ───────────────────────────────────────────────────────────

# Debug build
build:
    cargo build

# Optimized release build
release:
    cargo build --release

# Check without producing binaries (faster)
check:
    cargo check

# Install to ~/.cargo/bin
install: release
    cargo install --path . --force

# ─── Test ────────────────────────────────────────────────────────────

# Run Rust unit tests
test-unit:
    cargo test

# Run integration test suite (release build)
test: release test-unit
    ./tests/integration.sh

# Run integration tests against debug build
test-debug: build
    ./tests/integration.sh ./target/debug/ccq

# ─── Lint & Format ──────────────────────────────────────────────────

# Clippy with warnings denied (matches CI)
lint:
    cargo clippy --all-targets -- -D warnings

# Format all code
fmt:
    cargo fmt --all

# Check formatting without modifying (matches CI)
fmt-check:
    cargo fmt --all -- --check

# Full CI-equivalent gate
ci: fmt-check lint test
