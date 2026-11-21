//! What the renderer may start as a language server.
//!
//! `lsp_spawn` used to take an arbitrary `command`, `args` and `env` straight
//! from the webview: silent process execution with an arbitrary environment,
//! which is enough for full code execution (`LD_PRELOAD`, `PATH` hijacks,
//! `sh -c …`). Spawning is now gated three ways:
//!
//! 1. shape checks — bounded length/count and no control bytes or shell
//!    metacharacters, applied to `command` and `args`;
//! 2. an environment allowlist, because only shipped presets ever add
//!    variables (`fromCustom` drops `env`);
//! 3. authorization — the exact `command`/`args` pair must either match a
//!    shipped preset or be confirmed by the user in a native OS dialog, after
//!    which Rust remembers it in a file the renderer cannot write.
//!
//! The confirm dialog is the trust anchor: it is the same pattern as the file
//! pickers in [`crate::modules::dialog`], and it is the only way a
//! renderer-supplied program becomes runnable.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogResult};

/// Renderer-visible name of the Rust-owned allowlist. Listed in
/// [`crate::modules::fs::protect`] so no `fs_*` command can rewrite it.
const ALLOWLIST_FILE: &str = "lsp-allowed-servers.json";

/// Exact `command`/`args` pairs shipped in `LSP_PRESETS`. The frontend never
/// mutates these, so a mismatch means something other than a preset is asking.
const PRESET_SERVERS: &[(&str, &[&str])] = &[
    ("typescript-language-server", &["--stdio"]),
    ("rust-analyzer", &[]),
    ("pyright-langserver", &["--stdio"]),
    ("ruff", &["server"]),
    ("gopls", &[]),
    ("clangd", &[]),
    ("zls", &[]),
    ("lua-language-server", &[]),
    ("ruby-lsp", &[]),
    ("intelephense", &["--stdio"]),
    ("yaml-language-server", &["--stdio"]),
    ("bash-language-server", &["start"]),
    ("vscode-json-language-server", &["--stdio"]),
    ("vscode-css-language-server", &["--stdio"]),
    ("vscode-html-language-server", &["--stdio"]),
    ("svelteserver", &["--stdio"]),
    ("vue-language-server", &["--stdio"]),
    ("sourcekit-lsp", &[]),
];

/// Variables a preset may add on top of the inherited environment. Widening
/// this list is a deliberate act, not a side effect of a new preset.
const ALLOWED_ENV_KEYS: &[&str] = &["CARGO_BUILD_JOBS"];

const MAX_COMMAND_LEN: usize = 512;
const MAX_ARGS: usize = 64;
const MAX_ARG_LEN: usize = 4_096;
const MAX_ARGS_TOTAL: usize = 16 * 1024;
const MAX_ENV_VALUE_LEN: usize = 1_024;

/// Whitespace and shell metacharacters. Nothing legitimate appears in an
/// executable name, and rejecting them keeps a future shell-out from turning
/// these values into a script.
const FORBIDDEN_COMMAND_CHARS: &[char] = &[
    ' ', '\t', '\n', '\r', ';', '&', '|', '<', '>', '$', '`', '\'', '"', '(', ')', '{', '}', '[',
    ']', '*', '?', '!', '~', '#', '%',
];

fn has_control_bytes(value: &str) -> bool {
    value.chars().any(|c| (c as u32) < 0x20 || c == '\u{7f}')
}

/// Shape check for the executable name. Applied to `lsp_detect` too, so the
/// existence probe cannot be fed arbitrary strings either.
pub fn validate_command(command: &str) -> Result<(), String> {
    if command.trim().is_empty() {
        return Err("lsp: command is empty".into());
    }
    if command.len() > MAX_COMMAND_LEN {
        return Err(format!("lsp: command exceeds {MAX_COMMAND_LEN} bytes"));
    }
    if has_control_bytes(command) {
        return Err("lsp: command contains control bytes".into());
    }
    if let Some(found) = command
        .chars()
        .find(|c| FORBIDDEN_COMMAND_CHARS.contains(c))
    {
        return Err(format!(
            "lsp: command contains a forbidden character: {found:?}"
        ));
    }
    Ok(())
}

