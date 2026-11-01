//! The only place that calls OS APIs. Each OS folder exposes the same free
//! functions and types (no traits); the compiler checks them on each target.
//! `unsupported/` holds harmless stubs so the crate builds and `cargo test`
//! runs on any other host (e.g. a Mac). `linux/` is X11 first, Wayland best effort.

// Named `os` rather than `windows` so it never clashes with the `windows` crate.
#[cfg(windows)]
#[path = "windows/mod.rs"]
mod os;

#[cfg(target_os = "linux")]
#[path = "linux/mod.rs"]
mod os;

#[cfg(not(any(windows, target_os = "linux")))]
#[path = "unsupported/mod.rs"]
mod os;

pub use os::*;

/// Linux wording for the Windows-worded prompts (compiled everywhere so its
/// tests run on any host).
pub mod linux_prompt_wording;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformError {
    /// This OS has no implementation (the `unsupported` stubs).
    Unsupported,
    Failed(String),
}

impl std::fmt::Display for PlatformError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlatformError::Unsupported => write!(formatter, "not supported on this operating system"),
            PlatformError::Failed(message) => write!(formatter, "{message}"),
        }
    }
}

impl std::error::Error for PlatformError {}
