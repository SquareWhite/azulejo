//! The environment herdr runs with. Panes inherit the server's environment,
//! so a server started from inside an agent session leaks that session's
//! variables (e.g. `CLAUDE_CODE_*`) into every pane. Azulejo starts herdr,
//! and runs its CLI, with an allowlist instead.

use std::ffi::{OsStr, OsString};

/// Exact names passed through.
const ALLOW: &[&str] = &[
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "PATH",
    "LANG",
    "TERM",
    "TERMINFO",
    "TERMINFO_DIRS",
    "COLORTERM",
    "TMPDIR",
    "SSH_AUTH_SOCK",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "DBUS_SESSION_BUS_ADDRESS",
    "__CF_USER_TEXT_ENCODING",
];

/// Prefixes passed through.
const ALLOW_PREFIX: &[&str] = &["LC_", "XDG_"];

/// Prefixes always removed, even if listed in `extra_allow`.
const DENY_PREFIX: &[&str] = &["HERDR_", "CLAUDE", "AZULEJO_"];

/// Filter `vars` down to what a herdr server or CLI call should see.
/// `extra_allow` adds exact names (from user config).
pub fn clean_env<I, K, V>(vars: I, extra_allow: &[&str]) -> Vec<(OsString, OsString)>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<OsStr>,
    V: AsRef<OsStr>,
{
    vars.into_iter()
        .filter(|(k, _)| keep(k.as_ref(), extra_allow))
        .map(|(k, v)| (k.as_ref().to_owned(), v.as_ref().to_owned()))
        .collect()
}

/// [`clean_env`] over the current process environment.
pub fn clean_process_env(extra_allow: &[&str]) -> Vec<(OsString, OsString)> {
    clean_env(std::env::vars_os(), extra_allow)
}

fn keep(name: &OsStr, extra_allow: &[&str]) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    if DENY_PREFIX.iter().any(|p| name.starts_with(p)) {
        return false;
    }
    ALLOW.contains(&name)
        || extra_allow.contains(&name)
        || ALLOW_PREFIX.iter().any(|p| name.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(vars: &[(&str, &str)], extra: &[&str]) -> Vec<String> {
        clean_env(vars.iter().copied(), extra)
            .into_iter()
            .map(|(k, _)| k.into_string().unwrap())
            .collect()
    }

    #[test]
    fn keeps_allowlisted_and_prefixed() {
        let got = names(
            &[
                ("HOME", "/h"),
                ("PATH", "/bin"),
                ("LC_ALL", "C"),
                ("XDG_RUNTIME_DIR", "/run"),
                ("SSH_AUTH_SOCK", "/s"),
            ],
            &[],
        );
        assert_eq!(
            got,
            ["HOME", "PATH", "LC_ALL", "XDG_RUNTIME_DIR", "SSH_AUTH_SOCK"]
        );
    }

    #[test]
    fn strips_leaky_variables() {
        let got = names(
            &[
                ("HERDR_SOCKET_PATH", "/x"),
                ("HERDR_PANE_ID", "w1:p1"),
                ("CLAUDE_CODE_CHILD_SESSION", "1"),
                ("CLAUDECODE", "1"),
                ("AZULEJO_TILE_ID", "t1"),
                ("RANDOM_SECRET", "x"),
                ("HOME", "/h"),
            ],
            &[],
        );
        assert_eq!(got, ["HOME"]);
    }

    #[test]
    fn extra_allow_cannot_reopen_denied_prefixes() {
        let got = names(
            &[("EDITOR", "vim"), ("HERDR_SOCKET_PATH", "/x")],
            &["EDITOR", "HERDR_SOCKET_PATH"],
        );
        assert_eq!(got, ["EDITOR"]);
    }
}
