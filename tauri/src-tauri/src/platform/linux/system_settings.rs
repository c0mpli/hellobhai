//! Opens things for the user: the sound input settings from the settings panel,
//! and the targets of Claude's `[OPEN]` tags (xdg-open, or gtk-launch for an
//! installed app's .desktop file). Also the settings panel's note about what
//! Wayland limits.

use std::path::Path;
use std::process::{Command, Stdio};

use super::x11_connection::is_wayland_session;
use crate::platform::PlatformError;

/// The desktop's sound settings (input device and level live there; Linux has
/// no per-app microphone switch outside Flatpak): GNOME, KDE, then pavucontrol.
pub fn open_microphone_privacy_settings() -> Result<(), PlatformError> {
    let sound_settings_commands: [&[&str]; 4] = [
        &["gnome-control-center", "sound"],
        &["systemsettings", "kcm_pulseaudio"],
        &["systemsettings5", "kcm_pulseaudio"],
        &["pavucontrol", "--tab=4"],
    ];
    for sound_settings_command in sound_settings_commands {
        if spawn_detached(sound_settings_command[0], &sound_settings_command[1..]).is_ok() {
            return Ok(());
        }
    }
    Err(PlatformError::Failed("no sound settings app found (GNOME Settings, KDE System Settings or pavucontrol)".into()))
}

/// Opens a target `computer_control::allowed_open_target` already checked: an
/// installed app's .desktop file is launched (gtk-launch, then gio), anything
/// else (link, file, folder) goes to xdg-open, which picks the default app.
pub fn open_with_shell(checked_target: &str) -> Result<(), PlatformError> {
    if let Some(desktop_file_id) = desktop_file_id_for_launch(checked_target) {
        if spawn_detached("gtk-launch", &[desktop_file_id.as_str()]).is_ok() {
            return Ok(());
        }
        return spawn_detached("gio", &["launch", checked_target]);
    }
    spawn_detached("xdg-open", &[checked_target])
}

/// `/usr/share/applications/org.gnome.Calculator.desktop` -> `org.gnome.Calculator`
/// (the id gtk-launch takes); None for anything that isn't a .desktop file.
fn desktop_file_id_for_launch(checked_target: &str) -> Option<String> {
    let target_path = Path::new(checked_target);
    if !target_path.is_absolute() || target_path.extension().is_none_or(|extension| extension != "desktop") {
        return None;
    }
    target_path.file_stem().map(|file_stem| file_stem.to_string_lossy().into_owned())
}

/// Starts the program without waiting (xdg-open and gtk-launch hand off and
/// exit; their exit codes are not worth blocking for), reaping it on a thread.
fn spawn_detached(program: &str, arguments: &[&str]) -> Result<(), PlatformError> {
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|spawn_error| PlatformError::Failed(format!("couldn't start {program}: {spawn_error}")))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// A sentence for the settings panel when this desktop limits what bhai can do
/// (Wayland); None on X11, where everything works.
pub fn platform_limitation_note() -> Option<String> {
    if !is_wayland_session() {
        return None;
    }
    let keyboard_note = if super::keyboard_hook::keyboard_hook_reads_input_devices() {
        "The shortcut works everywhere."
    } else {
        "The shortcut only works while an X11 app is focused until you add yourself to the input group (sudo usermod -aG input $USER, then log out and back in)."
    };
    Some(format!(
        "You're on Wayland, so bhai runs through XWayland: it can only click and type in X11 apps, it can't point where the mouse is over Wayland apps, and screenshots go through your desktop, which may ask for permission. {keyboard_note} For everything, pick an Xorg session at login."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_files_launch_by_id_and_everything_else_by_xdg_open() {
        assert_eq!(desktop_file_id_for_launch("/usr/share/applications/org.gnome.Calculator.desktop"), Some("org.gnome.Calculator".into()));
        assert_eq!(desktop_file_id_for_launch("https://youtube.com"), None);
        assert_eq!(desktop_file_id_for_launch("/home/bhai/Videos/clip.mp4"), None);
        assert_eq!(desktop_file_id_for_launch("spotify.desktop"), None);
    }
}
