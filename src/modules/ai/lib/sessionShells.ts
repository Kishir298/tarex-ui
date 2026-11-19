import { native } from "./native";
import {
  currentWorkspaceEnv,
  useWorkspaceEnvStore,
  workspaceScopeKey,
} from "@/modules/workspace";

/**
 * Per-session lazy shell-session id. The agent gets one persistent shell per
 * chat session + workspace scope, so cwd survives across tool calls (cd,
 * mkdir+cd, etc).
 *
 * This registry lives in its own light module (no `ai`/`zod` imports) so the
 * chat store can close sessions on teardown without pulling the shell tool
 * implementations into the eager startup bundle.
 */
const sessionShells = new Map<string, Promise<number>>();

export function getSessionShell(
  sessionId: string,
  cwd: string | null,
): Promise<number> {
  let p = sessionShells.get(sessionId);
  if (!p) {
    p = native.shellSessionOpen(cwd);
    sessionShells.set(sessionId, p);
    // Never cache a rejection: a single transient open failure must not
    // permanently break `bash_run` for this key. Drop the entry so the next
    // call retries from scratch.
    void p.then(
      () => {},
      () => {
        if (sessionShells.get(sessionId) === p) sessionShells.delete(sessionId);
      },
    );
  }
  return p;
}

export function workspaceSessionKey(sessionId: string): string {
  return `${sessionId}:${workspaceScopeKey(currentWorkspaceEnv())}`;
}

/** Close and forget every shell session belonging to a chat session. */
export function closeSessionShells(sessionId: string): void {
  closeShellsWhere((key) => key.startsWith(`${sessionId}:`));
}

/** Close and forget every shell session opened under a workspace scope. */
export function closeWorkspaceShells(scopeKey: string): void {
  closeShellsWhere((key) => key.endsWith(`:${scopeKey}`));
}

function closeShellsWhere(predicate: (key: string) => boolean): void {
  for (const key of [...sessionShells.keys()]) {
    if (!predicate(key)) continue;
    const p = sessionShells.get(key);
    sessionShells.delete(key);
    if (!p) continue;
    void p.then(
      (id) => void native.shellSessionClose(id).catch(() => {}),
      () => {},
    );
  }
}

// Workspace teardown (local ⇄ WSL switch): shells opened in the old scope
// are stale — close them instead of leaking backend sessions until exit.
useWorkspaceEnvStore.subscribe((state, prev) => {
  if (state.env !== prev.env) closeWorkspaceShells(workspaceScopeKey(prev.env));
});
