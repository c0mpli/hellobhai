//! claude and whisper-server each start in their own process group, so dropping
//! the guard kills that child and everything it started (node behind an npm
//! `claude` script too), like the per-child job objects on Windows. The kernel
//! also kills them if the app dies (PR_SET_PDEATHSIG), so a crashed app never
//! leaves a 1 GB whisper-server behind.

/// New process group (pgid = the child's pid) and SIGKILL when the app dies.
pub fn configure_background_command(command: &mut tokio::process::Command) {
    command.process_group(0);
    // ponytail: PDEATHSIG fires when the spawning *thread* exits; tokio's worker
    // threads live as long as the app, so that is the app's exit in practice.
    unsafe {
        command.pre_exec(|| {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong, 0, 0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

/// Owns the child's process group id. Dropping it kills the whole group.
pub struct ProcessTreeKillGuard {
    process_group_id: Option<i32>,
}

impl Drop for ProcessTreeKillGuard {
    fn drop(&mut self) {
        if let Some(process_group_id) = self.process_group_id {
            // Negative pid = the whole process group.
            unsafe {
                libc::kill(-process_group_id, libc::SIGKILL);
            }
        }
    }
}

/// The child must have been spawned with `configure_background_command`
/// (its pid is then its process group id).
pub fn kill_process_tree_when_dropped(child: &tokio::process::Child) -> ProcessTreeKillGuard {
    let process_group_id = child.id().and_then(|process_id| i32::try_from(process_id).ok()).filter(|process_id| *process_id > 1);
    ProcessTreeKillGuard { process_group_id }
}
