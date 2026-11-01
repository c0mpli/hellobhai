//! The floating whiteboard / notes window (`generated/board.html`,
//! `generated/notes.html`), mirror of the Mac's `VisualExplanationPanelManager`:
//! placed beside the cursor, command lines queued until the page has loaded.

use std::sync::{Arc, Mutex};

use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Manager, PhysicalPosition, WebviewWindow, WindowEvent};

use crate::page_bridge::{
    emit_pen_position, javascript_call_with_string_argument, locked_down_window_builder, monitor_containing_point,
    BOARD_WINDOW_LABEL, PAGE_HOST_BRIDGE_SCRIPT,
};
use crate::screen_mapping::{monitors_from_tauri, PhysicalRect};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveContentKind {
    Drawing,
    Notes,
}

impl LiveContentKind {
    fn page_file_name(self) -> &'static str {
        match self {
            LiveContentKind::Drawing => "board.html",
            LiveContentKind::Notes => "notes.html",
        }
    }
}

/// Logical size of a new board window, as on the Mac.
const BOARD_WINDOW_LOGICAL_WIDTH: f64 = 760.0;
const BOARD_WINDOW_LOGICAL_HEIGHT: f64 = 580.0;
/// The canvas refits to any shape, but keep it usable.
const BOARD_WINDOW_MINIMUM_LOGICAL_WIDTH: f64 = 360.0;
const BOARD_WINDOW_MINIMUM_LOGICAL_HEIGHT: f64 = 260.0;
/// Cream paper (the page's background), so resizing never flashes white.
const BOARD_PAPER_COLOR: tauri::webview::Color = tauri::webview::Color(248, 245, 236, 255);

/// One queued item: a command line, or the "every step is shown" marker.
enum PendingBoardCall {
    AddCommandLine(String),
    Done,
}

#[derive(Default)]
struct BoardPanelState {
    window: Option<WebviewWindow>,
    /// The page the window was last sent to, so a late load of the previous page
    /// can't be mistaken for it.
    expected_page_file_name: Option<&'static str>,
    is_page_ready: bool,
    pending_calls: Vec<PendingBoardCall>,
}

/// Pending lines queue, page-ready flag and finished marker.
#[derive(Default)]
pub struct BoardPanel {
    state: Arc<Mutex<BoardPanelState>>,
}

