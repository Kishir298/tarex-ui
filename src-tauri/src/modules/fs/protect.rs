//! Native sensitive-path rules.
//!
//! The renderer carries an equivalent denylist in
//! `src/modules/ai/lib/security.ts`, but a renderer check is not a security
//! boundary: any direct `invoke` bypasses it. These rules are enforced in the
//! Tauri command layer so the same paths are refused regardless of what the
//! webview asks for.
//!
//! Two tiers, mirroring the frontend:
//! - **read**: secret-shaped basenames and directories that hold credentials,
//!   keys or host process state.
//! - **write**: everything in `read`, plus system directories where automated
//!   mutation is dangerous.
//!
//! Paths are compared on a normalized surface (forward slashes, Windows drive
//! and NTFS stream syntax stripped, trailing dots/spaces removed, lowercased)
//! so case-variants and `\\?\`-prefixed canonical paths match on
//! case-insensitive filesystems.

use std::path::Path;

/// Directories refused for both reads and writes. Matched as exact path or
/// descendant — never a raw substring, so `/.sshx` does not match `/.ssh`.
const PROTECTED_DIRS: &[&str] = &[
    "/.ssh",
    "/.gnupg",
    "/.aws",
    "/.azure",
    "/.kube",
    "/.docker",
    "/.config/gh",
    "/.config/git",
    "/.config/gcloud",
    "/.config/op", // 1Password CLI
    "/.git",       // git internals — refusing avoids tools mutating refs/objects
    "/.terraform.d",
    "/library/keychains",
    "/library/cookies",
    // System dirs holding host secrets/PII/process state. Per-PID files under
    // /proc leak env vars and command lines from other processes; /sys exposes
    // kernel state and hardware identifiers. /etc and /private/etc hold global
    // config that frequently contains credentials in basenames the regex won't
    // match (passwd, shadow, master.passwd, *.cnf, *.conf with creds).
    "/etc",
    "/private/etc",
    "/proc",
    "/sys",
    "/var/db",
    "/var/root",
    "/private/var/db",
    "/private/var/root",
    // Windows user profile equivalents (post drive-strip + lowercase).
    "/appdata/roaming/microsoft/credentials",
    "/appdata/local/microsoft/credentials",
    "/appdata/roaming/gcloud",
];

/// Files only Rust may write. They decide what the app will execute or which
/// credentials it holds, and the app data directory normally sits inside an
/// authorized home root — so a renderer `fs_write_file` must not reach them.
const RUST_OWNED_BASENAMES: &[&str] = &["lsp-allowed-servers.json", "secrets.json"];

/// Write-only deny prefixes (system locations). Read access is *not*
/// universally blocked — reading `/etc/hosts` is fine; writing to it isn't.
const WRITE_DENY_PREFIXES: &[&str] = &[
    "/etc/",
    "/var/db/",
    "/var/root/",
    "/system/",
    "/library/keychains/",
    "/library/launchagents/",
    "/library/launchdaemons/",
    "/private/etc/",
    "/private/var/db/",
    "/usr/bin/",
    "/usr/sbin/",
    "/usr/local/bin/",
    "/bin/",
    "/sbin/",
    "/boot/",
    // Windows (post drive-strip + lowercase). Drives are stripped, so any
    // /windows/... matches regardless of drive letter.
    "/windows/",
    "/program files/",
    "/program files (x86)/",
    "/programdata/",
];

/// Secret-shaped basenames. `(?:[.\s:]|$)` keeps `foo.pem.bak` and the Windows
/// `foo.pem:stream` / trailing-dot forms matching while leaving `foo.pemx`
/// alone.
fn secret_basename_patterns() -> &'static [regex::Regex] {
    use std::sync::OnceLock;
    static PATTERNS: OnceLock<Vec<regex::Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        const PATTERNS: &[&str] = &[
            // `.env` and `.env.<suffix>` with no required tail anchor — Windows
            // strips trailing dots/spaces at open time and NTFS exposes alternate
            // data streams via `name:stream`.
            r"(?i)^\.env(\..+)?(?:[.\s:]|$)",
            r"(?i)^.*\.pem(?:[.\s:]|$)",
            r"(?i)^.*\.key(?:[.\s:]|$)",
            r"(?i)^.*\.p12(?:[.\s:]|$)",
            r"(?i)^.*\.pfx(?:[.\s:]|$)",
            r"(?i)^.*\.asc(?:[.\s:]|$)",
            r"(?i)^.*\.gpg(?:[.\s:]|$)",
            r"(?i)^.*\.keystore(?:[.\s:]|$)",
            r"(?i)^.*\.jks(?:[.\s:]|$)",
            // id_rsa, id_rsa.pub and backups like id_rsa.bak / id_rsa_old.
            r"(?i)^id_(rsa|dsa|ecdsa|ed25519)([._-].*)?(?:[.\s:]|$)",
            r"(?i)^known_hosts(?:[.\s:]|$)",
            r"(?i)^authorized_keys(?:[.\s:]|$)",
            r"(?i)^htpasswd(?:[.\s:]|$)",
            r"(?i)^\.netrc(?:[.\s:]|$)",
            r"(?i)^_netrc(?:[.\s:]|$)",
            r"(?i)^credentials(?:[.\s:]|$)",
            r"(?i)^\.pgpass(?:[.\s:]|$)",
            r"(?i)^\.npmrc(?:[.\s:]|$)",
            r"(?i)^\.pypirc(?:[.\s:]|$)",
            r"(?i)^secrets?\.(json|ya?ml|toml|env)(?:[.\s:]|$)",
            r"(?i)^service[-_]?account.*\.json(?:[.\s:]|$)",
        ];
        PATTERNS
            .iter()
            .map(|p| regex::Regex::new(p).expect("static path pattern"))
            .collect()
    })
}

