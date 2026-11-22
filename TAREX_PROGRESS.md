# Tarex UI — Progress

Last updated: 2026-11-21

## Phase 0 — Baseline and documentation

- [x] Verify repository state and remotes
- [x] Install dependencies
- [x] Record baseline checks
- [x] Create `TAREX_AUDIT.md`
- [x] Create `TAREX_SECURITY_AUDIT.md`
- [x] Create `TAREX.md`
- [x] Create `TAREX_PROGRESS.md`

Baseline results (2026-11-19, after repository recovery and WASM rebuild):

- `pnpm check-types`: pass
- `pnpm lint`: exit 0, 88 warnings + 1 info (pre-existing; add none)
- `pnpm test`: 1370 pass, 0 fail (192 files)
- `cargo test`: 403 pass, 0 fail, 1 ignored
- `cargo clippy --all-targets -- -D warnings`: pass (after one pre-existing
  `unnecessary_sort_by` fix in `shared_storage.rs`)
- `cargo fmt --check`: FAIL — ~105 pre-existing hunks in files untouched by
  this work (upstream style; no CI workflow enforces rustfmt)

## Phase 1 — Identity, import, and branding

- [x] Product/binary/bundle identity (`Tarex UI`,
      `io.github.kishir298.tarex-ui`)
- [x] Version 0.1.0 (`package.json`, `tauri.conf.json`)
- [x] Rust crate/lib/CLI rename (`tarex`)
- [x] Env/event/store/keychain rename with compatibility shims (no residual
      `terax` identifiers; legacy `terax-ai` keychain service migrates to
      `tarex-ai` in `secrets.rs`)
- [x] New logo, icons, palette, and visible strings
- [x] Automatic non-secret import (`.tarex-import.json` marker)
- [x] Explicit credential import prompt (keychain import with one-time copy)
- [x] Docs/readme/metadata (repository URLs repointed to
      `Kishir298/tarex-ui`)

## Phase 2 — AI

- [x] Hydrate built-in provider keys in the main window
- [x] Built-in models in status-bar and default pickers
- [x] Open AI without provider gating
- [x] Fix `runSubagent` compat model handling
- [x] Restore previous workspace after AI closes
- [x] `TAREX.md` project memory with `TERAX.md` fallback
- [x] Tests (`useAiBootstrap`, `subagent`, `transport`, shell-session
      cleanup; eager-budget invariant kept green via `lib/sessionShells.ts`)

## Phase 3 — Process, efficiency, hardware

- [x] Safe terminate/force-kill process control (graceful `Terminate` =
      SIGTERM / Windows WM_CLOSE; `Force kill` stays explicitly destructive)
- [x] Protected-process policy and reasons (PID <= 1, self, ancestors,
      critical system processes, PID-reuse mismatches — each with a named
      reason)
- [x] Full process manager UI (memory column, search filter, context menu)
- [x] Resume suspended processes on exit (`RunEvent::Exit` drains the store)
- [x] Efficiency Mode (Tarex-only priority lowering + slower own sampling;
      Settings → General toggle; macOS cannot raise nice back without
      privileges — normal priority returns on next start)
- [x] Hardware/RAM details with unavailable states (`cpu_name: Option`,
      zero-capacity disks omitted, UI shows Unavailable instead of zeros)
- [x] Tests (Rust protection/exit-planning tests, `model.test.ts`)

## Phase 4 — Security

Findings are documented in `TAREX_SECURITY_AUDIT.md`. All 13 are remediated
on `tarex-ui/security-remediation-2026-11`; residual risks are listed there.

- [x] Path guard for all fs commands (`918d566`, `2318538`)
- [x] Remove home auto-authorization (bootstrap grants launch dir only)
- [x] Native picker authorization (`2318538`, `dbeb350`)
- [x] LSP allowlist (`cfd80a7`)
- [x] Secret service/account allowlist (`0fd3080`)
- [x] `fs_save_copy` restriction (source guarded, destination dialog-granted)
- [x] SSRF hardening (`6bdb2ee`)
- [x] Opener/asset/CSP tightening (`cae0ae7`)
- [x] Native plan-mode enforcement (`e6c7901`)
- [x] Legacy import command quarantine + review UI (`c5f1ba6`)
- [x] Local-only logging, verbose detail opt-in (`ccce391`)
- [x] Tests — see the Verification table in `TAREX_SECURITY_AUDIT.md`

## Phase 5 — Release and CI

- [x] Tarex updater endpoint — repointed to
      `https://github.com/Kishir298/tarex-ui/releases/latest/download/latest.json`
      and asserted by `updater_config_tests`
- [x] Fresh Tarex-owned updater signing keypair (`7fecf38`) — pubkey replaced
      in `tauri.conf.json`; `TAURI_SIGNING_PRIVATE_KEY` and
      `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` are repository secrets; both
      `release.yml` bindings read the secret. Signature round-trip verified
      locally against the committed pubkey.
