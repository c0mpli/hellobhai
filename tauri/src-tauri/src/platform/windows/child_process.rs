//! claude.exe and whisper-server.exe start without a console window, each in
//! its own kill-on-close job object. Closing the job handle (dropping the
//! guard, or Windows closing it when the app exits) kills that child and
//! everything it started, e.g. node.exe behind an npm `claude.cmd` shim.
//! One job per child, so cancelling one claude request never touches another.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Adds CREATE_NO_WINDOW (0x08000000) so no console flashes up.
pub fn configure_background_command(command: &mut tokio::process::Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}

/// Owns the job handle (as an integer: HANDLE is not Send). Dropping it kills the process tree.
pub struct ProcessTreeKillGuard {
    job_handle: Option<isize>,
}

impl Drop for ProcessTreeKillGuard {
    fn drop(&mut self) {
        if let Some(job_handle) = self.job_handle {
            let _ = unsafe { CloseHandle(HANDLE(job_handle as *mut core::ffi::c_void)) };
        }
    }
}

/// Puts the child in a new JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE job.
/// ponytail: assigned just after spawn, so a grandchild started in the first
/// milliseconds escapes the job; spawn suspended and resume if that ever shows up.
pub fn kill_process_tree_when_dropped(child: &tokio::process::Child) -> ProcessTreeKillGuard {
    let Some(process_handle) = child.raw_handle() else {
        return ProcessTreeKillGuard { job_handle: None };
    };
    unsafe {
        let Ok(job_handle) = CreateJobObjectW(None, PCWSTR::null()) else {
            return ProcessTreeKillGuard { job_handle: None };
        };
        let guard = ProcessTreeKillGuard { job_handle: Some(job_handle.0 as isize) };
        let mut extended_limit_information = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        extended_limit_information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configure_result = SetInformationJobObject(
            job_handle,
            JobObjectExtendedLimitInformation,
            &extended_limit_information as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        let assign_result = configure_result
            .and_then(|_| AssignProcessToJobObject(job_handle, HANDLE(process_handle as *mut core::ffi::c_void)));
        if let Err(assign_error) = assign_result {
            eprintln!("child process: couldn't add to a job object: {assign_error}");
        }
        guard
    }
}
