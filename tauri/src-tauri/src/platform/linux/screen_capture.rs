//! Screen capture. X11: XGetImage on the root window (global physical pixels,
//! the composited screen on a compositing window manager). Wayland: X11 apps
//! can't see other apps' windows, so the desktop's screenshot portal
//! (org.freedesktop.portal.Screenshot) is asked instead; the desktop may show a
//! permission prompt the first time.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use x11_dl::xlib;

use super::x11_connection::{is_wayland_session, with_x11_connection};
use crate::platform::PlatformError;
use crate::screen_mapping::PhysicalRect;

/// X11 and the portal both capture our own windows (no equivalent of Windows'
/// WDA_EXCLUDEFROMCAPTURE), so the shared capture code has to hide them.
pub const CAPTURE_EXCLUDES_OWN_WINDOWS: bool = false;

/// (width, height, RGBA bytes) of `global_rect` in root window pixels.
pub fn capture_screen_rect_rgba(global_rect: PhysicalRect) -> Result<(u32, u32, Vec<u8>), PlatformError> {
    if global_rect.width == 0 || global_rect.height == 0 {
        return Err(PlatformError::Failed("empty capture rect".to_string()));
    }
    if is_wayland_session() {
        return capture_with_screenshot_portal(global_rect);
    }
    capture_with_x11(global_rect)
}

fn capture_with_x11(global_rect: PhysicalRect) -> Result<(u32, u32, Vec<u8>), PlatformError> {
    with_x11_connection(|connection| {
        let xlib = connection.xlib;
        // XGetImage fails with BadMatch for any part outside the root window, so clamp first.
        let default_screen = unsafe { (xlib.XDefaultScreen)(connection.display) };
        let root_width = unsafe { (xlib.XDisplayWidth)(connection.display, default_screen) };
        let root_height = unsafe { (xlib.XDisplayHeight)(connection.display, default_screen) };
        let clamped_rect = clamp_rect_to_root(global_rect, root_width, root_height)
            .ok_or_else(|| PlatformError::Failed("capture rect is outside the screen".to_string()))?;

        let captured_image = unsafe {
            (xlib.XGetImage)(
                connection.display,
                connection.root_window,
                clamped_rect.x,
                clamped_rect.y,
                clamped_rect.width,
                clamped_rect.height,
                (xlib.XAllPlanes)(),
                xlib::ZPixmap,
            )
        };
        if captured_image.is_null() {
            return Err(PlatformError::Failed("XGetImage failed".to_string()));
        }
        let image = unsafe { &*captured_image };
        let rgba_result = if image.bits_per_pixel == 32 && image.red_mask == 0xff0000 && image.blue_mask == 0xff {
            let row_bytes = unsafe {
                std::slice::from_raw_parts(image.data as *const u8, image.bytes_per_line as usize * image.height as usize)
            };
            Ok(bgrx_rows_to_rgba(row_bytes, image.width as usize, image.height as usize, image.bytes_per_line as usize))
        } else {
            Err(PlatformError::Failed(format!("unsupported screen format ({} bits per pixel)", image.bits_per_pixel)))
        };
        let (width, height) = (image.width as u32, image.height as u32);
        unsafe { (xlib.XDestroyImage)(captured_image) };
        rgba_result.map(|rgba_bytes| (width, height, rgba_bytes))
    })?
}

/// The part of `rect` inside a root window of `root_width` x `root_height`, as (x, y, width, height).
struct ClampedRect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

fn clamp_rect_to_root(rect: PhysicalRect, root_width: i32, root_height: i32) -> Option<ClampedRect> {
    let left = rect.x.max(0);
    let top = rect.y.max(0);
    let right = (rect.x + rect.width as i32).min(root_width);
    let bottom = (rect.y + rect.height as i32).min(root_height);
    if right <= left || bottom <= top {
        return None;
    }
    Some(ClampedRect { x: left, y: top, width: (right - left) as u32, height: (bottom - top) as u32 })
}

