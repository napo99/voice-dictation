//! # vd-vad
//!
//! Voice Activity Detection for Voice-Dict using Silero VAD ONNX model.
//!
//! This crate provides real-time speech detection using the Silero VAD model,
//! which is lightweight and fast enough for streaming audio processing.
//!
//! ## Key Features
//!
//! - Sub-10ms inference time per audio chunk
//! - State machine for speech start/end detection
//! - Configurable sensitivity and timing parameters
//! - Memory-only processing (no disk writes)
//!
//! ## Example
//!
//! ```ignore
//! use vd_vad::{VoiceActivityDetector, VadConfig};
//! use vd_core::VadEvent;
//!
//! let model_path = vd_core::models_dir().join("silero_vad.onnx");
//! let mut vad = VoiceActivityDetector::new(&model_path, VadConfig::default())?;
//!
//! // Process audio chunks
//! let event = vad.process(&audio_samples)?;
//! match event {
//!     VadEvent::SpeechStart => println!("Speech started!"),
//!     VadEvent::SpeechEnd => println!("Speech ended!"),
//!     _ => {}
//! }
//! ```

use ndarray::{Array1, Array2, Array3};
use ort::{
    session::{builder::GraphOptimizationLevel, Session},
    value::Value,
};
use std::path::Path;
use tracing::{debug, info, trace};
use vd_core::{VadConfig, VadError, VadEvent, SAMPLE_RATE};

/// Silero VAD model constants
const VAD_SAMPLE_RATE: i64 = 16000;
const VAD_CHUNK_SIZE: usize = 512; // 32ms at 16kHz

/// Voice Activity Detector using Silero VAD
///
/// Processes audio chunks and detects speech start/end events.
pub struct VoiceActivityDetector {
    session: Session,
    config: VadConfig,
    /// Internal state for the LSTM model (h, c)
    state: (Array2<f32>, Array2<f32>),
    /// Sample rate tensor (needed by model)
    sr_tensor: Array1<i64>,
    /// Current speech state
    speech_active: bool,
    /// Samples since last state change
    samples_since_change: u32,
    /// Accumulated speech probability for smoothing
    prob_accumulator: f32,
    prob_count: u32,
}

impl VoiceActivityDetector {
    /// Create a new VAD instance
    ///
    /// # Arguments
    /// * `model_path` - Path to the silero_vad.onnx file
    /// * `config` - VAD configuration parameters
    pub fn new(model_path: &Path, config: VadConfig) -> Result<Self, VadError> {
        if !model_path.exists() {
            return Err(VadError::ModelLoadError(format!(
                "Model file not found: {}",
                model_path.display()
            )));
        }

        info!("Loading VAD model from: {}", model_path.display());

        let session = Session::builder()
            .map_err(|e| VadError::OnnxError(e.to_string()))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| VadError::OnnxError(e.to_string()))?
            .with_intra_threads(1)
            .map_err(|e| VadError::OnnxError(e.to_string()))?
            .commit_from_file(model_path)
            .map_err(|e| VadError::ModelLoadError(e.to_string()))?;

        info!("VAD model loaded successfully");

        // Initialize LSTM state (2 layers, 1 batch, 64 hidden size)
        let h = Array2::zeros((2, 64));
        let c = Array2::zeros((2, 64));
        let sr_tensor = Array1::from_vec(vec![VAD_SAMPLE_RATE]);

