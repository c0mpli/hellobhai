//! The only place that calls OS APIs. Each OS folder exposes the same free
//! functions and types (no traits); the compiler checks them on each target.
//! `unsupported/` holds harmless stubs so the crate builds and `cargo test`
//! runs on any other host (e.g. a Mac). Linux later: add `linux/` with the
//! same file names and a `#[cfg(target_os = "linux")]` line here.

// Named `os` rather than `windows` so it never clashes with the `windows` crate.
#[cfg(windows)]
#[path = "windows/mod.rs"]
mod os;

#[cfg(not(windows))]
#[path = "unsupported/mod.rs"]
mod os;

pub use os::*;

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
