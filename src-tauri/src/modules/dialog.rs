//! Native OS file dialogs.
//!
//! The renderer never opens a dialog itself — Rust does, and records the path
//! the user chose as a session grant (see
//! [`WorkspaceRegistry::grant_picked`](crate::modules::workspace::WorkspaceRegistry::grant_picked)).
//! That grant is the only way a path outside the authorized roots or behind
//! the sensitive-path rules becomes reachable, and obtaining one always
//! requires the user to confirm the choice in the OS.

use std::path::PathBuf;

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
