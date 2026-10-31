/// No console window to hide on this OS.
pub fn configure_background_command(_command: &mut tokio::process::Command) {}

/// No job object here; `kill_on_drop` still stops the child itself when its task ends.
pub struct ProcessTreeKillGuard;

pub fn kill_process_tree_when_dropped(_child: &tokio::process::Child) -> ProcessTreeKillGuard {
    ProcessTreeKillGuard
}
