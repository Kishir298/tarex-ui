import { native } from "./native";
import { usePlanStore } from "../store/planStore";

/**
 * Agent session token, minted once per app run by Rust.
 *
 * Every agent write carries it so Rust can tell an agent mutation apart from
 * the user's own editor saves and refuse it while plan mode is on. Creating
 * the token also pushes the renderer's plan-mode flag across, which heals the
 * one desync a webview reload can leave behind (Rust outlives the renderer).
 */
let token: string | null = null;
let inflight: Promise<string | null> | null = null;

export function ensureAiSession(): Promise<string | null> {
  if (token) return Promise.resolve(token);
  if (!inflight) {
    inflight = native
      .aiSessionCreate()
      .then((issued) => {
        token = issued;
        return syncPlanMode(issued);
      })
      .catch(() => null)
      .finally(() => {
        inflight = null;
      });
  }
  return inflight;
}

async function syncPlanMode(session: string): Promise<string> {
  try {
    await native.aiSetPlanMode(usePlanStore.getState().active);
  } catch {
    // A failed sync leaves Rust's flag stale, which can only ever refuse the
    // next write (fail closed). Toggling plan mode retries the push.
  }
  return session;
}

/** Test hook — drops the cached token so the next call mints a new one. */
export function resetAiSession(): void {
  token = null;
  inflight = null;
}
