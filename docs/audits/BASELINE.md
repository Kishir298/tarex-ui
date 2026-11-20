# Tarex UI — Baseline Record

Date: 2026-11-20
Commit: `81196e1` (`main`, in sync with `origin/main`)
Upstream: `49a799b` (no upstream commits beyond fork base)

## Environment

- macOS (Apple Silicon host), Node v24.20.0, pnpm 11.9.0
- rustc/cargo 1.98.0 (Homebrew)
- Git 2.39.5, `gh` authenticated as `Kishir298` (admin on `Kishir298/tarex-ui`)

## Checks

| Check | Command | Result |
| --- | --- | --- |
| Type-check | `pnpm check-types` | pass |
| Lint | `pnpm lint` | exit 0 — 88 warnings + 1 info (pre-existing, add none) |
| Frontend tests | `pnpm test` | pass — 1370/1370 (192 files), 0 flakes this run |
| Frontend build | `pnpm build` | pass |
| Release script tests | `node --test scripts/release.check.mjs` | pass — 3/3 |
| Rust check (macOS) | `cargo check --all-targets --locked` | pass |
| Rust clippy (macOS) | `cargo clippy --all-targets --locked -- -D warnings` | pass |
| Rust tests (macOS) | `cargo test --locked` | pass — 403 passed, 0 failed, 1 ignored |
| Rust format | `cargo fmt --check` | **FAIL** — 105 pre-existing hunks (upstream style) |

## Pre-existing failures (attributed before any change)

1. `cargo fmt --check` fails repo-wide (~105 hunks). No CI workflow enforces
   rustfmt. Fixed later by a dedicated mechanical formatting commit.
2. Linux CI fails at `cargo check` — duplicate `PathBuf` import in
   `src-tauri/src/modules/secrets.rs` (line 20 + line 44 under
   `#[cfg(target_os = "linux")]`). macOS/Windows do not see it.
3. Windows CI fails at `cargo check` — `windows_sys::Win32::Foundation::BOOL`
   unresolved in `src-tauri/src/modules/process_control.rs` (windows-sys 0.61
   exposes `BOOL` as `windows_sys::core::BOOL`).
4. Release workflow fails at "Build native installers" on every platform —
   `TAURI_SIGNING_PRIVATE_KEY` empty/malformed; the repo has no Tarex signing
   keypair (the configured pubkey is inherited from upstream `crynta`).

## Not covered by this baseline

- Windows/Linux runtime: only CI runners can prove these.
- Packaging/signing: blocked until the updater keypair exists.
