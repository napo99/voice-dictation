//! Voice-Dict Desktop Application
//!
//! A floating pill UI for voice dictation, inspired by Wispr Flow.
//!
//! Architecture:
//! - Floating, draggable, always-on-top pill window
//! - Global hotkey activation (no clicking)
//! - Real-time transcription preview
//! - Automatic text injection

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod pipeline;

use std::sync::{Arc, Mutex};
use tauri::{
    AppHandle, Emitter, Listener, Manager, State, WebviewWindow,
    WebviewWindowBuilder, LogicalPosition, LogicalSize,
};
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

use pipeline::{Pipeline, PipelineConfig, PipelineEvent, PipelineState};

// ============================================================================
// Application State
// ============================================================================

/// Shared application state
struct AppState {
    pipeline: Arc<Mutex<Option<Pipeline>>>,
    window_position: Arc<Mutex<(f64, f64)>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            pipeline: Arc::new(Mutex::new(None)),
            window_position: Arc::new(Mutex::new((100.0, 100.0))),
        }
    }
}

// ============================================================================
// IPC Types
// ============================================================================

/// Status sent to frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PillStatus {
    pub state: String,
    pub transcript: String,
    pub is_recording: bool,
    pub audio_level: f32,
}

/// Position update from frontend drag
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionUpdate {
    pub x: f64,
    pub y: f64,
}

// ============================================================================
// Tauri Commands
// ============================================================================

/// Get current pill status
#[tauri::command]
fn get_status(state: State<AppState>) -> PillStatus {
    let pipeline = state.pipeline.lock().unwrap();

    if let Some(ref p) = *pipeline {
        let pstate = p.state();
        PillStatus {
            state: format!("{:?}", pstate),
            transcript: p.current_transcript().unwrap_or_default(),
            is_recording: matches!(pstate, PipelineState::Recording),
            audio_level: p.audio_level(),
        }
    } else {
        PillStatus {
            state: "Idle".to_string(),
            transcript: String::new(),
            is_recording: false,
            audio_level: 0.0,
        }
    }
}

/// Start recording (called on hotkey press)
#[tauri::command]
async fn start_recording(state: State<'_, AppState>, app: AppHandle) -> Result<(), String> {
    info!("Hotkey pressed - starting recording");

    let mut pipeline = state.pipeline.lock().unwrap();

    if pipeline.is_none() {
        // Initialize pipeline on first use
        let config = PipelineConfig::default();
        let app_clone = app.clone();

        let event_callback = move |event: PipelineEvent| {
            // Forward pipeline events to frontend
            let _ = app_clone.emit("pipeline-event", event);
        };

        *pipeline = Some(Pipeline::new(config, Box::new(event_callback))
            .map_err(|e| e.to_string())?);
    }

    if let Some(ref mut p) = *pipeline {
        p.start_recording().map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Stop recording (called on hotkey release)
#[tauri::command]
async fn stop_recording(state: State<'_, AppState>) -> Result<(), String> {
    info!("Hotkey released - stopping recording");

    let mut pipeline = state.pipeline.lock().unwrap();

    if let Some(ref mut p) = *pipeline {
        p.stop_recording().map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Cancel current operation
#[tauri::command]
fn cancel(state: State<AppState>) -> Result<(), String> {
    let mut pipeline = state.pipeline.lock().unwrap();

    if let Some(ref mut p) = *pipeline {
        p.cancel();
    }

    Ok(())
}

/// Update window position (from drag)
#[tauri::command]
fn update_position(state: State<AppState>, pos: PositionUpdate) {
    let mut position = state.window_position.lock().unwrap();
    *position = (pos.x, pos.y);
    debug!("Window position updated to ({}, {})", pos.x, pos.y);
}

/// Check if models are available
#[tauri::command]
fn check_models() -> bool {
    let models_dir = vd_core::models_dir();
    let vad_exists = models_dir.join(vd_core::SILERO_VAD_MODEL.path).exists();
    let whisper_exists = models_dir.join(vd_core::WhisperModel::Base.filename()).exists();
    vad_exists && whisper_exists
}

/// Get models directory
#[tauri::command]
fn get_models_dir() -> String {
    vd_core::models_dir().display().to_string()
}

// ============================================================================
// Window Setup
// ============================================================================

/// Create the floating pill window
fn create_pill_window(app: &AppHandle) -> Result<WebviewWindow, tauri::Error> {
    let window = WebviewWindowBuilder::new(
        app,
        "pill",
        tauri::WebviewUrl::App("index.html".into())
    )
    .title("Voice-Dict")
    .inner_size(320.0, 52.0)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .resizable(false)
    .skip_taskbar(true)
    .center()
    .build()?;

    Ok(window)
}

// ============================================================================
// Main Entry Point
// ============================================================================

fn main() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,voice_dict=debug")),
        )
        .init();

    info!("Starting Voice-Dict");

    // Ensure models directory exists
    let models_dir = vd_core::models_dir();
    if let Err(e) = std::fs::create_dir_all(&models_dir) {
        warn!("Failed to create models directory: {}", e);
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(AppState::default())
        .setup(|app| {
            let app_handle = app.handle().clone();

            // Register global hotkey (Ctrl+Shift+Space)
            use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

            let shortcut: Shortcut = "CommandOrControl+Shift+Space".parse().unwrap();

            app.handle().plugin(
                tauri_plugin_global_shortcut::Builder::new()
                    .with_handler(move |_app, shortcut, event| {
                        match event.state() {
                            ShortcutState::Pressed => {
                                info!("Global hotkey pressed");
                                let _ = _app.emit("hotkey-pressed", ());
                            }
                            ShortcutState::Released => {
                                info!("Global hotkey released");
                                let _ = _app.emit("hotkey-released", ());
                            }
                        }
                    })
                    .build(),
            )?;

            app.global_shortcut().register(shortcut)?;
            info!("Registered global shortcut: Ctrl+Shift+Space");

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_status,
            start_recording,
            stop_recording,
            cancel,
            update_position,
            check_models,
            get_models_dir,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
