//! Fresh PATH resolution for spawned processes.
//!
//! The process environment is a snapshot taken when Tarex starts, so PATH
//! entries added to the user registry afterwards (e.g. a freshly installed
//! CLI) resolve in Windows Terminal but not in processes we spawn here. On
//! Windows the registry-merged PATH (HKLM then HKCU) is re-read on every
//! call; on other platforms the process PATH is returned unchanged.

use std::ffi::OsString;

/// PATH that freshly spawned processes should receive.
///
/// Windows: HKLM + HKCU registry PATH merged as `system;user`, `%VAR%`
/// expanded against the process environment, deduplicated case-insensitively
/// while preserving order. Falls back to the process PATH when the registry
/// cannot be read. The registry is read on every call so PATH edits made
/// after the app started are picked up.
pub fn effective_path() -> OsString {
    #[cfg(windows)]
    {
        effective_from(
            imp::registry_path_pairs(),
            process_path(),
            &imp::process_env_lookup,
        )
    }
    #[cfg(not(windows))]
    {
        process_path()
    }
}

fn process_path() -> OsString {
    std::env::var_os("PATH").unwrap_or_default()
}

/// Combine a registry read result with a fallback PATH. A failed registry
/// read (`None`) and a degenerate empty merge both yield `fallback`.
#[cfg(any(windows, test))]
fn effective_from(
    registry: Option<(String, String)>,
    fallback: OsString,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> OsString {
    match registry {
        Some((system, user)) => {
            let merged = merge_paths(&system, &user, lookup);
            if merged.is_empty() {
                fallback
            } else {
                OsString::from(merged)
            }
        }
        None => fallback,
    }
}

/// Merge Windows `system;user` PATH strings (both `;` separated): expand
/// `%VAR%` references via `lookup`, drop empty segments (they would mean the
/// current directory), and drop case-insensitive duplicates keeping the first
/// occurrence, preserving order.
#[cfg(any(windows, test))]
fn merge_paths(system: &str, user: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    let system = expand_env_vars(system, lookup);
    let user = expand_env_vars(user, lookup);
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<&str> = Vec::new();
    for entry in system.split(';').chain(user.split(';')) {
        if entry.is_empty() {
            continue;
        }
        if seen.insert(entry.to_lowercase()) {
            out.push(entry);
        }
    }
    out.join(";")
}

/// Replace `%NAME%` occurrences using `lookup`. Unknown names and unbalanced
/// `%` are preserved verbatim; expansion is a single left-to-right pass over
/// the input (replacement text is not rescanned), matching how Windows
/// expands `REG_EXPAND_SZ` values.
#[cfg(any(windows, test))]
fn expand_env_vars(input: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(open) = rest.find('%') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('%') {
            Some(close) => {
                let name = &after[..close];
                rest = &after[close + 1..];
                if name.is_empty() {
                    out.push_str("%%");
                } else {
                    match lookup(name) {
                        Some(value) => out.push_str(&value),
                        None => {
                            out.push('%');
                            out.push_str(name);
                            out.push('%');
                        }
                    }
                }
            }
            None => {
                out.push_str(&rest[open..]);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::{
        ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS,
    };
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
        KEY_READ, REG_EXPAND_SZ, REG_SZ,
    };

    const SYSTEM_PATH_KEY: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
    const USER_PATH_KEY: &str = "Environment";
    /// Guard against a corrupt registry value asking for an absurd allocation.
    const MAX_VALUE_BYTES: u32 = 64 * 1024;

    enum ReadOutcome {
        Value(String),
        Missing,
        Failed,
    }

    /// `(system, user)` raw PATH strings read from the registry, or `None`
    /// when either hive cannot be read reliably (the caller then falls back
    /// to the process PATH). A missing user key/value counts as an empty
    /// user PATH; a missing machine PATH counts as a read failure because a
    /// merge without it would silently drop system entries.
    pub(super) fn registry_path_pairs() -> Option<(String, String)> {
        let system = match read_value(HKEY_LOCAL_MACHINE, SYSTEM_PATH_KEY, "Path") {
            ReadOutcome::Value(value) => value,
            ReadOutcome::Missing | ReadOutcome::Failed => return None,
        };
        let user = match read_value(HKEY_CURRENT_USER, USER_PATH_KEY, "Path") {
            ReadOutcome::Value(value) => value,
            ReadOutcome::Missing => String::new(),
            ReadOutcome::Failed => return None,
        };
        Some((system, user))
    }

    pub(super) fn process_env_lookup(name: &str) -> Option<String> {
        std::env::var_os(name).map(|value| value.to_string_lossy().into_owned())
    }

    fn read_value(root: HKEY, subkey: &str, value_name: &str) -> ReadOutcome {
        let subkey: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
        let mut key: HKEY = std::ptr::null_mut();
        let opened = unsafe { RegOpenKeyExW(root, subkey.as_ptr(), 0, KEY_READ, &mut key) };
        if opened != ERROR_SUCCESS {
            return match opened {
                ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => ReadOutcome::Missing,
                _ => ReadOutcome::Failed,
            };
        }
        let outcome = query_value(key, value_name);
        unsafe {
            RegCloseKey(key);
        }
        outcome
    }

    fn query_value(key: HKEY, value_name: &str) -> ReadOutcome {
        let name: Vec<u16> = value_name
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut kind: u32 = 0;
        let mut size: u32 = 0;
        let probe = unsafe {
            RegQueryValueExW(
                key,
                name.as_ptr(),
                std::ptr::null(),
                &mut kind,
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if probe != ERROR_SUCCESS {
            return match probe {
                ERROR_FILE_NOT_FOUND => ReadOutcome::Missing,
                _ => ReadOutcome::Failed,
            };
        }
        if kind != REG_SZ && kind != REG_EXPAND_SZ {
            return ReadOutcome::Failed;
        }
        if size > MAX_VALUE_BYTES {
            return ReadOutcome::Failed;
        }
        // Two spare bytes so a value without a trailing NUL still decodes;
        // the decoder below stops at the first NUL unit anyway.
        let mut buf = vec![0u8; size as usize + 2];
        let mut got = size;
        let status = unsafe {
            RegQueryValueExW(
                key,
                name.as_ptr(),
                std::ptr::null(),
                &mut kind,
                buf.as_mut_ptr(),
                &mut got,
            )
        };
        if status != ERROR_SUCCESS {
            return ReadOutcome::Failed;
        }
        let units: Vec<u16> = buf
            .chunks_exact(2)
            .take(got as usize / 2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let end = units
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(units.len());
        ReadOutcome::Value(String::from_utf16_lossy(&units[..end]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_string())
        }
    }

    #[test]
    fn merges_system_before_user_and_dedupes_case_insensitively() {
        let system = r"C:\Windows\System32;C:\Tools";
        let user = r"c:\windows\system32;C:\Users\me\bin";
        let merged = merge_paths(system, user, &lookup(&[]));
        assert_eq!(merged, r"C:\Windows\System32;C:\Tools;C:\Users\me\bin");
    }

    #[test]
    fn expands_known_variables_in_both_hives() {
        let merged = merge_paths(
            r"%SystemRoot%\System32",
            r"%UserProfile%\bin",
            &lookup(&[
                ("SystemRoot", r"C:\Windows"),
                ("UserProfile", r"C:\Users\me"),
            ]),
        );
        assert_eq!(merged, r"C:\Windows\System32;C:\Users\me\bin");
    }

    #[test]
    fn leaves_unknown_and_unbalanced_variables_verbatim() {
        let merged = merge_paths(r"%TAREX_MISSING%\bin", "keep%odd", &lookup(&[]));
        assert_eq!(merged, r"%TAREX_MISSING%\bin;keep%odd");
    }

    #[test]
    fn expanded_duplicates_collapse_to_first_occurrence() {
        let merged = merge_paths(
            r"%ROOT%\one",
            r"%ROOT%\one;%ROOT%\two",
            &lookup(&[("ROOT", r"C:\Base")]),
        );
        assert_eq!(merged, r"C:\Base\one;C:\Base\two");
    }

    #[test]
    fn drops_empty_segments_and_trailing_separators() {
        let merged = merge_paths(r"C:\A;;C:\B", "", &lookup(&[]));
        assert_eq!(merged, r"C:\A;C:\B");
    }

    #[test]
    fn registry_unavailable_falls_back_to_process_path() {
        let fallback = OsString::from(r"C:\Stale\Path");
        let effective = effective_from(None, fallback.clone(), &lookup(&[]));
        assert_eq!(effective, fallback);
    }

    #[test]
    fn empty_registry_merge_falls_back_to_process_path() {
        let fallback = OsString::from(r"C:\Stale\Path");
        let effective = effective_from(
            Some((String::new(), String::new())),
            fallback.clone(),
            &lookup(&[]),
        );
        assert_eq!(effective, fallback);
    }

    #[test]
    fn registry_merge_wins_over_process_fallback() {
        let effective = effective_from(
            Some((r"C:\Sys".into(), r"C:\User".into())),
            OsString::from(r"C:\Stale"),
            &lookup(&[]),
        );
        assert_eq!(effective, OsString::from(r"C:\Sys;C:\User"));
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_returns_process_path_unchanged() {
        assert_eq!(
            effective_path(),
            std::env::var_os("PATH").unwrap_or_default()
        );
    }

    // Runs on Windows CI only (not on the macOS/Linux dev host): exercises
    // the real registry read end to end.
    #[cfg(windows)]
    #[test]
    fn registry_read_round_trips_into_effective_path() {
        let (system, user) =
            imp::registry_path_pairs().expect("registry PATH should be readable on Windows");
        assert!(!system.is_empty(), "machine PATH must not be empty");

        let effective = effective_path();
        assert!(!effective.is_empty(), "effective PATH must not be empty");
        let merged = merge_paths(&system, &user, &imp::process_env_lookup);
        if !merged.is_empty() {
            assert_eq!(effective.to_string_lossy(), merged.as_str());
        }

        let effective_entries: Vec<String> = effective
            .to_string_lossy()
            .split(';')
            .map(|entry| entry.to_lowercase())
            .collect();
        for entry in system.split(';').filter(|entry| !entry.is_empty()) {
            let expanded = expand_env_vars(entry, &imp::process_env_lookup);
            assert!(
                effective_entries.contains(&expanded.to_lowercase()),
                "machine PATH entry {expanded:?} missing from effective PATH"
            );
        }
    }
}
