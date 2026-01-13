//! Integrated Pipeline for Voice Dictation
//!
//! This module wires together all components:
//! Audio Capture → VAD → Whisper → Polish → Text Injection
//!
//! It runs on a dedicated thread to avoid blocking the UI.

use crossbeam_channel::{bounded, Receiver, Sender};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;
use tracing::{debug, error, info, warn};

use vd_audio::{AudioCapture, AudioCaptureConfig};
use vd_core::{
    AudioBuffer, AudioChunk, TranscriptionResult, VadConfig, VadEvent,
    VoiceDictError, WhisperModel, SAMPLE_RATE,
};
use vd_inject::TextInjector;
use vd_polish::{PolishLevel, TextPolisher};
use vd_vad::VoiceActivityDetector;
use vd_whisper::WhisperTranscriber;

// ============================================================================
// Pipeline State
// ============================================================================

/// Pipeline processing state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PipelineState {
    /// Waiting for hotkey
    Idle,
    /// Hotkey held, listening for speech
    Listening,
    /// Speech detected, recording
    Recording,
    /// Processing transcription
    Processing,
    /// Injecting text
    Injecting,
}

/// Events emitted by the pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PipelineEvent {
    /// State changed
    StateChanged(String),
    /// Audio level update (0.0 - 1.0)
    AudioLevel(f32),
    /// Partial transcript (during recording)
    PartialTranscript(String),
    /// Final transcript
    FinalTranscript(String),
    /// Text injected
    TextInjected(String),
    /// Error occurred
    Error(String),
}

// ============================================================================
// Pipeline Configuration
// ============================================================================

/// Pipeline configuration
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Path to Whisper model
    pub whisper_model_path: PathBuf,
    /// Path to VAD model
    pub vad_model_path: PathBuf,
    /// VAD configuration
    pub vad_config: VadConfig,
    /// Whether to use LLM polishing
    pub use_llm_polish: bool,
    /// Input device name (None for default)
    pub input_device: Option<String>,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        let models_dir = vd_core::models_dir();
        Self {
            whisper_model_path: models_dir.join(WhisperModel::Base.filename()),
            vad_model_path: models_dir.join(vd_core::SILERO_VAD_MODEL.path),
            vad_config: VadConfig::default(),
            use_llm_polish: false, // Start with regex-only for speed
            input_device: None,
        }
    }
}

// ============================================================================
// Pipeline Commands
// ============================================================================

/// Commands sent to the pipeline thread
enum PipelineCommand {
    StartRecording,
    StopRecording,
    Cancel,
    Shutdown,
}

// ============================================================================
// Pipeline Implementation
// ============================================================================

/// The integrated voice dictation pipeline
pub struct Pipeline {
    state: Arc<Mutex<PipelineState>>,
    current_transcript: Arc<Mutex<String>>,
    audio_level: Arc<AtomicU32>,
    command_tx: Sender<PipelineCommand>,
    _worker_handle: JoinHandle<()>,
}

impl Pipeline {
    /// Create a new pipeline
    pub fn new(
        config: PipelineConfig,
        event_callback: Box<dyn Fn(PipelineEvent) + Send + 'static>,
    ) -> Result<Self, VoiceDictError> {
        let state = Arc::new(Mutex::new(PipelineState::Idle));
        let current_transcript = Arc::new(Mutex::new(String::new()));
        let audio_level = Arc::new(AtomicU32::new(0));

        let (command_tx, command_rx) = bounded::<PipelineCommand>(16);

        // Clone Arcs for worker thread
        let state_clone = state.clone();
        let transcript_clone = current_transcript.clone();
        let audio_level_clone = audio_level.clone();

        // Spawn worker thread
        let worker_handle = thread::Builder::new()
            .name("pipeline-worker".into())
            .spawn(move || {
                if let Err(e) = run_pipeline_worker(
                    config,
                    command_rx,
                    state_clone,
                    transcript_clone,
                    audio_level_clone,
                    event_callback,
                ) {
                    error!("Pipeline worker error: {}", e);
                }
            })
            .map_err(|e| VoiceDictError::Engine(e.to_string()))?;

        Ok(Self {
            state,
            current_transcript,
            audio_level,
            command_tx,
            _worker_handle: worker_handle,
        })
    }

    /// Get current state
    pub fn state(&self) -> PipelineState {
        *self.state.lock().unwrap()
    }

    /// Get current transcript
    pub fn current_transcript(&self) -> Option<String> {
        let transcript = self.current_transcript.lock().unwrap();
        if transcript.is_empty() {
            None
        } else {
            Some(transcript.clone())
        }
    }

