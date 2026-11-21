use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

// Short TTL keeps the auth-check TOCTOU window tight while still coalescing the
// burst of canonicalize calls within a single panel refresh (~100ms).
const CANONICAL_TTL: Duration = Duration::from_secs(1);
const CANONICAL_CACHE_CAP: usize = 256;

/// Upper bound on session-long native-dialog grants. Oldest evicts first so a
/// runaway renderer cannot grow the set without bound.
const PICKED_PATH_CAP: usize = 128;

struct CanonicalEntry {
    canonical: PathBuf,
    inserted_at: Instant,
}

/// Hook that mirrors a newly authorized path into the asset protocol scope.
type AssetGrant = Arc<dyn Fn(&Path) + Send + Sync>;

/// A path the user confirmed in an OS file dialog, remembered for the session.
///
/// `writable` separates an "Open" pick (read-only) from a "Save as" pick.
struct PickedEntry {
    path: PathBuf,
    writable: bool,
}

#[derive(Default)]
pub struct WorkspaceRegistry {
    roots: Mutex<HashSet<PathBuf>>,
    canonical_cache: Mutex<HashMap<PathBuf, CanonicalEntry>>,
    picked: Mutex<Vec<PickedEntry>>,
    /// Mirrors every authorization into the asset protocol scope so
    /// `<img src="asset://…">` can only reach paths the filesystem commands
    /// already allow. Installed once during app setup; `None` outside the
    /// app (unit tests, library use) simply skips the mirror.
    asset_grant: RwLock<Option<AssetGrant>>,
}

impl WorkspaceRegistry {
    pub fn authorize<P: AsRef<Path>>(&self, path: P) -> std::io::Result<PathBuf> {
        let canonical = std::fs::canonicalize(path.as_ref())?;
        {
            let mut set = self.roots.lock().expect("workspace registry poisoned");
            set.insert(canonical.clone());
        }
        self.grant_asset(&canonical);
        Ok(canonical)
    }

    /// Installs the hook that mirrors authorizations into the asset protocol
    /// scope. Call once, during app setup, before any path is authorized.
    pub fn set_asset_grant(&self, grant: impl Fn(&Path) + Send + Sync + 'static) {
        *self.asset_grant.write().expect("workspace registry poisoned") =
            Some(Arc::new(grant));
    }

    /// Replays everything authorized before the hook was installed (the
    /// bootstrap roots and launch directory run while the builder is still
    /// being constructed, ahead of `setup`).
    pub fn replay_asset_grants(&self) {
        let hook = self.asset_grant.read().expect("workspace registry poisoned").clone();
        let Some(hook) = hook else { return; };
        for root in self.roots.lock().expect("workspace registry poisoned").iter() {
            hook(root);
        }
        for entry in self.picked.lock().expect("workspace registry poisoned").iter() {
            hook(&entry.path);
        }
    }

    fn grant_asset(&self, path: &Path) {
        let hook = self.asset_grant.read().expect("workspace registry poisoned").clone();
        if let Some(hook) = hook {
            hook(path);
        }
    }

    pub fn is_authorized(&self, target: &Path) -> bool {
        let set = self.roots.lock().expect("workspace registry poisoned");
        set.iter().any(|root| target.starts_with(root))
    }

    pub fn canonicalize_cached<P: AsRef<Path>>(&self, path: P) -> std::io::Result<PathBuf> {
        let key = path.as_ref().to_path_buf();
        {
            let cache = self
                .canonical_cache
                .lock()
                .expect("canonical cache poisoned");
            if let Some(entry) = cache.get(&key) {
                if entry.inserted_at.elapsed() < CANONICAL_TTL {
                    return Ok(entry.canonical.clone());
                }
            }
        }
        let canonical = std::fs::canonicalize(&key)?;
        let mut cache = self
            .canonical_cache
            .lock()
            .expect("canonical cache poisoned");
        if cache.len() >= CANONICAL_CACHE_CAP {
            cache.retain(|_, entry| entry.inserted_at.elapsed() < CANONICAL_TTL);
            if cache.len() >= CANONICAL_CACHE_CAP {
                cache.clear();
            }
        }
        cache.insert(
            key,
            CanonicalEntry {
                canonical: canonical.clone(),
                inserted_at: Instant::now(),
            },
        );
        Ok(canonical)
    }

    /// Canonicalizes `path` (following symlinks) and reattaches any
    /// not-yet-existing suffix, so callers see the full path the renderer
    /// asked for rather than only its surviving ancestor.
    fn reconstruct(&self, path: &Path) -> Result<PathBuf, String> {
        let mut current = path.to_path_buf();
        let mut pending: Vec<OsString> = Vec::new();

        loop {
            let canonical = match self.canonicalize_cached(&current) {
                Ok(canonical) => canonical,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let Some(name) = current.file_name() else {
                        return Err(format!("path is not accessible: {}", path.display()));
                    };
                    pending.push(name.to_os_string());
                    match current.parent() {
                        Some(parent) if parent != current => current = parent.to_path_buf(),
                        _ => return Err(format!("path is not accessible: {}", path.display())),
                    }
                    continue;
                }
                Err(error) => return Err(format!("path is not accessible: {error}")),
            };

            let mut authorized = canonical;
            for part in pending.iter().rev() {
                authorized.push(part);
            }
            return Ok(authorized);
        }
    }

    /// Remembers a path the user confirmed in a native OS dialog for the rest
    /// of the session, and returns its canonical form.
    ///
    /// This is the only way to obtain a grant that reaches outside the
    /// authorized roots or past the sensitive-path rules, and it can only be
    /// called from Rust code that actually opened the dialog — the renderer
    /// cannot mint one. Pass `writable = true` for a "Save as" destination.
    pub fn grant_picked(&self, path: &Path, writable: bool) -> Result<PathBuf, String> {
        let authorized = self.reconstruct(path)?;
        let mut picked = self.picked.lock().expect("picked path registry poisoned");
        picked.retain(|entry| entry.path != authorized);
        if picked.len() >= PICKED_PATH_CAP {
            picked.remove(0);
        }
        picked.push(PickedEntry {
            path: authorized.clone(),
            writable,
        });
        drop(picked);
        self.grant_asset(&authorized);
        Ok(authorized)
    }

    /// Whether `target` sits at or under a previously picked path.
    ///
    /// `Path::starts_with` compares whole components, so granting a file never
    /// grants a sibling or a directory named after it.
    fn is_picked(&self, target: &Path, need_writable: bool) -> bool {
        let picked = self.picked.lock().expect("picked path registry poisoned");
        picked.iter().any(|entry| {
            (!need_writable || entry.writable) && target.starts_with(&entry.path)
        })
    }
}