impl BoardPanel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates or shows window "board" beside the cursor and loads the board or notes page.
    pub fn begin_live_content(&self, app: &AppHandle, kind: LiveContentKind) {
        let page_file_name = kind.page_file_name();
        let existing_window = {
            let mut board_state = self.state.lock().unwrap();
            board_state.is_page_ready = false;
            board_state.pending_calls.clear();
            board_state.expected_page_file_name = Some(page_file_name);
            board_state.window.clone()
        };

        let board_window = match existing_window {
            Some(board_window) => {
                let next_page_url = board_window.url().ok().and_then(|current_url| current_url.join(page_file_name).ok());
                match next_page_url {
                    Some(next_page_url) => {
                        let _ = board_window.navigate(next_page_url);
                    }
                    None => eprintln!("board: couldn't build the {page_file_name} url"),
                }
                board_window
            }
            None => match self.create_board_window(app, page_file_name) {
                Some(board_window) => board_window,
                None => return,
            },
        };

        // Re-placed beside the cursor for every new visual, since that's where the user is looking.
        place_board_window_beside_cursor(app, &board_window);
        let _ = board_window.show();
    }

    fn create_board_window(&self, app: &AppHandle, page_file_name: &str) -> Option<WebviewWindow> {
        let state_for_page_load = self.state.clone();
        let state_for_window_events = self.state.clone();
        let app_for_window_events = app.clone();
        let build_result = locked_down_window_builder(app, BOARD_WINDOW_LABEL, page_file_name)
            .title("bhai")
            .inner_size(BOARD_WINDOW_LOGICAL_WIDTH, BOARD_WINDOW_LOGICAL_HEIGHT)
            .min_inner_size(BOARD_WINDOW_MINIMUM_LOGICAL_WIDTH, BOARD_WINDOW_MINIMUM_LOGICAL_HEIGHT)
            .background_color(BOARD_PAPER_COLOR)
            // Native frame: move, resize and close for free; never steals focus from the user's app.
            .decorations(true)
            .focusable(false)
            .always_on_top(true)
            // Nothing drawn here should ever be persisted to disk.
            .incognito(true)
            .initialization_script(PAGE_HOST_BRIDGE_SCRIPT)
            .on_page_load(move |board_window, page_load_payload| {
                if page_load_payload.event() != PageLoadEvent::Finished {
                    return;
                }
                let loaded_page_path = page_load_payload.url().path().to_string();
                let queued_calls = {
                    let mut board_state = state_for_page_load.lock().unwrap();
                    let is_expected_page = board_state
                        .expected_page_file_name
                        .is_some_and(|expected_page_file_name| loaded_page_path.ends_with(expected_page_file_name));
                    if !is_expected_page {
                        return;
                    }
                    board_state.is_page_ready = true;
                    std::mem::take(&mut board_state.pending_calls)
                };
                for queued_call in queued_calls {
                    run_board_call(&board_window, &queued_call);
                }
            })
            .build();

        let board_window = match build_result {
            Ok(board_window) => board_window,
            Err(build_error) => {
                eprintln!("couldn't create the board window: {build_error}");
                return None;
            }
        };

        // The close button hides the board instead of destroying it, so the next visual reuses it.
        board_window.on_window_event(move |window_event| {
            if let WindowEvent::CloseRequested { api, .. } = window_event {
                api.prevent_close();
                hide_board(&state_for_window_events, &app_for_window_events);
            }
        });

        self.state.lock().unwrap().window = Some(board_window.clone());
        Some(board_window)
    }

    /// Queued until the page has loaded, then `hb.add("<json string>")`.
    pub fn add_command_line(&self, line: String) {
        self.run_or_queue(PendingBoardCall::AddCommandLine(line));
    }

    /// `hb.done()` (queued as a marker before the page has loaded).
    pub fn finish_live_content(&self) {
        self.run_or_queue(PendingBoardCall::Done);
    }

    fn run_or_queue(&self, board_call: PendingBoardCall) {
        let mut board_state = self.state.lock().unwrap();
        match (&board_state.window, board_state.is_page_ready) {
            (Some(board_window), true) => run_board_call(board_window, &board_call),
            _ => board_state.pending_calls.push(board_call),
        }
    }

    /// `hb.stopDrawing()`: reveals everything at once and cuts the scratch sound.
    pub fn stop_drawing(&self) {
        stop_board_drawing(&self.state);
    }

    /// stop_drawing, hide, pen-position null.
    pub fn close(&self) {
        let Some(board_window) = self.state.lock().unwrap().window.clone() else { return };
        hide_board(&self.state, board_window.app_handle());
    }
}

fn run_board_call(board_window: &WebviewWindow, board_call: &PendingBoardCall) {
    let javascript = match board_call {
        PendingBoardCall::AddCommandLine(command_line) => javascript_call_with_string_argument("hb.add", command_line),
        PendingBoardCall::Done => "hb.done()".to_string(),
    };
    let _ = board_window.eval(javascript);
}

fn stop_board_drawing(state: &Mutex<BoardPanelState>) {
    let board_state = state.lock().unwrap();
    if let (Some(board_window), true) = (&board_state.window, board_state.is_page_ready) {
        let _ = board_window.eval("hb.stopDrawing()");
    }
}

fn hide_board(state: &Mutex<BoardPanelState>, app: &AppHandle) {
    // Stop the pen first: a hidden page may stop animating mid-scratch.
    stop_board_drawing(state);
    if let Some(board_window) = &state.lock().unwrap().window {
        crate::screen_capture::hide_window(board_window);
    }
    emit_pen_position(app, None);
}

