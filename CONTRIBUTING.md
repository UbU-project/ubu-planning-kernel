# Contributing

This repository is public, but Phase 1 is intentionally conservative.

Before opening a pull request:

- Run `cargo fmt --check`.
- Run `cargo clippy --workspace --all-targets`.
- Run `cargo test --workspace`.
- Keep default Rust tests CPU-only.
- Do not add GPU, CUDA, torch, Python, network, database, cloud, HTTP, UI, or Tauri requirements to default paths.

Cross-repo dependencies in committed manifests must be git dependencies pinned to an explicit `rev`. Use `.cargo/config.toml` only for local sibling overrides; that file is gitignored.
