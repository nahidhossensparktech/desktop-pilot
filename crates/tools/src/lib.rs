//! Custom tools built on the platform traits (implemented in M4):
//! `list_installed_apps`, `launch_app`, `list_open_windows`, `focus_window`,
//! `get_ui_tree`, `read_clipboard`, `write_clipboard`, `run_shell_command`.

#![forbid(unsafe_code)]

/// Names of the custom tools planned for M4. None may collide with a computer-toolset member.
pub const PLANNED_TOOLS: [&str; 8] = [
    "focus_window",
    "get_ui_tree",
    "launch_app",
    "list_installed_apps",
    "list_open_windows",
    "read_clipboard",
    "run_shell_command",
    "write_clipboard",
];
