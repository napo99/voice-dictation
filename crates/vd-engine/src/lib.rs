//! # vd-engine
//!
//! Orchestration engine for Voice-Dict.
//!
//! This crate coordinates all the voice dictation components:
//! - Audio capture from microphone
//! - Voice activity detection
//! - Speech-to-text transcription
//! - Text polishing
//! - Text injection into active window
//!
//! ## Threading Model
//!
//! The engine uses multiple threads for low-latency processing:
//! - **Main thread**: Tokio async runtime for UI/IPC
//! - **Audio thread**: Dedicated high-priority thread for capture
//! - **Inference thread**: Processing VAD, Whisper, and polishing
//!
//! Communication between threads uses `crossbeam-channel` for lock-free
//! message passing.
//!
//! ## State Machine
//!
//! The engine implements a state machine:
//! - `Idle` → (hotkey pressed) → `Listening`
//! - `Listening` → (speech detected) → `Recording`
//! - `Recording` → (speech ended) → `Processing`
//! - `Processing` → (transcription complete) → `Injecting`
//! - `Injecting` → (done) → `Idle`

use crossbeam_channel::{bounded, Receiver, Sender};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use tracing::{debug, error, info, warn};
use vd_core::{
    AppConfig, AudioBuffer, AudioChunk, EngineEvent, EngineState, TranscriptionResult,
    VadConfig, VadEvent, VoiceDictError, WhisperModel,
};

/// Channel capacity for audio chunks
const AUDIO_CHANNEL_CAPACITY: usize = 100;

/// Channel capacity for engine events
const EVENT_CHANNEL_CAPACITY: usize = 32;

/// Engine configuration
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Application configuration
    pub app: AppConfig,
    /// Path to Whisper model file
    pub whisper_model_path: PathBuf,
    /// Path to VAD model file
    pub vad_model_path: PathBuf,
}

impl Default for EngineConfig {
    fn default() -> Self {
        let models_dir = vd_core::models_dir();
        Self {
            app: AppConfig::default(),
            whisper_model_path: models_dir.join(WhisperModel::Base.filename()),
            vad_model_path: models_dir.join(vd_core::SILERO_VAD_MODEL.path),
        }
    }
}

/// Voice dictation engine
///
/// Coordinates audio capture, speech detection, transcription, and text injection.
pub struct Engine {
    config: EngineConfig,
    state: EngineState,
    running: Arc<AtomicBool>,

    // Channels for communication
    audio_tx: Option<Sender<AudioChunk>>,
    _audio_rx: Option<Receiver<AudioChunk>>,
    event_tx: Sender<EngineEvent>,
    event_rx: Receiver<EngineEvent>,

    // Thread handles
    _inference_thread: Option<JoinHandle<()>>,

    // Audio buffer for accumulating speech
    audio_buffer: AudioBuffer,
}

impl Engine {
    /// Create a new engine with the given configuration
    pub fn new(config: EngineConfig) -> Result<Self, VoiceDictError> {
        let (event_tx, event_rx) = bounded(EVENT_CHANNEL_CAPACITY);
        let (audio_tx, audio_rx) = bounded(AUDIO_CHANNEL_CAPACITY);

        Ok(Self {
            config,
            state: EngineState::Idle,
            running: Arc::new(AtomicBool::new(false)),
            audio_tx: Some(audio_tx),
            _audio_rx: Some(audio_rx),
            event_tx,
            event_rx,
            _inference_thread: None,
            audio_buffer: AudioBuffer::new(vd_core::SAMPLE_RATE),
        })
    }

    /// Get the audio sender for connecting to audio capture
    ///
    /// Call this before starting the engine to get the sender that
    /// should be passed to the audio capture component.
    pub fn audio_sender(&self) -> Option<Sender<AudioChunk>> {
        self.audio_tx.clone()
    }

    /// Get the event sender for sending events to the engine
    pub fn event_sender(&self) -> Sender<EngineEvent> {
        self.event_tx.clone()
    }

    /// Get the event receiver for receiving events from the engine
    pub fn event_receiver(&self) -> Receiver<EngineEvent> {
        self.event_rx.clone()
    }

    /// Get the current engine state
    pub fn state(&self) -> EngineState {
        self.state
    }

