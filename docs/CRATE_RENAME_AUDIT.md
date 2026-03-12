# Crate Rename Audit

Status: current as of 2026-03-12

## Target Identity

- repository name: `codex-runtime`
- published crate name: `codex-runtime`
- Rust import path: `codex_runtime`

## Verified Surfaces

- workspace manifest points at `crates/codex-runtime`
- package manifest name is `codex-runtime`
- root README and API reference use `codex-runtime` / `codex_runtime`
- release scripts use `CODEX_RUNTIME_*` environment names
- localized doc entrypoints describe the current identity
- tracked source, docs, and scripts contain no remaining `codekko` references

## Known Caveats

- old names can still appear under local `target/` build artifacts after a rename. Those files are generated, git-ignored, and not part of the published crate.
- `cargo publish --dry-run` validates packaging without needing a token. Real publishing still requires local Cargo credentials or `CARGO_REGISTRY_TOKEN`.

## Cleanup Result

- removed stale implementation-planning docs that no longer matched the active documentation surface
- added a stable docs index so active references stay discoverable after the rename
