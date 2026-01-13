//! # vd-core
//!
//! Core types, traits, and error definitions for Voice-Dict.
//!
//! This crate provides the foundational types shared across all Voice-Dict components:
//! - Audio sample types and buffers
//! - Transcription results
//! - VAD (Voice Activity Detection) events
//! - Engine state machine states
//! - Error types for the entire application

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

/// Sample rate used throughout the application (16kHz for Whisper compatibility)
pub const SAMPLE_RATE: u32 = 16000;

/// Number of audio channels (mono for speech recognition)
pub const CHANNELS: u16 = 1;

/// Audio chunk size in samples (20ms at 16kHz = 320 samples)
pub const CHUNK_SIZE: usize = 320;

// ============================================================================
// Error Types
// ============================================================================

/// Top-level error type for Voice-Dict operations
#[derive(Error, Debug)]
pub enum VoiceDictError {
    /// Audio capture or device errors
    #[error("Audio error: {0}")]
    Audio(#[from] AudioError),

    /// Voice activity detection errors
    #[error("VAD error: {0}")]
    Vad(#[from] VadError),

    /// Whisper transcription errors
    #[error("Transcription error: {0}")]
    Transcription(#[from] TranscriptionError),

    /// Text polishing errors
    #[error("Polish error: {0}")]
    Polish(#[from] PolishError),

    /// Key injection errors
    #[error("Injection error: {0}")]
    Injection(#[from] InjectionError),

    /// Model management errors
    #[error("Model error: {0}")]
    Model(#[from] ModelError),

    /// Engine orchestration errors
    #[error("Engine error: {0}")]
    Engine(String),
}

/// Audio capture and device errors
#[derive(Error, Debug)]
pub enum AudioError {
    #[error("No input device available")]
    NoInputDevice,

    #[error("Failed to get device config: {0}")]
    ConfigError(String),

    #[error("Failed to build audio stream: {0}")]
    StreamBuildError(String),

    #[error("Failed to start audio stream: {0}")]
    StreamStartError(String),

    #[error("Device disconnected")]
    DeviceDisconnected,

    #[error("Buffer overflow - audio processing too slow")]
    BufferOverflow,
}

/// Voice activity detection errors
#[derive(Error, Debug)]
pub enum VadError {
    #[error("Failed to load VAD model: {0}")]
    ModelLoadError(String),

    #[error("ONNX runtime error: {0}")]
    OnnxError(String),

    #[error("Invalid audio format: expected {expected}, got {got}")]
    InvalidAudioFormat { expected: String, got: String },
}

/// Whisper transcription errors
#[derive(Error, Debug)]
pub enum TranscriptionError {
    #[error("Failed to load Whisper model: {0}")]
    ModelLoadError(String),

    #[error("Transcription failed: {0}")]
    InferenceError(String),

    #[error("Audio too short for transcription")]
    AudioTooShort,

    #[error("Audio too long for transcription (max 30s)")]
    AudioTooLong,
}

/// Text polishing errors
#[derive(Error, Debug)]
pub enum PolishError {
    #[error("Failed to load LLM model: {0}")]
    ModelLoadError(String),

    #[error("Tokenization error: {0}")]
    TokenizationError(String),

    #[error("Inference error: {0}")]
    InferenceError(String),

    #[error("Regex compilation error: {0}")]
    RegexError(String),
}

/// Key injection errors
#[derive(Error, Debug)]
pub enum InjectionError {
    #[error("Failed to initialize input handler: {0}")]
    InitError(String),

    #[error("Failed to inject text: {0}")]
    InjectionFailed(String),

    #[error("No active window found")]
    NoActiveWindow,

    #[error("Platform not supported for injection")]
    UnsupportedPlatform,
}

/// Model download and management errors
#[derive(Error, Debug)]
pub enum ModelError {
    #[error("Model not found: {0}")]
    NotFound(String),

    #[error("Download failed: {0}")]
    DownloadFailed(String),

    #[error("Checksum mismatch for model: {0}")]
    ChecksumMismatch(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

// ============================================================================
// Audio Types
// ============================================================================

/// A buffer of audio samples, kept in memory only (never written to disk)
#[derive(Debug, Clone)]
pub struct AudioBuffer {
    /// Raw audio samples (f32 normalized to -1.0..1.0)
    samples: Vec<f32>,
    /// Sample rate in Hz
    sample_rate: u32,
}

impl AudioBuffer {
    /// Create a new empty audio buffer
    pub fn new(sample_rate: u32) -> Self {
        Self {
            samples: Vec::new(),
            sample_rate,
        }
    }

    /// Create an audio buffer with pre-allocated capacity
    pub fn with_capacity(sample_rate: u32, capacity: usize) -> Self {
        Self {
            samples: Vec::with_capacity(capacity),
            sample_rate,
        }
    }

    /// Create an audio buffer from existing samples
    pub fn from_samples(samples: Vec<f32>, sample_rate: u32) -> Self {
        Self {
            samples,
            sample_rate,
        }
    }

    /// Append samples to the buffer
    pub fn extend(&mut self, samples: &[f32]) {
        self.samples.extend_from_slice(samples);
    }

    /// Get the samples as a slice
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    /// Get the sample rate
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Get the duration in seconds
    pub fn duration_secs(&self) -> f32 {
        self.samples.len() as f32 / self.sample_rate as f32
    }

    /// Get the number of samples
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Check if the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Clear the buffer
    pub fn clear(&mut self) {
        self.samples.clear();
    }

    /// Take ownership of the samples, consuming the buffer
    pub fn into_samples(self) -> Vec<f32> {
        self.samples
    }
}

/// Audio chunk for streaming processing
#[derive(Debug, Clone)]
pub struct AudioChunk {
    /// Raw samples for this chunk
    pub samples: Vec<f32>,
    /// Sample rate of these samples (may differ from SAMPLE_RATE)
    pub sample_rate: u32,
    /// Timestamp in milliseconds from recording start
    pub timestamp_ms: u64,
}

// ============================================================================
// VAD Types
// ============================================================================

/// Voice activity detection result
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VadEvent {
    /// Speech detected, start buffering audio
    SpeechStart,
    /// Speech continues
    SpeechContinue,
    /// Speech ended, process buffered audio
    SpeechEnd,
    /// No speech detected
    Silence,
}

/// VAD configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VadConfig {
    /// Probability threshold for speech detection (0.0..1.0)
    pub threshold: f32,
    /// Minimum speech duration in milliseconds before triggering
    pub min_speech_duration_ms: u32,
    /// Silence duration in milliseconds before ending speech
    pub silence_duration_ms: u32,
    /// Padding to add before speech start in milliseconds
    pub speech_pad_ms: u32,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            threshold: 0.3,
            min_speech_duration_ms: 100,
            silence_duration_ms: 500,
            speech_pad_ms: 100,
        }
    }
}

// ============================================================================
// Transcription Types
// ============================================================================

/// Result from Whisper transcription
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionResult {
    /// The transcribed text
    pub text: String,
    /// Confidence score (0.0..1.0)
    pub confidence: f32,
    /// Processing time in milliseconds
    pub processing_time_ms: u64,
    /// Audio duration that was transcribed
    pub audio_duration_ms: u64,
}

impl TranscriptionResult {
    /// Create a new transcription result
    pub fn new(
        text: String,
        confidence: f32,
        processing_time_ms: u64,
        audio_duration_ms: u64,
    ) -> Self {
        Self {
            text,
            confidence,
            processing_time_ms,
            audio_duration_ms,
        }
    }
}

/// Whisper model size options
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WhisperModel {
    /// Tiny model (~39MB) - fastest, lower accuracy
    Tiny,
    /// Base model (~74MB) - fast, decent accuracy
    Base,
    /// Small model (~244MB) - better accuracy (recommended)
    #[default]
    Small,
    /// Medium model (~769MB) - high accuracy
    Medium,
}

impl WhisperModel {
    /// Get the model filename
    pub fn filename(&self) -> &'static str {
        match self {
            WhisperModel::Tiny => "ggml-tiny.en.bin",
            WhisperModel::Base => "ggml-base.en.bin",
            WhisperModel::Small => "ggml-small.en.bin",
            WhisperModel::Medium => "ggml-medium.en.bin",
        }
    }

    /// Get the HuggingFace download URL
    pub fn download_url(&self) -> &'static str {
        match self {
            WhisperModel::Tiny => {
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin"
            }
            WhisperModel::Base => {
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin"
            }
            WhisperModel::Small => {
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin"
            }
            WhisperModel::Medium => {
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.en.bin"
            }
        }
    }

    /// Get approximate model size in bytes
    pub fn size_bytes(&self) -> u64 {
        match self {
            WhisperModel::Tiny => 39_000_000,
            WhisperModel::Base => 74_000_000,
            WhisperModel::Small => 244_000_000,
            WhisperModel::Medium => 769_000_000,
        }
    }
}

// ============================================================================
// Engine State Machine
// ============================================================================

/// Current state of the voice dictation engine
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EngineState {
    /// Engine is idle, waiting for hotkey
    Idle,
    /// Listening for speech (hotkey pressed, recording active)
    Listening,
    /// Speech detected, buffering audio
    Recording,
    /// Processing audio (VAD detected end of speech)
    Processing,
    /// Injecting transcribed text into active window
    Injecting,
    /// Error state, requires user action
    Error,
}

impl EngineState {
    /// Check if the engine is in an active state (not idle or error)
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Listening | Self::Recording | Self::Processing | Self::Injecting
        )
    }
}

/// Events that can transition the engine state
#[derive(Debug, Clone)]
pub enum EngineEvent {
    /// Hotkey was pressed
    HotkeyPressed,
    /// Hotkey was released
    HotkeyReleased,
    /// Voice activity detected
    SpeechDetected,
    /// End of speech detected
    SpeechEnded,
    /// Transcription completed
    TranscriptionComplete(TranscriptionResult),
    /// Text injection completed
    InjectionComplete,
    /// An error occurred
    Error(String),
    /// Cancel current operation
    Cancel,
}

// ============================================================================
// Configuration
// ============================================================================

/// Main application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Selected input device name (None for default)
    pub input_device: Option<String>,
    /// Whisper model to use
    pub whisper_model: WhisperModel,
    /// VAD configuration
    pub vad: VadConfig,
    /// Whether to use LLM polishing (in addition to regex)
    pub use_llm_polish: bool,
    /// Hotkey configuration (platform-specific string)
    pub hotkey: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            input_device: None,
            whisper_model: WhisperModel::Base,
            vad: VadConfig::default(),
            use_llm_polish: true,
            hotkey: "ctrl+shift+space".to_string(),
        }
    }
}

