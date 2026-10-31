//! Parses the tags Claude appends to a reply ([CLICK]/[TYPE]/[KEY], [CLOSE_DRAWING],
//! [SHOW], [NOTES], [MODEL3D], [CIRCLE], [POINT]) in the Mac's order. Coordinates
//! stay in screenshot pixels here; `screen_mapping` turns them into global ones.
//! Screen numbers are 1-based; `None` means the cursor screen.

#[derive(Clone, Debug, PartialEq)]
pub enum ComputerAction {
    Click {
        screenshot_x: f64,
        screenshot_y: f64,
        label: Option<String>,
        screen_number: Option<usize>,
    },
    Type {
        text: String,
    },
    PressKey {
        key_combo: String,
    },
}

impl ComputerAction {
    /// "clicked <label|something>", "typed \"…\"", "pressed <combo>" (for history and loop detection).
    pub fn summary(&self) -> String {
        todo!("E: copy the Mac action summaries")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Model3DRequest {
    pub description: String,
    pub screenshot_point: Option<(f64, f64)>,
    pub screen_number: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CircleTag {
    /// Top-left corner (converted from the centre Claude writes).
    pub screenshot_x: f64,
    pub screenshot_y: f64,
    pub width: f64,
    pub height: f64,
    pub label: Option<String>,
    pub screen_number: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PointTag {
    /// The reply text before the POINT tag (the whole text when there is no tag).
    pub spoken_text: String,
    pub point: Option<(f64, f64)>,
    pub label: Option<String>,
    pub screen_number: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedReply {
    pub spoken_text: String,
    pub actions: Vec<ComputerAction>,
    pub close_drawing: bool,
    pub show_request: Option<String>,
    pub notes_request: Option<String>,
    pub model3d_request: Option<Model3DRequest>,
    pub circles: Vec<CircleTag>,
    pub point: PointTag,
}

/// Removes every [CLICK]/[TYPE]/[KEY] tag; returns the remaining text and the actions in order.
pub fn extract_computer_actions(text: &str) -> (String, Vec<ComputerAction>) {
    todo!("E: plan section 9 step 1")
}

/// Removes every [CLOSE_DRAWING]; the bool says whether there was one.
pub fn remove_close_drawing(text: &str) -> (String, bool) {
    todo!("E: plan section 9 step 2")
}

/// First `[tag_name:...]` removed; returns the remaining text and the trimmed non-empty payload.
pub fn extract_tagged_request(tag_name: &str, text: &str) -> (String, Option<String>) {
    todo!("E: plan section 9 step 3")
}

pub fn parse_model3d_request(request: &str) -> Model3DRequest {
    todo!("E: plan section 9 step 4")
}

pub fn extract_circle_tags(text: &str) -> (String, Vec<CircleTag>) {
    todo!("E: plan section 9 step 5")
}

pub fn parse_point_tag(text: &str) -> PointTag {
    todo!("E: plan section 9 step 6")
}

/// Mac order: actions, CLOSE_DRAWING, SHOW, NOTES, MODEL3D, CIRCLE, POINT.
pub fn parse_reply(full_response_text: &str) -> ParsedReply {
    todo!("E: chain the extractors in the Mac order")
}