    /// Get the current configuration
    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// Check if the engine is running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Handle an incoming event
    pub fn handle_event(&mut self, event: EngineEvent) -> Result<(), VoiceDictError> {
        debug!("Handling event: {:?} in state: {:?}", event, self.state);

        match (&self.state, event) {
            // Idle state transitions
            (EngineState::Idle, EngineEvent::HotkeyPressed) => {
                self.transition_to(EngineState::Listening)?;
            }

            // Listening state transitions
            (EngineState::Listening, EngineEvent::HotkeyReleased) => {
                // If hotkey is released before speech detected, go back to idle
                self.transition_to(EngineState::Idle)?;
            }
            (EngineState::Listening, EngineEvent::SpeechDetected) => {
                self.transition_to(EngineState::Recording)?;
            }

            // Recording state transitions
            (EngineState::Recording, EngineEvent::SpeechEnded) => {
                self.transition_to(EngineState::Processing)?;
            }
            (EngineState::Recording, EngineEvent::HotkeyReleased) => {
                // Hotkey released during recording - process what we have
                self.transition_to(EngineState::Processing)?;
            }

            // Processing state transitions
            (EngineState::Processing, EngineEvent::TranscriptionComplete(result)) => {
                self.on_transcription_complete(result)?;
            }

            // Injecting state transitions
            (EngineState::Injecting, EngineEvent::InjectionComplete) => {
                self.transition_to(EngineState::Idle)?;
            }

            // Cancel from any active state
            (state, EngineEvent::Cancel) if state.is_active() => {
                self.transition_to(EngineState::Idle)?;
            }

            // Error handling
            (_, EngineEvent::Error(msg)) => {
                error!("Engine error: {}", msg);
                self.transition_to(EngineState::Error)?;
            }

            // Ignore invalid transitions
            (state, event) => {
                debug!("Ignoring event {:?} in state {:?}", event, state);
            }
        }

        Ok(())
    }

    /// Transition to a new state
    fn transition_to(&mut self, new_state: EngineState) -> Result<(), VoiceDictError> {
        let old_state = self.state;
        info!("State transition: {:?} -> {:?}", old_state, new_state);

        // Perform state exit actions
        match old_state {
            EngineState::Recording => {
                // Stop accumulating audio when leaving recording state
            }
            EngineState::Processing => {
                // Clear audio buffer after processing
                self.audio_buffer.clear();
            }
            _ => {}
        }

        // Update state
        self.state = new_state;

        // Perform state entry actions
        match new_state {
            EngineState::Listening => {
                debug!("Started listening for speech");
            }
            EngineState::Recording => {
                debug!("Started recording speech");
            }
            EngineState::Processing => {
                debug!("Processing audio buffer ({} samples)", self.audio_buffer.len());
            }
            EngineState::Injecting => {
                debug!("Injecting transcribed text");
            }
            EngineState::Idle => {
                debug!("Returned to idle state");
            }
            EngineState::Error => {
                warn!("Entered error state");
            }
        }

        Ok(())
    }

    /// Handle completed transcription
    fn on_transcription_complete(
        &mut self,
        result: TranscriptionResult,
    ) -> Result<(), VoiceDictError> {
        info!(
            "Transcription complete: '{}' ({:.0}ms processing)",
            result.text, result.processing_time_ms
        );

        if result.text.is_empty() {
            warn!("Empty transcription, returning to idle");
            return self.transition_to(EngineState::Idle);
        }

        // Move to injecting state
        self.transition_to(EngineState::Injecting)?;

        // TODO: Actually inject the text
        // For now, just transition back to idle
        self.handle_event(EngineEvent::InjectionComplete)?;

        Ok(())
    }

    /// Process audio chunks from the audio receiver
    ///
    /// This should be called from the inference thread to process
    /// incoming audio and update VAD state.
    pub fn process_audio_chunk(&mut self, chunk: AudioChunk, vad_event: VadEvent) {
        match (&self.state, vad_event) {
            (EngineState::Listening, VadEvent::SpeechStart) => {
                // Speech detected while listening
                let _ = self.event_tx.send(EngineEvent::SpeechDetected);
                self.audio_buffer.extend(&chunk.samples);
            }
            (EngineState::Recording, VadEvent::SpeechContinue | VadEvent::SpeechStart) => {
                // Continue accumulating audio
                self.audio_buffer.extend(&chunk.samples);
            }
            (EngineState::Recording, VadEvent::SpeechEnd) => {
                // Speech ended, trigger processing
                self.audio_buffer.extend(&chunk.samples);
                let _ = self.event_tx.send(EngineEvent::SpeechEnded);
            }
            (EngineState::Recording, VadEvent::Silence) => {
                // Brief silence during speech, keep accumulating
                self.audio_buffer.extend(&chunk.samples);
            }
            _ => {
                // Ignore audio in other states
            }
        }
    }