/// Little-endian 32-bit TrueColor rows (B, G, R, X) to tightly packed RGBA.
fn bgrx_rows_to_rgba(row_bytes: &[u8], width: usize, height: usize, bytes_per_row: usize) -> Vec<u8> {
    let mut rgba_bytes = Vec::with_capacity(width * height * 4);
    for row_index in 0..height {
        let row = &row_bytes[row_index * bytes_per_row..row_index * bytes_per_row + width * 4];
        for pixel in row.chunks_exact(4) {
            rgba_bytes.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
        }
    }
    rgba_bytes
}

// MARK: - Wayland screenshot portal

/// The whole desktop from the last portal screenshot. A capture round asks for
/// every monitor (and region crops) within a moment, so one portal request
/// serves them all instead of prompting once per monitor.
struct PortalScreenshot {
    taken_at: Instant,
    width: u32,
    height: u32,
    rgba_bytes: Vec<u8>,
}

static LAST_PORTAL_SCREENSHOT: Mutex<Option<PortalScreenshot>> = Mutex::new(None);
const PORTAL_SCREENSHOT_REUSE_WINDOW: Duration = Duration::from_millis(1500);

fn capture_with_screenshot_portal(global_rect: PhysicalRect) -> Result<(u32, u32, Vec<u8>), PlatformError> {
    let mut last_portal_screenshot = LAST_PORTAL_SCREENSHOT.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let is_recent = last_portal_screenshot.as_ref().is_some_and(|screenshot| screenshot.taken_at.elapsed() < PORTAL_SCREENSHOT_REUSE_WINDOW);
    if !is_recent {
        *last_portal_screenshot = Some(take_portal_screenshot()?);
    }
    let screenshot = last_portal_screenshot.as_ref().expect("screenshot was just taken");

    // The portal image is in the compositor's pixels; XWayland's root window may
    // be scaled differently (fractional scaling), so map through the ratio.
    let (root_width, root_height) =
        with_x11_connection(|connection| unsafe {
            let default_screen = (connection.xlib.XDefaultScreen)(connection.display);
            ((connection.xlib.XDisplayWidth)(connection.display, default_screen), (connection.xlib.XDisplayHeight)(connection.display, default_screen))
        })
        .unwrap_or((screenshot.width as i32, screenshot.height as i32));
    let crop_rect = scale_rect_into_image(global_rect, root_width, root_height, screenshot.width, screenshot.height)
        .ok_or_else(|| PlatformError::Failed("capture rect is outside the screenshot".to_string()))?;
    let mut cropped_rgba_bytes = Vec::with_capacity(crop_rect.width as usize * crop_rect.height as usize * 4);
    for row_index in crop_rect.y as usize..(crop_rect.y as usize + crop_rect.height as usize) {
        let row_start = (row_index * screenshot.width as usize + crop_rect.x as usize) * 4;
        cropped_rgba_bytes.extend_from_slice(&screenshot.rgba_bytes[row_start..row_start + crop_rect.width as usize * 4]);
    }
    Ok((crop_rect.width, crop_rect.height, cropped_rgba_bytes))
}

/// `rect` in a root window of `root_width` x `root_height` -> the same area in
/// an image of `image_width` x `image_height`, clamped to the image.
fn scale_rect_into_image(rect: PhysicalRect, root_width: i32, root_height: i32, image_width: u32, image_height: u32) -> Option<ClampedRect> {
    if root_width <= 0 || root_height <= 0 {
        return None;
    }
    let horizontal_scale = image_width as f64 / root_width as f64;
    let vertical_scale = image_height as f64 / root_height as f64;
    let scaled_rect = PhysicalRect {
        x: (rect.x as f64 * horizontal_scale).round() as i32,
        y: (rect.y as f64 * vertical_scale).round() as i32,
        width: (rect.width as f64 * horizontal_scale).round() as u32,
        height: (rect.height as f64 * vertical_scale).round() as u32,
    };
    clamp_rect_to_root(scaled_rect, image_width as i32, image_height as i32)
}

