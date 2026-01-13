//! Pipeline integration tests
//!
//! Tests the full audio → transcription → polish → inject pipeline.
//! Note: Some tests require models to be downloaded.

use vd_core::{AudioBuffer, EngineEvent, EngineState, VadConfig, VadEvent, SAMPLE_RATE};
use vd_engine::{Engine, EngineBuilder, EngineConfig};

/// Test engine state transitions
#[test]
fn test_engine_state_machine() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();

    // Initial state should be Idle
    assert_eq!(engine.state(), EngineState::Idle);

    // Hotkey press → Listening
    engine.handle_event(EngineEvent::HotkeyPressed).unwrap();
    assert_eq!(engine.state(), EngineState::Listening);

    // Speech detected → Recording
    engine.handle_event(EngineEvent::SpeechDetected).unwrap();
    assert_eq!(engine.state(), EngineState::Recording);

    // Speech ended → Processing
    engine.handle_event(EngineEvent::SpeechEnded).unwrap();
    assert_eq!(engine.state(), EngineState::Processing);
}

/// Test cancel operation
#[test]
fn test_engine_cancel() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();

    // Start recording
    engine.handle_event(EngineEvent::HotkeyPressed).unwrap();
    engine.handle_event(EngineEvent::SpeechDetected).unwrap();
    assert_eq!(engine.state(), EngineState::Recording);

    // Cancel should return to Idle
    engine.handle_event(EngineEvent::Cancel).unwrap();
    assert_eq!(engine.state(), EngineState::Idle);
}

/// Test hotkey release during listening (no speech)
#[test]
fn test_hotkey_release_no_speech() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();

    // Press hotkey
    engine.handle_event(EngineEvent::HotkeyPressed).unwrap();
    assert_eq!(engine.state(), EngineState::Listening);

    // Release without speech → should go back to Idle
    engine.handle_event(EngineEvent::HotkeyReleased).unwrap();
    assert_eq!(engine.state(), EngineState::Idle);
}

/// Test hotkey release during recording
#[test]
fn test_hotkey_release_during_recording() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();

    // Start recording
    engine.handle_event(EngineEvent::HotkeyPressed).unwrap();
    engine.handle_event(EngineEvent::SpeechDetected).unwrap();
    assert_eq!(engine.state(), EngineState::Recording);

    // Release during recording → should process
    engine.handle_event(EngineEvent::HotkeyReleased).unwrap();
    assert_eq!(engine.state(), EngineState::Processing);
}

/// Test audio buffer accumulation
#[test]
fn test_audio_buffer_accumulation() {
    let mut engine = Engine::new(EngineConfig::default()).unwrap();

    // Start in recording state
    engine.handle_event(EngineEvent::HotkeyPressed).unwrap();
    engine.handle_event(EngineEvent::SpeechDetected).unwrap();

    // Simulate audio chunks
    let chunk1 = vd_core::AudioChunk {
        samples: vec![0.1, 0.2, 0.3],
        timestamp_ms: 0,
    };
    let chunk2 = vd_core::AudioChunk {
        samples: vec![0.4, 0.5, 0.6],
        timestamp_ms: 20,
    };

    engine.process_audio_chunk(chunk1, VadEvent::SpeechContinue);
    engine.process_audio_chunk(chunk2, VadEvent::SpeechContinue);

    // Buffer should have accumulated samples
    let buffer = engine.audio_buffer();
    assert_eq!(buffer.len(), 6);
}

/// Test builder pattern
#[test]
fn test_engine_builder() {
    let engine = EngineBuilder::new()
        .whisper_model(vd_core::WhisperModel::Tiny)
        .use_llm_polish(false)
        .hotkey("ctrl+alt+v".to_string())
        .build()
        .unwrap();

    assert_eq!(engine.state(), EngineState::Idle);
    assert!(!engine.is_running());
}

/// Test config path defaults
#[test]
fn test_config_paths() {
    let config = EngineConfig::default();

    // Paths should contain expected filenames
    assert!(config.whisper_model_path.to_string_lossy().contains("ggml-base.en.bin"));
    assert!(config.vad_model_path.to_string_lossy().contains("silero_vad.onnx"));
}
