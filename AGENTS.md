# AGENTS.md — Instructions for AI coding agents

This file contains permanent operating instructions for OpenCode, Codex, Claude, and other AI agents working in this repository. Read it before making any change. Project-specific working memory lives in `TAREX.md`; day-to-day progress is tracked in `TAREX_PROGRESS.md`.

## Repository identity

- Destination repository: `https://github.com/Kishir298/tarex-ui` (local remote `origin`, default branch `main`).
- Upstream repository: `https://github.com/3xecutablefile/terex-ui` (local remote `upstream`).
- This project is a fork: **Tarex UI** is derived from **Terex UI**. Preserve upstream attribution and the Apache-2.0 license.
- Never modify the upstream working tree. Never modify the older `Kishir298/Tarex-Ui` repository.
- Never overwrite `.git`, never force-push, never rewrite shared history.

## Mandatory startup procedure

When an agent starts work in this repository:

1. Confirm the working directory is the repo root and verify `git remote -v` shows `origin` → `Kishir298/tarex-ui` and `upstream` → `3xecutablefile/terex-ui`. If `.git/config` is missing, restore it before anything else.
2. Read `AGENTS.md`, `README.md`, `TAREX.md`, `TAREX_PROGRESS.md`, and `package.json`.
3. Inspect `git status`, the active branch, and `git log --oneline -10`.
4. `git fetch --prune` both remotes; pull only with fast-forward behaviour when safe.
5. Never discard uncommitted changes; if the working tree looks partially deleted, recover with `git restore` from `HEAD` before assuming files were lost.
6. Audit the repository for existing errors (run the baseline checks below).
7. Compare relevant code against the original Terex-UI source (`upstream/main`) before rewriting anything.
8. Verify dependencies (`pnpm install --frozen-lockfile`) and project structure.
9. Establish a green baseline before changing source code; record any pre-existing failures.
10. Only then start implementing.

## Project structure invariants

The root hierarchy must match upstream:

```text
tarex-ui/
├── .github/  docs/  packages/  public/  scripts/  src/  src-tauri/  nix/
├── AGENTS.md  README.md  TAREX.md  package.json  vite.config.ts  tsconfig.json
└── ...
```

- The legitimate `src/` directory contains application source only (`src/app`, `src/modules`, `src/settings`, …).
- **Never** create `src/src/`, `src/src-tauri/`, `src/docs/`, `src/packages/`, or any nested project-level wrapper.
- Never delete the root `src/` directory. Never move project-level files inside it.
- Generated/ignored artifacts: `node_modules/`, `dist/`, `src-tauri/target/`, `src-tauri/binaries/`, `.aider*`, `.secrets/` — never commit them.

## Mandatory audit rules

Before substantial changes:

- Inspect all relevant files and their dependencies; identify root causes, not symptoms.
- Check existing tests and add or update tests for changed invariants.
- Check platform compatibility (macOS, Windows, Linux); never assume.
- Review security implications at every boundary (IPC, filesystem, network, AI tool surface).
- Verify the requested functionality is not already implemented; avoid unnecessary rewrites.
- Preserve Tarex-specific improvements; do not blindly overwrite files with upstream versions.

## Commands

```sh
pnpm install --frozen-lockfile   # dependencies (pnpm, Node >= 22)
pnpm check-types                 # tsc --noEmit
pnpm lint                        # biome lint (warnings are pre-existing; add none)
pnpm test                        # vitest run
pnpm build                       # production frontend build
pnpm tauri build                 # full desktop build
node --test scripts/release.check.mjs

cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --locked -- -D warnings
cargo fmt --manifest-path src-tauri/Cargo.toml --check
```

Run focused tests immediately after each edit (`pnpm exec vitest run <file>` or `cargo test --test <name>`), then the full suites before committing.

## Git rules

- Never force-push, never rewrite shared history, never overwrite user changes.
- Use separate, meaningful commits — one logical topic per commit (see conventional messages: `fix(...)`, `feat(...)`, `refactor(...)`, `docs(...)`, `chore(...)`, `test(...)`).
- Stage only the files belonging to the topic; inspect `git diff --staged` before committing.
- Run relevant tests before committing; never commit a change that breaks the build or tests.
- Never commit credentials, secrets, signing keys, `.env` files, local app data, caches, or build artifacts.
- Do not push without explicit authorization from the user. When authorized, push the working branch; never force-push.
- Preserve upstream license and attribution requirements in every change.

## Security and product rules

- Do not silently migrate credentials; API key import requires an explicit prompt.
- Do not fabricate hardware, telemetry, battery, or VPN capabilities — show “Unavailable” instead.
- Do not weaken AI tool approval rules or plan-mode constraints.
- Keep agent/tool access constrained to the active workspace at the native layer.
- Validate every input at the native boundary; UI checks are not a security boundary.

## Completion rules

Every substantial task must finish with:

1. Source changes (and tests for changed invariants)
2. Test results (frontend, Rust, lint, typecheck) with pass/fail counts
3. `git status` and the list of commit hashes created
4. Remaining issues and untested platforms
5. An updated entry in the relevant audit/progress document when the task changed documented behaviour

## Documentation

- `README.md` — user-facing overview; must stay accurate; never claim unfinished features work.
- `TAREX.md` — project working memory, architecture, naming map, non-negotiable rules.
- `docs/audits/UPSTREAM_SYNC_AUDIT.md` — upstream/fork classification and audit findings.
- `docs/architecture/*` — subsystem documentation; update when behaviour changes.
