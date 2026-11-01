//! The long-lived parts of the app, shared with Tauri commands via
//! `tauri::State<AppState>`. Managed once in `lib.rs` setup.

use std::sync::Arc;

use crate::board_panel::BoardPanel;
use crate::chat_panel::ChatPanel;
use crate::companion_brain::CompanionBrain;
use crate::model3d_window::Model3DWindow;
use crate::settings::SharedSettings;
use crate::speech_player::SpeechPlayer;
use crate::whisper_transcriber::WhisperTranscriber;

pub struct AppState {
    pub settings: SharedSettings,
    pub companion_brain: Arc<CompanionBrain>,
    pub speech_player: Arc<SpeechPlayer>,
    pub whisper_transcriber: Arc<WhisperTranscriber>,
    pub board_panel: Arc<BoardPanel>,
    pub model3d_window: Arc<Model3DWindow>,
    pub chat_panel: Arc<ChatPanel>,
}