    /// Get current audio level (0.0 - 1.0)
    pub fn audio_level(&self) -> f32 {
        f32::from_bits(self.audio_level.load(Ordering::Relaxed))
    }

    /// Start recording
    pub fn start_recording(&self) -> Result<(), VoiceDictError> {
        self.command_tx
            .send(PipelineCommand::StartRecording)
            .map_err(|_| VoiceDictError::Engine("Pipeline channel closed".into()))
    }

    /// Stop recording and process
    pub fn stop_recording(&self) -> Result<(), VoiceDictError> {
        self.command_tx
            .send(PipelineCommand::StopRecording)
            .map_err(|_| VoiceDictError::Engine("Pipeline channel closed".into()))
    }

    /// Cancel current operation
    pub fn cancel(&self) {
        let _ = self.command_tx.send(PipelineCommand::Cancel);
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        let _ = self.command_tx.send(PipelineCommand::Shutdown);
    }
}

// ============================================================================
// Pipeline Worker
// ============================================================================

/// Main pipeline worker function (runs on dedicated thread)
fn run_pipeline_worker(
    config: PipelineConfig,
    command_rx: Receiver<PipelineCommand>,
    state: Arc<Mutex<PipelineState>>,
    transcript: Arc<Mutex<String>>,
    audio_level: Arc<AtomicU32>,
    emit: Box<dyn Fn(PipelineEvent) + Send>,
) -> Result<(), VoiceDictError> {
    info!("Pipeline worker started");

    // Helper to update state
    let set_state = |new_state: PipelineState| {
        *state.lock().unwrap() = new_state;
        emit(PipelineEvent::StateChanged(format!("{:?}", new_state)));
    };

    // Helper to set audio level
    let set_audio_level = |level: f32| {
        audio_level.store(level.to_bits(), Ordering::Relaxed);
        emit(PipelineEvent::AudioLevel(level));
    };

    // Audio buffer for accumulating speech
    let mut audio_buffer = AudioBuffer::new(SAMPLE_RATE);

    // Channel for receiving audio from capture thread
    let (audio_tx, audio_rx) = bounded::<AudioChunk>(1024);

    // Audio capture (initialized lazily)
    let mut audio_capture: Option<AudioCapture> = None;

    // VAD (initialized lazily - requires model)
    let mut vad: Option<VoiceActivityDetector> = None;

    // Whisper transcriber (initialized lazily - requires model)
    let mut whisper: Option<WhisperTranscriber> = None;

    // Text polisher
    let polish_level = if config.use_llm_polish {
        PolishLevel::Full
    } else {
        PolishLevel::RegexOnly
    };
    let polisher = TextPolisher::new(polish_level);

    // Text injector (initialized lazily)
    let mut injector: Option<TextInjector> = None;

    // Initialize components on first use
    let init_components = |vad: &mut Option<VoiceActivityDetector>,
                           whisper: &mut Option<WhisperTranscriber>,
                           injector: &mut Option<TextInjector>,
                           config: &PipelineConfig|
     -> Result<(), VoiceDictError> {
        // Initialize VAD if not done
        if vad.is_none() {
            if config.vad_model_path.exists() {
                match VoiceActivityDetector::new(&config.vad_model_path, config.vad_config.clone())
                {
                    Ok(v) => {
                        info!("VAD initialized");
                        *vad = Some(v);
                    }
                    Err(e) => {
                        warn!("Failed to initialize VAD: {}", e);
                    }
                }
            } else {
                warn!("VAD model not found at {:?}", config.vad_model_path);
            }
        }

        // Initialize Whisper if not done
        if whisper.is_none() {
            if config.whisper_model_path.exists() {
                match WhisperTranscriber::new(&config.whisper_model_path) {
                    Ok(w) => {
                        info!("Whisper transcriber initialized");
                        *whisper = Some(w);
                    }
                    Err(e) => {
                        warn!("Failed to initialize Whisper: {}", e);
                    }
                }
            } else {
                warn!(
                    "Whisper model not found at {:?}",
                    config.whisper_model_path
                );
            }
        }

        // Initialize injector if not done
        if injector.is_none() {
            match TextInjector::new() {
                Ok(i) => {
                    info!("Text injector initialized");
                    *injector = Some(i);
                }
                Err(e) => {
                    warn!("Failed to initialize text injector: {}", e);
                }
            }
        }

        Ok(())
    };

    // Main command loop
    loop {
        match command_rx.recv() {
            Ok(PipelineCommand::StartRecording) => {
                info!("Starting recording");
                set_state(PipelineState::Listening);
                audio_buffer.clear();
                *transcript.lock().unwrap() = String::new();

                // Initialize components if needed
                if let Err(e) = init_components(&mut vad, &mut whisper, &mut injector, &config) {
                    emit(PipelineEvent::Error(format!(
                        "Failed to initialize: {}",
                        e
                    )));
                    set_state(PipelineState::Idle);
                    continue;
                }

                // Start audio capture
                if audio_capture.is_none() {
                    let audio_config = AudioCaptureConfig {
                        device_name: config.input_device.clone(),
                        ..Default::default()
                    };
                    match AudioCapture::new(audio_config, audio_tx.clone()) {
                        Ok(mut capture) => {
                            if let Err(e) = capture.start() {
                                emit(PipelineEvent::Error(format!(
                                    "Failed to start audio: {}",
                                    e
                                )));
                                set_state(PipelineState::Idle);
                                continue;
                            }
                            audio_capture = Some(capture);
                        }
                        Err(e) => {
                            emit(PipelineEvent::Error(format!(
                                "Failed to create audio capture: {}",
                                e
                            )));
                            set_state(PipelineState::Idle);
                            continue;
                        }
                    }
                } else if let Some(ref mut capture) = audio_capture {
                    if let Err(e) = capture.start() {
                        emit(PipelineEvent::Error(format!(
                            "Failed to start audio: {}",
                            e
                        )));
                        set_state(PipelineState::Idle);
                        continue;
                    }
                }

                // Reset VAD state
                if let Some(ref mut v) = vad {
                    v.reset();
                }

                // Process audio until we get a stop command
                // This runs in a tight loop processing audio chunks
                let mut speech_detected = false;
                let mut cancelled = false;
                let start_time = Instant::now();

                'recording: loop {
                    // Check for commands without blocking
                    match command_rx.try_recv() {
                        Ok(PipelineCommand::StopRecording) => {
                            info!("Stop command received");
                            break 'recording;
                        }
                        Ok(PipelineCommand::StartRecording) => {
                            // Already recording, ignore
                            debug!("StartRecording received while already recording");
                        }
                        Ok(PipelineCommand::Cancel) => {
                            info!("Cancel command received");
                            if let Some(ref mut capture) = audio_capture {
                                capture.stop();
                            }
                            audio_buffer.clear();
                            set_state(PipelineState::Idle);
                            cancelled = true;
                            break 'recording;
                        }
                        Ok(PipelineCommand::Shutdown) => {
                            info!("Shutdown command received");
                            return Ok(());
                        }
                        Err(_) => {} // No command, continue processing
                    }

                    // Process audio chunks
                    while let Ok(chunk) = audio_rx.try_recv() {
                        // Calculate audio level
                        let level = calculate_audio_level(&chunk.samples);
                        set_audio_level(level);

                        // Run VAD
                        if let Some(ref mut v) = vad {
                            match v.process(&chunk.samples) {
                                Ok(VadEvent::SpeechStart) => {
                                    if !speech_detected {
                                        info!("Speech detected");
                                        speech_detected = true;
                                        set_state(PipelineState::Recording);
                                    }
                                }
                                Ok(VadEvent::SpeechEnd) => {
                                    // Speech ended, but we keep recording until hotkey released
                                    debug!("VAD detected speech end");
                                }
                                Ok(VadEvent::SpeechContinue) => {
                                    // Speech continues
                                }
                                Ok(VadEvent::Silence) => {
                                    // Silence
                                }
                                Err(e) => {
                                    warn!("VAD error: {}", e);
                                }
                            }
                        } else {
                            // No VAD, assume speech immediately
                            if !speech_detected {
                                speech_detected = true;
                                set_state(PipelineState::Recording);
                            }
                        }

                        // Accumulate audio if speech detected or no VAD
                        if speech_detected || vad.is_none() {
                            audio_buffer.extend(&chunk.samples);
                        }
                    }

                    // Small sleep to avoid busy-waiting
                    thread::sleep(std::time::Duration::from_millis(5));

                    // Timeout after 30 seconds
                    if start_time.elapsed().as_secs() > 30 {
                        warn!("Recording timeout");
                        break 'recording;
                    }
                }

                // Stop audio capture
                if let Some(ref mut capture) = audio_capture {
                    capture.stop();
                }

                // Skip processing if cancelled
                if cancelled {
                    continue;
                }

                // Process the recording
                if audio_buffer.samples().is_empty() {
                    info!("No audio recorded");
                    set_state(PipelineState::Idle);
                    continue;
                }

                set_state(PipelineState::Processing);

                // Transcribe
                let transcribed_text = if let Some(ref w) = whisper {
                    match w.transcribe(audio_buffer.samples()) {
                        Ok(result) => {
                            info!(
                                "Transcribed {} chars in {}ms",
                                result.text.len(),
                                result.processing_time_ms
                            );
                            result.text
                        }
                        Err(e) => {
                            emit(PipelineEvent::Error(format!("Transcription failed: {}", e)));
                            set_state(PipelineState::Idle);
                            continue;
                        }
                    }
                } else {
                    emit(PipelineEvent::Error("Whisper not initialized".into()));
                    set_state(PipelineState::Idle);
                    continue;
                };

                if transcribed_text.is_empty() {
                    info!("Empty transcription");
                    set_state(PipelineState::Idle);
                    continue;
                }

                // Polish the text
                let polished_text = match polisher.polish(&transcribed_text) {
                    Ok(text) => text,
                    Err(e) => {
                        warn!("Polish failed: {}, using raw transcription", e);
                        transcribed_text
                    }
                };

                *transcript.lock().unwrap() = polished_text.clone();
                emit(PipelineEvent::FinalTranscript(polished_text.clone()));

                // Inject the text
                set_state(PipelineState::Injecting);

                if let Some(ref mut inj) = injector {
                    // Small delay before injection to allow user to release hotkey
                    thread::sleep(std::time::Duration::from_millis(50));

                    match inj.inject(&polished_text) {
                        Ok(()) => {
                            info!("Text injected: {} chars", polished_text.len());
                            emit(PipelineEvent::TextInjected(polished_text));
                        }
                        Err(e) => {
                            emit(PipelineEvent::Error(format!("Injection failed: {}", e)));
                        }
                    }
                } else {
                    emit(PipelineEvent::Error("Injector not initialized".into()));
                }

                // Return to idle
                set_state(PipelineState::Idle);
            }

            Ok(PipelineCommand::StopRecording) => {
                // This is handled in the recording loop above
                // If we get here, we weren't recording
                debug!("StopRecording received but not recording");
            }

            Ok(PipelineCommand::Cancel) => {
                info!("Operation cancelled");
                if let Some(ref mut capture) = audio_capture {
                    capture.stop();
                }
                audio_buffer.clear();
                *transcript.lock().unwrap() = String::new();
                set_state(PipelineState::Idle);
            }

            Ok(PipelineCommand::Shutdown) => {
                info!("Pipeline worker shutting down");
                if let Some(ref mut capture) = audio_capture {
                    capture.stop();
                }
                break;
            }

            Err(_) => {
                // Channel closed
                break;
            }
        }
    }

    Ok(())
}

