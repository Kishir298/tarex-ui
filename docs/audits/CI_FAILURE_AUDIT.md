# Tarex UI — CI / Release Failure Audit

Date: 2026-11-20
Repository: `Kishir298/tarex-ui`
Method: full failed-job logs retrieved with `gh run view <id> --log-failed`
(all four runs retrieved successfully; no log was unavailable).

## Summary

Every recent run of both workflows on this fork fails. Three root causes
account for all failures:

1. **Duplicate `PathBuf` import** in `src-tauri/src/modules/secrets.rs:44`
   (unconditional import already at line 20) — kills the **Linux** compile.
2. **Wrong `BOOL` import path** in `src-tauri/src/modules/process_control.rs:199`
   (`windows_sys::Win32::Foundation::BOOL` does not exist in windows-sys 0.61;
   the type is `windows_sys::core::BOOL`) — kills the **Windows** compile.
3. **No Tarex updater signing key** — `TAURI_SIGNING_PRIVATE_KEY` is empty in
   the repository secrets, so every macOS installer build fails at the
   "Build native installers" step.

macOS compiles fine; it is only the release build on macOS that fails, and
only because of (3).

## Run overview

| Run | Workflow | Commit | Conclusion | Trigger | Root cause(s) |
| --- | --- | --- | --- | --- | --- |
| [44548039611](https://github.com/Kishir298/tarex-ui/actions/runs/44548039611) | CI | `81196e1` | failure | push to `main` | (1) Linux, (2) Windows |
| [44548039811](https://github.com/Kishir298/tarex-ui/actions/runs/44548039811) | Release Tarex UI | `81196e1` | failure | push to `main` | (1) Linux, (2) Windows, (3) macOS signing |
| [44547873819](https://github.com/Kishir298/tarex-ui/actions/runs/44547873819) | Release Tarex UI | `be71ee0` | failure | push to `main` | (1) Linux, (2) Windows, (3) macOS signing |
| [44547873749](https://github.com/Kishir298/tarex-ui/actions/runs/44547873749) | CI | `be71ee0` | cancelled | push to `main` | superseded by `44548039611` (concurrency) |

## Per-run detail

### CI run `44548039611` — commit `81196e1`, 2026-11-19T14:50:03Z

Job results:

| Job | Conclusion |
| --- | --- |
| `frontend` | success |
| `terminal-test (windows-latest)` | success |
| `rust` (ubuntu-latest) | **failure** |
| `rust-test (windows-latest)` | **failure** |
| `terminal-test (macos-latest)` | cancelled |
| `rust-test (macos-latest)` | cancelled |
| `coverage` | skipped |

`rust` (Linux) — failing step `cargo check`:

```text
error[E0252]: the name `PathBuf` is defined multiple times
  --> src/modules/secrets.rs:44:5
   |
20 | use std::path::PathBuf;
   |     ------------------ previous import of the type `PathBuf` here
...
44 | use std::path::PathBuf;
   |     ^^^^^^^^^^^^^^^^^^^ `PathBuf` reimported here
   |
   = note: `PathBuf` must be defined only once in the type namespace of this module

error: could not compile `tarex` (lib) due to 1 previous error
```

`rust-test (windows-latest)` — failing step `cargo check`:

```text
error[E0432]: unresolved import `windows_sys::Win32::Foundation::BOOL`
   --> src\modules\process_control.rs:199:42
    |
199 |     use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
    |                                          ^^^^ no `BOOL` in `Win32::Foundation`

error: could not compile `tarex` (lib) due to 1 previous error
```

Runner-environment note (non-blocking): the Windows job logged a transient
`install-action: installation failed due to bash startup failure`
([actions/partner-runner-images#169](https://github.com/actions/partner-runner-images/issues/169))
before `cargo check` ran; the job recovered and proceeded to the real
E0432 failure. Not a source defect.

### Release run `44548039811` — commit `81196e1`, 2026-11-19T14:50:04Z

| Job | Conclusion | Failing step | Cause |
| --- | --- | --- | --- |
| `Build macOS Apple Silicon` | failure | Build native installers | (3) empty signing key |
| `Build Linux x64` | failure | Build native installers | (1) compile E0252 |
| `Build Windows x64` | failure | Build native installers | (2) compile E0432 |

macOS Apple Silicon (reached the signing stage because it compiles):

```text
  TAURI_SIGNING_PRIVATE_KEY:
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD:

failed to decode secret key: incorrect updater private key password: Missing comment in secret key
       Error failed to decode secret key: incorrect updater private key password: Missing comment in secret key
[ELIFECYCLE] Command failed with exit code 1.
```

Linux and Windows never reached signing; both died in `cargo check` with the
same E0252 / E0432 errors as the CI run, surfaced as:

```text
error: could not compile `tarex` (lib) due to 1 previous error
failed to build app: failed to build app
```

### Release run `44547873819` — commit `be71ee0`, 2026-11-19T14:49:08Z

Identical failure pattern to `44548039811`:

| Job | Cause |
| --- | --- |
| `Build macOS Intel` | (3) `failed to decode secret key: ... Missing comment in secret key` |
| `Build Linux x64` | (1) compile failure — log contains `E0252` |
| `Build Windows x64` | (2) compile failure — log contains `E0432` |

This run also proves the failure is not commit-specific: it reproduces on
`be71ee0` exactly as on `81196e1`.

### CI run `44547873749` — commit `be71ee0`, 2026-11-19T14:49:08Z

Conclusion: `cancelled`. All seven jobs (`rust`, `rust-test` ×2,
`terminal-test` ×2, `frontend`, `coverage`) were cancelled. The concurrent
CI run `44548039611` on the next push superseded it via the workflow
`concurrency` group. No job produced a failure signal of its own —
this is a workflow-design artifact, not a defect.

## Root causes

| # | Root cause | Files | Classification | Remediation status |
| --- | --- | --- | --- | --- |
| 1 | Duplicate `use std::path::PathBuf;` at file scope (`#[cfg(target_os = "linux")]` copy shadowed by unconditional import) | `src-tauri/src/modules/secrets.rs:20,44` | source code | **fixed** (duplicate removed) |
| 2 | `BOOL` moved out of `windows_sys::Win32::Foundation` in windows-sys 0.61 → must be `windows_sys::core::BOOL` | `src-tauri/src/modules/process_control.rs:199` | dependency upgrade drift | **fixed** (import corrected) |
| 3 | No Tarex updater signing keypair exists; `TAURI_SIGNING_PRIVATE_KEY` repository secret is empty, and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` is hardcoded `""` in the workflow | `release.yml`, repo secrets, `tauri.conf.json` pubkey | configuration / secrets | pending (Phase 7) |
| 4 | Release workflow triggers on every push to `main`, so a broken commit immediately starts a doomed release | `.github/workflows/release.yml` | workflow design | pending (Phase 7) |
| 5 | CI workflow has no rustfmt gate, no top-level `permissions`, no step timeouts | `.github/workflows/ci.yml` | workflow design | pending (Phase 8) |
| 6 | Windows runner transient `install-action` bash startup failure | GitHub runner infra | runner environment | not actionable; re-run if it recurs |

## Cascading effects

- `coverage` is skipped whenever any Rust job fails, so coverage has never
  reported on this fork.
- macOS CI jobs (`rust-test (macos-latest)`, `terminal-test (macos-latest)`)
  were cancelled, so **no CI signal exists for macOS at all** on this fork.
- Because the Release workflow runs on every `main` push, each broken push
  produces two failing runs (CI + Release), doubling noise.

## Verification

Both source root causes (1 and 2) were reproduced against the exact log
text above and fixed locally; `cargo check --all-targets --locked` passes on
macOS after the fix. Linux/Windows compile proof requires CI on the pushed
branch. Root cause 3 requires generating a Tarex-owned updater keypair and
populating repository secrets.
