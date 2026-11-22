# Tarex UI — Security Audit and Remediation

Date: 2026-11-11
Status: remediation complete on `tarex-ui/security-remediation-2026-11`; see Residual risks.

## Threat model

Tarex is a local desktop terminal, editor, and AI agent. The primary untrusted inputs are:

- Model output and prompt-injected content that drives agent tools.
- Project files, repository names, terminal output, and web content shown in the UI.
- Renderer content if a remote URL or crafted asset ever enters the webview.
- Legacy Terex data imported on first launch.

The security boundary must not rely on UI text or prompt instructions. Native commands must enforce path, process, secret, and command constraints.

## Findings and fixes

### Critical / High

1. **Unrestricted filesystem reads and writes**
   - Finding: `fs_read_file`, `fs_write_file`, `fs_stat`, and `fs_canonicalize` resolve paths without checking `WorkspaceRegistry`.
   - Impact: a compromised renderer or prompt-injected tool can read/write outside the active workspace.
   - Fix: require a `PathAccess` guard for all filesystem commands. Agent tools use an explicit workspace-only scope. UI paths are authorized by native pickers, workspace roots, or explicit user confirmation.
   - Status: **fixed** — every `fs_*` command now resolves through `ensure_authorized_path` (full reconstructed path, sensitive rules before the workspace check).

2. **Home directory automatically authorized**
   - Finding: `bootstrap_registry` authorizes the home directory on every launch.
   - Impact: the workspace boundary is effectively the entire home directory.
   - Fix: remove automatic home authorization. Authorize only the launch workspace and explicitly selected paths.
   - Status: **fixed** — `bootstrap_registry` no longer grants home; Home is granted only by the explicit workspace picker on boot.

3. **Renderer can authorize arbitrary paths**
   - Finding: `workspace_authorize(path)` trusts any renderer-supplied path.
   - Impact: an XSS or compromised renderer can expand its own access.
   - Fix: add native folder/file picker commands that return one-time authorization tokens; `workspace_authorize` only accepts a path proven to come from a native picker or an explicit user confirmation flow.
   - Status: **fixed** — `workspace_authorize` was replaced by native `dialog_open_files` / `dialog_save_as`, which mint one-time picked-path grants in Rust; the renderer cannot self-authorize.

4. **Arbitrary LSP process execution**
   - Finding: `lsp_spawn` accepts command, args, env, and root from the renderer.
   - Impact: arbitrary local code execution.
   - Fix: allowlist known LSP commands and arguments; restrict environment keys; require the root to be an authorized workspace; reject shells and interpreter `-e` forms.
   - Status: **fixed** — preset allowlist of 18 known servers, validated command/args/env bounds, registered custom servers persisted Rust-side, and a native confirm dialog before any custom server runs.

5. **Arbitrary copy source/destination**
   - Finding: `fs_save_copy` copies any readable source to any absolute destination.
   - Impact: exfiltration of sensitive files through a user-visible save dialog.
   - Fix: source must be authorized; destination must come from a native save dialog. Never follow symlinks outside the authorized source.
   - Status: **fixed** — `fs_save_copy` authorizes the source and requires the destination to be a Rust-minted writable grant from `dialog_save_as`.

6. **Unscoped secret access**
   - Finding: `secrets_get`, `secrets_set`, `secrets_delete`, and `secrets_get_all` accept arbitrary service/account names.
   - Impact: a compromised renderer can read other applications' keychain entries.
   - Fix: restrict service to `tarex-ai` and accounts to a known provider/endpoint allowlist. Add explicit legacy credential import command for the old `terax-ai` service.
   - Status: **fixed** — service fixed to `tarex-ai`, accounts limited to a provider allowlist plus `compat-*-api-key`; legacy `terax-ai` values stay behind an explicit import prompt.

7. **Plan mode enforced only by prompt**
   - Finding: plan mode adds prompt text; native writes/spawns are not blocked.
   - Impact: a model can ignore the prompt and mutate the workspace.
   - Fix: store plan mode state natively. Reject mutating agent commands and process spawns while plan mode is active unless a user-approved diff is applied.
   - Status: **fixed** — plan mode and an agent session token live in Rust; `ai_write_file` / `ai_create_file` / `ai_create_dir` refuse writes without a live token and refuse them outright while plan mode is on.

### Medium

8. **Broad opener capability**
   - Finding: `opener:allow-open-path` allows `**`.
   - Impact: opening arbitrary paths/executables from renderer content.
   - Fix: remove the broad permission. Add a native `open_path` command that validates the path against authorized files and known safe operations, or use opener URL-only permissions.
   - Status: **fixed** — `opener:allow-open-path` (scope `**`) removed; a Rust `open_path` command authorizes through the path registry and refuses payloads the OS would execute.

9. **Broad asset protocol scope**
   - Finding: asset protocol scope is `**`.
   - Impact: renderer can load arbitrary local files as images/media.
   - Fix: add a custom asset protocol or validate paths against the path registry; restrict scope to app cache/assets where possible.
   - Status: **fixed** — the asset protocol scope is `$RESOURCE/**`, and every path authorization is mirrored into it by a registry hook, so previews reach exactly what `fs_*` allows.