/// Normalized comparison surface. Never used as a real path.
fn comparison_form(path: &str) -> String {
    let mut s = path.replace('\\', "/");
    // Extended-length prefix `//?/C:/foo` or `//?/UNC/server/share`.
    if let Some(rest) = s.strip_prefix("//?/") {
        s = rest.to_owned();
    }
    // Drive prefix `C:/foo` -> `/foo`. Before lowercasing so `C:`/`c:` are
    // handled by the same rule.
    if s.len() >= 2 && s.as_bytes()[1] == b':' && s[..1].chars().all(|c| c.is_ascii_alphabetic()) {
        s = s[2..].to_owned();
    }
    if !s.starts_with('/') {
        s = format!("/{s}");
    }
    // NTFS alternate-data streams: `name:stream` reads as `name`.
    let s = s
        .split('/')
        .map(|seg| match seg.find(':') {
            Some(colon) => &seg[..colon],
            None => seg,
        })
        .collect::<Vec<_>>()
        .join("/");
    // Windows discards trailing dots/spaces at open time.
    let s = s
        .split('/')
        .map(|seg| seg.trim_end_matches(['.', ' ']))
        .collect::<Vec<_>>()
        .join("/");
    // Collapse duplicate slashes, preserving the leading one.
    let mut collapsed = String::with_capacity(s.len());
    let mut prev_slash = false;
    for ch in s.chars() {
        if ch == '/' {
            if prev_slash {
                continue;
            }
            prev_slash = true;
        } else {
            prev_slash = false;
        }
        collapsed.push(ch);
    }
    let collapsed = collapsed.to_lowercase();
    if collapsed.len() > 1 {
        collapsed.trim_end_matches('/').to_owned()
    } else {
        collapsed
    }
}

fn basename(path: &str) -> &str {
    path.rfind(['/', '\\'])
        .map(|i| &path[i + 1..])
        .unwrap_or(path)
}

/// `true` when `comparison` is exactly `dir` or lives beneath it.
fn is_under_protected(comparison: &str, dir: &str) -> bool {
    format!("{comparison}/").contains(&format!("{dir}/"))
}

/// Read-tier refusal, or `None` when the path may be read.
pub fn readable_violation(path: &Path) -> Option<String> {
    let raw = path.to_string_lossy();
    if raw.is_empty() {
        return Some("Refused: empty path.".into());
    }
    // NUL and control bytes are never legitimate in a path.
    if raw.chars().any(|c| (c as u32) < 0x20) {
        return Some("Refused: path contains control bytes.".into());
    }

    let base = basename(&raw);
    for pattern in secret_basename_patterns() {
        if pattern.is_match(base) {
            return Some(format!(
                "Refused: \"{base}\" matches a sensitive-file pattern."
            ));
        }
    }

    let comparison = comparison_form(&raw);
    for dir in PROTECTED_DIRS {
        if is_under_protected(&comparison, dir) {
            return Some(format!(
                "Refused: path is inside a protected directory ({}).",
                dir.trim_start_matches('/')
            ));
        }
    }
    None
}

