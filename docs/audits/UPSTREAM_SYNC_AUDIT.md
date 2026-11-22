# Upstream Sync Audit — Tarex UI

Last updated: 2026-11-19

## Fork classification

| Role | Repository | Local remote | Revision |
| --- | --- | --- | --- |
| Destination (this fork) | `https://github.com/Kishir298/tarex-ui` | `origin` | `main` |
| Upstream | `https://github.com/3xecutablefile/terex-ui` | `upstream` | `main` |

- Tarex UI is a derivative work of **Terex UI**, which is itself derived from
  [Terax](https://github.com/crynta/terax-ai) by Crynta. Both attributions are
  preserved in `README.md`; the Apache-2.0 license and all third-party notices
  are retained.
- The fork base is `49a799b`, which equals `upstream/main` at the time of the
  fork. The rebrand landed in `1dd077e` ("chore: complete Tarex UI rebrand").
- Never modify the upstream working tree, never force-push, never rewrite
  shared history (see `AGENTS.md`).

## Upstream sync status

- `git fetch --prune` on both remotes (2026-11-19): `upstream/main` is still
  `49a799b` — no upstream commits exist beyond the fork base, so **no sync is
  required**.
- When upstream advances, compare per-file against `upstream/main`, preserve
  Tarex-specific improvements (rebrand, WASM marker, AI/session fixes, Phase 3
  process work, shortcut deferral), and merge deliberately — never overwrite
  local files with upstream versions wholesale.

## Divergence introduced by this fork (49a799b..HEAD)

- `1dd077e` — full Tarex rebrand (identity, naming, palette, import markers).
- `4a458dd` — rebuilt `ghostty-vt*.wasm` artifacts so the `tarex_blocks`
  marker matches the renamed source (stale `terax_blocks` in committed WASM
  broke six vitest suites).
- `1eefc78` — `AGENTS.md` operating instructions for agents.
- `f8f8893` — explorer always shows recognised dev config files
  (`.gitignore`, `.gitattributes`, `.editorconfig`, `.dockerignore`) while
  keeping other dotfiles behind Show Hidden.
- `d363e4e` — Windows environment inheritance: `effective_path()` merges
  registry PATHs for shells, git, and LSP spawns (new `windows_env.rs`).
- `93a9257` — clippy `unnecessary_sort_by` fix (pre-existing baseline
  failure).
- `955cf6f`, `c956c14`, `c72df14` — AI fixes: provider-key hydration,
  ungated panel opening, built-in models in pickers, subagent custom/local
  model support, session-shell cleanup, `TAREX.md`/`TERAX.md` project memory,
  workspace restore on AI close.
- `851dfdc` — Phase 3: graceful Terminate vs Force kill, ancestor/critical
  process protection with reasons, resume-suspended-on-exit, Efficiency Mode
  (Tarex-only priority + sampling), hardware "Unavailable" honesty, process
  manager UI.
- `19404a3` — curated plain-Ctrl deferral to focused terminals
  (`C,V,Z,A,D,E,F,K,L,N,P,R,U,W,Y`) with deferral-matrix tests.
- `07561e8` — repository metadata URLs repointed to `Kishir298/tarex-ui`
  (updater endpoint included; asserted by `updater_config_tests`).
- `9247bde..HEAD` — `tarex-ui/security-remediation-2026-11`, 23 commits:
  CI repair (`b1de0ba`, `0a63ee1`), renderer path authorization and native
  file dialogs (`918d566`, `2318538`, `dbeb350`), native-boundary guards for
  secrets/LSP/AI plan mode (`0fd3080`, `cfd80a7`, `e6c7901`), SSRF hardening
  (`6bdb2ee`), renderer capability tightening (`cae0ae7`), Tarex-owned updater
  keypair (`7fecf38`), workflow hardening and `cargo fmt` (`0024562`,
  `a12f035`), legacy-import quarantine (`c5f1ba6`), logging scope (`ccce391`),
  and documentation (`9247bde`, `c1f76cc`, `678cacd`, `8731d29`, plus the
  README and audit-doc updates on this branch). A future upstream merge must
  not clobber any of these — see `TAREX_SECURITY_AUDIT.md` for what each one
  protects.

## Audit findings

1. **Stale WASM artifacts** (fixed): the rebrand renamed the marker in Zig
   sources without rebuilding the committed `.wasm` files. Rebuild procedure
   documented in `packages/ghostty-core/adapted/UPSTREAM.md`.
2. **Eager-bundle budget** (fixed): closing a chat session must not import
   the AI SDK into the startup graph — session shells live in
   `src/modules/ai/lib/sessionShells.ts`; `src/app/eager-budget.test.ts`
   enforces this.
3. **Security remediation deferred** (open, by decision): findings live in
   `TAREX_SECURITY_AUDIT.md` (path guards, home auto-authorization, SSRF,
   allowlists, plan-mode enforcement). No approval rules or plan-mode
   constraints were weakened in this work.
4. **`cargo fmt --check` fails repo-wide** (open, pre-existing): ~105 hunks
   in upstream-style files; no CI workflow enforces rustfmt. Either adopt a
   dedicated mechanical `cargo fmt` commit or record the gate as advisory.
5. **Windows/Linux untested** (open): this session validated macOS only.
   `cfg(windows)` code (PATH registry merge, WM_CLOSE terminate, priority
   class) compiles but was not executed; CI or external machines required.
6. **macOS Efficiency Mode is one-way** (documented): unprivileged
   processes can only worsen nice; normal priority returns on next start.
   The Settings toggle surfaces OS refusals rather than hiding them.
7. **Release signing key** (open): the updater endpoint now points at
   `Kishir298/tarex-ui`, but releases cannot publish until a fresh Tarex
   signing keypair exists as the `TAURI_SIGNING_PRIVATE_KEY` repository
   secret matching the public key in `src-tauri/tauri.conf.json`.

## Validation snapshot (2026-11-19, macOS)

- `pnpm check-types` — pass
- `pnpm lint` — exit 0 (88 pre-existing warnings, 1 info; zero added)
- `pnpm test` — 1370 / 1370 pass
- `cargo test --locked` — 403 pass, 0 fail, 1 ignored
- `cargo clippy --all-targets -- -D warnings` — pass
- `cargo fmt --check` — fails on pre-existing hunks only (finding 4)
