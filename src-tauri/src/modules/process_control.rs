use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, sync::Mutex};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use tauri::Manager;

#[derive(Clone, Serialize, Deserialize)]
pub struct SuspendedProcess {
    pid: u32,
    started: u64,
    name: String,
}

#[derive(Default)]
struct Control {
    path: Option<PathBuf>,
    suspended: BTreeMap<u32, SuspendedProcess>,
}

#[derive(Default)]
pub struct ProcessControlState(Mutex<Control>);

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    /// Graceful request: SIGTERM on Unix, WM_CLOSE to a main window on Windows.
    Terminate,
    /// Destructive: SIGKILL / TerminateProcess.
    Kill,
    Suspend,
    Resume,
}

/// Obviously critical processes only. Ancestor protection is the main rule;
/// this list is a second line of defence, so it stays conservative.
const CRITICAL_SYSTEM_NAMES: &[&str] = &[
    "launchd",
    "init",
    "systemd",
    "kernel_task",
    "csrss",
    "lsass",
    "smss",
    "system",
    "winlogon",
];

/// Bound on the parent-chain walk; cycles or reparenting stop it earlier.
const MAX_ANCESTOR_DEPTH: u32 = 32;

/// Rules that need no process table (pure, unit-testable).
fn pid_protection(pid: u32, self_pid: u32) -> Option<String> {
    if pid == 0 {
        return Some("protected: system process (PID 0)".into());
    }
    if pid == 1 {
        return Some("protected: system init process (PID 1)".into());
    }
    if pid > i32::MAX as u32 {
        return Some("protected: PID out of range".into());
    }
    if pid == self_pid {
        return Some("protected: Tarex's own process".into());
    }
    None
}

fn is_critical_system_process(name: &str) -> bool {
    let lowered = name.trim().to_ascii_lowercase();
    let base = lowered.strip_suffix(".exe").unwrap_or(lowered.as_str());
    CRITICAL_SYSTEM_NAMES.contains(&base)
}

/// Pids between Tarex and the root of its parent chain. The caller supplies
/// the lookup so cycles, gaps, and the depth cap stay unit-testable.
fn ancestor_pids<F: FnMut(Pid) -> Option<Pid>>(self_pid: u32, mut parent_of: F) -> Vec<u32> {
    let mut chain = Vec::new();
    let mut current = Pid::from_u32(self_pid);
    for _ in 0..MAX_ANCESTOR_DEPTH {
        let Some(parent) = parent_of(current) else {
            break;
        };
        if parent == current {
            break;
        }
        chain.push(parent.as_u32());
        current = parent;
    }
    chain
}

/// Reasons that need the process table: name and ancestry.
fn protection_reason(pid: u32, name: &str, ancestors: &[u32]) -> Option<String> {
    if is_critical_system_process(name) {
        return Some("protected: critical system process".into());
    }
    if ancestors.contains(&pid) {
        return Some("protected: Tarex ancestor".into());
    }
    None
}

fn checked_process(pid: u32, started: u64) -> Result<String, String> {
    if let Some(reason) = pid_protection(pid, std::process::id()) {
        return Err(reason);
    }
    if started == 0 {
        return Err("protected: process start time unavailable".into());
    }
    let id = Pid::from_u32(pid);
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    let process = system.process(id).ok_or("Process has exited")?;
    if process.start_time() != started {
        return Err("PID was reused; refresh the list".into());
    }
    let name = process.name().to_string_lossy().into_owned();
    let self_pid = std::process::id();
    let ancestors = ancestor_pids(self_pid, |parent| {
        system.process(parent).and_then(|p| p.parent())
    });
    if let Some(reason) = protection_reason(pid, &name, &ancestors) {
        return Err(reason);
    }
    Ok(name)
}