/// Write-tier refusal: the read tier plus system-directory write blocks.
pub fn writable_violation(path: &Path) -> Option<String> {
    if let Some(reason) = readable_violation(path) {
        return Some(reason);
    }
    let comparison = comparison_form(&path.to_string_lossy());
    let base = basename(&comparison);
    if RUST_OWNED_BASENAMES.contains(&base) {
        return Some(format!(
            "Refused: \"{base}\" is managed by Tarex and cannot be written from the workspace."
        ));
    }
    for prefix in WRITE_DENY_PREFIXES {
        // Prefix match, plus the directory itself: `/usr/bin` does not start
        // with `/usr/bin/`, but appending the separator makes it match.
        if comparison.starts_with(prefix) || format!("{comparison}/").starts_with(prefix) {
            return Some(format!(
                "Refused: writes under \"{}\" are not allowed.",
                prefix.trim_end_matches('/')
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn read(p: &str) -> Option<String> {
        readable_violation(Path::new(p))
    }

    fn write(p: &str) -> Option<String> {
        writable_violation(Path::new(p))
    }

    #[test]
    fn allows_ordinary_workspace_paths() {
        assert!(read("/Users/dev/proj/src/main.rs").is_none());
        assert!(write("/Users/dev/proj/src/main.rs").is_none());
        assert!(read("/home/dev/.config/nvim/init.lua").is_none());
    }

    #[test]
    fn blocks_protected_directories_for_reads() {
        for p in [
            "/Users/dev/.ssh/id_rsa",
            "/Users/dev/.ssh",
            "/Users/dev/.SSH/config", // case variant
            "/Users/dev/.aws/credentials",
            "/Users/dev/.kube/config",
            "/Users/dev/.config/gh/hosts.yml",
            "/etc/passwd",
            "/private/etc/shadow",
            "/proc/self/environ",
        ] {
            assert!(read(p).is_some(), "expected refusal for {p}");
        }
    }

    #[test]
    fn does_not_match_protected_dir_prefix_siblings() {
        // `/.sshx` must not be caught by the `/.ssh` rule.
        assert!(read("/Users/dev/.sshx/notes.txt").is_none());
        assert!(read("/Users/dev/.awsome/file").is_none());
    }

    #[test]
    fn blocks_secret_shaped_basenames() {
        for p in [
            "/Users/dev/proj/.env",
            "/Users/dev/proj/.env.production",
            "/Users/dev/proj/.env.",
            "/Users/dev/proj/server.pem",
            "/Users/dev/proj/id_rsa",
            "/Users/dev/proj/id_ed25519.pub.bak",
            "/Users/dev/proj/credentials",
            "/Users/dev/proj/secrets.json",
            "/Users/dev/proj/service-account-key.json",
            "/Users/dev/proj/.npmrc",
        ] {
            assert!(read(p).is_some(), "expected refusal for {p}");
        }
    }

    #[test]
    fn allows_names_that_only_resemble_secrets() {
        for p in [
            "/Users/dev/proj/env.txt",
            "/Users/dev/proj/.environment.md",
            "/Users/dev/proj/pemfile.rs",
            // The `credentials` rule anchors at the basename start and needs a
            // `.`, space, `:` or end after it — a suffix word is not enough.
            "/Users/dev/proj/mycredentials",
            "/Users/dev/proj/credentials_backup",
            "/Users/dev/proj/known_hosts_example",
        ] {
            assert!(read(p).is_none(), "expected allowance for {p}");
        }
    }

    #[test]
    fn write_tier_adds_system_directories() {
        // `/etc` sits in the read tier too, so the write tier's value is the
        // locations that stay readable: /System, /usr/bin, /bin, ...
        assert!(read("/System/Library/foo").is_none());
        assert!(write("/System/Library/foo").is_some());
        assert!(read("/usr/bin/env").is_none());
        assert!(write("/usr/bin/env").is_some());
        assert!(write("/bin/sh").is_some());
        // Already refused for reads, so also refused for writes.
        assert!(read("/etc/hosts").is_some());
        assert!(write("/etc/hosts").is_some());
    }

    #[test]
    fn rust_owned_files_are_write_protected() {
        // The app data dir normally lives under an authorized home root, so
        // these need an explicit rule rather than the workspace check.
        for p in [
            "/Users/dev/Library/Application Support/tarex/lsp-allowed-servers.json",
            "/home/dev/.local/share/tarex/lsp-allowed-servers.json",
            "/home/dev/.local/share/tarex/secrets.json",
        ] {
            assert!(write(p).is_some(), "expected refusal for {p}");
        }
        // A same-named file inside a project is still the project's file.
        assert!(write("/Users/dev/proj/lsp-allowed-servers.json.bak").is_none());
    }

    #[test]
    fn normalizes_windows_canonical_forms() {
        // Verbatim prefix and drive letter must still match `/.ssh`.
        assert!(read(r"\\?\C:\Users\dev\.ssh\id_rsa").is_some());
        assert!(read(r"C:\Users\dev\.ssh\id_rsa").is_some());
        assert!(read("C:/Users/dev/.ssh/id_rsa").is_some());
        // NTFS stream syntax reads as the base name.
        assert!(read(r"C:\Users\dev\secrets.json::$DATA").is_some());
    }

    #[test]
    fn comparison_form_is_stable() {
        assert_eq!(
            comparison_form(r"\\?\C:\Users\Dev\.SSH\"),
            "/users/dev/.ssh"
        );
        assert_eq!(
            comparison_form("/Users/dev/proj//src/"),
            "/users/dev/proj/src"
        );
        assert_eq!(comparison_form("/Users/dev/.env."), "/users/dev/.env");
        assert_eq!(
            comparison_form(&PathBuf::from("/a/b").to_string_lossy()),
            "/a/b"
        );
    }

    #[test]
    fn rejects_control_bytes() {
        assert!(read("/Users/dev/proj/a\u{0}b").is_some());
        assert!(read("/Users/dev/proj/a\nb").is_some());
    }
}
