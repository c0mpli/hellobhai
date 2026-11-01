//! Replays [CLICK]/[TYPE]/[KEY]/[OPEN] actions (thin async wrappers around
//! `platform` calls via `spawn_blocking`, logging like the Mac). The buddy's
//! flight 1.2 s before a click, and cancelling the remaining actions on the
//! hotkey, live in the brain's agent loop.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::key_combo::parse_key_combo;
use crate::platform;

/// Global physical pixels.
pub async fn perform_click(x: i32, y: i32) {
    let click_result = tauri::async_runtime::spawn_blocking(move || platform::click_at_global_point(x, y)).await;
    match click_result {
        Ok(Ok(())) => eprintln!("computer control: clicked at {x}, {y}"),
        Ok(Err(error)) => eprintln!("computer control: click at {x}, {y} failed: {error}"),
        Err(join_error) => eprintln!("computer control: click task failed: {join_error}"),
    }
}

/// Sets the flag when dropped: aborting the response task drops `perform_type`'s
/// future, which tells the typing thread (not abortable itself) to stop.
struct SetFlagWhenDropped(Arc<AtomicBool>);

impl Drop for SetFlagWhenDropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

pub async fn perform_type(text: &str) {
    let text_to_type = text.to_string();
    let should_stop_typing = Arc::new(AtomicBool::new(false));
    let _stop_typing_if_cancelled = SetFlagWhenDropped(should_stop_typing.clone());
    let typing_result =
        tauri::async_runtime::spawn_blocking(move || platform::type_text(&text_to_type, &should_stop_typing)).await;
    match typing_result {
        Ok(Ok(())) => eprintln!("computer control: typed {} characters", text.chars().count()),
        Ok(Err(error)) => eprintln!("computer control: typing failed: {error}"),
        Err(join_error) => eprintln!("computer control: typing task failed: {join_error}"),
    }
}

/// false when the combo could not be parsed or sent.
pub async fn perform_key_combo(key_combo: &str) -> bool {
    let Some(parsed_key_combo) = parse_key_combo(key_combo) else {
        return false;
    };
    let key_press_result = tauri::async_runtime::spawn_blocking(move || platform::press_key_combo(&parsed_key_combo)).await;
    match key_press_result {
        Ok(Ok(())) => {
            eprintln!("computer control: pressed {key_combo}");
            true
        }
        Ok(Err(error)) => {
            eprintln!("computer control: pressing {key_combo} failed: {error}");
            false
        }
        Err(join_error) => {
            eprintln!("computer control: key task failed: {join_error}");
            false
        }
    }
}

/// [OPEN:target]: the Windows stand-in for the Mac's `open`, checked by `allowed_open_target`.
pub async fn perform_open(target: &str, home_directory: &Path) {
    let checked_target = match allowed_open_target(target, home_directory, &platform::start_menu_shortcut_directories()) {
        Ok(checked_target) => checked_target,
        Err(refusal) => {
            eprintln!("computer control: not opening {target}: {refusal}");
            return;
        }
    };
    let open_result = tauri::async_runtime::spawn_blocking(move || platform::open_with_shell(&checked_target)).await;
    match open_result {
        Ok(Ok(())) => eprintln!("computer control: opened {target}"),
        Ok(Err(error)) => eprintln!("computer control: opening {target} failed: {error}"),
        Err(join_error) => eprintln!("computer control: open task failed: {join_error}"),
    }
}