impl Control {
    fn load(&mut self, app: &tauri::AppHandle) -> Result<(), String> {
        if self.path.is_some() {
            return Ok(());
        }
        let dir = app.path().app_cache_dir().map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join("suspended-processes.json");
        if path.exists() {
            self.suspended =
                serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
        }
        self.path = Some(path);
        Ok(())
    }
    fn save(&self) -> Result<(), String> {
        use std::io::Write;
        let path = self.path.as_ref().ok_or("Process store unavailable")?;
        let mut file = tempfile::NamedTempFile::new_in(
            path.parent().ok_or("Missing process store directory")?,
        )
        .map_err(|e| e.to_string())?;
        file.write_all(&serde_json::to_vec(&self.suspended).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        file.as_file().sync_all().map_err(|e| e.to_string())?;
        file.persist(path).map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Pure bookkeeping for exit: take every recorded pid and clear the store.
fn drain_suspended(suspended: &mut BTreeMap<u32, SuspendedProcess>) -> Vec<u32> {
    std::mem::take(suspended).into_keys().collect()
}

impl ProcessControlState {
    /// Resume every process this session recorded as suspended — including
    /// records left on disk by an earlier run — then clear the store.
    /// Best-effort by design: the app is exiting.
    pub fn resume_suspended_on_exit(&self, app: &tauri::AppHandle) {
        let Ok(mut control) = self.0.lock() else {
            return;
        };
        let _ = control.load(app);
        for pid in drain_suspended(&mut control.suspended) {
            let _ = signal(pid, &Action::Resume);
        }
        let _ = control.save();
    }
}

#[cfg(unix)]
fn signal(pid: u32, action: &Action) -> Result<(), String> {
    let signal = match action {
        Action::Terminate => libc::SIGTERM,
        Action::Kill => libc::SIGKILL,
        Action::Suspend => libc::SIGSTOP,
        Action::Resume => libc::SIGCONT,
    };
    if unsafe { libc::kill(pid as i32, signal) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

/// Graceful close on Windows: post WM_CLOSE to a visible top-level window of
/// the process. Best-effort — a process without a window gets a clear error
/// instead of a silent force kill.
#[cfg(windows)]
fn close_main_window(pid: u32) -> Result<(), String> {
    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
    };

    struct CloseRequest {
        pid: u32,
        posted: bool,
    }

    unsafe extern "system" fn close_visible_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let request = &mut *(lparam as *mut CloseRequest);
        let mut window_pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut window_pid);
        if window_pid == request.pid
            && IsWindowVisible(hwnd) != 0
            && PostMessageW(hwnd, WM_CLOSE, 0, 0) != 0
        {
            request.posted = true;
            return 0;
        }
        1
    }

    let mut request = CloseRequest { pid, posted: false };
    let _ = unsafe {
        EnumWindows(
            Some(close_visible_window),
            &mut request as *mut CloseRequest as LPARAM,
        )
    };
    if request.posted {
        Ok(())
    } else {
        Err(format!(
            "terminate unavailable: PID {pid} has no visible window to close; use force kill"
        ))
    }
}

#[cfg(windows)]
fn signal(pid: u32, action: &Action) -> Result<(), String> {
    if matches!(action, Action::Terminate) {
        return close_main_window(pid);
    }
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{
            OpenProcess, TerminateProcess, PROCESS_SUSPEND_RESUME, PROCESS_TERMINATE,
        },
    };
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtSuspendProcess(process: *mut core::ffi::c_void) -> i32;
        fn NtResumeProcess(process: *mut core::ffi::c_void) -> i32;
    }
    unsafe {
        let access = if matches!(action, Action::Kill) {
            PROCESS_TERMINATE
        } else {
            PROCESS_SUSPEND_RESUME
        };
        let handle = OpenProcess(access, 0, pid);
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let ok = match action {
            Action::Kill => TerminateProcess(handle, 1) != 0,
            Action::Suspend => NtSuspendProcess(handle) >= 0,
            Action::Resume => NtResumeProcess(handle) >= 0,
            Action::Terminate => unreachable!("terminate is handled before OpenProcess"),
        };
        CloseHandle(handle);
        if ok {
            Ok(())
        } else {
            Err("Process action was denied".into())
        }
    }
}

/// Efficiency Mode touches only Tarex's own process priority, never other
/// processes. Unprivileged priority raising is refused by the OS on some
/// platforms; the refusal is reported verbatim instead of hidden.
#[cfg(unix)]
fn set_own_priority(enabled: bool) -> Result<(), String> {
    let nice_value: libc::c_int = if enabled { 10 } else { 0 };
    if unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, nice_value) } != 0 {
        let error = std::io::Error::last_os_error();
        if !enabled && error.raw_os_error() == Some(libc::EPERM) {
            return Err(
                "The OS did not let Tarex raise its own priority; normal priority returns after restarting Tarex."
                    .into(),
            );
        }
        return Err(error.to_string());
    }
    Ok(())
}

#[cfg(windows)]
fn set_own_priority(enabled: bool) -> Result<(), String> {
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, SetPriorityClass, BELOW_NORMAL_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS,
    };
    let class = if enabled {
        BELOW_NORMAL_PRIORITY_CLASS
    } else {
        NORMAL_PRIORITY_CLASS
    };
    if unsafe { SetPriorityClass(GetCurrentProcess(), class) } != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

#[tauri::command]
pub fn suspended_processes(
    app: tauri::AppHandle,
    state: tauri::State<'_, ProcessControlState>,
) -> Result<Vec<SuspendedProcess>, String> {
    let mut control = state.0.lock().map_err(|e| e.to_string())?;
    control.load(&app)?;
    control
        .suspended
        .retain(|_, process| checked_process(process.pid, process.started).is_ok());
    Ok(control.suspended.values().cloned().collect())
}

