//! Agent session tokens and native plan-mode enforcement.
//!
//! Plan mode ("propose changes, do not apply them") used to be a renderer-only
//! flag, so every write tool had to remember to check it. The check now also
//! lives here: agent writes go through [`ai_write_file`] / [`ai_create_file`] /
//! [`ai_create_dir`], which require a live session token and refuse while plan
//! mode is on. A write tool that forgets its own check still cannot mutate.
//!
//! This is defence in depth for the agent tool surface, not a boundary against
//! a compromised renderer — the renderer picks which command to call, and the
//! user's own editor writes through `fs_write_file` and stays unaffected.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;

use tauri::State;

use crate::modules::fs;
use crate::modules::workspace::{WorkspaceEnv, WorkspaceRegistry};

/// Refusal returned while plan mode is on. The frontend matches the prefix to
/// queue the edit for review instead of surfacing it as a failure.
pub const PLAN_MODE_ERROR: &str = "plan mode is active: the change was queued for review instead";

/// Refusal for a token Rust did not issue (or one that has been superseded).
const SESSION_ERROR: &str = "agent session token is not active";

#[derive(Default)]
pub struct AiSessionState {
    /// Opaque token identifying the agent write surface. Created on first use
    /// and kept for the lifetime of the process.
    token: RwLock<Option<String>>,
    plan_mode: AtomicBool,
}

impl AiSessionState {
    fn issue(&self) -> String {
        if let Ok(token) = self.token.read() {
            if let Some(existing) = token.as_ref() {
                return existing.clone();
            }
        }
        let issued = new_token();
        if let Ok(mut slot) = self.token.write() {
            // A concurrent creator may have won the race; keep the first one so
            // every renderer holds the same token.
            if let Some(existing) = slot.as_ref() {
                return existing.clone();
            }
            *slot = Some(issued.clone());
        }
        issued
    }

    fn matches(&self, candidate: &str) -> bool {
        self.token
            .read()
            .map(|slot| slot.as_deref() == Some(candidate))
            .unwrap_or(false)
    }

    fn set_plan_mode(&self, active: bool) {
        self.plan_mode.store(active, Ordering::Relaxed);
    }

    fn plan_mode(&self) -> bool {
        self.plan_mode.load(Ordering::Relaxed)
    }
}

/// Process-unique, non-sequential token. Secrecy is not the point — the
/// renderer could mint one anyway — uniqueness is, so a stale token from a
/// previous run can never satisfy [`ensure_writable`].
fn new_token() -> String {
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    format!(
        "as-{}-{}-{}",
        std::process::id(),
        nanos,
        COUNTER.fetch_add(1, AtomicOrdering::Relaxed)
    )
}

/// The single gate every agent write passes through.
pub fn ensure_writable(state: &AiSessionState, token: &str) -> Result<(), String> {
    if !state.matches(token) {
        return Err(SESSION_ERROR.into());
    }
    if state.plan_mode() {
        return Err(PLAN_MODE_ERROR.into());
    }
    Ok(())
}

#[tauri::command]
pub fn ai_session_create(state: State<'_, AiSessionState>) -> String {
    state.issue()
}

/// Returns the new plan-mode state so the caller can treat a refused IPC the
/// same way as a local toggle failure.
#[tauri::command]
pub fn ai_session_set_plan_mode(state: State<'_, AiSessionState>, active: bool) -> bool {
    state.set_plan_mode(active);
    active
}

#[tauri::command]
pub fn ai_session_plan_mode(state: State<'_, AiSessionState>) -> bool {
    state.plan_mode()
}

/// Agent counterpart of `fs_write_file`. Gated on the session token, then
/// handed to the same implementation the editor uses so the two paths can
/// never diverge on authorisation or atomicity.
#[tauri::command]
pub async fn ai_write_file(
    path: String,
    content: String,
    workspace: Option<WorkspaceEnv>,
    session: String,
    agent: State<'_, AiSessionState>,
    app: tauri::AppHandle,
    registry: State<'_, WorkspaceRegistry>,
) -> Result<u64, String> {
    ensure_writable(agent.inner(), &session)?;
    fs::file::fs_write_file(path, content, workspace, Some("ai".into()), app, registry).await
}

#[tauri::command]
pub fn ai_create_file(
    path: String,
    workspace: Option<WorkspaceEnv>,
    session: String,
    agent: State<'_, AiSessionState>,
    registry: State<'_, WorkspaceRegistry>,
) -> Result<(), String> {
    ensure_writable(agent.inner(), &session)?;
    fs::mutate::fs_create_file(path, workspace, registry)
}

#[tauri::command]
pub fn ai_create_dir(
    path: String,
    workspace: Option<WorkspaceEnv>,
    session: String,
    agent: State<'_, AiSessionState>,
    registry: State<'_, WorkspaceRegistry>,
) -> Result<(), String> {
    ensure_writable(agent.inner(), &session)?;
    fs::mutate::fs_create_dir(path, workspace, registry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_is_refused_without_a_live_token() {
        let state = AiSessionState::default();
        assert!(ensure_writable(&state, "made-up").is_err());
        let token = state.issue();
        assert!(ensure_writable(&state, &token).is_ok());
        assert!(ensure_writable(&state, "made-up").is_err());
    }

    #[test]
    fn issue_is_stable_for_the_life_of_the_state() {
        let state = AiSessionState::default();
        let first = state.issue();
        assert_eq!(state.issue(), first, "a second create must not invalidate the first token");
        assert!(ensure_writable(&state, &first).is_ok());
    }

    #[test]
    fn tokens_are_unique_across_states() {
        let a = AiSessionState::default().issue();
        let b = AiSessionState::default().issue();
        assert_ne!(a, b, "a token from one run must not satisfy another");
    }

    #[test]
    fn plan_mode_refuses_writes_but_still_requires_a_live_token() {
        let state = AiSessionState::default();
        let token = state.issue();

        state.set_plan_mode(true);
        let error = ensure_writable(&state, &token).unwrap_err();
        assert!(
            error.starts_with("plan mode is active"),
            "unexpected refusal: {error}"
        );

        // An unknown token is refused for its own reason, not mistaken for
        // plan mode — the two failures must stay distinguishable.
        let unknown = ensure_writable(&state, "other").unwrap_err();
        assert_eq!(unknown, SESSION_ERROR);

        state.set_plan_mode(false);
        assert!(ensure_writable(&state, &token).is_ok());
        assert!(!state.plan_mode());
    }

    #[test]
    fn plan_mode_flag_round_trips_through_the_state() {
        let state = AiSessionState::default();
        assert!(!state.plan_mode());
        state.set_plan_mode(true);
        assert!(state.plan_mode());
        state.set_plan_mode(false);
        assert!(!state.plan_mode());
    }
}