10. **CSP too broad**
    - Finding: `connect-src` allows all `https:` and localhost; `frame-src` allows `http:`/`https:`.
    - Impact: increased exfiltration and framing surface.
    - Fix: keep only required provider endpoints and local preview origins; use an allowlist for frames and document preview needs.
    - Status: **fixed** — `connect-src` narrowed from `https:` to `api.groq.com` and `api.openai.com` (plus `ipc:` and localhost). Residual: `frame-src` still allows `http:`/`https:` — see Residual risks.

11. **SSRF edge cases in local proxy**
    - Finding: audit identified IPv4-mapped IPv6 and metadata/link-local gaps.
    - Impact: local proxy could reach unintended local services.
    - Fix: normalize IPs, block IPv4-mapped IPv6, loopback, link-local, unique-local, unspecified, and cloud metadata ranges. Resolve DNS and re-check before connect. Add tests.
    - Status: **fixed** — link-local/cloud-metadata, loopback, RFC1918, CGNAT, benchmarking, IETF, IPv4-mapped/compatible IPv6, `fe80::/10`, `fc00::/7` and `fd00:ec2::/48` are all classified; redirects are followed manually with per-hop revalidation and header stripping.

12. **Legacy import risks**
    - Finding: automatic import can carry command-bearing settings and stale process state.
    - Impact: imported commands or suspended PIDs could be activated unexpectedly.
    - Fix: import only known non-secret files; skip `suspended-processes.json`, `secrets.json`, and window state; reject symlinks; validate JSON; flag imported LSP/formatter/agent commands for review; subject them to new allowlists.
    - Status: **fixed** — the import allowlists files, rejects symlinks, validates JSON, skips `suspended-processes.json`/`secrets.json`/window state, and now holds `terminalShell`, `editorCustomFormatCommand`, `agentLaunchCommands` and `lspCustomServers` in a quarantine file that only a user review can activate.

### Low

13. **Verbose diagnostics**
    - Finding: process paths/names can be logged in debug output.
    - Fix: keep logs user-local and avoid logging secrets, full environment, or credential values.
    - Status: **fixed** — release builds log only to the app log directory, full LSP command lines/spawn records are `debug`-level, and the crate logs no secrets, environment dumps, or credential values (`TAREX_LOG=debug|trace` opts back in locally).

## Verification

Run on macOS on 2026-11-21 against `tarex-ui/security-remediation-2026-11`:

| Check | Result |
| --- | --- |
| `cargo test --locked` | **468 passed**, 0 failed, 1 ignored |
| `cargo clippy --all-targets --locked -- -D warnings` | pass |
| `cargo fmt --all --check` | pass (after a dedicated repo-wide rustfmt commit) |
| `pnpm check-types` | pass |
| `pnpm lint` | exit 0, 88 warnings + 1 info (pre-existing baseline, none added) |
| `pnpm test` | **1392 passed**, 0 failed (194 files) |
| `pnpm build` | pass |
| `node --test scripts/release.check.mjs` | 3/3 |

Test coverage added by this remediation: path/symlink escapes, sensitive-path
and picked-path rules, Rust-owned file protection, LSP preset allowlist and
request validation, secrets service/account allowlist, plan-mode and
session-token rejection, SSRF range classification and per-hop redirect
validation, legacy-import quarantine, and log target/level configuration.

One transient frontend failure was observed on a single run and did not
reproduce in three consecutive full runs; it matches the pre-existing flaky
`languageResolver.test.ts` recorded in `TAREX_PROGRESS.md`.

Cross-platform proof (Linux/Windows compile and test) comes from CI on the
pushed branch rather than this macOS session.

## Residual risks

- **The renderer is still a trusted UI surface.** These changes stop a
  prompt-injected model tool or remote content from bypassing native
  constraints; they do not (and cannot) stop a fully compromised webview,
  which can already drive the PTY. The path registry, session tokens and
  plan-mode gate are defense in depth, not a boundary against a hostile
  renderer.
- **Shell tools are not gated by plan mode.** `bash_run` / `bash_background`
  can still write files while plan mode is on. Each shell command already
  requires `needsApproval: true`, so a user or an approving model must
  sanction it; this is accepted rather than silently blocked, because
  blocking the shell outright would break the product.
- **`frame-src` still allows `http:` and `https:`.** `connect-src` is now an
  allowlist of the two provider endpoints plus localhost, but frames were left
  open because the app embeds remote previews. Narrowing it needs a decision
  on which origins the preview feature may load.
- **An accepted custom formatter runs without per-run confirmation.** That is
  the point of the review: the user saw and applied it. Declining it at the
  quarantine prompt leaves it inert.
- **Asset-protocol grants are additive for the session.** Revocation would
  require clearing the registry; restarting the app clears them.
- Hardware details may require platform commands; these are fixed-argument,
  read-only, and never accept model/user input.
- Legacy API key import is explicit and user-confirmed; keys are never logged
  and never leave the OS keychain.