/// Link schemes [OPEN] accepts: the web, mail, and common chat/music/meeting apps.
const ALLOWED_LINK_SCHEMES: [&str; 10] = ["http", "https", "mailto", "whatsapp", "spotify", "slack", "msteams", "discord", "tg", "zoommtg"];
/// The Windows Settings app's links.
const WINDOWS_SETTINGS_LINK_SCHEME: &str = "ms-settings";
/// Built-in apps that have no Start menu shortcut on every Windows version.
const ALLOWED_BUILT_IN_APPS: [&str; 4] = ["notepad", "calc", "mspaint", "explorer"];
/// Windows-only targets (`shell:AppsFolder\…`, `ms-settings:`, the built-in
/// apps) mean nothing to xdg-open, so Linux refuses them. Earlier history or
/// the Codex brain can still produce them.
const ALLOWS_WINDOWS_ONLY_TARGETS: bool = !cfg!(target_os = "linux");
/// Files that run code when opened. Documents, media and folders are fine.
/// The second line is Linux: scripts, launchers and installers.
const BLOCKED_FILE_EXTENSIONS: &[&str] = &[
    "exe", "com", "bat", "cmd", "ps1", "psm1", "vbs", "vbe", "js", "jse", "wsf", "wsh", "msi", "msp", "scr", "lnk", "url",
    "pif", "cpl", "hta", "jar", "reg", "inf", "scf", "msc", "application", "appref-ms",
    "sh", "bash", "zsh", "fish", "py", "pl", "rb", "desktop", "appimage", "run", "bin", "deb", "rpm", "snap", "flatpakref",
];
/// Installed apps are Start menu shortcuts on Windows and .desktop files on Linux.
const APP_SHORTCUT_FILE_EXTENSION: &str = if cfg!(target_os = "linux") { "desktop" } else { "lnk" };

/// What [OPEN] may open, as the string to hand to the shell:
/// - an http(s), mailto or allowed app link (`whatsapp://send?...`)
/// - `shell:AppsFolder\<packaged app id>`
/// - notepad, calc, mspaint or explorer
/// - an existing file or folder inside the home folder, except programs and scripts
/// - an app by its Start menu name (the matching .lnk under the Start menu folders)
///
/// Programs, scripts and other link schemes are refused: Claude's tools are
/// read-only, and this tag must not become a way to run anything.
pub fn allowed_open_target(target: &str, home_directory: &Path, start_menu_directories: &[PathBuf]) -> Result<String, String> {
    let target = target.trim();
    if target.is_empty() {
        return Err("nothing to open".into());
    }

    let apps_folder_prefix = "shell:appsfolder\\";
    if ALLOWS_WINDOWS_ONLY_TARGETS && target.to_ascii_lowercase().starts_with(apps_folder_prefix) {
        let app_id = &target[apps_folder_prefix.len()..];
        let is_plain_app_id =
            !app_id.is_empty() && app_id.chars().all(|character| character.is_ascii_alphanumeric() || "._!-".contains(character));
        return if is_plain_app_id { Ok(target.to_string()) } else { Err("not a packaged app id".into()) };
    }

    // A scheme is 2+ characters, so a drive letter ("C:\...") is not one.
    if let Some((scheme, _)) = target.split_once(':') {
        let looks_like_scheme =
            scheme.len() >= 2 && scheme.chars().all(|character| character.is_ascii_alphanumeric() || "+-.".contains(character));
        if looks_like_scheme {
            let scheme = scheme.to_ascii_lowercase();
            let is_allowed_scheme = ALLOWED_LINK_SCHEMES.contains(&scheme.as_str())
                || (ALLOWS_WINDOWS_ONLY_TARGETS && scheme == WINDOWS_SETTINGS_LINK_SCHEME);
            return if is_allowed_scheme {
                Ok(target.to_string())
            } else {
                Err(format!("{scheme}: links can't be opened"))
            };
        }
    }

    if ALLOWS_WINDOWS_ONLY_TARGETS && ALLOWED_BUILT_IN_APPS.contains(&target.to_ascii_lowercase().as_str()) {
        return Ok(target.to_string());
    }

    let target_path = Path::new(target);
    if target_path.is_absolute() {
        let canonical_target = std::fs::canonicalize(target_path).map_err(|_| "no such file or folder".to_string())?;
        let canonical_home = std::fs::canonicalize(home_directory).map_err(|_| "home folder not found".to_string())?;
        if !canonical_target.starts_with(&canonical_home) {
            return Err("only files and folders inside the home folder can be opened".into());
        }
        let extension = canonical_target.extension().map(|extension| extension.to_string_lossy().to_ascii_lowercase());
        if canonical_target.is_file() && extension.is_some_and(|extension| BLOCKED_FILE_EXTENSIONS.contains(&extension.as_str())) {
            return Err("programs and scripts can't be opened".into());
        }
        if canonical_target.is_file() && is_executable_file(&canonical_target) {
            return Err("programs and scripts can't be opened".into());
        }
        return Ok(target.to_string());
    }

    // Linux: the Windows name for the calculator finds the "Calculator" app.
    let app_name = if !ALLOWS_WINDOWS_ONLY_TARGETS && target.eq_ignore_ascii_case("calc") { "Calculator" } else { target };
    for start_menu_directory in start_menu_directories {
        if let Some(shortcut_path) = find_start_menu_shortcut(start_menu_directory, app_name, 0) {
            return Ok(shortcut_path.to_string_lossy().into_owned());
        }
    }
    Err(format!("no app called {target} in the start menu"))
}