fn validate_args(args: &[String]) -> Result<(), String> {
    if args.len() > MAX_ARGS {
        return Err(format!("lsp: too many arguments (max {MAX_ARGS})"));
    }
    let mut total = 0usize;
    for arg in args {
        if arg.len() > MAX_ARG_LEN {
            return Err(format!("lsp: argument exceeds {MAX_ARG_LEN} bytes"));
        }
        if has_control_bytes(arg) {
            return Err("lsp: argument contains control bytes".into());
        }
        total += arg.len();
    }
    if total > MAX_ARGS_TOTAL {
        return Err(format!(
            "lsp: arguments exceed {MAX_ARGS_TOTAL} bytes in total"
        ));
    }
    Ok(())
}

fn validate_env(env: Option<&HashMap<String, String>>) -> Result<(), String> {
    let Some(env) = env else { return Ok(()) };
    for (key, value) in env {
        if !ALLOWED_ENV_KEYS.contains(&key.as_str()) {
            return Err(format!("lsp: environment variable is not allowed: {key}"));
        }
        if value.len() > MAX_ENV_VALUE_LEN {
            return Err(format!(
                "lsp: environment variable {key} exceeds {MAX_ENV_VALUE_LEN} bytes"
            ));
        }
        if has_control_bytes(value) {
            return Err(format!(
                "lsp: environment variable {key} contains control bytes"
            ));
        }
    }
    Ok(())
}

/// Everything checkable without user interaction. Runs before any dialog so a
/// malformed request never prompts the user.
pub fn validate_request(
    command: &str,
    args: &[String],
    env: Option<&HashMap<String, String>>,
) -> Result<(), String> {
    validate_command(command)?;
    validate_args(args)?;
    validate_env(env)
}

