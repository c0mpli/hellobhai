//! Parses the tags Claude appends to a reply ([CLICK]/[TYPE]/[KEY], [CLOSE_DRAWING],
//! [SHOW], [NOTES], [MODEL3D], [CIRCLE], [POINT]) in the Mac's order. Coordinates
//! stay in screenshot pixels here; `screen_mapping` turns them into global ones.
//! Screen numbers are 1-based; `None` means the cursor screen.
//!
//! The patterns are copied from `ComputerControl.swift` and `CompanionManager.swift`.
//! Rust's `regex` has no lookahead, so the CLICK payload's "bare :screenN is not a
//! label" rule is applied after matching instead.

use std::sync::LazyLock;

use regex::Regex;

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
        match self {
            ComputerAction::Click { label, .. } => format!("clicked {}", label.as_deref().unwrap_or("something")),
            ComputerAction::Type { text } => format!("typed \"{text}\""),
            ComputerAction::PressKey { key_combo } => format!("pressed {key_combo}"),
        }
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

/// The ":label:screenN" tail shared by the [POINT] and [CIRCLE] tag patterns.
const TAG_LABEL_AND_SCREEN_PATTERN: &str = r"(?::([^\]:\s][^\]:]*?))?(?::screen(\d+))?";

static ACTION_TAG_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[(CLICK|TYPE|KEY):([^\]]*)\]").unwrap());
static CLICK_PAYLOAD_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(\d+)\s*,\s*(\d+)(?::([^:]*?))?(?::screen(\d+))?\s*$").unwrap());
static BARE_SCREEN_LABEL_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^screen(\d+)\s*$").unwrap());
static MODEL3D_REQUEST_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)^(.*?)(?::\s*(-?\d+(?:\.\d+)?)\s*,\s*(-?\d+(?:\.\d+)?))?(?::\s*screen(\d+))?\s*$").unwrap()
});
static CIRCLE_TAG_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"\[CIRCLE:(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+){TAG_LABEL_AND_SCREEN_PATTERN}\]")).unwrap()
});
static POINT_TAG_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"\[POINT:(?:none|(\d+)\s*,\s*(\d+){TAG_LABEL_AND_SCREEN_PATTERN})\]\s*$")).unwrap()
});

/// Swift's `.whitespaces` trim: spaces and tabs, but not newlines.
fn trim_spaces_but_not_newlines(text: &str) -> &str {
    text.trim_matches(|character: char| {
        character.is_whitespace() && !matches!(character, '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}')
    })
}

/// Parses "x,y", "x,y:label", or "x,y:label:screenN".
fn parse_click_payload(click_payload: &str) -> Option<ComputerAction> {
    let captures = CLICK_PAYLOAD_REGEX.captures(click_payload)?;
    let screenshot_x: f64 = captures[1].parse().ok()?;
    let screenshot_y: f64 = captures[2].parse().ok()?;
    let mut label = captures.get(3).map(|label_match| label_match.as_str().to_string());
    let mut screen_number = captures.get(4).and_then(|screen_match| screen_match.as_str().parse().ok());
    // Swift's lookahead stops a bare ":screen2" from being read as the label; do the same here.
    if captures.get(4).is_none() {
        if let Some(bare_screen_captures) = label.as_deref().and_then(|label_text| BARE_SCREEN_LABEL_REGEX.captures(label_text)) {
            screen_number = bare_screen_captures[1].parse().ok();
            label = None;
        }
    }
    Some(ComputerAction::Click { screenshot_x, screenshot_y, label, screen_number })
}

/// Removes every [CLICK]/[TYPE]/[KEY] tag; returns the remaining text and the actions in order.
pub fn extract_computer_actions(text: &str) -> (String, Vec<ComputerAction>) {
    let mut parsed_actions = Vec::new();
    for captures in ACTION_TAG_REGEX.captures_iter(text) {
        let action_payload = &captures[2];
        match &captures[1] {
            "CLICK" => {
                if let Some(click_action) = parse_click_payload(action_payload) {
                    parsed_actions.push(click_action);
                }
            }
            "TYPE" => parsed_actions.push(ComputerAction::Type { text: action_payload.to_string() }),
            "KEY" => parsed_actions.push(ComputerAction::PressKey { key_combo: trim_spaces_but_not_newlines(action_payload).to_string() }),
            _ => {}
        }
    }
    let text_without_actions = ACTION_TAG_REGEX.replace_all(text, "").trim().to_string();
    (text_without_actions, parsed_actions)
}

