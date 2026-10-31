//! claude.exe and whisper-server.exe start without a console window and sit in
//! a kill-on-close job object, so they (and claude's PowerShell children) die
//! with the app.

use std::sync::OnceLock;

use windows::core::PCWSTR;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Adds CREATE_NO_WINDOW (0x08000000) so no console flashes up.
pub fn configure_background_command(command: &mut tokio::process::Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}

/// The job handle as an integer (HANDLE is not Send/Sync). Never closed on purpose:
/// Windows closes it when the app exits, which kills every process in the job.
fn kill_on_close_job_handle() -> Option<isize> {
    static JOB_HANDLE: OnceLock<Option<isize>> = OnceLock::new();
    *JOB_HANDLE.get_or_init(|| unsafe {
        let job_handle = CreateJobObjectW(None, PCWSTR::null()).ok()?;
        let mut extended_limit_information = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        extended_limit_information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            job_handle,
            JobObjectExtendedLimitInformation,
            &extended_limit_information as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
        .ok()?;
        Some(job_handle.0 as isize)
    })
}

/// Puts the child in a job object with JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE; the job handle lives in a static.
pub fn attach_child_to_app_lifetime(child: &tokio::process::Child) {
    let (Some(job_handle), Some(process_handle)) = (kill_on_close_job_handle(), child.raw_handle()) else {
        return;
    };
    let assign_result =
        unsafe { AssignProcessToJobObject(HANDLE(job_handle as *mut core::ffi::c_void), HANDLE(process_handle as *mut core::ffi::c_void)) };
    if let Err(assign_error) = assign_result {
        eprintln!("child process: couldn't add to the job object: {assign_error}");
    }
}