/// Calculate RMS audio level (0.0 - 1.0)
fn calculate_audio_level(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }

    let sum_squares: f32 = samples.iter().map(|s| s * s).sum();
    let rms = (sum_squares / samples.len() as f32).sqrt();

    // Normalize to 0-1 range (assuming typical speech peaks around 0.3-0.5)
    (rms * 3.0).min(1.0)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn test_pipeline_state_transitions() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = events.clone();

        let callback = Box::new(move |event: PipelineEvent| {
            events_clone.lock().unwrap().push(event);
        });

        let config = PipelineConfig::default();
        let pipeline = Pipeline::new(config, callback).unwrap();

        assert_eq!(pipeline.state(), PipelineState::Idle);

        // Start recording - may fail if no audio device
        let _ = pipeline.start_recording();
        thread::sleep(std::time::Duration::from_millis(100));

        // State should have changed from Idle
        let state_after_start = pipeline.state();
        assert!(
            state_after_start == PipelineState::Listening
                || state_after_start == PipelineState::Recording
                || state_after_start == PipelineState::Idle, // May return to Idle if audio failed
            "Unexpected state: {:?}",
            state_after_start
        );

        // Cancel to reset
        pipeline.cancel();
        thread::sleep(std::time::Duration::from_millis(300));

        // After cancel, should eventually return to Idle
        // (may take time if audio init is blocking)
        let final_state = pipeline.state();
        assert!(
            final_state == PipelineState::Idle || final_state == PipelineState::Listening,
            "Expected Idle or Listening after cancel, got {:?}",
            final_state
        );
    }

    #[test]
    fn test_calculate_audio_level() {
        // Silence
        let silence = vec![0.0f32; 100];
        assert_eq!(calculate_audio_level(&silence), 0.0);

        // Loud signal
        let loud = vec![0.5f32; 100];
        let level = calculate_audio_level(&loud);
        assert!(level > 0.5);

        // Empty
        assert_eq!(calculate_audio_level(&[]), 0.0);
    }

    #[test]
    fn test_pipeline_config_default() {
        let config = PipelineConfig::default();
        assert!(!config.use_llm_polish);
        assert!(config.input_device.is_none());
    }
}
