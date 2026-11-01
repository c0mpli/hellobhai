//! Where claude and codex live on Linux and which tools a voice turn may use.
//! Same rules as Windows: read-only tools, and opening apps/files/links is the
//! `[OPEN]` tag (checked by `computer_control::allowed_open_target`, run with
//! xdg-open or gtk-launch). A shell allowlist can't be made tight: xdg-open on a
//! .desktop file or `sh -c` runs anything.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The Mac's lookup order with Linux install places: the native installer
/// (~/.local/bin), the old local install, /usr/local/bin, /usr/bin, an npm
/// global prefix, then the user's login shell (`$SHELL -ilc 'command -v claude'`),
/// because apps started from the desktop get a minimal PATH.
pub fn claude_executable_candidates() -> Vec<PathBuf> {
    executable_candidates("claude", &[".local/bin/claude", ".claude/local/claude", ".npm-global/bin/claude"])
}

/// Same lookup for `codex` (Mac `CodexCLI.codexExecutableURL`).
pub fn codex_executable_candidates() -> Vec<PathBuf> {
    executable_candidates("codex", &[".local/bin/codex", ".npm-global/bin/codex"])
}

fn executable_candidates(command_name: &str, paths_inside_home: &[&str]) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(home_directory) = std::env::var_os("HOME") {
        for path_inside_home in paths_inside_home {
            candidates.push(Path::new(&home_directory).join(path_inside_home));
        }
    }
    candidates.push(PathBuf::from(format!("/usr/local/bin/{command_name}")));
    candidates.push(PathBuf::from(format!("/usr/bin/{command_name}")));
    // The login shell is slow (it reads the user's rc files), so only when nothing else is there.
    if !candidates.iter().any(|candidate| candidate.is_file()) {
        if let Some(path_from_login_shell) = executable_path_from_login_shell(command_name) {
            candidates.push(path_from_login_shell);
        }
    }
    candidates
}

/// `$SHELL -ilc 'command -v <name>'` (bash when SHELL is unset), last output
/// line, killed after 5 seconds in case an rc file waits for input.
fn executable_path_from_login_shell(command_name: &str) -> Option<PathBuf> {
    let login_shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    let mut shell_process = std::process::Command::new(login_shell)
        .args(["-ilc", &format!("command -v {command_name}")])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        match shell_process.try_wait() {
            Ok(Some(_exit_status)) => break,
            Ok(None) if started.elapsed() < Duration::from_secs(5) => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = shell_process.kill();
                let _ = shell_process.wait();
                return None;
            }
        }
    }
    let mut shell_output = String::new();
    std::io::Read::read_to_string(&mut shell_process.stdout.take()?, &mut shell_output).ok()?;
    last_absolute_path_line(&shell_output)
}

/// An interactive shell may print a banner first; the answer is the last line.
fn last_absolute_path_line(shell_output: &str) -> Option<PathBuf> {
    let last_line = shell_output.lines().map(str::trim).rfind(|line| !line.is_empty())?;
    let resolved_path = PathBuf::from(last_line);
    (resolved_path.is_absolute() && resolved_path.is_file()).then_some(resolved_path)
}

/// `--tools Read,Glob,Grep` (all read-only, allowed without asking) plus
/// `--add-dir` for the home folder and the system application folders, so Glob
/// can list installed apps (their .desktop files).
pub fn voice_turn_tool_arguments(home_directory: &Path) -> Vec<String> {
    let mut arguments: Vec<String> = ["--tools", "Read,Glob,Grep", "--allowedTools", "Read", "Glob", "Grep", "--add-dir"]
        .iter()
        .map(|argument| argument.to_string())
        .collect();
    arguments.push(home_directory.to_string_lossy().into_owned());
    for applications_directory in start_menu_shortcut_directories() {
        if !applications_directory.starts_with(home_directory) {
            arguments.push("--add-dir".to_string());
            arguments.push(applications_directory.to_string_lossy().into_owned());
        }
    }
    arguments
}

/// The Linux "Start menu": every existing `applications` folder of the XDG
/// data dirs (user first), plus Flatpak and Snap exports. Their .desktop files
/// are the installed apps `[OPEN:app name]` may start.
pub fn start_menu_shortcut_directories() -> Vec<PathBuf> {
    let home_directory = std::env::var_os("HOME").map(PathBuf::from);
    let user_data_directory = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| home_directory.as_ref().map(|home| home.join(".local/share")));
    let system_data_directories = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_string());

    let mut applications_directories: Vec<PathBuf> = Vec::new();
    if let Some(user_data_directory) = user_data_directory {
        applications_directories.push(user_data_directory.join("applications"));
    }
    for system_data_directory in system_data_directories.split(':').map(PathBuf::from).filter(|path| path.is_absolute()) {
        applications_directories.push(system_data_directory.join("applications"));
    }
    if let Some(home_directory) = &home_directory {
        applications_directories.push(home_directory.join(".local/share/flatpak/exports/share/applications"));
    }
    applications_directories.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
    applications_directories.push(PathBuf::from("/var/lib/snapd/desktop/applications"));

    let mut unique_existing_directories = Vec::new();
    for applications_directory in applications_directories {
        if applications_directory.is_dir() && !unique_existing_directories.contains(&applications_directory) {
            unique_existing_directories.push(applications_directory);
        }
    }
    unique_existing_directories
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_turns_get_only_read_only_tools() {
        let arguments = voice_turn_tool_arguments(Path::new("/home/bhai"));
        assert_eq!(&arguments[..8], ["--tools", "Read,Glob,Grep", "--allowedTools", "Read", "Glob", "Grep", "--add-dir", "/home/bhai"]);
        assert!(!arguments.iter().any(|argument| argument.contains("Bash")));
    }

    #[test]
    fn login_shell_answer_is_the_last_absolute_line() {
        assert_eq!(last_absolute_path_line("welcome banner\n/bin/sh\n\n"), Some(PathBuf::from("/bin/sh")));
        assert_eq!(last_absolute_path_line("claude: not found\n"), None);
        assert_eq!(last_absolute_path_line(""), None);
    }
}