        Ok(Self {
            session,
            config,
            state: (h, c),
            sr_tensor,
            speech_active: false,
            samples_since_change: 0,
            prob_accumulator: 0.0,
            prob_count: 0,
        })
    }

    /// Process a chunk of audio samples and return a VAD event
    ///
    /// Audio should be:
    /// - 16kHz sample rate
    /// - Mono (single channel)
    /// - Float32 normalized to -1.0..1.0
    ///
    /// For best results, process chunks of 512 samples (32ms).
    pub fn process(&mut self, samples: &[f32]) -> Result<VadEvent, VadError> {
        // Process in chunks of VAD_CHUNK_SIZE
        let mut last_event = VadEvent::Silence;

        for chunk in samples.chunks(VAD_CHUNK_SIZE) {
            if chunk.len() < VAD_CHUNK_SIZE {
                // Pad short chunks with zeros
                let mut padded = vec![0.0f32; VAD_CHUNK_SIZE];
                padded[..chunk.len()].copy_from_slice(chunk);
                last_event = self.process_chunk(&padded)?;
            } else {
                last_event = self.process_chunk(chunk)?;
            }
        }

        Ok(last_event)
    }

    /// Process a single chunk of exactly VAD_CHUNK_SIZE samples
    fn process_chunk(&mut self, chunk: &[f32]) -> Result<VadEvent, VadError> {
        debug_assert_eq!(chunk.len(), VAD_CHUNK_SIZE);

        // Prepare input tensor: [1, chunk_size]
        let input = Array2::from_shape_vec((1, VAD_CHUNK_SIZE), chunk.to_vec())
            .map_err(|e| VadError::OnnxError(e.to_string()))?;

        // Prepare state tensors
        let h = self.state.0.clone().insert_axis(ndarray::Axis(0)); // [1, 2, 64]
        let c = self.state.1.clone().insert_axis(ndarray::Axis(0)); // [1, 2, 64]

        // Create ONNX values
        let input_value =
            Value::from_array(input).map_err(|e| VadError::OnnxError(e.to_string()))?;
        let sr_value = Value::from_array(self.sr_tensor.clone())
            .map_err(|e| VadError::OnnxError(e.to_string()))?;
        let h_value = Value::from_array(h).map_err(|e| VadError::OnnxError(e.to_string()))?;
        let c_value = Value::from_array(c).map_err(|e| VadError::OnnxError(e.to_string()))?;

        // Run inference
        let inputs = ort::inputs![input_value, sr_value, h_value, c_value]
            .map_err(|e| VadError::OnnxError(e.to_string()))?;
        let outputs = self
            .session
            .run(inputs)
            .map_err(|e| VadError::OnnxError(e.to_string()))?;

        // Extract outputs
        let prob: f32 = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| VadError::OnnxError(e.to_string()))?
            .view()
            .iter()
            .next()
            .copied()
            .unwrap_or(0.0);

        // Update LSTM state from outputs
        let new_h: Array3<f32> = outputs[1]
            .try_extract_tensor::<f32>()
            .map_err(|e| VadError::OnnxError(e.to_string()))?
            .to_owned()
            .into_dimensionality()
            .map_err(|e| VadError::OnnxError(e.to_string()))?;

        let new_c: Array3<f32> = outputs[2]
            .try_extract_tensor::<f32>()
            .map_err(|e| VadError::OnnxError(e.to_string()))?
            .to_owned()
            .into_dimensionality()
            .map_err(|e| VadError::OnnxError(e.to_string()))?;

        // Remove batch dimension from state
        self.state.0 = new_h.index_axis(ndarray::Axis(0), 0).to_owned();
        self.state.1 = new_c.index_axis(ndarray::Axis(0), 0).to_owned();

        // Update probability accumulator for smoothing
        self.prob_accumulator += prob;
        self.prob_count += 1;

        // Calculate samples in this chunk
        let samples_in_chunk = VAD_CHUNK_SIZE as u32;
        self.samples_since_change += samples_in_chunk;

        // Calculate time since last change in ms
        let time_since_change_ms = (self.samples_since_change as f32 / SAMPLE_RATE as f32) * 1000.0;

        // Use smoothed probability
        let avg_prob = self.prob_accumulator / self.prob_count as f32;

        trace!(
            "VAD prob: {:.3}, avg: {:.3}, speech_active: {}, time_since_change: {:.0}ms",
            prob,
            avg_prob,
            self.speech_active,
            time_since_change_ms
        );

        let event = if !self.speech_active {
            // Currently silent, check for speech start
            if avg_prob > self.config.threshold
                && time_since_change_ms >= self.config.min_speech_duration_ms as f32
            {
                self.speech_active = true;
                self.samples_since_change = 0;
                self.prob_accumulator = 0.0;
                self.prob_count = 0;
                debug!("Speech started (prob: {:.3})", avg_prob);
                VadEvent::SpeechStart
            } else if prob > self.config.threshold {
                // Speech detected but not long enough yet
                VadEvent::Silence
            } else {
                // Reset accumulator if we drop below threshold
                self.prob_accumulator = 0.0;
                self.prob_count = 0;
                VadEvent::Silence
            }
        } else {
            // Currently speaking, check for speech end
            if avg_prob < self.config.threshold
                && time_since_change_ms >= self.config.silence_duration_ms as f32
            {
                self.speech_active = false;
                self.samples_since_change = 0;
                self.prob_accumulator = 0.0;
                self.prob_count = 0;
                debug!("Speech ended (prob: {:.3})", avg_prob);
                VadEvent::SpeechEnd
            } else if prob < self.config.threshold {
                // Below threshold but not long enough to end
                VadEvent::SpeechContinue
            } else {
                // Reset silence counter if speech resumes
                self.samples_since_change = 0;
                self.prob_accumulator = 0.0;
                self.prob_count = 0;
                VadEvent::SpeechContinue
            }
        };

        Ok(event)
    }

    /// Reset the VAD state
    ///
    /// Call this when starting a new recording session.
    pub fn reset(&mut self) {
        self.state = (Array2::zeros((2, 64)), Array2::zeros((2, 64)));
        self.speech_active = false;
        self.samples_since_change = 0;
        self.prob_accumulator = 0.0;
        self.prob_count = 0;
        debug!("VAD state reset");
    }

    /// Check if speech is currently active
    pub fn is_speech_active(&self) -> bool {
        self.speech_active
    }

    /// Get the current configuration
    pub fn config(&self) -> &VadConfig {
        &self.config
    }

    /// Update the configuration
    pub fn set_config(&mut self, config: VadConfig) {
        self.config = config;
    }
}

/// Calculate the number of padding samples needed for speech start
///
/// This is used to include a small amount of audio before the detected
/// speech start, to ensure we don't cut off the beginning of words.
pub fn calculate_padding_samples(config: &VadConfig) -> usize {
    let samples_per_ms = SAMPLE_RATE / 1000;
    (config.speech_pad_ms * samples_per_ms) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vad_config_default() {
        let config = VadConfig::default();
        assert_eq!(config.threshold, 0.5);
        assert!(config.min_speech_duration_ms > 0);
        assert!(config.silence_duration_ms > 0);
    }

    #[test]
    fn test_calculate_padding_samples() {
        let config = VadConfig {
            speech_pad_ms: 100,
            ..Default::default()
        };
        let samples = calculate_padding_samples(&config);
        // 100ms at 16kHz = 1600 samples
        assert_eq!(samples, 1600);
    }

    #[test]
    fn test_vad_chunk_size() {
        // 512 samples at 16kHz = 32ms
        let duration_ms = VAD_CHUNK_SIZE as f32 / VAD_SAMPLE_RATE as f32 * 1000.0;
        assert!((duration_ms - 32.0).abs() < 0.1);
    }
}