/// Screenshot(parent_window "", {handle_token, interactive: false}), then the
/// Request's Response signal carries a file:// URI of a PNG, which is read and
/// deleted (the desktop saves it into the user's Pictures folder).
fn take_portal_screenshot() -> Result<PortalScreenshot, PlatformError> {
    let (result_sender, result_receiver) = std::sync::mpsc::channel();
    // On its own thread so a portal that never answers can be given up on.
    std::thread::Builder::new()
        .name("screenshot-portal".into())
        .spawn(move || {
            let _ = result_sender.send(request_portal_screenshot_uri());
        })
        .map_err(|spawn_error| PlatformError::Failed(format!("cannot start the screenshot request: {spawn_error}")))?;
    // Long enough for the user to answer a first-time permission prompt.
    let screenshot_uri = result_receiver
        .recv_timeout(Duration::from_secs(30))
        .map_err(|_| PlatformError::Failed("the screenshot portal did not answer".into()))??;
    let screenshot_path = file_path_from_file_uri(&screenshot_uri)
        .ok_or_else(|| PlatformError::Failed(format!("unexpected screenshot location {screenshot_uri}")))?;
    let decode_result = decode_png_file_to_rgba(&screenshot_path);
    let _ = std::fs::remove_file(&screenshot_path);
    let (width, height, rgba_bytes) = decode_result?;
    Ok(PortalScreenshot { taken_at: Instant::now(), width, height, rgba_bytes })
}

fn request_portal_screenshot_uri() -> Result<String, PlatformError> {
    use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
    let portal_error = |error: zbus::Error| PlatformError::Failed(format!("screenshot portal: {error}"));

    let session_bus = zbus::blocking::Connection::session().map_err(portal_error)?;
    let unique_bus_name = session_bus.unique_name().map(|name| name.to_string()).unwrap_or_default();
    // Unique per request, as the portal asks.
    static REQUEST_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let request_number = REQUEST_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let request_token = format!("hellobhai{}_{request_number}", std::process::id());
    let request_path = portal_request_path(&unique_bus_name, &request_token);

    // Subscribe before calling, so a fast answer can't be missed.
    let request_proxy = zbus::blocking::Proxy::new(
        &session_bus,
        "org.freedesktop.portal.Desktop",
        request_path.as_str(),
        "org.freedesktop.portal.Request",
    )
    .map_err(portal_error)?;
    let mut responses = request_proxy.receive_signal("Response").map_err(portal_error)?;

    let screenshot_proxy = zbus::blocking::Proxy::new(
        &session_bus,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Screenshot",
    )
    .map_err(portal_error)?;
    let mut screenshot_options: HashMap<&str, Value> = HashMap::new();
    screenshot_options.insert("handle_token", Value::from(request_token.as_str()));
    screenshot_options.insert("interactive", Value::from(false));
    let _request_handle: OwnedObjectPath = screenshot_proxy.call("Screenshot", &("", screenshot_options)).map_err(portal_error)?;

    let response_message = responses.next().ok_or_else(|| PlatformError::Failed("screenshot portal closed".into()))?;
    let (response_code, response_results): (u32, HashMap<String, OwnedValue>) =
        response_message.body().deserialize().map_err(portal_error)?;
    if response_code != 0 {
        return Err(PlatformError::Failed("the screenshot was not allowed".into()));
    }
    response_results
        .get("uri")
        .and_then(|uri_value| String::try_from(uri_value.clone()).ok())
        .ok_or_else(|| PlatformError::Failed("the screenshot portal returned no file".into()))
}

/// /org/freedesktop/portal/desktop/request/SENDER/TOKEN, where SENDER is the
/// unique bus name without ':' and with '.' replaced by '_'.
fn portal_request_path(unique_bus_name: &str, request_token: &str) -> String {
    let sender_part = unique_bus_name.trim_start_matches(':').replace('.', "_");
    format!("/org/freedesktop/portal/desktop/request/{sender_part}/{request_token}")
}