/// Unix: any execute bit (a script without an extension still runs when a file
/// manager "opens" it). Windows decides by extension alone.
fn is_executable_file(file_path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(file_path).is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        let _ = file_path;
        false
    }
}

/// The app's name in a .desktop file: the `Name=` line of `[Desktop Entry]`.
fn desktop_entry_name(desktop_file_contents: &str) -> Option<&str> {
    let mut is_in_desktop_entry_group = false;
    for line in desktop_file_contents.lines().map(str::trim) {
        if line.starts_with('[') {
            is_in_desktop_entry_group = line == "[Desktop Entry]";
        } else if is_in_desktop_entry_group {
            if let Some(app_name) = line.strip_prefix("Name=") {
                return Some(app_name.trim());
            }
        }
    }
    None
}

/// The first `<app name>.lnk` (Windows) or `<app name>.desktop` (Linux, also
/// matched by its `Name=` line: "Calculator" is `org.gnome.Calculator.desktop`),
/// any case, in `directory` or its subfolders.
fn find_start_menu_shortcut(directory: &Path, app_name: &str, folder_depth: usize) -> Option<PathBuf> {
    const MAXIMUM_FOLDER_DEPTH: usize = 4;
    let wanted_file_name = format!("{app_name}.{APP_SHORTCUT_FILE_EXTENSION}").to_lowercase();
    let mut subdirectories = Vec::new();
    let mut desktop_files_to_check_by_name = Vec::new();
    for directory_entry in std::fs::read_dir(directory).ok()?.flatten() {
        let entry_path = directory_entry.path();
        let entry_file_name = directory_entry.file_name().to_string_lossy().to_lowercase();
        if entry_path.is_dir() {
            subdirectories.push(entry_path);
        } else if entry_file_name == wanted_file_name {
            return Some(entry_path);
        } else if APP_SHORTCUT_FILE_EXTENSION == "desktop" && entry_file_name.ends_with(".desktop") {
            desktop_files_to_check_by_name.push(entry_path);
        }
    }
    // File names first (exact), then the display names.
    let found_by_display_name = desktop_files_to_check_by_name.into_iter().find(|desktop_file_path| {
        std::fs::read_to_string(desktop_file_path)
            .ok()
            .is_some_and(|desktop_file_contents| desktop_entry_name(&desktop_file_contents).is_some_and(|name| name.eq_ignore_ascii_case(app_name)))
    });
    if found_by_display_name.is_some() {
        return found_by_display_name;
    }
    if folder_depth >= MAXIMUM_FOLDER_DEPTH {
        return None;
    }
    subdirectories.iter().find_map(|subdirectory| find_start_menu_shortcut(subdirectory, app_name, folder_depth + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_allows_links_apps_and_home_documents_but_not_programs() {
        let test_directory = std::env::temp_dir().join(format!("hellobhai-open-test-{}", std::process::id()));
        let home_directory = test_directory.join("home");
        let start_menu_directory = test_directory.join("start-menu");
        std::fs::create_dir_all(home_directory.join("Videos")).unwrap();
        std::fs::create_dir_all(start_menu_directory.join("Spotify")).unwrap();
        std::fs::write(home_directory.join("Videos").join("clip.mp4"), b"").unwrap();
        std::fs::write(home_directory.join("setup.EXE"), b"").unwrap();
        std::fs::write(test_directory.join("outside.txt"), b"").unwrap();
        std::fs::write(start_menu_directory.join("Spotify").join(format!("Spotify.{APP_SHORTCUT_FILE_EXTENSION}")), b"").unwrap();
        let start_menu_directories = vec![start_menu_directory.clone()];
        let check = |target: &str| allowed_open_target(target, &home_directory, &start_menu_directories);

        assert_eq!(check("https://youtube.com"), Ok("https://youtube.com".to_string()));
        assert!(check("whatsapp://send?phone=919876543210&text=hi").is_ok());
        assert!(check("file:///C:/Windows/System32/cmd.exe").is_err());
        assert!(check("ms-msdt:/id PCWDiagnostic").is_err());
        assert!(check("shell:startup").is_err());
        assert_eq!(check(r"shell:AppsFolder\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App").is_ok(), ALLOWS_WINDOWS_ONLY_TARGETS);
        assert!(check(r"shell:AppsFolder\..\cmd").is_err());
        assert_eq!(check("Notepad").is_ok(), ALLOWS_WINDOWS_ONLY_TARGETS);
        assert_eq!(check("ms-settings:display").is_ok(), ALLOWS_WINDOWS_ONLY_TARGETS);
        assert!(check("cmd").is_err());
        assert!(check("powershell").is_err());
        assert_eq!(check("spotify"), Ok(start_menu_directory.join("Spotify").join(format!("Spotify.{APP_SHORTCUT_FILE_EXTENSION}")).to_string_lossy().into_owned()));
        let clip_path = home_directory.join("Videos").join("clip.mp4");
        assert!(check(&clip_path.to_string_lossy()).is_ok());
        assert!(check(&home_directory.join("Videos").to_string_lossy()).is_ok());
        assert!(check(&home_directory.join("setup.EXE").to_string_lossy()).is_err());
        assert!(check(&test_directory.join("outside.txt").to_string_lossy()).is_err());
        assert!(check(&home_directory.join("Videos").join("..").join("..").join("outside.txt").to_string_lossy()).is_err());
        assert!(check("").is_err());
        let _ = std::fs::remove_dir_all(&test_directory);
    }

    #[test]
    fn desktop_entry_name_comes_from_the_desktop_entry_group() {
        let calculator_desktop_file = "[Desktop Entry]\nName=Calculator\nExec=gnome-calculator\n[Desktop Action new]\nName=New Window\n";
        assert_eq!(desktop_entry_name(calculator_desktop_file), Some("Calculator"));
        assert_eq!(desktop_entry_name("[Desktop Action x]\nName=Not the app\n"), None);
    }

    #[cfg(unix)]
    #[test]
    fn executable_files_in_home_are_refused_whatever_their_name() {
        use std::os::unix::fs::PermissionsExt;
        let home_directory = std::env::temp_dir().join(format!("hellobhai-open-exec-test-{}", std::process::id()));
        std::fs::create_dir_all(&home_directory).unwrap();
        let script_path = home_directory.join("notes");
        std::fs::write(&script_path, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(allowed_open_target(&script_path.to_string_lossy(), &home_directory, &[]).is_err());
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(allowed_open_target(&script_path.to_string_lossy(), &home_directory, &[]).is_ok());
        let _ = std::fs::remove_dir_all(&home_directory);
    }
}
