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
use once_cell::sync::Lazy;
use tauri::{
    AppHandle, Emitter, Manager, State, WebviewWindow,
    WebviewWindowBuilder,
};
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

use pipeline::{Pipeline, PipelineConfig, PipelineEvent, PipelineState};

// ============================================================================
// Global Pipeline (for hotkey access)
// ============================================================================

/// Global pipeline instance for hotkey handler access
static GLOBAL_PIPELINE: Lazy<Arc<Mutex<Option<Pipeline>>>> = Lazy::new(|| Arc::new(Mutex::new(None)));

/// Global app handle for event emission
static GLOBAL_APP: Lazy<Arc<Mutex<Option<AppHandle>>>> = Lazy::new(|| Arc::new(Mutex::new(None)));

/// Toggle mode flag - if true, single press toggles recording on/off (like Wispr Flow)
/// If false, uses push-to-talk (hold to record)
static TOGGLE_MODE: Lazy<Arc<Mutex<bool>>> = Lazy::new(|| Arc::new(Mutex::new(true)));

/// Recording state for toggle mode
static IS_RECORDING: Lazy<Arc<Mutex<bool>>> = Lazy::new(|| Arc::new(Mutex::new(false)));

// ============================================================================
// Application State
// ============================================================================

/// Shared application state
struct AppState {
    window_position: Arc<Mutex<(f64, f64)>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
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
fn get_status() -> PillStatus {
    let pipeline = GLOBAL_PIPELINE.lock().unwrap();

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

/// Initialize pipeline if not already done
fn ensure_pipeline_initialized() -> Result<(), String> {
    let mut pipeline = GLOBAL_PIPELINE.lock().unwrap();

    if pipeline.is_none() {
        info!("Initializing pipeline on first use");
        let config = PipelineConfig::default();

        // Get app handle for event emission
        let app_handle = {
            let app = GLOBAL_APP.lock().unwrap();
            app.clone()
        };

        let event_callback = move |event: PipelineEvent| {
            // Reset IS_RECORDING when pipeline completes (goes back to Idle after injection)
            if let PipelineEvent::StateChanged(ref state) = event {
                if state == "Idle" {
                    let mut is_recording = IS_RECORDING.lock().unwrap();
                    if *is_recording {
                        info!("Pipeline returned to Idle - resetting recording flag");
                        *is_recording = false;
                    }
                }
            }
            if let Some(ref app) = app_handle {
                let _ = app.emit("pipeline-event", &event);
            }
        };

        *pipeline = Some(Pipeline::new(config, Box::new(event_callback))
            .map_err(|e| e.to_string())?);
        info!("Pipeline initialized successfully");
    }

    Ok(())
}

/// Start recording - called directly from hotkey handler
fn do_start_recording() {
    info!(">>> Starting recording (direct call) <<<");

    if let Err(e) = ensure_pipeline_initialized() {
        error!("Failed to initialize pipeline: {}", e);
        return;
    }

    let mut pipeline = GLOBAL_PIPELINE.lock().unwrap();
    if let Some(ref mut p) = *pipeline {
        if let Err(e) = p.start_recording() {
            error!("Failed to start recording: {}", e);
        } else {
            info!("Recording started successfully");
        }
    }
}

/// Stop recording - called directly from hotkey handler
fn do_stop_recording() {
    info!(">>> Stopping recording (direct call) <<<");

    let mut pipeline = GLOBAL_PIPELINE.lock().unwrap();
    if let Some(ref mut p) = *pipeline {
        if let Err(e) = p.stop_recording() {
            error!("Failed to stop recording: {}", e);
        } else {
            info!("Recording stopped, processing...");
        }
    }
}

/// Start recording (called on hotkey press)
#[tauri::command]
async fn start_recording() -> Result<(), String> {
    do_start_recording();
    Ok(())
}

/// Stop recording (called on hotkey release)
#[tauri::command]
async fn stop_recording() -> Result<(), String> {
    do_stop_recording();
    Ok(())
}

/// Cancel current operation
#[tauri::command]
fn cancel() -> Result<(), String> {
    let mut pipeline = GLOBAL_PIPELINE.lock().unwrap();

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
    let whisper_path = models_dir.join(vd_core::WhisperModel::Small.filename());

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
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, _shortcut, event| {
                    use tauri_plugin_global_shortcut::ShortcutState;
                    let toggle_mode = *TOGGLE_MODE.lock().unwrap();

                    match event.state() {
                        ShortcutState::Pressed => {
                            if toggle_mode {
                                // Toggle mode: press toggles recording on/off
                                let mut is_recording = IS_RECORDING.lock().unwrap();
                                if *is_recording {
                                    info!("===========================================");
                                    info!(">>> HOTKEY: STOP RECORDING (toggle) <<<");
                                    info!("===========================================");
                                    *is_recording = false;
                                    drop(is_recording); // Release lock before calling
                                    do_stop_recording();
                                    let _ = app.emit("hotkey-released", ());
                                } else {
                                    info!("===========================================");
                                    info!(">>> HOTKEY: START RECORDING (toggle) <<<");
                                    info!("===========================================");
                                    *is_recording = true;
                                    drop(is_recording); // Release lock before calling
                                    do_start_recording();
                                    let _ = app.emit("hotkey-pressed", ());
                                }
                            } else {
                                // Push-to-talk mode: hold to record
                                info!("===========================================");
                                info!(">>> HOTKEY PRESSED (push-to-talk) <<<");
                                info!("===========================================");
                                do_start_recording();
                                let _ = app.emit("hotkey-pressed", ());
                            }
                        }
                        ShortcutState::Released => {
                            if !toggle_mode {
                                // Push-to-talk mode: release stops recording
                                info!("===========================================");
                                info!(">>> HOTKEY RELEASED (push-to-talk) <<<");
                                info!("===========================================");
                                do_stop_recording();
                                let _ = app.emit("hotkey-released", ());
                            }
                            // In toggle mode, release is ignored
                        }
                    }
                })
                .build(),
        )
        .manage(AppState::default())
        .setup(|app| {
            use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

            // Store app handle globally for pipeline event emission
            {
                let mut global_app = GLOBAL_APP.lock().unwrap();
                *global_app = Some(app.handle().clone());
            }
            info!("Global app handle stored");

            let shortcut: Shortcut = "Alt+Shift+V".parse().unwrap();
            app.global_shortcut().register(shortcut)?;
            info!("Registered global shortcut: Alt+Shift+V");

            let toggle_mode = *TOGGLE_MODE.lock().unwrap();
            if toggle_mode {
                info!("Mode: TOGGLE (press once to start, press again to stop)");
            } else {
                info!("Mode: PUSH-TO-TALK (hold to record, release to stop)");
            }

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
