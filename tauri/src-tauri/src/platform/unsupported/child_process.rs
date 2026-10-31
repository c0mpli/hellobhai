/// No console window to hide on this OS.
pub fn configure_background_command(command: &mut tokio::process::Command) {}

/// No job object here; `kill_on_drop` still stops the child when its task ends.
pub fn attach_child_to_app_lifetime(child: &tokio::process::Child) {}
