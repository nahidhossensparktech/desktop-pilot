//! Computer tool definitions shared by every engine that is not Claude's built-in
//! computer use (MCP server for Claude Code / Antigravity, Gemini function calling).

use serde_json::{Value, json};

fn coordinate(desc: &str) -> Value {
    json!({
        "type": "array",
        "items": {"type": "number"},
        "minItems": 2,
        "maxItems": 2,
        "description": desc,
    })
}

const MODIFIERS: &str = "Optional modifier keys to hold, e.g. \"shift\" or \"ctrl+shift\".";

fn tool(
    name: &str,
    description: &str,
    properties: Value,
    required: &[&str],
    read_only: bool,
) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        },
        "annotations": {"readOnlyHint": read_only},
    })
}

/// The computer tools as MCP-style definitions (`name`, `description`,
/// `inputSchema`, `annotations.readOnlyHint`), same names and inputs as Claude's
/// computer-use toolset. Shared by the MCP server and the Gemini engine.
pub fn tool_definitions() -> Vec<Value> {
    let at = || coordinate("[x, y] in pixels of the most recent screenshot");
    let click = |name: &str, what: &str| {
        tool(
            name,
            &format!("{what}. Moves to `coordinate` first if given."),
            json!({"coordinate": at(), "text": {"type": "string", "description": MODIFIERS}}),
            &[],
            false,
        )
    };
    vec![
        tool(
            "screenshot",
            "Take a screenshot of the user's primary monitor. Call this first: all coordinates refer to the most recent screenshot.",
            json!({}),
            &[],
            true,
        ),
        tool(
            "zoom",
            "Show a region of the screen at full resolution to read small text. Coordinates stay in full-screenshot space.",
            json!({"region": {"type": "array", "items": {"type": "number"}, "minItems": 4, "maxItems": 4,
                              "description": "[x0, y0, x1, y1] in screenshot pixels"}}),
            &["region"],
            true,
        ),
        click("left_click", "Left-click"),
        click("right_click", "Right-click"),
        click("middle_click", "Middle-click"),
        click("double_click", "Double-click"),
        click("triple_click", "Triple-click (e.g. select a line)"),
        tool(
            "left_click_drag",
            "Press the left button at `start_coordinate`, drag to `coordinate`, release.",
            json!({"start_coordinate": coordinate("[x, y] where the drag starts"),
                   "coordinate": coordinate("[x, y] where the drag ends"),
                   "text": {"type": "string", "description": MODIFIERS}}),
            &["start_coordinate", "coordinate"],
            false,
        ),
        tool(
            "mouse_move",
            "Move the mouse pointer.",
            json!({"coordinate": at()}),
            &["coordinate"],
            false,
        ),
        tool(
            "left_mouse_down",
            "Press and hold the left mouse button.",
            json!({}),
            &[],
            false,
        ),
        tool(
            "left_mouse_up",
            "Release the left mouse button.",
            json!({}),
            &[],
            false,
        ),
        tool(
            "cursor_position",
            "Get the mouse position in screenshot pixels.",
            json!({}),
            &[],
            true,
        ),
        tool(
            "scroll",
            "Scroll at `coordinate` (or where the mouse is).",
            json!({"coordinate": at(),
                   "scroll_direction": {"type": "string", "enum": ["up", "down", "left", "right"]},
                   "scroll_amount": {"type": "integer", "minimum": 1, "maximum": 50, "description": "Wheel notches (default 3)"},
                   "text": {"type": "string", "description": MODIFIERS}}),
            &["scroll_direction"],
            false,
        ),
        tool(
            "type",
            "Type text into the focused field (click the field first). Newlines press Enter.",
            json!({"text": {"type": "string", "maxLength": 2000}}),
            &["text"],
            false,
        ),
        tool(
            "key",
            "Press a key or key combination, xdotool style: \"Return\", \"Escape\", \"ctrl+s\", \"alt+Tab\", \"super\" (Windows key). Space-separated for a sequence.",
            json!({"text": {"type": "string"},
                   "repeat": {"type": "integer", "minimum": 1, "maximum": 100}}),
            &["text"],
            false,
        ),
        tool(
            "hold_key",
            "Hold a key or combination for `duration` seconds.",
            json!({"text": {"type": "string"}, "duration": {"type": "number", "minimum": 0, "maximum": 300}}),
            &["text", "duration"],
            false,
        ),
        tool(
            "wait",
            "Wait `duration` seconds (e.g. for an app to open).",
            json!({"duration": {"type": "number", "minimum": 0, "maximum": 300}}),
            &["duration"],
            true,
        ),
    ]
}