fn place_board_window_beside_cursor(app: &AppHandle, board_window: &WebviewWindow) {
    let Ok(cursor_position) = app.cursor_position() else { return };
    let monitors = monitors_from_tauri(app);
    let Some(cursor_monitor) = monitor_containing_point(&monitors, cursor_position.x, cursor_position.y) else { return };
    let (Ok(board_size), Ok(current_scale_factor)) = (board_window.outer_size(), board_window.scale_factor()) else { return };
    // outer_size is in the board's current monitor's pixels; Windows rescales the
    // board when it moves to a monitor with another DPI, so place it at that size.
    let scale_change = cursor_monitor.scale_factor / current_scale_factor.max(0.1);
    let board_size_on_cursor_monitor = (
        (board_size.width as f64 * scale_change).round() as u32,
        (board_size.height as f64 * scale_change).round() as u32,
    );
    let (board_x, board_y) = board_origin_beside_cursor(
        (cursor_position.x, cursor_position.y),
        board_size_on_cursor_monitor,
        cursor_monitor.work_area,
        cursor_monitor.scale_factor,
    );
    let _ = board_window.set_position(PhysicalPosition::new(board_x, board_y));
}

/// Port of `panelOriginNearCursor`: 40 px right of the cursor and vertically
/// centred on it, flipped to the cursor's left only when it doesn't fit, kept
/// 12 px inside the cursor monitor's work area. All physical pixels; the 40 and
/// 12 are logical, scaled by the monitor.
pub fn board_origin_beside_cursor(
    cursor: (f64, f64),
    board_size: (u32, u32),
    work_area: PhysicalRect,
    scale_factor: f64,
) -> (i32, i32) {
    let gap_from_cursor = 40.0 * scale_factor;
    let margin_inside_work_area = 12.0 * scale_factor;
    let (board_width, board_height) = (board_size.0 as f64, board_size.1 as f64);
    let work_area_left = work_area.x as f64;
    let work_area_top = work_area.y as f64;
    let work_area_right = work_area_left + work_area.width as f64;
    let work_area_bottom = work_area_top + work_area.height as f64;

    let fits_right_of_cursor = cursor.0 + gap_from_cursor + board_width <= work_area_right - margin_inside_work_area;
    let unclamped_x = if fits_right_of_cursor {
        cursor.0 + gap_from_cursor
    } else {
        cursor.0 - gap_from_cursor - board_width
    };
    let unclamped_y = cursor.1 - board_height / 2.0;

    // max then min (not clamp): a board larger than the work area pins to its top-left.
    let board_x = unclamped_x
        .min(work_area_right - board_width - margin_inside_work_area)
        .max(work_area_left + margin_inside_work_area);
    let board_y = unclamped_y
        .min(work_area_bottom - board_height - margin_inside_work_area)
        .max(work_area_top + margin_inside_work_area);
    (board_x.round() as i32, board_y.round() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK_AREA: PhysicalRect = PhysicalRect { x: 0, y: 0, width: 1920, height: 1040 };

    #[test]
    fn the_board_sits_right_of_the_cursor_when_it_fits() {
        assert_eq!(board_origin_beside_cursor((200.0, 500.0), (760, 580), WORK_AREA, 1.0), (240, 210));
    }

    #[test]
    fn the_board_flips_left_of_the_cursor_near_the_right_edge() {
        assert_eq!(board_origin_beside_cursor((1800.0, 500.0), (760, 580), WORK_AREA, 1.0), (1000, 210));
    }

    #[test]
    fn the_board_stays_inside_the_work_area_and_scales_its_margins() {
        // Near the top: pinned 12 px (24 at 200%) below the work area's top.
        assert_eq!(board_origin_beside_cursor((200.0, 10.0), (760, 580), WORK_AREA, 2.0), (280, 24));
        // Near the taskbar: pinned above it.
        assert_eq!(board_origin_beside_cursor((200.0, 1030.0), (760, 580), WORK_AREA, 1.0), (240, 448));
        // A second monitor to the left (negative coordinates).
        let left_monitor_work_area = PhysicalRect { x: -1920, y: 0, width: 1920, height: 1040 };
        assert_eq!(board_origin_beside_cursor((-100.0, 500.0), (760, 580), left_monitor_work_area, 1.0), (-900, 210));
    }
}
