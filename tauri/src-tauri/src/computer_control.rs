//! Replays [CLICK]/[TYPE]/[KEY] actions as synthetic input (thin async wrappers
//! around `platform` input calls via `spawn_blocking`, logging like the Mac).
//! The buddy's flight 1.2 s before a click, and cancelling the remaining
//! actions on the hotkey, live in the brain's agent loop.

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

pub async fn perform_type(text: &str) {
    let text_to_type = text.to_string();
    let typing_result = tauri::async_runtime::spawn_blocking(move || platform::type_text(&text_to_type)).await;
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
