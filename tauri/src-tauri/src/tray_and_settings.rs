//! The tray icon and the settings panel window (`ui/settings.html`), the Windows
//! counterpart of the Mac menu bar icon and `CompanionPanelView`.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, PhysicalPosition, WebviewWindow, WindowEvent};

use crate::page_bridge::{locked_down_window_builder, monitor_containing_point, SETTINGS_WINDOW_LABEL};
use crate::screen_mapping::{monitors_from_tauri, PhysicalRect};

const SETTINGS_WINDOW_LOGICAL_WIDTH: f64 = 340.0;
/// The page resizes the window to fit its content once it has rendered.
const SETTINGS_WINDOW_INITIAL_LOGICAL_HEIGHT: f64 = 560.0;
const QUIT_MENU_ITEM_ID: &str = "quit";

/// When the panel last hid because it lost focus. Clicking the tray icon first
/// takes focus away from the panel (hiding it); without this, the click itself
/// would then show it again instead of closing it.
static LAST_HIDDEN_BY_FOCUS_LOSS: Mutex<Option<Instant>> = Mutex::new(None);
const TRAY_CLICK_AFTER_FOCUS_LOSS_WINDOW: Duration = Duration::from_millis(300);

/// Left click toggles the settings window near the tray; menu: "Quit hellobhai".
pub fn create_tray_icon(app: &AppHandle) -> tauri::Result<()> {
    let quit_menu_item = MenuItem::with_id(app, QUIT_MENU_ITEM_ID, "Quit hellobhai", true, None::<&str>)?;
    let tray_menu = Menu::with_items(app, &[&quit_menu_item])?;

    let mut tray_icon_builder = TrayIconBuilder::with_id("hellobhai")
        .tooltip("hellobhai")
        .menu(&tray_menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, menu_event| {
            if menu_event.id().as_ref() == QUIT_MENU_ITEM_ID {
                app.exit(0);
            }
        })
        .on_tray_icon_event(|tray_icon, tray_icon_event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = tray_icon_event
            {
                toggle_settings_window(tray_icon.app_handle());
            }
        });
    if let Some(app_icon) = app.default_window_icon() {
        tray_icon_builder = tray_icon_builder.icon(app_icon.clone());
    }
    tray_icon_builder.build(app)?;
    Ok(())
}

fn toggle_settings_window(app: &AppHandle) {
    let was_just_hidden_by_this_click = LAST_HIDDEN_BY_FOCUS_LOSS
        .lock()
        .unwrap()
        .is_some_and(|hidden_at| hidden_at.elapsed() < TRAY_CLICK_AFTER_FOCUS_LOSS_WINDOW);
    let is_settings_window_visible = app
        .get_webview_window(SETTINGS_WINDOW_LABEL)
        .is_some_and(|settings_window| settings_window.is_visible().unwrap_or(false));
    if is_settings_window_visible {
        hide_settings_window(app);
    } else if !was_just_hidden_by_this_click {
        show_settings_window(app);
    }
}

/// Label "settings", 340 px wide, no decorations, always on top, hides when it loses focus.
pub fn show_settings_window(app: &AppHandle) {
    let settings_window = match app.get_webview_window(SETTINGS_WINDOW_LABEL) {
        Some(settings_window) => settings_window,
        None => match create_settings_window(app) {
            Some(settings_window) => settings_window,
            None => return,
        },
    };
    place_settings_window_above_taskbar(app, &settings_window);
    let _ = settings_window.show();
    let _ = settings_window.set_focus();
}

fn create_settings_window(app: &AppHandle) -> Option<WebviewWindow> {
    let build_result = locked_down_window_builder(app, SETTINGS_WINDOW_LABEL, "settings.html")
        .title("hellobhai")
        .inner_size(SETTINGS_WINDOW_LOGICAL_WIDTH, SETTINGS_WINDOW_INITIAL_LOGICAL_HEIGHT)
        // The page draws the dark rounded panel; the corners around it stay see-through.
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .build();
    let settings_window = match build_result {
        Ok(settings_window) => settings_window,
        Err(build_error) => {
            eprintln!("couldn't create the settings window: {build_error}");
            return None;
        }
    };

    let app_for_window_events = app.clone();
    settings_window.on_window_event(move |window_event| match window_event {
        // Like the Mac panel, clicking anywhere else dismisses it.
        WindowEvent::Focused(false) => {
            *LAST_HIDDEN_BY_FOCUS_LOSS.lock().unwrap() = Some(Instant::now());
            hide_settings_window(&app_for_window_events);
        }
        // The page fits the window to its content; keep it anchored above the taskbar.
        WindowEvent::Resized(_) => {
            if let Some(settings_window) = app_for_window_events.get_webview_window(SETTINGS_WINDOW_LABEL) {
                place_settings_window_above_taskbar(&app_for_window_events, &settings_window);
            }
        }
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            hide_settings_window(&app_for_window_events);
        }
        _ => {}
    });
    Some(settings_window)
}

pub fn hide_settings_window(app: &AppHandle) {
    if let Some(settings_window) = app.get_webview_window(SETTINGS_WINDOW_LABEL) {
        let _ = settings_window.hide();
    }
}

fn place_settings_window_above_taskbar(app: &AppHandle, settings_window: &WebviewWindow) {
    let Ok(cursor_position) = app.cursor_position() else { return };
    let monitors = monitors_from_tauri(app);
    let Some(cursor_monitor) = monitor_containing_point(&monitors, cursor_position.x, cursor_position.y) else { return };
    let Ok(settings_window_size) = settings_window.outer_size() else { return };
    let (settings_x, settings_y) = settings_window_origin_in_work_area_corner(
        cursor_monitor.work_area,
        (settings_window_size.width, settings_window_size.height),
        cursor_monitor.scale_factor,
    );
    let _ = settings_window.set_position(PhysicalPosition::new(settings_x, settings_y));
}

/// Bottom-right corner of the work area (just above the taskbar and its tray
/// icons), 12 logical px in from each edge; physical pixels.
pub fn settings_window_origin_in_work_area_corner(
    work_area: PhysicalRect,
    settings_window_size: (u32, u32),
    scale_factor: f64,
) -> (i32, i32) {
    let margin = (12.0 * scale_factor).round() as i32;
    let settings_x = work_area.x + work_area.width as i32 - settings_window_size.0 as i32 - margin;
    let settings_y = work_area.y + work_area.height as i32 - settings_window_size.1 as i32 - margin;
    (settings_x.max(work_area.x), settings_y.max(work_area.y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_panel_sits_in_the_bottom_right_corner_of_the_work_area() {
        let work_area = PhysicalRect { x: 0, y: 0, width: 1920, height: 1032 };
        assert_eq!(settings_window_origin_in_work_area_corner(work_area, (340, 560), 1.0), (1568, 460));
        assert_eq!(settings_window_origin_in_work_area_corner(work_area, (680, 1120), 2.0), (1216, 0));
        let left_monitor_work_area = PhysicalRect { x: -1920, y: 48, width: 1920, height: 1032 };
        assert_eq!(settings_window_origin_in_work_area_corner(left_monitor_work_area, (340, 560), 1.0), (-352, 508));
    }
}
