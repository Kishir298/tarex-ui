//! Native OS file dialogs.
//!
//! The renderer never opens a dialog itself — Rust does, and records the path
//! the user chose as a session grant (see
//! [`WorkspaceRegistry::grant_picked`](crate::modules::workspace::WorkspaceRegistry::grant_picked)).
//! That grant is the only way a path outside the authorized roots or behind
//! the sensitive-path rules becomes reachable, and obtaining one always
//! requires the user to confirm the choice in the OS.

use std::path::{Path, PathBuf};

use tauri_plugin_dialog::{DialogExt, FilePath};

use crate::modules::workspace::WorkspaceRegistry;

fn into_path(entry: FilePath) -> Result<PathBuf, String> {
    match entry {
        FilePath::Path(path) => Ok(path),
        FilePath::Url(url) => url
            .to_file_path()
            .map_err(|_| "the dialog returned a non-file URL".to_string()),
    }
}

/// Opens the OS file picker and returns the files the user chose, each granted
/// for reading for the rest of the session.
///
/// Returns an empty list when the dialog is dismissed. Only regular files are
/// granted: a directory would otherwise hand out its entire tree.
#[tauri::command]
pub async fn dialog_open_files(
    app: tauri::AppHandle,
    registry: tauri::State<'_, WorkspaceRegistry>,
    title: Option<String>,
) -> Result<Vec<String>, String> {
    let mut dialog = app.dialog().file();
    if let Some(title) = title.filter(|t| !t.trim().is_empty()) {
        dialog = dialog.set_title(title);
    }

    let picked = dialog.blocking_pick_files().unwrap_or_default();

    let mut granted = Vec::with_capacity(picked.len());
    for entry in picked {
        let path = into_path(entry)?;
        if !path.is_file() {
            continue;
        }
        let canonical = registry.grant_picked(&path, false)?;
        granted.push(canonical.to_string_lossy().into_owned());
    }
    Ok(granted)
}

/// Opens the OS save dialog and returns the destination the user confirmed,
/// granted for reading and writing for the rest of the session.
///
/// Returns `None` when the dialog is dismissed. The destination does not have
/// to exist yet, which is the point — it is authorized through its nearest
/// existing ancestor plus the exact name the user typed.
#[tauri::command]
pub async fn dialog_save_as(
    app: tauri::AppHandle,
    registry: tauri::State<'_, WorkspaceRegistry>,
    title: Option<String>,
    directory: Option<String>,
    file_name: Option<String>,
) -> Result<Option<String>, String> {
    let mut dialog = app.dialog().file();
    if let Some(title) = title.filter(|t| !t.trim().is_empty()) {
        dialog = dialog.set_title(title);
    }
    if let Some(directory) = directory.filter(|t| !t.trim().is_empty()) {
        dialog = dialog.set_directory(directory);
    }
    if let Some(file_name) = file_name.filter(|t| !t.trim().is_empty()) {
        dialog = dialog.set_file_name(file_name);
    }

    let Some(entry) = dialog.blocking_save_file() else {
        return Ok(None);
    };

    let path = into_path(entry)?;
    if path.is_dir() {
        return Err("the chosen destination is a directory".to_string());
    }
    let canonical = registry.grant_picked(&path, true)?;
    Ok(Some(canonical.to_string_lossy().into_owned()))
}

/// Extensions whose default OS handler *runs* the target rather than showing
/// it. The renderer picks these paths, so launching one would be arbitrary
/// code execution with the user's privileges.
const EXECUTABLE_EXTENSIONS: &[&str] = &[
    // macOS
    "app", "appex", "command", "tool", "pkg", "dmg", "dylib", "workflow",
    // Windows
    "exe", "bat", "cmd", "com", "msi", "msp", "msix", "scr", "pif", "vbs",
    "vbe", "wsf", "hta", "ps1", "psm1", "lnk", "reg", "jar", "msc", "cpl",
    // Linux
    "bin", "run", "deb", "rpm", "appimage", "sh", "bash", "zsh", "desktop",
];

/// `true` when the default handler for `path` would execute it instead of
/// displaying it.
fn is_executable_payload(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    // macOS bundles are directories named `*.app`; their handler launches them.
    if name.ends_with(".app") || name.ends_with(".appex") {
        return true;
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if EXECUTABLE_EXTENSIONS.contains(&ext.as_str()) {
        return true;
    }
    // Unix binaries carry no extension — the executable bit is the only signal.
    if ext.is_empty() {
        return has_executable_bit(path);
    }
    false
}

#[cfg(unix)]
fn has_executable_bit(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn has_executable_bit(_path: &Path) -> bool {
    false
}

/// Hands `path` to the OS default application.
///
/// This replaces the renderer-side `opener:allow-open-path` capability, which
/// was scoped to `**` and therefore let any renderer content launch any file.
/// The path must already be reachable through the path registry, and payloads
/// the OS would execute are refused outright — the same treatment the file
/// dialogs get.
#[tauri::command]
pub async fn open_path(
    path: String,
    workspace: Option<crate::modules::workspace::WorkspaceEnv>,
    registry: tauri::State<'_, WorkspaceRegistry>,
) -> Result<(), String> {
    let workspace = crate::modules::workspace::WorkspaceEnv::from_option(workspace);
    let target = crate::modules::workspace::ensure_authorized_path(
        &registry,
        &crate::modules::workspace::resolve_path(&path, &workspace),
    )?;
    if is_executable_payload(&target) {
        return Err(format!(
            "Refused: opening \"{}\" would execute it rather than display it.",
            target.display()
        ));
    }
    tauri_plugin_opener::open_path(&target, None::<&str>).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn executable_payloads_are_detected_by_extension() {
        for name in [
            "Evil.app",
            "setup.exe",
            "run.bat",
            "Installer.msi",
            "link.lnk",
            "merge.reg",
            "script.sh",
            "Thing.desktop",
            "lib.dylib",
        ] {
            let path = Path::new("/tmp").join(name);
            assert!(is_executable_payload(&path), "{name} must be refused");
        }
    }

    #[test]
    fn ordinary_documents_are_openable() {
        for name in ["notes.txt", "photo.png", "doc.pdf", "data.json", "a.md"] {
            let path = Path::new("/tmp").join(name);
            assert!(!is_executable_payload(&path), "{name} must be allowed");
        }
    }

    #[cfg(unix)]
    #[test]
    fn extensionless_executables_are_refused_but_regular_files_are_not() {
        let dir = std::env::temp_dir().join(format!("tarex-open-path-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let binary = dir.join("mytool");
        fs::write(&binary, b"\x7fELF").unwrap();
        let mut perms = fs::metadata(&binary).unwrap().permissions();
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
        fs::set_permissions(&binary, perms).unwrap();
        assert!(is_executable_payload(&binary));

        let plain = dir.join("readme");
        fs::write(&plain, b"hello").unwrap();
        assert!(!is_executable_payload(&plain));

        fs::remove_dir_all(&dir).unwrap();
    }
}