    /// Get the accumulated audio buffer
    pub fn audio_buffer(&self) -> &AudioBuffer {
        &self.audio_buffer
    }

    /// Take ownership of the audio buffer (for processing)
    pub fn take_audio_buffer(&mut self) -> AudioBuffer {
        std::mem::replace(&mut self.audio_buffer, AudioBuffer::new(vd_core::SAMPLE_RATE))
    }
}

/// Builder for Engine configuration
pub struct EngineBuilder {
    config: EngineConfig,
}

impl EngineBuilder {
    /// Create a new engine builder with default configuration
    pub fn new() -> Self {
        Self {
            config: EngineConfig::default(),
        }
    }

    /// Set the Whisper model to use
    pub fn whisper_model(mut self, model: WhisperModel) -> Self {
        self.config.whisper_model_path = vd_core::models_dir().join(model.filename());
        self.config.app.whisper_model = model;
        self
    }

    /// Set the VAD configuration
    pub fn vad_config(mut self, config: VadConfig) -> Self {
        self.config.app.vad = config;
        self
    }

    /// Set whether to use LLM polishing
    pub fn use_llm_polish(mut self, enabled: bool) -> Self {
        self.config.app.use_llm_polish = enabled;
        self
    }

    /// Set the input device name
    pub fn input_device(mut self, device: Option<String>) -> Self {
        self.config.app.input_device = device;
        self
    }

    /// Set the hotkey
    pub fn hotkey(mut self, hotkey: String) -> Self {
        self.config.app.hotkey = hotkey;
        self
    }

    /// Build the engine
    pub fn build(self) -> Result<Engine, VoiceDictError> {
        Engine::new(self.config)
    }
}

impl Default for EngineBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if all required models are available
pub fn check_models(config: &EngineConfig) -> Result<(), VoiceDictError> {
    if !config.whisper_model_path.exists() {
        return Err(VoiceDictError::Model(vd_core::ModelError::NotFound(
            config.whisper_model_path.display().to_string(),
        )));
    }

    if !config.vad_model_path.exists() {
        return Err(VoiceDictError::Model(vd_core::ModelError::NotFound(
            config.vad_model_path.display().to_string(),
        )));
    }

    Ok(())
}

/// Get paths to all required models
pub fn required_model_paths(config: &EngineConfig) -> Vec<PathBuf> {
    vec![
        config.whisper_model_path.clone(),
        config.vad_model_path.clone(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_config_default() {
        let config = EngineConfig::default();
        assert!(config.whisper_model_path.to_string_lossy().contains("ggml-base.en.bin"));
        assert!(config.vad_model_path.to_string_lossy().contains("silero_vad.onnx"));
    }

    #[test]
    fn test_engine_builder() {
        let engine = EngineBuilder::new()
            .whisper_model(WhisperModel::Tiny)
            .use_llm_polish(false)
            .build()
            .unwrap();

        assert_eq!(engine.state(), EngineState::Idle);
        assert!(!engine.is_running());
    }

    #[test]
    fn test_engine_state_transitions() {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();

        // Idle -> Listening
        engine.handle_event(EngineEvent::HotkeyPressed).unwrap();
        assert_eq!(engine.state(), EngineState::Listening);

        // Listening -> Recording
        engine.handle_event(EngineEvent::SpeechDetected).unwrap();
        assert_eq!(engine.state(), EngineState::Recording);

        // Recording -> Processing
        engine.handle_event(EngineEvent::SpeechEnded).unwrap();
        assert_eq!(engine.state(), EngineState::Processing);
    }

    #[test]
    fn test_engine_cancel() {
        let mut engine = Engine::new(EngineConfig::default()).unwrap();

        // Start listening
        engine.handle_event(EngineEvent::HotkeyPressed).unwrap();
        assert_eq!(engine.state(), EngineState::Listening);

        // Cancel should return to idle
        engine.handle_event(EngineEvent::Cancel).unwrap();
        assert_eq!(engine.state(), EngineState::Idle);
    }

    #[test]
    fn test_audio_channel_creation() {
        let engine = Engine::new(EngineConfig::default()).unwrap();
        assert!(engine.audio_sender().is_some());
    }
}
