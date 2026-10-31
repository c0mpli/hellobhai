//! hellobhai's Tauri app: builder, plugins, command registration and startup
//! wiring. Lives in the system tray; every window is created in Rust.
//!
//! Only `platform/` may call OS APIs (Windows today, Linux later); everything
//! else here is cross-platform.

// scaffold: the module bodies are still todo!(); remove once every module is filled in.
#![allow(dead_code, unused_variables, unused_imports)]

// (A) voice input
pub mod audio_capture;
pub mod downloads;
pub mod push_to_talk;
pub mod whisper_transcriber;
// (B) voice output
pub mod sentence_splitter;
pub mod speech_player;
pub mod supertonic;
// (C) screen + hands
pub mod computer_control;
pub mod key_combo;
pub mod screen_capture;
pub mod screen_mapping;
// (D) windows + pages
pub mod board_panel;
pub mod model3d_window;
pub mod overlay_windows;
pub mod page_bridge;
pub mod tray_and_settings;
pub mod ui_commands;
// (E) brain
pub mod app_paths;
pub mod app_state;
pub mod claude_cli;
pub mod companion_brain;
pub mod conversation_history;
pub mod narrated_step_queue;
pub mod prompts;
pub mod response_tags;
pub mod settings;
pub mod stream_json;

pub mod platform;

use std::sync::Arc;

use tauri::Manager;
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

use crate::app_state::AppState;
use crate::board_panel::BoardPanel;
use crate::companion_brain::CompanionBrain;
use crate::model3d_window::Model3DWindow;
use crate::speech_player::SpeechPlayer;
use crate::whisper_transcriber::WhisperTranscriber;

pub fn run() {
    tauri::Builder::default()
        // A second launch just exits; the running instance keeps the hook and whisper server.
        .plugin(tauri_plugin_single_instance::init(|_app, _arguments, _working_directory| {}))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .invoke_handler(tauri::generate_handler![
            page_bridge::page_message,
            overlay_windows::overlay_ready,
            overlay_windows::user_stroke_added,
            ui_commands::get_settings,
            ui_commands::get_status,
            ui_commands::set_claude_model,
            ui_commands::set_supertonic_voice,
            ui_commands::preview_voice,
            ui_commands::set_push_to_talk_shortcut,
            ui_commands::set_launch_at_login,
            ui_commands::replay_onboarding,
            ui_commands::open_microphone_settings,
            ui_commands::quit_app,
        ])
        .setup(|app| {
            let app_handle = app.handle().clone();
            let shared_settings = settings::load_settings(&app_handle);
            prompts::write_prompt_files(&app_paths::prompts_directory(&app_handle))?;

            // The Mac app registers its login item automatically; do the same once.
            let is_first_launch = !shared_settings.read().unwrap().has_completed_onboarding;
            if is_first_launch {
                let _ = app_handle.autolaunch().enable();
            }

            tray_and_settings::create_tray_icon(&app_handle)?;
            overlay_windows::create_overlay_windows(&app_handle);
            overlay_windows::start_pointer_state_emitter(app_handle.clone());

            let whisper_transcriber = WhisperTranscriber::start(app_handle.clone());
            let initial_voice = shared_settings.read().unwrap().supertonic_voice;
            let speech_player = SpeechPlayer::start(app_handle.clone(), initial_voice);
            let board_panel = Arc::new(BoardPanel::new());
            let model3d_window = Arc::new(Model3DWindow::new());

            let companion_brain = CompanionBrain::new(
                app_handle.clone(),
                shared_settings.clone(),
                speech_player.clone(),
                whisper_transcriber.clone(),
                board_panel.clone(),
                model3d_window.clone(),
            );

            app.manage(AppState {
                settings: shared_settings.clone(),
                companion_brain: companion_brain.clone(),
                speech_player,
                whisper_transcriber,
                board_panel,
                model3d_window,
            });

            let (hotkey_sender, hotkey_receiver) = tokio::sync::mpsc::unbounded_channel();
            push_to_talk::start_push_to_talk_listener(shared_settings.clone(), hotkey_sender);
            tauri::async_runtime::spawn(companion_brain.clone().run_hotkey_loop(hotkey_receiver));

            if is_first_launch {
                companion_brain.start_onboarding();
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building hellobhai")
        .run(|_app_handle, run_event| {
            // A tray app keeps running when its last window closes.
            if let tauri::RunEvent::ExitRequested { code: None, api, .. } = run_event {
                api.prevent_exit();
            }
        });
}
