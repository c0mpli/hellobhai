//! The real mouse pointer's state: the buddy hides while the pointer is hidden
//! (videos, games), and the 3D overlay skips hit-testing during drags.

use x11_dl::xlib;

use super::x11_connection::{is_wayland_session, with_x11_connection, xfixes};

/// X11 has no "cursor hidden" flag: players hide it by switching to an empty
/// cursor image, so the pointer counts as hidden when XFixes reports a cursor
/// with no visible pixel. Unknown (Wayland, no XFixes) means visible.
pub fn is_system_pointer_visible() -> bool {
    if is_wayland_session() {
        return true;
    }
    let Some(xfixes) = xfixes() else {
        return true;
    };
    with_x11_connection(|connection| {
        let cursor_image = unsafe { (xfixes.XFixesGetCursorImage)(connection.display) };
        if cursor_image.is_null() {
            return true;
        }
        let cursor = unsafe { &*cursor_image };
        let pixel_count = cursor.width as usize * cursor.height as usize;
        let pixels = if cursor.pixels.is_null() { &[][..] } else { unsafe { std::slice::from_raw_parts(cursor.pixels, pixel_count) } };
        let is_visible = cursor_has_visible_pixel(pixels);
        unsafe { (connection.xlib.XFree)(cursor_image as *mut std::ffi::c_void) };
        is_visible
    })
    .unwrap_or(true)
}

/// Pixels are ARGB in the low 32 bits of each C long.
fn cursor_has_visible_pixel(argb_pixels: &[std::os::raw::c_ulong]) -> bool {
    argb_pixels.iter().any(|pixel| (*pixel as u64 >> 24) & 0xff != 0)
}

/// XQueryPointer's button mask has Button1Mask set.
pub fn is_left_mouse_button_down() -> bool {
    with_x11_connection(|connection| {
        let mut root_return: xlib::Window = 0;
        let mut child_return: xlib::Window = 0;
        let (mut root_x, mut root_y, mut window_x, mut window_y) = (0, 0, 0, 0);
        let mut button_mask: std::os::raw::c_uint = 0;
        unsafe {
            (connection.xlib.XQueryPointer)(
                connection.display,
                connection.root_window,
                &mut root_return,
                &mut child_return,
                &mut root_x,
                &mut root_y,
                &mut window_x,
                &mut window_y,
                &mut button_mask,
            );
        }
        button_mask & xlib::Button1Mask != 0
    })
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_all_transparent_cursor_is_hidden() {
        assert!(!cursor_has_visible_pixel(&[]));
        assert!(!cursor_has_visible_pixel(&[0x00ff_ffff, 0x0000_0000]));
        assert!(cursor_has_visible_pixel(&[0x0000_0000, 0xff00_0000]));
    }
}