/// Read/write choke point for every renderer-supplied filesystem path.
///
/// Canonicalizes `path` (following symlinks, so a link pointing outside the
/// workspace resolves to its target and is rejected) and requires the result
/// to sit under an authorized root. Paths that do not exist yet — create,
/// write, rename and delete targets — are authorized through their nearest
/// existing ancestor, so the parent must already be inside the workspace.
///
/// Also applies the native sensitive-path rules from [`crate::modules::fs::protect`],
/// which the renderer mirrors but cannot be trusted to enforce on its own.
///
/// Returns the canonical path on success. Note this is a point-in-time check:
/// like the rest of the registry it does not close a TOCTOU window against a
/// concurrent symlink swap.
///
/// Paths the user confirmed in a native OS dialog (see
/// [`WorkspaceRegistry::grant_picked`]) are exempt from both the sensitive-path
/// rules and the workspace root, because that grant is minted in Rust.
pub fn ensure_authorized_path(
    registry: &WorkspaceRegistry,
    path: &Path,
) -> Result<PathBuf, String> {
    authorize_path(
        registry,
        path,
        crate::modules::fs::protect::readable_violation,
        false,
    )
}

/// [`ensure_authorized_path`] plus the write-tier system-directory blocks.
pub fn ensure_writable_path(
    registry: &WorkspaceRegistry,
    path: &Path,
) -> Result<PathBuf, String> {
    authorize_path(
        registry,
        path,
        crate::modules::fs::protect::writable_violation,
        true,
    )
}

fn authorize_path(
    registry: &WorkspaceRegistry,
    path: &Path,
    sensitivity: fn(&Path) -> Option<String>,
    need_writable: bool,
) -> Result<PathBuf, String> {
    // Reconstruct the full path before checking: a not-yet-created target is
    // only known once the surviving ancestor is canonical, and the sensitivity
    // rules must see the whole path (`/usr/bin/planted`), not just `/usr/bin`.
    let authorized = registry.reconstruct(path)?;

    if registry.is_picked(&authorized, need_writable) {
        return Ok(authorized);
    }

    if let Some(reason) = sensitivity(&authorized) {
        return Err(reason);
    }

    if !registry.is_authorized(&authorized) {
        return Err(format!(
            "path is outside the authorized workspace: {}",
            path.display()
        ));
    }

    Ok(authorized)
}

/// [`ensure_authorized_path`] for callers that only need a yes/no answer.
pub fn ensure_authorized(registry: &WorkspaceRegistry, path: &Path) -> Result<(), String> {
    ensure_authorized_path(registry, path).map(|_| ())
}

/// [`ensure_writable_path`] for callers that only need a yes/no answer.
pub fn ensure_writable(registry: &WorkspaceRegistry, path: &Path) -> Result<(), String> {
    ensure_writable_path(registry, path).map(|_| ())
}

// `None` means "use bootstrapped default". `Some` is canonicalized to defeat
// symlink/`..` traversal and must sit under an authorized root.
pub fn authorize_spawn_cwd(
    registry: &WorkspaceRegistry,
    cwd: Option<&str>,
    workspace: &WorkspaceEnv,
) -> Result<Option<PathBuf>, String> {
    let Some(cwd) = cwd.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let resolved = resolve_path(cwd, workspace);
    let canonical =
        std::fs::canonicalize(&resolved).map_err(|e| format!("cwd not accessible: {e}"))?;
    if !canonical.is_dir() {
        return Err(format!("cwd is not a directory: {}", canonical.display()));
    }
    if !registry.is_authorized(&canonical) {
        return Err(format!(
            "cwd is outside the authorized workspace: {}",
            canonical.display()
        ));
    }
    Ok(Some(canonical))
}

// User-initiated terminal spawn: canonicalize, require a real dir, and register
// it as a root instead of rejecting paths outside existing roots.
pub fn authorize_user_spawn_cwd(
    registry: &WorkspaceRegistry,
    cwd: Option<&str>,
    workspace: &WorkspaceEnv,
) -> Result<Option<PathBuf>, String> {
    let Some(cwd) = cwd.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let resolved = resolve_path(cwd, workspace);
    let canonical =
        std::fs::canonicalize(&resolved).map_err(|e| format!("cwd not accessible: {e}"))?;
    if !canonical.is_dir() {
        return Err(format!("cwd is not a directory: {}", canonical.display()));
    }
    registry.authorize(&canonical).map_err(|e| e.to_string())?;
    Ok(Some(canonical))
}

// A saved cwd can be stale or from another environment (e.g. a Windows path in
// a now-WSL space); the terminal must still open, so fall back to home.
pub fn user_spawn_cwd_or_home(
    registry: &WorkspaceRegistry,
    cwd: Option<&str>,
    workspace: &WorkspaceEnv,
) -> Option<String> {
    let cwd = cwd.map(str::trim).filter(|s| !s.is_empty())?;
    match authorize_user_spawn_cwd(registry, Some(cwd), workspace) {
        Ok(_) => Some(cwd.to_owned()),
        Err(e) => {
            log::warn!("pty cwd {cwd:?} unusable in {workspace:?} ({e}); opening home");
            None
        }
    }
}

/// Seeds the registry with the directory the app was launched from.
///
/// The user's home directory is deliberately *not* granted here: a blanket
/// home grant would make every workspace check a no-op for everything under
/// `$HOME` (including `~/.ssh` and `~/.aws`). Home becomes authorized only
/// when the frontend explicitly selects the Home workspace.
pub fn bootstrap_registry(registry: &WorkspaceRegistry) {
    let _ = registry.authorize(resolve_launch_dir());
}

#[tauri::command]
pub async fn workspace_authorize(
    path: String,
    workspace: Option<WorkspaceEnv>,
    registry: tauri::State<'_, WorkspaceRegistry>,
) -> Result<String, String> {
    let workspace = WorkspaceEnv::from_option(workspace);
    let resolved = resolve_path(&path, &workspace);
    let canonical = registry.authorize(&resolved).map_err(|e| e.to_string())?;
    Ok(crate::modules::fs::to_canon(&canonical))
}

#[tauri::command]
pub async fn workspace_current_dir(
    registry: tauri::State<'_, WorkspaceRegistry>,
) -> Result<String, String> {
    let launch = resolve_launch_dir();
    let canonical = registry.authorize(&launch).map_err(|e| e.to_string())?;
    Ok(crate::modules::fs::to_canon(&canonical))
}

// Snapshotted once at app startup so the live `current_dir()` drifting later
// (file dialogs, plugin chdir) can't shift the value seen by IPC or spawn.
static LAUNCH_CWD: OnceLock<Option<PathBuf>> = OnceLock::new();