// ============================================================================
// Model Management
// ============================================================================

/// Information about a required model
#[derive(Debug, Clone)]
pub struct ModelInfo {
    /// Model identifier
    pub name: &'static str,
    /// Download URL
    pub url: &'static str,
    /// Expected file size in bytes
    pub size_bytes: u64,
    /// Relative path within models directory
    pub path: &'static str,
}

/// Silero VAD model info
pub const SILERO_VAD_MODEL: ModelInfo = ModelInfo {
    name: "Silero VAD",
    url: "https://github.com/snakers4/silero-vad/raw/master/files/silero_vad.onnx",
    size_bytes: 2_000_000,
    path: "silero_vad.onnx",
};

/// Get the models directory path
pub fn models_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("voice-dict")
        .join("models")
}

/// Get the config directory path
pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("voice-dict")
}

// ============================================================================
// Result Type
// ============================================================================

/// Convenience type alias for Voice-Dict results
pub type Result<T> = std::result::Result<T, VoiceDictError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_buffer_operations() {
        let mut buffer = AudioBuffer::new(SAMPLE_RATE);
        assert!(buffer.is_empty());
        assert_eq!(buffer.len(), 0);
        assert_eq!(buffer.duration_secs(), 0.0);

        buffer.extend(&[0.1, 0.2, 0.3]);
        assert_eq!(buffer.len(), 3);
        assert!(!buffer.is_empty());

        let samples = buffer.samples();
        assert_eq!(samples, &[0.1, 0.2, 0.3]);
    }

    #[test]
    fn test_audio_buffer_duration() {
        let samples: Vec<f32> = vec![0.0; SAMPLE_RATE as usize]; // 1 second of audio
        let buffer = AudioBuffer::from_samples(samples, SAMPLE_RATE);
        assert!((buffer.duration_secs() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_engine_state_is_active() {
        assert!(!EngineState::Idle.is_active());
        assert!(EngineState::Listening.is_active());
        assert!(EngineState::Recording.is_active());
        assert!(EngineState::Processing.is_active());
        assert!(EngineState::Injecting.is_active());
        assert!(!EngineState::Error.is_active());
    }

    #[test]
    fn test_vad_config_default() {
        let config = VadConfig::default();
        assert_eq!(config.threshold, 0.3);
        assert_eq!(config.min_speech_duration_ms, 100);
        assert_eq!(config.silence_duration_ms, 500);
    }

    #[test]
    fn test_whisper_model_info() {
        assert_eq!(WhisperModel::Base.filename(), "ggml-base.en.bin");
        assert!(WhisperModel::Base.download_url().contains("huggingface"));
    }
}
