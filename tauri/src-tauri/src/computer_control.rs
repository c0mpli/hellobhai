//! Replays [CLICK]/[TYPE]/[KEY] actions as synthetic input (thin async wrappers
//! around `platform` input calls via `spawn_blocking`, logging like the Mac).

/// Global physical pixels.
pub async fn perform_click(x: i32, y: i32) {
    todo!("C: platform::click_at_global_point")
}

pub async fn perform_type(text: &str) {
    todo!("C: platform::type_text")
}

/// false when the combo could not be parsed or sent.
pub async fn perform_key_combo(key_combo: &str) -> bool {
    todo!("C: parse_key_combo + platform::press_key_combo")
}
