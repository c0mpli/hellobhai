//! The tray icon and the settings panel window (`ui/settings.html`), the Windows
//! counterpart of the Mac menu bar icon and `CompanionPanelView`.

use tauri::AppHandle;

/// Left click toggles the settings window near the tray; menu: "Quit hellobhai".
pub fn create_tray_icon(app: &AppHandle) -> tauri::Result<()> {
    todo!("D: TrayIconBuilder")
}

/// Label "settings", 340 px wide, no decorations, always on top, hides when it loses focus.
pub fn show_settings_window(app: &AppHandle) {
    todo!("D: create/show settings window above the taskbar")
}

pub fn hide_settings_window(app: &AppHandle) {
    todo!("D: hide settings window")
}
