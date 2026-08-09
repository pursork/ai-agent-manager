//! Opens a real OS terminal window with a Profile's launch env already
//! injected and a command already running (`docs/06-gui-terminal-shell.md`
//! §6.5: the GUI owns the terminal it opens, so it's allowed to actually
//! run the command rather than degrading to "just print it").
//!
//! Prefers Windows Terminal (`wt.exe`) when available (better multi-tab
//! UX, `wt.exe` "既然wt.exe体验感更好，那么就优先考虑"); always falls back
//! to a plain `powershell.exe` window when it isn't -- that fallback path
//! must never be allowed to become a hard failure, since not everyone has
//! Windows Terminal installed.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

/// Directories to look for `wt.exe` in, beyond whatever a plain PATH
/// lookup via `Command::new("wt.exe")` would already try -- specifically
/// the Microsoft Store package's App Execution Alias location, which is
/// sometimes not on PATH depending on how it was installed.
fn extra_search_dirs() -> Vec<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|dir| vec![PathBuf::from(dir).join("Microsoft").join("WindowsApps")])
        .unwrap_or_default()
}

fn path_search_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default()
}

/// Pure search over an injected directory list -- the testable part.
/// Real callers use [`find_wt_exe`], which supplies the real PATH +
/// App Execution Alias directory.
fn find_wt_exe_in(dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter().map(|d| d.join("wt.exe")).find(|p| p.is_file())
}

/// Locates `wt.exe` on this machine, or `None` if it isn't installed
/// (or isn't somewhere we know to look).
pub fn find_wt_exe() -> Option<PathBuf> {
    let mut dirs = path_search_dirs();
    dirs.extend(extra_search_dirs());
    find_wt_exe_in(&dirs)
}

pub fn wt_available() -> bool {
    find_wt_exe().is_some()
}

/// The argv (excluding argv[0]) for launching `powershell.exe` with
/// `command` already running and `-NoExit` so the window stays open
/// afterwards for the user to keep working in. Pure -- no filesystem or
/// process access -- so it's unit-testable on its own.
fn powershell_args(command: &str) -> Vec<String> {
    vec!["-NoExit".to_string(), "-Command".to_string(), command.to_string()]
}

/// The argv for launching `wt.exe` with an optional starting directory,
/// running the same PowerShell command line as [`powershell_args`] inside
/// the new tab/window it opens. Pure, same reasoning as above.
fn wt_args(cwd: Option<&Path>, command: &str) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(cwd) = cwd {
        args.push("-d".to_string());
        args.push(cwd.display().to_string());
    }
    args.push("powershell.exe".to_string());
    args.extend(powershell_args(command));
    args
}

#[cfg(windows)]
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

/// Opens a new terminal window (Windows Terminal if available, otherwise
/// a plain PowerShell console) with `env` injected, `cwd` as the starting
/// directory (defaults to whatever `aam-gui` itself is running under if
/// `None` -- matches `aam claude/codex <label>`'s deliberate choice to
/// never force a `current_dir`, `03.6`), running `command` immediately.
pub fn open_terminal(cwd: Option<&Path>, env: &[(String, String)], command: &str) -> io::Result<Child> {
    let mut cmd = match find_wt_exe() {
        Some(wt) => {
            let mut c = Command::new(wt);
            c.args(wt_args(cwd, command));
            c
        }
        None => {
            let mut c = Command::new("powershell.exe");
            c.args(powershell_args(command));
            if let Some(cwd) = cwd {
                c.current_dir(cwd);
            }
            c
        }
    };
    for (key, value) in env {
        cmd.env(key, value);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NEW_CONSOLE);
    }
    cmd.spawn()
}

/// Best-effort installer, only ever called from an explicit GUI button
/// click (never automatically) -- fires `winget install` detached and
/// returns immediately; doesn't wait for or report the outcome, since
/// a freshly-installed `wt.exe` may need a new shell session to become
/// visible on PATH anyway. Not finding `winget` itself is reported as an
/// error for the GUI to surface, not silently swallowed.
pub fn install_windows_terminal() -> io::Result<Child> {
    Command::new("winget")
        .args([
            "install",
            "--id",
            "Microsoft.WindowsTerminal",
            "-e",
            "--accept-package-agreements",
            "--accept-source-agreements",
        ])
        .spawn()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("aam-gui-terminal-test-{label}-{}-{unique}", std::process::id()));
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn find_wt_exe_in_returns_none_when_absent_from_every_dir() {
        let base = TempDir::new("absent");
        let empty_dir = base.0.join("empty");
        fs::create_dir_all(&empty_dir).unwrap();
        assert_eq!(find_wt_exe_in(&[empty_dir]), None);
    }

    #[test]
    fn find_wt_exe_in_finds_it_in_a_later_directory() {
        let base = TempDir::new("present");
        let miss_dir = base.0.join("miss");
        let hit_dir = base.0.join("hit");
        fs::create_dir_all(&miss_dir).unwrap();
        fs::create_dir_all(&hit_dir).unwrap();
        fs::write(hit_dir.join("wt.exe"), b"").unwrap();

        let found = find_wt_exe_in(&[miss_dir, hit_dir.clone()]);
        assert_eq!(found, Some(hit_dir.join("wt.exe")));
    }

    #[test]
    fn find_wt_exe_in_ignores_a_directory_that_does_not_exist() {
        let base = TempDir::new("missing-dir");
        let nonexistent = base.0.join("does-not-exist");
        assert_eq!(find_wt_exe_in(&[nonexistent]), None);
    }

    #[test]
    fn powershell_args_keeps_window_open_and_runs_the_command() {
        let args = powershell_args("claude --resume abc123");
        assert_eq!(args, vec!["-NoExit", "-Command", "claude --resume abc123"]);
    }

    #[test]
    fn wt_args_includes_start_dir_when_given() {
        let args = wt_args(Some(Path::new("C:\\projects\\x")), "claude");
        assert_eq!(
            args,
            vec!["-d", "C:\\projects\\x", "powershell.exe", "-NoExit", "-Command", "claude"]
        );
    }

    #[test]
    fn wt_args_omits_start_dir_when_none() {
        let args = wt_args(None, "codex");
        assert_eq!(args, vec!["powershell.exe", "-NoExit", "-Command", "codex"]);
    }
}
