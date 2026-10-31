//! claude.exe and whisper-server.exe start without a console window and sit in
//! a kill-on-close job object, so they (and claude's PowerShell children) die
//! with the app.

/// Adds CREATE_NO_WINDOW (0x08000000) so no console flashes up.
pub fn configure_background_command(command: &mut tokio::process::Command) {
    todo!("E: command.creation_flags(0x08000000)")
}

/// Puts the child in a job object with JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE; the job handle lives in a static.
pub fn attach_child_to_app_lifetime(child: &tokio::process::Child) {
    todo!("E: AssignProcessToJobObject")
}
