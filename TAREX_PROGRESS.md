# Tarex UI — Progress

Last updated: 2026-11-19

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

Findings are documented in `TAREX_SECURITY_AUDIT.md`. Full remediation is
deferred by decision; no security gates were weakened in this work.

- [ ] Path guard for all fs commands
- [ ] Remove home auto-authorization
- [ ] Native picker authorization
- [ ] LSP allowlist
- [ ] Secret service/account allowlist
- [ ] `fs_save_copy` restriction
- [ ] SSRF hardening
- [ ] Opener/asset/CSP tightening
- [ ] Native plan-mode enforcement
- [ ] Tests

## Phase 5 — Release and CI

- [~] Tarex updater endpoint — repointed to
      `https://github.com/Kishir298/tarex-ui/releases/latest/download/latest.json`
      and asserted by `updater_config_tests`; the fresh signing keypair still
      has to be created and stored as the `TAURI_SIGNING_PRIVATE_KEY`
      repository secret before releases can publish
- [x] Release scripts/tests (`scripts/release.check.mjs`)
- [ ] Remove dead workflows
- [ ] Cross-platform CI
- [ ] Release dry run

## Phase 6 — VPN

- [x] Document deferred VPN support (`README.md` “VPN functionality is
      deferred”, this file, and `TAREX.md`)

## Phase 7 — Final verification and delivery

- [x] Full checks (2026-11-19, macOS): `pnpm check-types` pass;
      `pnpm lint` exit 0 (88 pre-existing warnings, 1 info, zero added);
      `pnpm test` 1370/1370 (one pre-existing flaky Svelte test, see known
      issues); `cargo test --locked` 403 pass / 0 fail / 1 ignored;
      `cargo clippy --all-targets -- -D warnings` pass;
      `cargo fmt --check` fails only on pre-existing hunks; `pnpm build`
      pass; `node --test scripts/release.check.mjs` 3/3 pass;
      `pnpm tauri build --debug --bundles app` produced
      `src-tauri/target/debug/bundle/macos/Tarex UI.app`
- [ ] Smoke tests
- [x] `requirements.txt` / `.gitignore` review — no new packages were
      installed (pnpm only, so no `requirements.txt` is needed); `.gitignore`
      now also covers `.env*`, `secrets.json`, `credentials.json`,
      `*.pem`/`*.key`/`*.p12`/`*.pfx`, and `.npmrc`
- [x] Staged commits (one logical topic per commit)
- [ ] Push branch and fast-forward `main`
- [ ] Open PR (only if requested)

## Known issues / follow-ups

- `cargo fmt --check` fails with ~105 pre-existing hunks in upstream-style
  files; no CI workflow enforces rustfmt. Decide whether to run a repo-wide
  `cargo fmt` as a dedicated mechanical commit.
- Windows/Linux runtime verification requires CI or external machines; this
  session validated macOS only (Rust `cfg(windows)` paths compile but were
  not executed).
- Efficiency Mode priority lowering is one-way on macOS without privileges;
  the Settings toggle surfaces OS refusals honestly.
- Security remediation (Phase 4) remains open by decision; see
  `TAREX_SECURITY_AUDIT.md`.
- Releases cannot publish until the Tarex signing keypair exists as a
  repository secret.
- `src/modules/editor/lib/languageResolver.test.ts` ("loads Svelte files…")
  is flaky under parallel vitest load (Svelte language mode not ready);
  passes standalone. Pre-existing; unrelated to this work.
- One transient WASM vitest setup flake was observed under heavy parallel
  load and passes standalone; unrelated to these changes.
