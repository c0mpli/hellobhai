//! Windows implementations. Same file and function names as `../unsupported/`.

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