/// Removes every [CLOSE_DRAWING]; the bool says whether there was one.
pub fn remove_close_drawing(text: &str) -> (String, bool) {
    if !text.contains("[CLOSE_DRAWING]") {
        return (text.to_string(), false);
    }
    (text.replace("[CLOSE_DRAWING]", "").trim().to_string(), true)
}

/// First `[tag_name:...]` removed; returns the remaining text and the trimmed non-empty payload.
pub fn extract_tagged_request(tag_name: &str, text: &str) -> (String, Option<String>) {
    let tag_regex = Regex::new(&format!(r"\[{}:([^\]]*)\]", regex::escape(tag_name))).unwrap();
    let Some(captures) = tag_regex.captures(text) else {
        return (text.to_string(), None);
    };
    let whole_tag_range = captures.get(0).unwrap().range();
    let request_description = trim_spaces_but_not_newlines(&captures[1]).to_string();
    let mut remaining_text = text.to_string();
    remaining_text.replace_range(whole_tag_range, "");
    let request_description = if request_description.is_empty() { None } else { Some(request_description) };
    (remaining_text.trim().to_string(), request_description)
}

/// Splits "what to show:x,y:screenN"; position and screen are optional.
pub fn parse_model3d_request(request: &str) -> Model3DRequest {
    let Some(captures) = MODEL3D_REQUEST_REGEX.captures(request) else {
        return Model3DRequest { description: request.to_string(), screenshot_point: None, screen_number: None };
    };
    let description = trim_spaces_but_not_newlines(&captures[1]).to_string();
    let screenshot_point = match (captures.get(2), captures.get(3)) {
        (Some(x_match), Some(y_match)) => match (x_match.as_str().parse::<f64>(), y_match.as_str().parse::<f64>()) {
            (Ok(screenshot_x), Ok(screenshot_y)) => Some((screenshot_x, screenshot_y)),
            _ => None,
        },
        _ => None,
    };
    let screen_number = captures.get(4).and_then(|screen_match| screen_match.as_str().parse().ok());
    Model3DRequest {
        description: if description.is_empty() { request.to_string() } else { description },
        screenshot_point,
        screen_number,
    }
}

/// Pulls every [CIRCLE:centerX,centerY,width,height:label:screenN] tag out of the reply.
pub fn extract_circle_tags(text: &str) -> (String, Vec<CircleTag>) {
    let circle_tags = CIRCLE_TAG_REGEX
        .captures_iter(text)
        .filter_map(|captures| {
            let center_x: f64 = captures[1].parse().ok()?;
            let center_y: f64 = captures[2].parse().ok()?;
            let width: f64 = captures[3].parse().ok()?;
            let height: f64 = captures[4].parse().ok()?;
            Some(CircleTag {
                screenshot_x: center_x - width / 2.0,
                screenshot_y: center_y - height / 2.0,
                width,
                height,
                label: captures.get(5).map(|label_match| trim_spaces_but_not_newlines(label_match.as_str()).to_string()),
                screen_number: captures.get(6).and_then(|screen_match| screen_match.as_str().parse().ok()),
            })
        })
        .collect();
    let remaining_text = CIRCLE_TAG_REGEX.replace_all(text, "").trim().to_string();
    (remaining_text, circle_tags)
}

/// Parses a [POINT:x,y:label:screenN] or [POINT:none] tag at the very end of the reply.
pub fn parse_point_tag(text: &str) -> PointTag {
    let Some(captures) = POINT_TAG_REGEX.captures(text) else {
        return PointTag { spoken_text: text.to_string(), point: None, label: None, screen_number: None };
    };
    let tag_start = captures.get(0).unwrap().start();
    let spoken_text = text[..tag_start].trim().to_string();

    let coordinates = match (captures.get(1), captures.get(2)) {
        (Some(x_match), Some(y_match)) => match (x_match.as_str().parse::<f64>(), y_match.as_str().parse::<f64>()) {
            (Ok(screenshot_x), Ok(screenshot_y)) => Some((screenshot_x, screenshot_y)),
            _ => None,
        },
        _ => None,
    };
    let Some(point) = coordinates else {
        return PointTag { spoken_text, point: None, label: Some("none".to_string()), screen_number: None };
    };
    PointTag {
        spoken_text,
        point: Some(point),
        label: captures.get(3).map(|label_match| trim_spaces_but_not_newlines(label_match.as_str()).to_string()),
        screen_number: captures.get(4).and_then(|screen_match| screen_match.as_str().parse().ok()),
    }
}

