//! The floating whiteboard / notes window (`generated/board.html`,
//! `generated/notes.html`), mirror of the Mac's `VisualExplanationPanelManager`:
//! placed beside the cursor, command lines queued until the page has loaded.

use tauri::AppHandle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveContentKind {
    Drawing,
    Notes,
}

/// Pending lines queue, page-ready flag and finished marker.
#[derive(Default)]
pub struct BoardPanel {}

impl BoardPanel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates or shows window "board" beside the cursor and loads the board or notes page.
    pub fn begin_live_content(&self, app: &AppHandle, kind: LiveContentKind) {
        todo!("D: create/show board window")
    }

    /// Queued until the page has loaded, then `hb.add("<json string>")`.
    pub fn add_command_line(&self, line: String) {
        todo!("D: queue or eval")
    }

    /// `hb.done()` (queued as a marker before the page has loaded).
    pub fn finish_live_content(&self) {
        todo!("D: hb.done()")
    }

    /// `hb.stopDrawing()`: reveals everything at once and cuts the scratch sound.
    pub fn stop_drawing(&self) {
        todo!("D: hb.stopDrawing()")
    }

    /// stop_drawing, hide, pen-position null.
    pub fn close(&self) {
        todo!("D: hide board")
    }
}
