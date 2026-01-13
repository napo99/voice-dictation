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
    AppHandle, Emitter, State, WebviewWindow,
    WebviewWindowBuilder,
};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};
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
    let vad_path = models_dir.join(vd_core::SILERO_VAD_MODEL.path);
    let whisper_path = models_dir.join(vd_core::WhisperModel::Base.filename());

    let vad_exists = vad_path.exists();
    let whisper_exists = whisper_path.exists();

    info!("=== Model Check ===");
    info!("VAD model: {:?} -> {}", vad_path, if vad_exists { "FOUND" } else { "MISSING" });
    info!("Whisper model: {:?} -> {}", whisper_path, if whisper_exists { "FOUND" } else { "MISSING" });

    vad_exists && whisper_exists
}

/// Get models directory
#[tauri::command]
fn get_models_dir() -> String {
    vd_core::models_dir().display().to_string()
}

/// List available audio input devices
#[tauri::command]
fn list_audio_devices() -> Vec<String> {
    info!("=== Listing Audio Devices ===");
    match vd_audio::list_input_devices() {
        Ok(devices) => {
            let mut result = Vec::new();
            for device in &devices {
                let marker = if device.is_default { " [DEFAULT]" } else { "" };
                info!("  - {}{}", device.name, marker);
                result.push(format!("{}{}", device.name, marker));
            }
            if devices.is_empty() {
                warn!("No audio input devices found!");
            }
            result
        }
        Err(e) => {
            warn!("Failed to list audio devices: {}", e);
            vec![format!("Error: {}", e)]
        }
    }
}

// ============================================================================
// Window Setup
// ============================================================================

/// Create the floating pill window
#[allow(dead_code)]
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
    // Initialize logging with VERBOSE output
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("debug,voice_dict_desktop=trace,vd_audio=debug,vd_vad=debug,vd_whisper=debug")),
        )
        .with_target(true)
        .with_line_number(true)
        .init();

    info!("========================================");
    info!("Starting Voice-Dict");
    info!("========================================");

    // Log system info
    info!("Platform: {}", std::env::consts::OS);
    info!("Arch: {}", std::env::consts::ARCH);

    // Ensure models directory exists
    let models_dir = vd_core::models_dir();
    info!("Models directory: {:?}", models_dir);
    if let Err(e) = std::fs::create_dir_all(&models_dir) {
        warn!("Failed to create models directory: {}", e);
    }

    // List audio devices at startup
    info!("=== Audio Input Devices ===");
    match vd_audio::list_input_devices() {
        Ok(devices) => {
            for device in &devices {
                let marker = if device.is_default { " [DEFAULT]" } else { "" };
                info!("  🎤 {}{}", device.name, marker);
            }
            if devices.is_empty() {
                warn!("⚠️  No audio input devices found! Check your microphone connection.");
            }
        }
        Err(e) => {
            warn!("Failed to list audio devices: {}", e);
        }
    }
    info!("============================");

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(AppState::default())
        .setup(|app| {
            // Register global hotkey (Ctrl+Shift+Space)
            use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

            let shortcut: Shortcut = "Alt+Shift+V".parse().unwrap();

            app.handle().plugin(
                tauri_plugin_global_shortcut::Builder::new()
                    .with_handler(move |_app, _shortcut, event| {
                        match event.state() {
                            ShortcutState::Pressed => {
                                info!("===========================================");
                                info!(">>> HOTKEY PRESSED - Starting recording <<<");
                                info!("===========================================");
                                let _ = _app.emit("hotkey-pressed", ());
                            }
                            ShortcutState::Released => {
                                info!("===========================================");
                                info!(">>> HOTKEY RELEASED - Stopping recording <<<");
                                info!("===========================================");
                                let _ = _app.emit("hotkey-released", ());
                            }
                        }
                    })
                    .build(),
            )?;

            app.global_shortcut().register(shortcut)?;
            info!("Registered global shortcut: Alt+Shift+V");

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
            list_audio_devices,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