pub fn init_launch_cwd(cli_dir: Option<&str>) {
    LAUNCH_CWD.get_or_init(|| resolve_launch_cwd(cli_dir, std::env::current_dir().ok()));
}

fn resolve_launch_cwd(cli_dir: Option<&str>, env_cwd: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(dir) = cli_dir {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            return Some(p);
        }
    }
    env_cwd.filter(|p| is_usable_launch_dir(p))
}

pub fn launch_cwd_snapshot() -> Option<PathBuf> {
    LAUNCH_CWD.get().and_then(|o| o.clone())
}

fn resolve_launch_dir() -> PathBuf {
    if let Some(cwd) = launch_cwd_snapshot() {
        return cwd;
    }
    if let Some(cwd) = std::env::current_dir()
        .ok()
        .filter(|p| is_usable_launch_dir(p))
    {
        return cwd;
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

fn is_usable_launch_dir(path: &Path) -> bool {
    if !path.is_dir() || path == Path::new("/") {
        return false;
    }
    if is_executable_dir(path) {
        return false;
    }
    let s = path.to_string_lossy();
    if s.contains(".app/Contents/") {
        return false;
    }
    // The AppImage mount (/tmp/.mount_*) is not a real working directory.
    #[cfg(target_os = "linux")]
    if std::env::var_os("APPDIR").is_some_and(|appdir| path.starts_with(&appdir)) {
        return false;
    }
    if cfg!(debug_assertions) && path.file_name().and_then(|s| s.to_str()) == Some("src-tauri") {
        return false;
    }
    true
}

fn is_executable_dir(path: &Path) -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let Some(exe_dir) = exe.parent() else {
        return false;
    };
    match (std::fs::canonicalize(path), std::fs::canonicalize(exe_dir)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(target_os = "linux")]
const APPIMAGE_PATH_VARS: &[&str] = &[
    "LD_LIBRARY_PATH",
    "PATH",
    "XDG_DATA_DIRS",
    "GST_PLUGIN_SYSTEM_PATH",
    "GST_PLUGIN_SYSTEM_PATH_1_0",
    "GST_PLUGIN_PATH",
    "GI_TYPELIB_PATH",
    "GDK_PIXBUF_MODULEDIR",
    "GIO_MODULE_DIR",
    "GSETTINGS_SCHEMA_DIR",
];

#[cfg(target_os = "linux")]
const APPIMAGE_VALUE_VARS: &[&str] = &[
    "GDK_PIXBUF_MODULE_FILE",
    "LD_PRELOAD",
    "FONTCONFIG_FILE",
    "FONTCONFIG_PATH",
];

#[cfg(target_os = "linux")]
const APPIMAGE_MARKER_VARS: &[&str] = &["APPDIR", "APPIMAGE", "ARGV0"];

pub fn appimage_env_overrides() -> Vec<(&'static str, Option<OsString>)> {
    #[cfg(target_os = "linux")]
    {
        let Some(appdir) = std::env::var_os("APPDIR") else {
            return Vec::new();
        };
        compute_appimage_env_overrides(Path::new(&appdir), |k| std::env::var_os(k))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "linux")]
fn compute_appimage_env_overrides(
    appdir: &Path,
    read: impl Fn(&str) -> Option<OsString>,
) -> Vec<(&'static str, Option<OsString>)> {
    let mut out = Vec::new();

    for &key in APPIMAGE_PATH_VARS {
        let Some(val) = read(key) else { continue };
        let original: Vec<PathBuf> = std::env::split_paths(&val).collect();
        let kept: Vec<PathBuf> = original
            .iter()
            .filter(|p| !p.as_os_str().is_empty() && !p.starts_with(appdir))
            .cloned()
            .collect();
        if kept.len() == original.len() {
            continue; // nothing AppImage-injected; leave as-is
        }
        match std::env::join_paths(&kept) {
            Ok(joined) if !kept.is_empty() => out.push((key, Some(joined))),
            _ => out.push((key, None)),
        }
    }

    for &key in APPIMAGE_VALUE_VARS {
        if read(key).is_some_and(|v| Path::new(&v).starts_with(appdir)) {
            out.push((key, None));
        }
    }

    for &key in APPIMAGE_MARKER_VARS {
        if read(key).is_some() {
            out.push((key, None));
        }
    }

    out
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum WorkspaceEnv {
    #[default]
    Local,
    Wsl {
        distro: String,
    },
}

impl WorkspaceEnv {
    pub fn from_option(workspace: Option<Self>) -> Self {
        workspace.unwrap_or_default()
    }

    pub fn is_wsl(&self) -> bool {
        matches!(self, Self::Wsl { .. })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct WslDistro {
    pub name: String,
    pub default: bool,
    pub running: bool,
}

#[cfg(windows)]
pub fn resolve_path(path: &str, workspace: &WorkspaceEnv) -> PathBuf {
    match workspace {
        WorkspaceEnv::Local => PathBuf::from(path),
        WorkspaceEnv::Wsl { distro } => wsl_path_to_host(distro, path),
    }
}

#[cfg(not(windows))]
pub fn resolve_path(path: &str, _workspace: &WorkspaceEnv) -> PathBuf {
    PathBuf::from(path)
}

/// True for WSL distro names safe to splice into a UNC path. Real WSL distros
/// are alphanumeric with `.`, `_`, `-` separators (e.g. `Ubuntu-22.04`). Reject
/// anything that could traverse out of the `\\wsl.localhost\<distro>\` prefix
/// (`..`, `\`, `/`, `:`, `?`, `*`, control bytes) or empty names.
#[cfg(windows)]
fn is_safe_distro_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 255 {
        return false;
    }
    if name == "." || name == ".." || name.starts_with('.') {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ' '))
        && !name.contains("..")
}

#[cfg(windows)]
pub(crate) fn validate_wsl_distro_name(distro: &str) -> Result<(), String> {
    if is_safe_distro_name(distro) {
        Ok(())
    } else {
        Err(format!("unsafe WSL distro name: {distro}"))
    }
}

#[cfg(windows)]
fn wsl_drvfs_to_windows(path: &str) -> Option<PathBuf> {
    let normalized = path.replace('\\', "/");
    let rest = normalized.strip_prefix("/mnt/")?;
    let mut parts = rest.splitn(2, '/');
    let drive = parts.next()?;
    if drive.len() != 1 {
        return None;
    }
    let drive = drive.chars().next()?;
    if !drive.is_ascii_alphabetic() {
        return None;
    }
    let suffix = parts.next().unwrap_or("").replace('/', "\\");
    let mut host = format!("{}:\\", drive.to_ascii_uppercase());
    if !suffix.is_empty() {
        host.push_str(&suffix);
    }
    Some(PathBuf::from(host))
}

#[cfg(windows)]
pub fn wsl_path_to_unc(distro: &str, path: &str) -> PathBuf {
    // Defense-in-depth: refuse to construct a UNC path with a distro name that
    // could escape the WSL share root via `..`, `\`, or other path metachars.
    // Returns a clearly-invalid path that downstream `is_dir()`/`metadata()`
    // checks will reject. The webview's distro list comes from `wsl.exe --list`
    // and is normally trustworthy, but a locally-registered malicious distro
    // can name itself with traversal characters; this filter blocks that.
    if !is_safe_distro_name(distro) {
        return PathBuf::from(r"\\wsl.localhost\__tarex_invalid_distro__");
    }
    let normalized = path.replace('\\', "/");
    let trimmed = normalized.trim_start_matches('/');
    let primary = PathBuf::from(format!(
        r"\\wsl.localhost\{}\{}",
        distro,
        trimmed.replace('/', r"\")
    ));
    if primary.exists() {
        return primary;
    }
    PathBuf::from(format!(r"\\wsl$\{}\{}", distro, trimmed.replace('/', r"\")))
}

#[cfg(windows)]
pub fn wsl_path_to_host(distro: &str, path: &str) -> PathBuf {
    // `/mnt/<drive>` is drvfs-backed Windows storage. Accessing it through the
    // WSL UNC share can return "Access is denied" on Windows even though the
    // same path is readable inside WSL. Use the native drive path instead.
    wsl_drvfs_to_windows(path).unwrap_or_else(|| wsl_path_to_unc(distro, path))
}

#[cfg(windows)]
pub fn decode_command_output(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xff, 0xfe]) || looks_utf16le(bytes) {
        let start = if bytes.starts_with(&[0xff, 0xfe]) {
            2
        } else {
            0
        };
        let units: Vec<u16> = bytes[start..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

#[cfg(windows)]
fn looks_utf16le(bytes: &[u8]) -> bool {
    if bytes.len() < 4 || !bytes.len().is_multiple_of(2) {
        return false;
    }
    let nul_odd = bytes.iter().skip(1).step_by(2).filter(|b| **b == 0).count();
    nul_odd * 2 >= bytes.len() / 2
}

#[cfg(windows)]
fn run_wsl(args: &[&str]) -> Result<String, String> {
    let mut cmd = std::process::Command::new("wsl.exe");
    cmd.args(args);
    crate::modules::proc::hide_console(&mut cmd);
    let out = cmd.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        let stderr = decode_command_output(&out.stderr);
        return Err(stderr.trim().to_string());
    }
    Ok(decode_command_output(&out.stdout))
}

#[cfg(windows)]
pub(crate) fn wsl_exec_capture(
    distro: &str,
    program: &str,
    args: &[&str],
) -> Result<String, String> {
    validate_wsl_distro_name(distro)?;
    let mut cmd = std::process::Command::new("wsl.exe");
    cmd.arg("-d")
        .arg(distro)
        .arg("--exec")
        .arg(program)
        .args(args);
    crate::modules::proc::hide_console(&mut cmd);
    let out = cmd.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        let stderr = decode_command_output(&out.stderr);
        return Err(stderr.trim().to_string());
    }
    Ok(decode_command_output(&out.stdout))
}

#[cfg(windows)]
fn run_wsl_sh(distro: &str, script: &str) -> Result<String, String> {
    // Probe helpers must avoid login-shell startup files. User `.profile`
    // output on stdout would corrupt the parsed value (`$HOME`, login shell).
    wsl_exec_capture(distro, "sh", &["-c", script])
}

#[cfg(windows)]
pub(crate) fn normalize_wsl_value(output: String, fallback: &str) -> String {
    let value = output
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    if value.is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

#[cfg(windows)]
fn list_distros_blocking() -> Result<Vec<WslDistro>, String> {
    let out = run_wsl(&["--list", "--verbose"])?;
    let mut distros = Vec::new();
    for raw in out.lines().skip(1) {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let default = line.starts_with('*');
        let line = line.trim_start_matches('*').trim();
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }
        let state_idx = parts.len() - 2;
        let name = parts[..state_idx].join(" ");
        let state = parts[state_idx];
        distros.push(WslDistro {
            name,
            default,
            running: state.eq_ignore_ascii_case("Running"),
        });
    }
    Ok(distros)
}

#[tauri::command]
pub async fn wsl_list_distros() -> Result<Vec<WslDistro>, String> {
    #[cfg(not(windows))]
    {
        Ok(Vec::new())
    }
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn_blocking(list_distros_blocking)
            .await
            .map_err(|e| e.to_string())?
    }
}

#[tauri::command]
pub async fn wsl_default_distro() -> Result<Option<String>, String> {
    #[cfg(not(windows))]
    {
        Ok(None)
    }
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn_blocking(|| {
            let distros = list_distros_blocking()?;
            Ok(distros
                .iter()
                .find(|d| d.default)
                .map(|d| d.name.clone())
                .or_else(|| distros.first().map(|d| d.name.clone())))
        })
        .await
        .map_err(|e| e.to_string())?
    }
}

#[tauri::command]
pub fn wsl_home(distro: String) -> Result<String, String> {
    #[cfg(not(windows))]
    {
        let _ = distro;
        Err("WSL is only available on Windows".into())
    }
    #[cfg(windows)]
    {
        let out = run_wsl_sh(&distro, "printf %s \"$HOME\"")?;
        let home = normalize_wsl_value(out, "");
        if home.is_empty() {
            Err(format!("could not resolve WSL home for {distro}"))
        } else {
            Ok(home)
        }
    }
}

#[cfg(windows)]
pub fn wsl_login_shell(distro: String) -> Result<String, String> {
    const SCRIPT: &str = r#"uid="$(id -u 2>/dev/null || printf '')"
entry=''
if [ -n "$uid" ] && command -v getent >/dev/null 2>&1; then
  entry="$(getent passwd "$uid" 2>/dev/null || true)"
fi
if [ -z "$entry" ] && [ -n "$uid" ] && [ -r /etc/passwd ]; then
  entry="$(awk -F: -v u="$uid" '$3 == u { print; exit }' /etc/passwd 2>/dev/null)"
fi
shell=''
if [ -n "$entry" ]; then
  shell="${entry##*:}"
fi
if [ -z "$shell" ] && [ -n "$SHELL" ]; then
  shell="$SHELL"
fi
if [ -z "$shell" ]; then
  shell=/bin/sh
fi
printf %s "$shell""#;

    let out = run_wsl_sh(&distro, SCRIPT)?;
    Ok(normalize_wsl_value(out, "/bin/sh"))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn decode_command_output_reads_plain_utf8() {
        assert_eq!(decode_command_output(b"hello world"), "hello world");
    }

    #[test]
    fn decode_command_output_strips_utf16le_bom() {
        // BOM (FF FE) followed by "hi" as little-endian UTF-16.
        let bytes = [0xff, 0xfe, 0x68, 0x00, 0x69, 0x00];
        assert_eq!(decode_command_output(&bytes), "hi");
    }

    #[test]
    fn decode_command_output_detects_bomless_utf16le() {
        // "test" as little-endian UTF-16, no BOM; the odd-position NULs trip the
        // heuristic.
        let bytes = [0x74, 0x00, 0x65, 0x00, 0x73, 0x00, 0x74, 0x00];
        assert_eq!(decode_command_output(&bytes), "test");
    }

    #[test]
    fn decode_command_output_keeps_ascii_without_nuls_as_utf8() {
        assert_eq!(decode_command_output(b"C:\\Users"), "C:\\Users");
    }

    #[test]
    fn distro_validator_accepts_real_names() {
        assert!(is_safe_distro_name("Ubuntu"));
        assert!(is_safe_distro_name("Ubuntu-22.04"));
        assert!(is_safe_distro_name("Debian"));
        assert!(is_safe_distro_name("Alpine_3.18"));
        assert!(is_safe_distro_name("openSUSE-Tumbleweed"));
    }

    #[test]
    fn distro_validator_rejects_path_traversal() {
        assert!(!is_safe_distro_name(".."));
        assert!(!is_safe_distro_name("..\\..\\Windows"));
        assert!(!is_safe_distro_name("../foo"));
        assert!(!is_safe_distro_name("foo/bar"));
        assert!(!is_safe_distro_name("foo\\bar"));
        assert!(!is_safe_distro_name("foo..bar"));
    }

    #[test]
    fn distro_validator_rejects_special_chars() {
        assert!(!is_safe_distro_name("foo:bar"));
        assert!(!is_safe_distro_name("foo?bar"));
        assert!(!is_safe_distro_name("foo*bar"));
        assert!(!is_safe_distro_name("foo\0bar"));
        assert!(!is_safe_distro_name(""));
        assert!(!is_safe_distro_name(".hidden"));
    }

    #[test]
    fn wsl_path_to_unc_blocks_traversal_distro() {
        // Malicious distro name must produce a path that is_dir() will reject,
        // never escape the WSL share root.
        let p = wsl_path_to_unc("..\\..\\..\\Windows", "/etc/passwd");
        let s = p.to_string_lossy();
        assert!(s.contains("__tarex_invalid_distro__"), "got: {s}");
        assert!(!s.contains("\\..\\"), "got: {s}");
    }

    #[test]
    fn wsl_path_to_unc_accepts_valid_distro() {
        let p = wsl_path_to_unc("Ubuntu", "/etc/hosts");
        let s = p.to_string_lossy();
        assert!(!s.contains("__tarex_invalid_distro__"), "got: {s}");
    }

    #[test]
    fn resolve_path_keeps_local_paths_unchanged() {
        let path = r"C:\Users\vinicios\repo";
        assert_eq!(
            resolve_path(path, &WorkspaceEnv::Local),
            PathBuf::from(path)
        );
    }

    #[test]
    fn resolve_path_maps_wsl_paths_to_host() {
        let workspace = WorkspaceEnv::Wsl {
            distro: "Ubuntu".into(),
        };
        assert_eq!(
            resolve_path("/home/vinicios/repo", &workspace),
            wsl_path_to_host("Ubuntu", "/home/vinicios/repo")
        );
    }

    #[test]
    fn wsl_drvfs_root_maps_to_windows_drive() {
        assert_eq!(wsl_drvfs_to_windows("/mnt/c"), Some(PathBuf::from(r"C:\")));
    }

    #[test]
    fn wsl_drvfs_child_maps_to_windows_drive() {
        assert_eq!(
            wsl_drvfs_to_windows("/mnt/d/Users/vinicios/repo"),
            Some(PathBuf::from(r"D:\Users\vinicios\repo"))
        );
    }

    #[test]
    fn wsl_drvfs_rejects_non_drive_mounts() {
        assert_eq!(wsl_drvfs_to_windows("/mnt/wsl"), None);
        assert_eq!(wsl_drvfs_to_windows("/home/vinicios"), None);
    }

    #[test]
    fn normalize_wsl_value_uses_last_nonempty_line() {
        assert_eq!(
            normalize_wsl_value("banner\n  /bin/zsh \n".into(), "/bin/sh"),
            "/bin/zsh"
        );
    }

    #[test]
    fn normalize_wsl_value_falls_back_when_empty() {
        assert_eq!(normalize_wsl_value(" \n".into(), "/bin/sh"), "/bin/sh");
    }
}

#[cfg(test)]
mod auth_tests {
    use super::*;
    use std::env;
    use std::fs;

    fn tempdir(label: &str) -> PathBuf {
        let mut p = env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        p.push(format!("tarex-auth-{label}-{nanos}-{}", std::process::id()));
        fs::create_dir_all(&p).expect("create tempdir");
        fs::canonicalize(&p).expect("canonicalize tempdir")
    }

    #[test]
    fn authorize_spawn_cwd_accepts_none() {
        let reg = WorkspaceRegistry::default();
        assert!(authorize_spawn_cwd(&reg, None, &WorkspaceEnv::Local)
            .unwrap()
            .is_none());
    }

    #[test]
    fn authorize_spawn_cwd_accepts_empty_string() {
        let reg = WorkspaceRegistry::default();
        assert!(authorize_spawn_cwd(&reg, Some("   "), &WorkspaceEnv::Local)
            .unwrap()
            .is_none());
    }

    #[test]
    fn authorize_spawn_cwd_accepts_authorized_path() {
        let dir = tempdir("ok");
        let reg = WorkspaceRegistry::default();
        reg.authorize(&dir).expect("authorize root");
        let s = dir.to_string_lossy().into_owned();
        let resolved = authorize_spawn_cwd(&reg, Some(&s), &WorkspaceEnv::Local)
            .expect("authorized")
            .expect("returned canonical");
        assert_eq!(resolved, dir);
    }

    #[test]
    fn authorize_spawn_cwd_accepts_subdir_of_authorized_root() {
        let root = tempdir("subroot");
        let sub = root.join("inside");
        fs::create_dir_all(&sub).expect("subdir");
        let canonical_sub = fs::canonicalize(&sub).expect("canon sub");
        let reg = WorkspaceRegistry::default();
        reg.authorize(&root).expect("authorize root");
        let s = canonical_sub.to_string_lossy().into_owned();
        let resolved = authorize_spawn_cwd(&reg, Some(&s), &WorkspaceEnv::Local)
            .expect("subdir authorized")
            .expect("returned canonical");
        assert_eq!(resolved, canonical_sub);
    }

    #[test]
    fn authorize_spawn_cwd_rejects_unauthorized_path() {
        let allowed = tempdir("allowed");
        let foreign = tempdir("foreign");
        let reg = WorkspaceRegistry::default();
        reg.authorize(&allowed).expect("authorize root");
        let s = foreign.to_string_lossy().into_owned();
        let err = authorize_spawn_cwd(&reg, Some(&s), &WorkspaceEnv::Local)
            .expect_err("should reject unauthorized cwd");
        assert!(err.contains("outside"), "got: {err}");
    }

    #[test]
    fn authorize_spawn_cwd_rejects_missing_path() {
        let mut missing = env::temp_dir();
        missing.push(format!(
            "tarex-missing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let reg = WorkspaceRegistry::default();
        let s = missing.to_string_lossy().into_owned();
        let err = authorize_spawn_cwd(&reg, Some(&s), &WorkspaceEnv::Local)
            .expect_err("should reject missing path");
        assert!(err.contains("cwd not accessible"), "got: {err}");
    }

    #[test]
    fn authorize_user_spawn_cwd_registers_unauthorized_path() {
        let dir = tempdir("userspawn");
        let reg = WorkspaceRegistry::default();
        let s = dir.to_string_lossy().into_owned();
        assert!(!reg.is_authorized(&dir));
        let resolved = authorize_user_spawn_cwd(&reg, Some(&s), &WorkspaceEnv::Local)
            .expect("user spawn allowed anywhere")
            .expect("returned canonical");
        assert_eq!(resolved, dir);
        assert!(reg.is_authorized(&dir));
    }

    #[test]
    fn authorize_user_spawn_cwd_rejects_missing_path() {
        let mut missing = env::temp_dir();
        missing.push(format!("tarex-user-missing-{}", std::process::id()));
        let reg = WorkspaceRegistry::default();
        let s = missing.to_string_lossy().into_owned();
        let err = authorize_user_spawn_cwd(&reg, Some(&s), &WorkspaceEnv::Local)
            .expect_err("missing path must fail");
        assert!(err.contains("cwd not accessible"), "got: {err}");
    }

    #[test]
    fn user_spawn_cwd_or_home_keeps_accessible_dir() {
        let dir = tempdir("orhome-ok");
        let reg = WorkspaceRegistry::default();
        let s = dir.to_string_lossy().into_owned();
        assert_eq!(
            user_spawn_cwd_or_home(&reg, Some(&s), &WorkspaceEnv::Local),
            Some(s)
        );
        assert!(reg.is_authorized(&dir));
    }

    #[test]
    fn user_spawn_cwd_or_home_falls_back_when_inaccessible() {
        let mut missing = env::temp_dir();
        missing.push(format!("tarex-orhome-missing-{}", std::process::id()));
        let reg = WorkspaceRegistry::default();
        let s = missing.to_string_lossy().into_owned();
        assert_eq!(
            user_spawn_cwd_or_home(&reg, Some(&s), &WorkspaceEnv::Local),
            None
        );
    }

    #[test]
    fn user_spawn_cwd_or_home_passes_through_empty() {
        let reg = WorkspaceRegistry::default();
        assert_eq!(
            user_spawn_cwd_or_home(&reg, None, &WorkspaceEnv::Local),
            None
        );
        assert_eq!(
            user_spawn_cwd_or_home(&reg, Some("  "), &WorkspaceEnv::Local),
            None
        );
    }

    #[test]
    fn authorize_spawn_cwd_blocks_symlink_escape() {
        let allowed = tempdir("symroot");
        let outside = tempdir("symtarget");
        let link = allowed.join("escape");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &link).expect("symlink");
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&outside, &link).expect("symlink");
        let reg = WorkspaceRegistry::default();
        reg.authorize(&allowed).expect("authorize root");
        let s = link.to_string_lossy().into_owned();
        let err = authorize_spawn_cwd(&reg, Some(&s), &WorkspaceEnv::Local)
            .expect_err("symlink-escape must be rejected");
        assert!(err.contains("outside"), "got: {err}");
    }

    #[test]
    fn resolve_launch_cwd_prefers_cli_dir_over_env() {
        let cli = tempdir("cli");
        let env = tempdir("env");
        let s = cli.to_string_lossy().into_owned();
        let resolved = resolve_launch_cwd(Some(&s), Some(env.clone()));
        assert_eq!(resolved.as_deref(), Some(cli.as_path()));
    }

    #[test]
    fn resolve_launch_cwd_falls_back_to_env_when_cli_missing() {
        let env = tempdir("envonly");
        assert_eq!(resolve_launch_cwd(None, Some(env.clone())), Some(env));
    }

    #[test]
    fn resolve_launch_cwd_ignores_nonexistent_cli_dir() {
        let env = tempdir("envfb");
        let resolved = resolve_launch_cwd(Some("/no/such/tarex/dir"), Some(env.clone()));
        assert_eq!(resolved, Some(env));
    }
}

#[cfg(all(test, target_os = "linux"))]
mod appimage_tests {
    use super::*;
    use std::collections::HashMap;

    fn reader(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: HashMap<String, OsString> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        move |k: &str| map.get(k).cloned()
    }

    fn find<'a>(
        out: &'a [(&'static str, Option<OsString>)],
        key: &str,
    ) -> Option<&'a Option<OsString>> {
        out.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
    }

    #[test]
    fn strips_appdir_from_path_lists_and_unsets_when_empty() {
        let appdir = Path::new("/tmp/.mount_Tarex_X");
        let env = reader(&[
            ("LD_LIBRARY_PATH", "/tmp/.mount_Tarex_X/usr/lib:/usr/lib"),
            ("PATH", "/tmp/.mount_Tarex_X/usr/bin:/usr/bin:/bin"),
            ("GST_PLUGIN_SYSTEM_PATH", "/tmp/.mount_Tarex_X/usr/lib/gstreamer-1.0"),
            ("APPDIR", "/tmp/.mount_Tarex_X"),
        ]);
        let out = compute_appimage_env_overrides(appdir, env);

        assert_eq!(find(&out, "LD_LIBRARY_PATH"), Some(&Some(OsString::from("/usr/lib"))));
        assert_eq!(find(&out, "PATH"), Some(&Some(OsString::from("/usr/bin:/bin"))));
        // Only an APPDIR entry, so the var is removed entirely.
        assert_eq!(find(&out, "GST_PLUGIN_SYSTEM_PATH"), Some(&None));
        assert_eq!(find(&out, "APPDIR"), Some(&None));
    }

    #[test]
    fn leaves_untouched_vars_alone() {
        let appdir = Path::new("/tmp/.mount_Tarex_X");
        let env = reader(&[
            ("LD_LIBRARY_PATH", "/usr/lib:/usr/local/lib"),
            ("LD_PRELOAD", "/home/u/my.so"),
        ]);
        let out = compute_appimage_env_overrides(appdir, env);

        // No APPDIR component => no override emitted for these.
        assert!(find(&out, "LD_LIBRARY_PATH").is_none());
        assert!(find(&out, "LD_PRELOAD").is_none());
    }

    #[test]
    fn unsets_value_vars_only_when_pointing_into_appdir() {
        let appdir = Path::new("/tmp/.mount_Tarex_X");
        let into = reader(&[("LD_PRELOAD", "/tmp/.mount_Tarex_X/usr/lib/x.so")]);
        assert_eq!(find(&compute_appimage_env_overrides(appdir, into), "LD_PRELOAD"), Some(&None));

        let outside = reader(&[("FONTCONFIG_FILE", "/etc/fonts/fonts.conf")]);
        assert!(find(&compute_appimage_env_overrides(appdir, outside), "FONTCONFIG_FILE").is_none());
    }
}

#[cfg(test)]
mod authorization_tests {
    use super::*;

    fn authorized_registry(root: &Path) -> WorkspaceRegistry {
        let registry = WorkspaceRegistry::default();
        registry.authorize(root).expect("authorize fixture root");
        registry
    }

    #[test]
    fn accepts_paths_inside_an_authorized_root() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, b"x").unwrap();
        let registry = authorized_registry(dir.path());

        assert!(ensure_authorized(&registry, &file).is_ok());
    }

    #[test]
    fn rejects_paths_outside_every_authorized_root() {
        let allowed = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.txt");
        std::fs::write(&secret, b"sssh").unwrap();
        let registry = authorized_registry(allowed.path());

        let error = ensure_authorized(&registry, &secret).unwrap_err();
        assert!(
            error.contains("outside the authorized workspace"),
            "got: {error}"
        );
    }

    #[test]
    fn accepts_not_yet_created_targets_under_an_authorized_root() {
        let dir = tempfile::tempdir().unwrap();
        let registry = authorized_registry(dir.path());

        // The file itself does not exist: authorization walks to the parent.
        let target = dir.path().join("new/nested/file.txt");
        assert!(ensure_authorized(&registry, &target).is_ok());
    }

    #[test]
    fn rejects_not_yet_created_targets_outside_the_workspace() {
        let allowed = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let registry = authorized_registry(allowed.path());

        let target = outside.path().join("planted.txt");
        let error = ensure_authorized(&registry, &target).unwrap_err();
        assert!(
            error.contains("outside the authorized workspace"),
            "got: {error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_that_escape_the_workspace() {
        use std::os::unix::fs::symlink;
        let allowed = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.txt");
        std::fs::write(&secret, b"sssh").unwrap();

        let link = allowed.path().join("innocent.txt");
        symlink(&secret, &link).unwrap();
        let registry = authorized_registry(allowed.path());

        // The link lives inside the workspace but resolves outside it.
        let error = ensure_authorized(&registry, &link).unwrap_err();
        assert!(
            error.contains("outside the authorized workspace"),
            "got: {error}"
        );
    }

    #[test]
    fn accepts_nested_roots_and_rejects_siblings() {
        let parent = tempfile::tempdir().unwrap();
        let child = parent.path().join("project");
        std::fs::create_dir(&child).unwrap();
        let sibling = parent.path().join("other");
        std::fs::create_dir(&sibling).unwrap();

        // Only the nested project is authorized, not its parent.
        let registry = authorized_registry(&child);

        assert!(ensure_authorized(&registry, &child.join("src/main.rs")).is_ok());
        // Sibling under the same parent must still be rejected.
        assert!(ensure_authorized(&registry, &sibling.join("x.txt")).is_err());
        // The parent itself is not authorized either.
        assert!(ensure_authorized(&registry, parent.path()).is_err());
    }

    #[test]
    fn does_not_treat_a_prefix_sibling_as_inside_the_root() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("proj");
        std::fs::create_dir(&root).unwrap();
        // `proj-evil` shares the string prefix of `proj` but is not inside it.
        let prefix_sibling = parent.path().join("proj-evil");
        std::fs::create_dir(&prefix_sibling).unwrap();

        let registry = authorized_registry(&root);
        assert!(ensure_authorized(&registry, &prefix_sibling.join("f.txt")).is_err());
    }
}

#[cfg(test)]
mod sensitive_path_tests {
    use super::*;

    /// An authorized root must not become a blanket grant: the native
    /// sensitive-path rules apply on top of the workspace check.
    #[test]
    fn authorized_root_still_refuses_protected_paths() {
        let home = tempfile::tempdir().unwrap();
        let ssh = home.path().join(".ssh");
        std::fs::create_dir(&ssh).unwrap();
        let key = ssh.join("id_rsa");
        std::fs::write(&key, b"PRIVATE").unwrap();

        let registry = WorkspaceRegistry::default();
        let home = registry.authorize(home.path()).unwrap();
        let key = std::fs::canonicalize(&key).unwrap();

        // The workspace check alone would allow this...
        assert!(registry.is_authorized(&key));
        // ...but the combined guard refuses it.
        let error = ensure_authorized(&registry, &key).unwrap_err();
        assert!(
            error.contains("protected directory") || error.contains("sensitive-file"),
            "expected sensitive-path refusal, got: {error}"
        );

        // Ordinary files under the same root stay readable.
        let normal = home.join("notes.txt");
        std::fs::write(&normal, b"hi").unwrap();
        assert!(ensure_authorized(&registry, &normal).is_ok());
    }

    #[test]
    fn write_tier_refuses_system_dirs_even_when_authorized() {
        let root = std::path::Path::new("/usr/bin");
        if !root.is_dir() {
            return;
        }
        let registry = WorkspaceRegistry::default();
        // Even an explicit grant over /usr/bin must not permit writes.
        registry.authorize(root).expect("authorize /usr/bin");
        let error = ensure_writable(&registry, &root.join("planted")).unwrap_err();
        assert!(
            error.contains("not allowed") || error.contains("protected"),
            "got: {error}"
        );
        // The exact directory itself is covered too, not just its children.
        let error = ensure_writable(&registry, root).unwrap_err();
        assert!(
            error.contains("not allowed") || error.contains("protected"),
            "got: {error}"
        );
    }
}

#[cfg(test)]
mod picked_path_tests {
    use super::*;

    fn workspace_with_secret() -> (tempfile::TempDir, WorkspaceRegistry, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let env = root.path().join(".env");
        std::fs::write(&env, b"SECRET=1").unwrap();
        let registry = WorkspaceRegistry::default();
        registry.authorize(root.path()).expect("authorize fixture root");
        (root, registry, env)
    }

    /// A native "Open" pick is a read grant only: it lifts the sensitive-path
    /// refusal without turning into a write grant.
    #[test]
    fn open_pick_unblocks_reads_but_not_writes() {
        let (_root, registry, env) = workspace_with_secret();

        let error = ensure_authorized(&registry, &env).unwrap_err();
        assert!(
            error.contains("protected directory") || error.contains("sensitive-file"),
            "expected sensitive-path refusal, got: {error}"
        );

        registry.grant_picked(&env, false).expect("grant picked");

        assert!(ensure_authorized(&registry, &env).is_ok());
        let error = ensure_writable(&registry, &env).unwrap_err();
        assert!(
            error.contains("not allowed")
                || error.contains("protected")
                || error.contains("sensitive-file"),
            "a read-only pick must not permit writes, got: {error}"
        );
    }

    /// A native "Save as" pick is a read+write grant for that destination.
    #[test]
    fn save_pick_unblocks_writes() {
        let (_root, registry, env) = workspace_with_secret();

        registry.grant_picked(&env, true).expect("grant picked");

        assert!(ensure_writable(&registry, &env).is_ok());
        assert!(ensure_authorized(&registry, &env).is_ok());
    }

    /// Outside the workspace the `writable` flag is what separates an "Open"
    /// pick from a "Save as" pick, so the refusal comes from the grant itself.
    #[test]
    fn open_pick_does_not_grant_writes_outside_the_workspace() {
        let outside = tempfile::tempdir().unwrap();
        let file = outside.path().join("notes.txt");
        std::fs::write(&file, b"hi").unwrap();

        let registry = WorkspaceRegistry::default();
        registry.grant_picked(&file, false).expect("grant picked");

        assert!(ensure_authorized(&registry, &file).is_ok());
        let error = ensure_writable(&registry, &file).unwrap_err();
        assert!(
            error.contains("outside the authorized workspace"),
            "got: {error}"
        );
    }

    /// The dialog grant is the only way past the workspace root, and it covers
    /// the chosen file — not the directory that happens to contain it.
    #[test]
    fn pick_reaches_outside_the_workspace_but_not_its_directory() {
        let outside = tempfile::tempdir().unwrap();
        let file = outside.path().join("notes.txt");
        std::fs::write(&file, b"hi").unwrap();

        let registry = WorkspaceRegistry::default();
        let error = ensure_authorized(&registry, &file).unwrap_err();
        assert!(
            error.contains("outside the authorized workspace"),
            "got: {error}"
        );

        registry.grant_picked(&file, false).expect("grant picked");

        assert!(ensure_authorized(&registry, &file).is_ok());
        assert!(ensure_authorized(&registry, outside.path()).is_err());
    }

    /// `starts_with` compares whole components, so `secret.txt` must not also
    /// admit `secret.txt2`.
    #[test]
    fn grant_does_not_leak_to_a_similarly_named_sibling() {
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("secret.txt");
        let sibling = outside.path().join("secret.txt2");
        std::fs::write(&target, b"a").unwrap();
        std::fs::write(&sibling, b"b").unwrap();

        let registry = WorkspaceRegistry::default();
        registry.grant_picked(&target, false).expect("grant picked");

        assert!(ensure_authorized(&registry, &target).is_ok());
        assert!(ensure_authorized(&registry, &sibling).is_err());
    }

    /// A grant made inside one workspace must not survive into another check
    /// against a path that was never picked.
    #[test]
    fn grants_are_scoped_to_the_paths_they_name() {
        let (_root, registry, env) = workspace_with_secret();
        let other = env.parent().expect("secret has a parent").join(".env.local");
        std::fs::write(&other, b"OTHER=1").unwrap();

        registry.grant_picked(&env, false).expect("grant picked");

        assert!(ensure_authorized(&registry, &env).is_ok());
        // `.env.local` is still a secret basename and was never picked.
        assert!(ensure_authorized(&registry, &other).is_err());
    }
}

#[cfg(test)]
mod asset_grant_tests {
    use super::*;
    use std::sync::Mutex;

    fn recorder() -> (WorkspaceRegistry, Arc<Mutex<Vec<PathBuf>>>) {
        let registry = WorkspaceRegistry::default();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        registry.set_asset_grant(move |p| {
            sink.lock().expect("asset grant sink").push(p.to_path_buf());
        });
        (registry, seen)
    }

    #[test]
    fn authorize_mirrors_into_the_asset_grant() {
        let (registry, seen) = recorder();
        let dir = std::env::temp_dir();
        registry.authorize(&dir).expect("authorize");

        let seen = seen.lock().expect("asset grant sink");
        assert_eq!(seen.len(), 1, "the first authorization must be mirrored");
        assert_eq!(seen[0], dir.canonicalize().expect("canonical temp"));
    }

    #[test]
    fn replay_covers_paths_authorized_before_the_hook_existed() {
        // Bootstrap runs while the builder is still constructing, so roots can
        // be authorized before setup installs the hook.
        let registry = WorkspaceRegistry::default();
        let dir = std::env::temp_dir();
        registry.authorize(&dir).expect("authorize");

        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        registry.set_asset_grant(move |p| {
            sink.lock().expect("asset grant sink").push(p.to_path_buf());
        });
        assert!(
            seen.lock().expect("asset grant sink").is_empty(),
            "nothing is mirrored until the hook is installed"
        );

        registry.replay_asset_grants();
        assert_eq!(
            seen.lock().expect("asset grant sink").len(),
            1,
            "replay must cover the pre-existing root"
        );
    }

    #[test]
    fn picked_paths_are_mirrored_too() {
        let (registry, seen) = recorder();
        let file = std::env::temp_dir().join(format!(
            "tarex-asset-grant-{}.txt",
            std::process::id()
        ));
        std::fs::write(&file, b"x").unwrap();

        registry.grant_picked(&file, false).expect("grant picked");
        {
            let seen = seen.lock().expect("asset grant sink");
            assert_eq!(seen.len(), 1);
            assert_eq!(seen[0], file.canonicalize().expect("canonical file"));
        }
        std::fs::remove_file(&file).unwrap();
    }

    #[test]
    fn no_hook_is_not_an_error() {
        let registry = WorkspaceRegistry::default();
        registry.authorize(std::env::temp_dir()).expect("authorize");
        registry.replay_asset_grants();
    }
}
