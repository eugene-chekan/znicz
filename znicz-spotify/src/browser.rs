//! Opening the login page in a browser, and whether to try. Front ends (the
//! CLI now, the TUI later) decide; the player never opens a browser.

use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

/// The `open_browser` setting under `[spotify]`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpenBrowser {
    #[default]
    Auto,
    Always,
    Never,
}

impl OpenBrowser {
    /// The config spelling (`auto`, `always`, `never`); `None` for anything else.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "auto" => Some(Self::Auto),
            "always" => Some(Self::Always),
            "never" => Some(Self::Never),
            _ => None,
        }
    }
}

const REMOTE_VARS: [&str; 3] = ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"];

/// Whether to open the login page. `env` reads one environment variable; an
/// empty value counts as unset.
pub fn should_open(setting: OpenBrowser, env: &dyn Fn(&str) -> Option<String>) -> bool {
    let set = |key: &str| env(key).is_some_and(|value| !value.is_empty());
    match setting {
        OpenBrowser::Always => true,
        OpenBrowser::Never => false,
        OpenBrowser::Auto => {
            if REMOTE_VARS.iter().any(|key| set(key)) {
                return false;
            }
            if cfg!(target_os = "linux") {
                return set("DISPLAY") || set("WAYLAND_DISPLAY");
            }
            true
        }
    }
}

/// Start the platform's opener on `url` and return without waiting for it.
/// Its output goes to null. A small thread waits for the opener to exit, so a
/// long-lived caller (the TUI) does not collect zombie processes.
pub fn open(url: &str) -> std::io::Result<()> {
    spawn_reaped(opener(url)).map(|_| ())
}

/// Spawn `command` with its stdio on null, and reap it on a thread. The handle
/// finishes once the child has exited.
fn spawn_reaped(mut command: Command) -> std::io::Result<std::thread::JoinHandle<()>> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    std::thread::Builder::new()
        .name("znicz-browser-reap".into())
        .spawn(move || {
            let _ = child.wait();
        })
}

#[cfg(target_os = "windows")]
fn opener(url: &str) -> Command {
    // Not through `cmd`: it would expand the URL's `%xx` escapes and stop at `&`.
    let mut command = Command::new("rundll32.exe");
    command.arg("url.dll,FileProtocolHandler").arg(url);
    command
}

#[cfg(target_os = "macos")]
fn opener(url: &str) -> Command {
    let mut command = Command::new("open");
    command.arg(url);
    command
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn opener(url: &str) -> Command {
    let mut command = Command::new("xdg-open");
    command.arg(url);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake environment: the listed variables are set, every other one is unset.
    fn env_of(pairs: Vec<(&'static str, &'static str)>) -> impl Fn(&str) -> Option<String> {
        move |key: &str| {
            pairs
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value.to_string())
        }
    }

    #[test]
    fn always_and_never_ignore_the_environment() {
        let remote = env_of(vec![("SSH_TTY", "/dev/pts/1"), ("DISPLAY", ":0")]);
        assert!(should_open(OpenBrowser::Always, &remote));
        assert!(!should_open(
            OpenBrowser::Never,
            &env_of(vec![("DISPLAY", ":0")])
        ));
    }

    #[test]
    fn auto_never_opens_over_ssh() {
        for key in ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"] {
            let set = env_of(vec![(key, "1"), ("DISPLAY", ":0")]);
            assert!(!should_open(OpenBrowser::Auto, &set), "{key} set");
        }
    }

    #[test]
    fn an_empty_ssh_variable_counts_as_unset() {
        let empty = env_of(vec![("SSH_TTY", ""), ("DISPLAY", ":0")]);
        assert!(should_open(OpenBrowser::Auto, &empty));
    }

    #[test]
    fn auto_on_linux_needs_a_display() {
        if cfg!(target_os = "linux") {
            assert!(!should_open(OpenBrowser::Auto, &env_of(vec![])));
            assert!(should_open(
                OpenBrowser::Auto,
                &env_of(vec![("DISPLAY", ":0")])
            ));
            assert!(should_open(
                OpenBrowser::Auto,
                &env_of(vec![("WAYLAND_DISPLAY", "wayland-0")])
            ));
        } else {
            assert!(should_open(OpenBrowser::Auto, &env_of(vec![])));
        }
    }

    #[test]
    fn the_setting_reads_its_config_spelling() {
        assert_eq!(OpenBrowser::parse("auto"), Some(OpenBrowser::Auto));
        assert_eq!(OpenBrowser::parse(" always "), Some(OpenBrowser::Always));
        assert_eq!(OpenBrowser::parse("never"), Some(OpenBrowser::Never));
        assert_eq!(OpenBrowser::parse("sometimes"), None);
        let from_serde: OpenBrowser = serde_json::from_str("\"never\"").expect("serde");
        assert_eq!(from_serde, OpenBrowser::Never);
        assert_eq!(OpenBrowser::default(), OpenBrowser::Auto);
    }

    #[cfg(unix)]
    #[test]
    fn the_opener_is_waited_for_so_it_leaves_no_zombie() {
        // The reaper thread ends once the child has exited and been waited on.
        let reaper = spawn_reaped(Command::new("true")).expect("spawn");
        reaper.join().expect("reaper thread");
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_opener_is_an_error() {
        assert!(spawn_reaped(Command::new("/nonexistent/znicz-opener")).is_err());
    }
}