/// file:///home/a/Pictures/Screenshot%20from.png -> /home/a/Pictures/Screenshot from.png
fn file_path_from_file_uri(file_uri: &str) -> Option<std::path::PathBuf> {
    let encoded_path = file_uri.strip_prefix("file://")?;
    let mut decoded_bytes = Vec::with_capacity(encoded_path.len());
    let encoded_bytes = encoded_path.as_bytes();
    let mut byte_index = 0;
    while byte_index < encoded_bytes.len() {
        if encoded_bytes[byte_index] == b'%' && byte_index + 2 < encoded_bytes.len() {
            let hex_digits = std::str::from_utf8(&encoded_bytes[byte_index + 1..byte_index + 3]).ok()?;
            decoded_bytes.push(u8::from_str_radix(hex_digits, 16).ok()?);
            byte_index += 3;
        } else {
            decoded_bytes.push(encoded_bytes[byte_index]);
            byte_index += 1;
        }
    }
    let decoded_path = std::path::PathBuf::from(String::from_utf8(decoded_bytes).ok()?);
    decoded_path.is_absolute().then_some(decoded_path)
}

fn decode_png_file_to_rgba(png_path: &std::path::Path) -> Result<(u32, u32, Vec<u8>), PlatformError> {
    let decode_error = |error: png::DecodingError| PlatformError::Failed(format!("cannot read the screenshot: {error}"));
    let png_file = std::fs::File::open(png_path).map_err(|error| PlatformError::Failed(format!("cannot open the screenshot: {error}")))?;
    let mut png_decoder = png::Decoder::new(std::io::BufReader::new(png_file));
    // 8-bit samples, palettes expanded.
    png_decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut png_reader = png_decoder.read_info().map_err(decode_error)?;
    let mut pixel_buffer = vec![0u8; png_reader.output_buffer_size().unwrap_or(0)];
    let frame_info = png_reader.next_frame(&mut pixel_buffer).map_err(decode_error)?;
    pixel_buffer.truncate(frame_info.buffer_size());
    let rgba_bytes = match frame_info.color_type {
        png::ColorType::Rgba => pixel_buffer,
        png::ColorType::Rgb => pixel_buffer.chunks_exact(3).flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => pixel_buffer.chunks_exact(2).flat_map(|pixel| [pixel[0], pixel[0], pixel[0], 255]).collect(),
        png::ColorType::Grayscale => pixel_buffer.iter().flat_map(|gray| [*gray, *gray, *gray, 255]).collect(),
        png::ColorType::Indexed => return Err(PlatformError::Failed("unexpected palette screenshot".into())),
    };
    Ok((frame_info.width, frame_info.height, rgba_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rects_are_clamped_to_the_root_window() {
        let clamped = clamp_rect_to_root(PhysicalRect { x: -10, y: 5, width: 100, height: 2000 }, 1920, 1080).unwrap();
        assert_eq!((clamped.x, clamped.y, clamped.width, clamped.height), (0, 5, 90, 1075));
        assert!(clamp_rect_to_root(PhysicalRect { x: 3000, y: 0, width: 10, height: 10 }, 1920, 1080).is_none());
    }

    #[test]
    fn bgrx_rows_with_padding_become_rgba() {
        // Two pixels per row, 12 bytes per row (4 bytes of padding).
        let rows = [1, 2, 3, 0, 4, 5, 6, 0, 9, 9, 9, 9, 7, 8, 9, 0, 10, 11, 12, 0, 9, 9, 9, 9];
        assert_eq!(bgrx_rows_to_rgba(&rows, 2, 2, 12), vec![3, 2, 1, 255, 6, 5, 4, 255, 9, 8, 7, 255, 12, 11, 10, 255]);
    }

    #[test]
    fn rects_scale_into_a_hidpi_portal_image() {
        let scaled = scale_rect_into_image(PhysicalRect { x: 960, y: 0, width: 960, height: 1080 }, 1920, 1080, 3840, 2160).unwrap();
        assert_eq!((scaled.x, scaled.y, scaled.width, scaled.height), (1920, 0, 1920, 2160));
    }

    #[test]
    fn portal_paths_and_uris() {
        assert_eq!(portal_request_path(":1.42", "hellobhai7"), "/org/freedesktop/portal/desktop/request/1_42/hellobhai7");
        assert_eq!(
            file_path_from_file_uri("file:///home/bhai/Pictures/Screenshot%20from%202026.png"),
            Some(std::path::PathBuf::from("/home/bhai/Pictures/Screenshot from 2026.png"))
        );
        assert_eq!(file_path_from_file_uri("https://example.com/a.png"), None);
        assert_eq!(file_path_from_file_uri("file:///bad%zz"), None);
    }
}
