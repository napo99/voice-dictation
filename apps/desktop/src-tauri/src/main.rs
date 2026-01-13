//! Voice-Dict Desktop Application
//!
//! Tauri-based desktop app for voice dictation.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{Manager, State};
use tracing::{debug, error, info};
use tracing_subscriber::EnvFilter;
use vd_core::{AppConfig, EngineState, WhisperModel};
use vd_engine::{Engine, EngineBuilder, EngineConfig};

/// Application state managed by Tauri
struct AppState {
    engine: Mutex<Option<Engine>>,
    config: Mutex<AppConfig>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            engine: Mutex::new(None),
            config: Mutex::new(AppConfig::default()),
        }
    }
}

/// Status information for the frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusInfo {
    pub state: String,
    pub is_recording: bool,
    pub is_processing: bool,
    pub models_available: bool,
}

/// Get the current engine status
#[tauri::command]
fn get_status(state: State<AppState>) -> StatusInfo {
    let engine = state.engine.lock().unwrap();

    let (engine_state, models_available) = if let Some(ref e) = *engine {
        (e.state(), vd_engine::check_models(e.config()).is_ok())
    } else {
        (EngineState::Idle, false)
    };

    StatusInfo {
        state: format!("{:?}", engine_state),
        is_recording: matches!(engine_state, EngineState::Recording),
        is_processing: matches!(engine_state, EngineState::Processing),
        models_available,
    }
}

/// Get the current configuration
#[tauri::command]
fn get_config(state: State<AppState>) -> AppConfig {
    state.config.lock().unwrap().clone()
}

/// Update the configuration
#[tauri::command]
fn set_config(state: State<AppState>, config: AppConfig) -> Result<(), String> {
    *state.config.lock().unwrap() = config;
    info!("Configuration updated");
    Ok(())
}

/// Check if models are downloaded
#[tauri::command]
fn check_models_available() -> bool {
    let config = EngineConfig::default();
    vd_engine::check_models(&config).is_ok()
}

/// Get model download information
#[tauri::command]
fn get_model_info() -> Vec<ModelDownloadInfo> {
    vec![
        ModelDownloadInfo {
            name: "Silero VAD".to_string(),
            filename: vd_core::SILERO_VAD_MODEL.path.to_string(),
            url: vd_core::SILERO_VAD_MODEL.url.to_string(),
            size_bytes: vd_core::SILERO_VAD_MODEL.size_bytes,
            downloaded: vd_core::models_dir()
                .join(vd_core::SILERO_VAD_MODEL.path)
                .exists(),
        },
        ModelDownloadInfo {
            name: "Whisper Base".to_string(),
            filename: WhisperModel::Base.filename().to_string(),
            url: WhisperModel::Base.download_url().to_string(),
            size_bytes: WhisperModel::Base.size_bytes(),
            downloaded: vd_core::models_dir()
                .join(WhisperModel::Base.filename())
                .exists(),
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDownloadInfo {
    pub name: String,
    pub filename: String,
    pub url: String,
    pub size_bytes: u64,
    pub downloaded: bool,
}

/// Get models directory path
#[tauri::command]
fn get_models_dir() -> String {
    vd_core::models_dir().display().to_string()
}

/// Initialize the engine
#[tauri::command]
fn initialize_engine(state: State<AppState>) -> Result<(), String> {
    let config = state.config.lock().unwrap().clone();

    let engine = EngineBuilder::new()
        .whisper_model(config.whisper_model)
        .use_llm_polish(config.use_llm_polish)
        .input_device(config.input_device.clone())
        .hotkey(config.hotkey.clone())
        .build()
        .map_err(|e| e.to_string())?;

    *state.engine.lock().unwrap() = Some(engine);
    info!("Engine initialized");

    Ok(())
}

/// Start recording
#[tauri::command]
fn start_recording(state: State<AppState>) -> Result<(), String> {
    let mut engine = state.engine.lock().unwrap();

    if let Some(ref mut e) = *engine {
        e.handle_event(vd_core::EngineEvent::HotkeyPressed)
            .map_err(|e| e.to_string())?;
        debug!("Recording started");
        Ok(())
    } else {
        Err("Engine not initialized".to_string())
    }
}

/// Stop recording
#[tauri::command]
fn stop_recording(state: State<AppState>) -> Result<(), String> {
    let mut engine = state.engine.lock().unwrap();

    if let Some(ref mut e) = *engine {
        e.handle_event(vd_core::EngineEvent::HotkeyReleased)
            .map_err(|e| e.to_string())?;
        debug!("Recording stopped");
        Ok(())
    } else {
        Err("Engine not initialized".to_string())
    }
}

/// Cancel current operation
#[tauri::command]
fn cancel_operation(state: State<AppState>) -> Result<(), String> {
    let mut engine = state.engine.lock().unwrap();

    if let Some(ref mut e) = *engine {
        e.handle_event(vd_core::EngineEvent::Cancel)
            .map_err(|e| e.to_string())?;
        debug!("Operation cancelled");
        Ok(())
    } else {
        Err("Engine not initialized".to_string())
    }
}

fn main() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    info!("Starting Voice-Dict desktop application");

    // Ensure models directory exists
    let models_dir = vd_core::models_dir();
    if !models_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&models_dir) {
            error!("Failed to create models directory: {}", e);
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            get_status,
            get_config,
            set_config,
            check_models_available,
            get_model_info,
            get_models_dir,
            initialize_engine,
            start_recording,
            stop_recording,
            cancel_operation,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
