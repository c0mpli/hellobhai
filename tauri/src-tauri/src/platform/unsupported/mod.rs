//! Stubs for operating systems without an implementation yet. Same file and
//! function names as `../windows/`; they return `PlatformError::Unsupported`,
//! `false`, `None` or empty lists so the crate builds and tests run anywhere.

// Stubs ignore their arguments by design.
#![allow(unused_variables)]

mod child_process;
mod claude_tools;
mod input;
mod keyboard_hook;
mod pointer;
mod runtime_downloads;
mod screen_capture;
mod system_settings;
mod system_voice;
mod window_focus;

pub use child_process::*;
pub use claude_tools::*;
pub use input::*;
pub use keyboard_hook::*;
pub use pointer::*;
pub use runtime_downloads::*;
pub use screen_capture::*;
pub use system_settings::*;
pub use system_voice::*;
pub use window_focus::*;