/// Mac order: actions, CLOSE_DRAWING, SHOW, NOTES, MODEL3D, CIRCLE, POINT.
pub fn parse_reply(full_response_text: &str) -> ParsedReply {
    let (text_without_actions, actions) = extract_computer_actions(full_response_text);
    let (text_without_close_drawing, close_drawing) = remove_close_drawing(&text_without_actions);
    let (text_without_show_request, show_request) = extract_tagged_request("SHOW", &text_without_close_drawing);
    let (text_without_notes_request, notes_request) = extract_tagged_request("NOTES", &text_without_show_request);
    let (text_without_model3d_request, model3d_request_text) = extract_tagged_request("MODEL3D", &text_without_notes_request);
    let (text_without_circle_tags, circles) = extract_circle_tags(&text_without_model3d_request);
    let point = parse_point_tag(&text_without_circle_tags);
    ParsedReply {
        spoken_text: point.spoken_text.clone(),
        actions,
        close_drawing,
        show_request,
        notes_request,
        model3d_request: model3d_request_text.as_deref().map(parse_model3d_request),
        circles,
        point,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_tags_from_the_prompt_examples() {
        let color_inspector = parse_point_tag(
            "you'll want the color inspector — click that little paint palette icon in the toolbar up top. [POINT:1100,42:color inspector]",
        );
        assert_eq!(color_inspector.spoken_text, "you'll want the color inspector — click that little paint palette icon in the toolbar up top.");
        assert_eq!(color_inspector.point, Some((1100.0, 42.0)));
        assert_eq!(color_inspector.label.as_deref(), Some("color inspector"));
        assert_eq!(color_inspector.screen_number, None);

        let none_tag = parse_point_tag("html is the skeleton of a web page. [POINT:none]");
        assert_eq!(none_tag.spoken_text, "html is the skeleton of a web page.");
        assert_eq!(none_tag.point, None);
        assert_eq!(none_tag.label.as_deref(), Some("none"));

        let second_screen = parse_point_tag("that's over in your terminal. [POINT:400,300:terminal:screen2]\n");
        assert_eq!(second_screen.point, Some((400.0, 300.0)));
        assert_eq!(second_screen.label.as_deref(), Some("terminal"));
        assert_eq!(second_screen.screen_number, Some(2));

        let no_tag = parse_point_tag("just talking. ");
        assert_eq!(no_tag.spoken_text, "just talking. ");
        assert_eq!(no_tag.point, None);
        assert_eq!(no_tag.label, None);

        // Only a tag at the very end counts.
        let tag_in_the_middle = parse_point_tag("[POINT:1,2:x] and then more text");
        assert_eq!(tag_in_the_middle.point, None);
        assert_eq!(tag_in_the_middle.spoken_text, "[POINT:1,2:x] and then more text");
    }

    #[test]
    fn click_type_and_key_actions_are_extracted_in_order() {
        let (remaining_text, actions) = extract_computer_actions(
            "searching for it. [CLICK:640,40:search bar][TYPE:लाल किला दिल्ली][KEY: cmd+enter ][CLICK:10,20:screen2][CLICK:5,6][CLICK:7,8:send:screen3][CLICK:oops] done",
        );
        assert_eq!(remaining_text, "searching for it.  done");
        assert_eq!(
            actions,
            vec![
                ComputerAction::Click { screenshot_x: 640.0, screenshot_y: 40.0, label: Some("search bar".to_string()), screen_number: None },
                ComputerAction::Type { text: "लाल किला दिल्ली".to_string() },
                ComputerAction::PressKey { key_combo: "cmd+enter".to_string() },
                ComputerAction::Click { screenshot_x: 10.0, screenshot_y: 20.0, label: None, screen_number: Some(2) },
                ComputerAction::Click { screenshot_x: 5.0, screenshot_y: 6.0, label: None, screen_number: None },
                ComputerAction::Click { screenshot_x: 7.0, screenshot_y: 8.0, label: Some("send".to_string()), screen_number: Some(3) },
            ]
        );
        assert_eq!(actions[0].summary(), "clicked search bar");
        assert_eq!(actions[1].summary(), "typed \"लाल किला दिल्ली\"");
        assert_eq!(actions[2].summary(), "pressed cmd+enter");
        assert_eq!(actions[3].summary(), "clicked something");
    }

    #[test]
    fn close_drawing_and_tagged_requests() {
        assert_eq!(remove_close_drawing("ok closing it. [CLOSE_DRAWING]"), ("ok closing it.".to_string(), true));
        assert_eq!(remove_close_drawing("nothing here "), ("nothing here ".to_string(), false));

        assert_eq!(
            extract_tagged_request("SHOW", "let me sketch it. [SHOW: how dns resolves a name ] [POINT:none]"),
            ("let me sketch it.  [POINT:none]".to_string(), Some("how dns resolves a name".to_string()))
        );
        assert_eq!(extract_tagged_request("NOTES", "hi [NOTES:   ]"), ("hi".to_string(), None));
        assert_eq!(extract_tagged_request("NOTES", "no tag"), ("no tag".to_string(), None));
    }

    #[test]
    fn model3d_requests_with_and_without_position_and_screen() {
        assert_eq!(
            parse_model3d_request("v8 engine:640,400:screen2"),
            Model3DRequest { description: "v8 engine".to_string(), screenshot_point: Some((640.0, 400.0)), screen_number: Some(2) }
        );
        assert_eq!(
            parse_model3d_request("raspberry pi 5 board: 320.5 , 210"),
            Model3DRequest { description: "raspberry pi 5 board".to_string(), screenshot_point: Some((320.5, 210.0)), screen_number: None }
        );
        assert_eq!(
            parse_model3d_request("a jet engine"),
            Model3DRequest { description: "a jet engine".to_string(), screenshot_point: None, screen_number: None }
        );
        assert_eq!(
            parse_model3d_request("human heart:screen1"),
            Model3DRequest { description: "human heart".to_string(), screenshot_point: None, screen_number: Some(1) }
        );
    }

    #[test]
    fn circle_tags_are_converted_from_centre_to_top_left() {
        let (remaining_text, circles) = extract_circle_tags(
            "this line [CIRCLE:400,300,200,40:the error] and this one [CIRCLE:100,50,20,10:fix:screen2] [POINT:none]",
        );
        assert_eq!(remaining_text, "this line  and this one  [POINT:none]");
        assert_eq!(
            circles,
            vec![
                CircleTag { screenshot_x: 300.0, screenshot_y: 280.0, width: 200.0, height: 40.0, label: Some("the error".to_string()), screen_number: None },
                CircleTag { screenshot_x: 90.0, screenshot_y: 45.0, width: 20.0, height: 10.0, label: Some("fix".to_string()), screen_number: Some(2) },
            ]
        );
    }

    #[test]
    fn full_reply_is_parsed_in_the_mac_order() {
        let parsed_reply = parse_reply(
            "opening a new tab. [KEY:ctrl+t][CLOSE_DRAWING] [NOTES:summary of this page] [CIRCLE:50,50,10,10:tab] [POINT:none]",
        );
        assert_eq!(parsed_reply.spoken_text, "opening a new tab.");
        assert_eq!(parsed_reply.actions, vec![ComputerAction::PressKey { key_combo: "ctrl+t".to_string() }]);
        assert!(parsed_reply.close_drawing);
        assert_eq!(parsed_reply.show_request, None);
        assert_eq!(parsed_reply.notes_request.as_deref(), Some("summary of this page"));
        assert_eq!(parsed_reply.model3d_request, None);
        assert_eq!(parsed_reply.circles.len(), 1);
        assert_eq!(parsed_reply.point.label.as_deref(), Some("none"));

        // A CLICK written before the POINT tag must not stop POINT from being "at the end".
        let click_then_point = parse_reply("clicking send. [POINT:10,20:send] [CLICK:10,20:send]");
        assert_eq!(click_then_point.point.point, Some((10.0, 20.0)));
        assert_eq!(click_then_point.spoken_text, "clicking send.");
        assert_eq!(click_then_point.actions.len(), 1);

        let model_reply = parse_reply("here's the engine. [MODEL3D:v8 engine:600,300] [POINT:none]");
        assert_eq!(model_reply.model3d_request.unwrap().screenshot_point, Some((600.0, 300.0)));
    }
}