- [x] Release scripts/tests (`scripts/release.check.mjs`, 3/3)
- [x] Remove dead workflows — only `ci.yml` and `release.yml` remain, both
      necessary
- [x] Cross-platform CI — `ci.yml` runs frontend, rustfmt, `cargo check` +
      clippy + nextest on Linux, `cargo check` + nextest on Windows and
      macOS, plus terminal tests on Windows and macOS
- [x] Release trigger — `release.yml` is `workflow_dispatch` only (`a12f035`)
- [x] CI hardening — top-level `permissions: contents: read`, `timeout-minutes`
      on every job, `cargo fmt --all --check` gate (`a12f035`)
- [x] Repo-wide `cargo fmt` as a mechanical commit (`0024562`)
- [x] Release dry run — `pnpm tauri build --debug --bundles app` produced
      `src-tauri/target/debug/bundle/macos/Tarex UI.app` (80M) and the app
      launched, opened a PTY, and quit cleanly. `release.yml` was
      deliberately **not** dispatched: that would publish a release, which is
      out of scope for this task.

## Phase 6 — VPN

- [x] Document deferred VPN support (`README.md` “VPN functionality is
      deferred”, this file, and `TAREX.md`)

## Phase 7 — Final verification and delivery

- [x] Full checks (2026-11-22, macOS): `pnpm check-types` pass; `pnpm lint`
      exit 0 (88 pre-existing warnings, 1 info, zero added); `pnpm test`
      1392/1392 (194 files); `cargo test --locked` 468 pass / 0 fail /
      1 ignored; `cargo clippy --all-targets --locked -- -D warnings` pass;
      `cargo fmt --all --check` pass; `pnpm build` pass;
      `node --test scripts/release.check.mjs` 3/3
- [x] Cross-platform CI verification on the pushed branch — run
      [44992973687](https://github.com/Kishir298/tarex-ui/actions/runs/44992973687)
      green on all 8 jobs: `cargo fmt`, `frontend`, `rust`, `coverage`,
      `terminal-test` (Windows + macOS), `rust-test` (Windows + macOS).
      nextest: Linux 477/477, macOS 468/468, Windows 452/452 (0 skipped).
      Two Windows-only PTY test defects were exposed and fixed
      (`94ff8e4`); `--no-fail-fast` was added so one failure cannot hide
      the rest (`62957e3`).
- [x] Smoke tests — built `Tarex UI.app` launched from the CLI: process
      stayed healthy, opened a PTY (`pty opened id=1`), logged no panics,
      and exited cleanly on SIGTERM. The single `ERROR` line is the updater
      probing `…/releases/latest/download/latest.json`, which returns 404
      until the first release is published — expected.
- [x] Dependency advisories cleared (`3b57d7f`) — `pnpm audit --prod`
      reports no known vulnerabilities; CI's audit step stays advisory.
- [x] Known flaky test fixed (`8731d29`) — the Svelte case in
      `languageResolver.test.ts` read a partially parsed CodeMirror tree;
      it now forces the parse with `ensureSyntaxTree`. Six consecutive full
      `pnpm test` runs passed afterwards.
- [x] `.gitignore` review — covers `.env*`, `secrets.json`,
      `credentials.json`, `*.pem`/`*.key`/`*.p12`/`*.pfx`, `.npmrc`
- [x] Staged commits (one logical topic per commit)
- [x] Push branch `tarex-ui/security-remediation-2026-11`
- [ ] Open PR (only if requested)

## Known issues / follow-ups

- `pnpm audit --prod` is clean as of `3b57d7f` (zod bumped to 4.6.7 for
  GHSA-mrhx-c7xc-m3w5; `@ai-sdk/provider-utils` pinned to ^4.0.36 for
  GHSA-866g-f22w-33x8). The CI audit step is still advisory
  (`continue-on-error: true`), so a future transitive advisory would not
  fail the build.
- Windows/Linux **unit and integration** verification now runs on CI
      (Rust tests on Windows/macOS/Linux, terminal tests on Windows/macOS),
      but no GUI smoke test has been run on Windows or Linux — only macOS
      launched the built app in this session.
- Until the first release is published, the updater logs
      `update endpoint did not respond with a successful status code` on
      every start, because `…/releases/latest/download/latest.json` is 404.
      Expected; disappears once `release.yml` has produced a release.
- Efficiency Mode priority lowering is one-way on macOS without privileges;
  the Settings toggle surfaces OS refusals honestly.
- `frame-src` in the CSP still allows `http:`/`https:` for remote previews —
  narrowing it needs a product decision on allowed origins.
- Releases are manual (`workflow_dispatch`) until an automatic trigger is
  wanted again.