/// True when this exact pair is one of the shipped presets.
pub fn is_preset(command: &str, args: &[String]) -> bool {
    PRESET_SERVERS.iter().any(|(preset, preset_args)| {
        preset == &command
            && preset_args.len() == args.len()
            && preset_args
                .iter()
                .zip(args)
                .all(|(expected, got)| expected == got)
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AllowedServer {
    command: String,
    args: Vec<String>,
}

fn allowlist_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_local_data_dir()
        .map(|dir| dir.join(ALLOWLIST_FILE))
        .map_err(|error| error.to_string())
}

/// Fail closed: a missing or unreadable file yields no grants.
fn load(app: &tauri::AppHandle) -> Vec<AllowedServer> {
    let Ok(path) = allowlist_path(app) else {
        return Vec::new();
    };
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    serde_json::from_slice::<Vec<AllowedServer>>(&bytes).unwrap_or_default()
}

fn save(app: &tauri::AppHandle, list: &[AllowedServer]) -> Result<(), String> {
    let path = allowlist_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(list).map_err(|error| error.to_string())?;
    std::fs::write(&tmp, bytes).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, &path).map_err(|error| error.to_string())
}

fn is_registered(app: &tauri::AppHandle, command: &str, args: &[String]) -> bool {
    load(app)
        .iter()
        .any(|entry| entry.command == command && entry.args == args)
}

/// The gate in front of `lsp_spawn`.
///
/// Presets pass silently. Anything else is confirmed once in a native dialog
/// and then remembered, so the prompt appears the first time a custom server
/// is used rather than on every document.
pub async fn authorize(
    app: &tauri::AppHandle,
    command: &str,
    args: &[String],
) -> Result<(), String> {
    validate_command(command)?;
    validate_args(args)?;

    if is_preset(command, args) || is_registered(app, command, args) {
        return Ok(());
    }

    let rendered = if args.is_empty() {
        command.to_string()
    } else {
        format!("{command} {}", args.join(" "))
    };
    let answer = app
        .dialog()
        .message(format!(
            "Tarex wants to run this program as a language server:\n\n{rendered}\n\n\
             Only allow this if you added the server yourself in Settings."
        ))
        .title("Language server")
        .buttons(MessageDialogButtons::OkCancel)
        .blocking_show_with_result();
    if answer != MessageDialogResult::Ok {
        return Err(format!("lsp: permission to run \"{command}\" was declined"));
    }

    let mut list = load(app);
    list.push(AllowedServer {
        command: command.to_string(),
        args: args.to_vec(),
    });
    save(app, &list)?;
    log::debug!("lsp: allowed custom server {rendered}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn every_shipped_preset_is_accepted() {
        for (command, preset_args) in PRESET_SERVERS {
            let args = args(preset_args);
            assert!(is_preset(command, &args), "{command} must be a preset");
            validate_request(command, &args, None).expect("preset must validate");
        }
    }

    #[test]
    fn preset_pairs_are_matched_exactly() {
        // Same binary, different arguments, is not a preset.
        assert!(!is_preset("clangd", &args(&["--background-index"])));
        // A preset name with unexpected arguments must not slip through.
        assert!(!is_preset("rust-analyzer", &args(&["--verbose"])));
        assert!(!is_preset("sh", &args(&["-c", "id"])));
        assert!(!is_preset("", &[]));
    }

    #[test]
    fn command_shape_rejects_scripts_and_control_bytes() {
        for command in [
            "", "   ", "sh -c id", "my;lsp", "my|lsp", "$(id)", "a\u{0}b", "a\nb", "lsp?", "*",
        ] {
            assert!(
                validate_command(command).is_err(),
                "{command:?} must be refused"
            );
        }
        assert!(validate_command("typescript-language-server").is_ok());
        assert!(validate_command("/usr/local/bin/my-lsp").is_ok());
        assert!(validate_command(&"a".repeat(MAX_COMMAND_LEN + 1)).is_err());
    }

    #[test]
    fn argument_bounds_are_enforced() {
        assert!(validate_args(&[]).is_ok());
        assert!(validate_args(&args(&["--stdio"])).is_ok());
        assert!(validate_args(&args(&["a\u{0}b"])).is_err());
        assert!(validate_args(&args(&[&"x".repeat(MAX_ARG_LEN + 1)])).is_err());
        assert!(validate_args(&args(&["ok"; MAX_ARGS + 1])).is_err());

        let big = "y".repeat(MAX_ARG_LEN);
        let many: Vec<String> = (0..(MAX_ARGS_TOTAL / MAX_ARG_LEN + 1))
            .map(|_| big.clone())
            .take(MAX_ARGS)
            .collect();
        assert!(validate_args(&many).is_err());
    }

    #[test]
    fn only_preset_environment_keys_are_allowed() {
        let mut env = HashMap::new();
        env.insert("CARGO_BUILD_JOBS".to_string(), "2".to_string());
        assert!(validate_env(Some(&env)).is_ok());

        for key in [
            "LD_PRELOAD",
            "DYLD_INSERT_LIBRARIES",
            "PATH",
            "HOME",
            "PYTHONSTARTUP",
        ] {
            let mut blocked = HashMap::new();
            blocked.insert(key.to_string(), "/tmp/evil.so".to_string());
            let error = validate_env(Some(&blocked)).unwrap_err();
            assert!(
                error.contains("not allowed"),
                "{key} must be refused: {error}"
            );
        }

        let mut too_long = HashMap::new();
        too_long.insert(
            "CARGO_BUILD_JOBS".to_string(),
            "x".repeat(MAX_ENV_VALUE_LEN + 1),
        );
        assert!(validate_env(Some(&too_long)).is_err());
        assert!(validate_env(None).is_ok());
    }

    #[test]
    fn request_validation_covers_all_three_inputs() {
        assert!(validate_request("rust-analyzer", &args(&[]), None).is_ok());
        // Shape-valid but not a preset: `validate_request` lets it through and
        // `authorize` is what forces the confirmation dialog.
        assert!(validate_request("sh", &args(&["-c", "id"]), None).is_ok());
        assert!(!is_preset("sh", &args(&["-c", "id"])));
        // A malformed command never reaches the dialog at all.
        assert!(validate_request("sh -c id", &args(&[]), None).is_err());
        let mut env = HashMap::new();
        env.insert("LD_PRELOAD".to_string(), "x".to_string());
        assert!(validate_request("rust-analyzer", &args(&[]), Some(&env)).is_err());
    }
}