#[tauri::command]
pub fn control_process(
    app: tauri::AppHandle,
    state: tauri::State<'_, ProcessControlState>,
    pid: u32,
    started: u64,
    action: Action,
) -> Result<Vec<SuspendedProcess>, String> {
    let name = checked_process(pid, started)?;
    let mut control = state.0.lock().map_err(|e| e.to_string())?;
    control.load(&app)?;
    match action {
        Action::Suspend => {
            if control
                .suspended
                .get(&pid)
                .is_some_and(|process| process.started == started)
            {
                return Ok(control.suspended.values().cloned().collect());
            }
            control
                .suspended
                .insert(pid, SuspendedProcess { pid, started, name });
            control.save()?;
            if let Err(error) = signal(pid, &Action::Suspend) {
                control.suspended.remove(&pid);
                let _ = control.save();
                return Err(error);
            }
        }
        Action::Resume => {
            signal(pid, &Action::Resume)?;
            control.suspended.remove(&pid);
            control.save()?;
        }
        Action::Kill => {
            signal(pid, &Action::Kill)?;
            control.suspended.remove(&pid);
            control.save()?;
        }
        Action::Terminate => {
            // A stopped process cannot act on a close request; continue it
            // first so SIGTERM / WM_CLOSE is actually delivered.
            if control.suspended.remove(&pid).is_some() {
                let _ = signal(pid, &Action::Resume);
                control.save()?;
            }
            signal(pid, &Action::Terminate)?;
        }
    }
    Ok(control.suspended.values().cloned().collect())
}

/// Lowers (or restores) only Tarex's own process priority. Best-effort: a
/// failure is returned to the caller, never swallowed as success.
#[tauri::command]
pub fn efficiency_mode_set(enabled: bool) -> Result<(), String> {
    set_own_priority(enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_pids_are_rejected() {
        assert!(checked_process(0, 1).is_err());
        assert!(checked_process(1, 1).is_err());
        assert!(checked_process(std::process::id(), 1).is_err());
        assert!(checked_process(u32::MAX, 1).is_err());
        assert!(checked_process(42, 0).is_err());
    }

    #[test]
    fn protection_reasons_name_the_rule() {
        assert_eq!(
            pid_protection(0, 7).as_deref(),
            Some("protected: system process (PID 0)")
        );
        assert_eq!(
            pid_protection(1, 7).as_deref(),
            Some("protected: system init process (PID 1)")
        );
        assert_eq!(
            pid_protection(7, 7).as_deref(),
            Some("protected: Tarex's own process")
        );
        assert_eq!(
            pid_protection(u32::MAX, 7).as_deref(),
            Some("protected: PID out of range")
        );
        assert_eq!(pid_protection(99, 7), None);

        assert!(is_critical_system_process("launchd"));
        assert!(is_critical_system_process("System"));
        assert!(is_critical_system_process("winlogon.exe"));
        assert!(is_critical_system_process(" systemd "));
        assert!(!is_critical_system_process("Finder"));
        assert!(!is_critical_system_process("systemd-run"));

        assert_eq!(
            protection_reason(42, "launchd", &[]).as_deref(),
            Some("protected: critical system process")
        );
        assert_eq!(
            protection_reason(42, "node", &[3, 42, 41]).as_deref(),
            Some("protected: Tarex ancestor")
        );
        assert_eq!(protection_reason(42, "node", &[3]), None);
    }

    #[test]
    fn ancestor_walk_follows_parents_with_a_depth_cap() {
        let parents: BTreeMap<u32, u32> = [(10u32, 20u32), (20, 30), (30, 1)].into();
        let chain = ancestor_pids(10, |pid| {
            parents.get(&pid.as_u32()).copied().map(Pid::from_u32)
        });
        assert_eq!(chain, vec![20, 30, 1]);

        // A process that is its own parent stops instead of looping forever.
        assert!(ancestor_pids(10, |_| Some(Pid::from_u32(10))).is_empty());

        // A chain longer than the cap stays bounded.
        let deep = ancestor_pids(0, |pid| Some(Pid::from_u32(pid.as_u32() + 1)));
        assert_eq!(deep.len() as u32, MAX_ANCESTOR_DEPTH);
    }

    #[test]
    fn live_tarex_parent_is_protected() {
        let self_pid = std::process::id();
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing(),
        );
        let Some(parent) = system
            .process(Pid::from_u32(self_pid))
            .and_then(|p| p.parent())
        else {
            return;
        };
        let started = system.process(parent).map(|p| p.start_time()).unwrap_or(0);
        if started == 0 {
            return;
        }
        let error = checked_process(parent.as_u32(), started).unwrap_err();
        assert!(error.contains("protected: Tarex ancestor"), "{error}");
    }

    #[test]
    fn exit_planning_drains_every_suspended_record() {
        let mut suspended = BTreeMap::new();
        suspended.insert(
            11,
            SuspendedProcess {
                pid: 11,
                started: 1,
                name: "alpha".into(),
            },
        );
        suspended.insert(
            12,
            SuspendedProcess {
                pid: 12,
                started: 2,
                name: "beta".into(),
            },
        );
        let mut pids = drain_suspended(&mut suspended);
        pids.sort_unstable();
        assert_eq!(pids, vec![11, 12]);
        assert!(suspended.is_empty());
        assert!(drain_suspended(&mut suspended).is_empty());
    }
}
