//! # vd-whisper
//!
//! Whisper transcription for Voice-Dict using whisper-rs.
//!
//! This crate provides local speech-to-text transcription using OpenAI's
//! Whisper model via the whisper.cpp bindings.
//!
//! ## Key Features
//!
//! - Local-only transcription (no cloud APIs)
//! - Multiple model sizes (tiny, base, small, medium)
//! - Optimized for English language
//! - In-memory audio processing
//!
//! ## Example
//!
//! ```ignore
//! use vd_whisper::WhisperTranscriber;
//! use vd_core::WhisperModel;
//!
//! let model_path = vd_core::models_dir().join(WhisperModel::Base.filename());
//! let transcriber = WhisperTranscriber::new(&model_path)?;
//!
//! let result = transcriber.transcribe(&audio_samples)?;
//! println!("Transcribed: {}", result.text);
//! ```

use std::path::Path;
use std::time::Instant;
use tracing::{debug, info, warn};
use vd_core::{AudioBuffer, TranscriptionError, TranscriptionResult, SAMPLE_RATE};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// Minimum audio duration in seconds for transcription
const MIN_AUDIO_DURATION: f32 = 0.5;

/// Maximum audio duration in seconds (Whisper limit)
const MAX_AUDIO_DURATION: f32 = 30.0;

/// Whisper transcriber
///
/// Provides speech-to-text transcription using a local Whisper model.
pub struct WhisperTranscriber {
    ctx: WhisperContext,
}

impl WhisperTranscriber {
    /// Create a new transcriber with the specified model
    ///
    /// # Arguments
    /// * `model_path` - Path to the GGML Whisper model file (e.g., ggml-base.en.bin)
    pub fn new(model_path: &Path) -> Result<Self, TranscriptionError> {
        if !model_path.exists() {
            return Err(TranscriptionError::ModelLoadError(format!(
                "Model file not found: {}",
                model_path.display()
            )));
        }

        info!("Loading Whisper model from: {}", model_path.display());

        let params = WhisperContextParameters::default();
        let ctx = WhisperContext::new_with_params(
            model_path.to_str().ok_or_else(|| {
                TranscriptionError::ModelLoadError("Invalid model path encoding".to_string())
            })?,
            params,
        )
        .map_err(|e| TranscriptionError::ModelLoadError(e.to_string()))?;

        info!("Whisper model loaded successfully");

        Ok(Self { ctx })
    }

    /// Transcribe audio samples to text
    ///
    /// # Arguments
    /// * `samples` - Audio samples (16kHz, mono, f32 normalized to -1.0..1.0)
    ///
    /// # Returns
    /// * `TranscriptionResult` containing the transcribed text and metadata
    pub fn transcribe(&self, samples: &[f32]) -> Result<TranscriptionResult, TranscriptionError> {
        let duration_secs = samples.len() as f32 / SAMPLE_RATE as f32;

        if duration_secs < MIN_AUDIO_DURATION {
            return Err(TranscriptionError::AudioTooShort);
        }

        if duration_secs > MAX_AUDIO_DURATION {
            return Err(TranscriptionError::AudioTooLong);
        }

        debug!(
            "Transcribing {:.2}s of audio ({} samples)",
            duration_secs,
            samples.len()
        );

        let start = Instant::now();

        // Create transcription state
        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| TranscriptionError::InferenceError(e.to_string()))?;

        // Configure transcription parameters
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

        // Optimize for speed and English
        params.set_language(Some("en"));
        params.set_translate(false);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_token_timestamps(false);
        params.set_single_segment(true);
        params.set_no_context(true);

        // Suppress non-speech tokens
        params.set_suppress_blank(true);
        params.set_suppress_non_speech_tokens(true);

        // Run transcription
        state
            .full(params, samples)
            .map_err(|e| TranscriptionError::InferenceError(e.to_string()))?;

        // Extract results
        let num_segments = state.full_n_segments().map_err(|e| {
            TranscriptionError::InferenceError(format!("Failed to get segments: {}", e))
        })?;

        let mut text = String::new();
        let mut total_prob = 0.0f32;
        let mut prob_count = 0;

        for i in 0..num_segments {
            if let Ok(segment_text) = state.full_get_segment_text(i) {
                text.push_str(&segment_text);
            }

            // Get average probability for confidence calculation
            let n_tokens = state.full_n_tokens(i).unwrap_or(0);
            for j in 0..n_tokens {
                if let Ok(prob) = state.full_get_token_prob(i, j) {
                    total_prob += prob;
                    prob_count += 1;
                }
            }
        }

        let processing_time_ms = start.elapsed().as_millis() as u64;
        let audio_duration_ms = (duration_secs * 1000.0) as u64;

        // Calculate average confidence
        let confidence = if prob_count > 0 {
            total_prob / prob_count as f32
        } else {
            0.0
        };

        // Clean up the text
        let text = text.trim().to_string();

        debug!(
            "Transcription complete: {} chars, {:.2}s processing, {:.2} confidence",
            text.len(),
            processing_time_ms as f32 / 1000.0,
            confidence
        );

        if text.is_empty() {
            warn!("Transcription produced empty result");
        }

        Ok(TranscriptionResult::new(
            text,
            confidence,
            processing_time_ms,
            audio_duration_ms,
        ))
    }

    /// Transcribe an AudioBuffer
    pub fn transcribe_buffer(
        &self,
        buffer: &AudioBuffer,
    ) -> Result<TranscriptionResult, TranscriptionError> {
        self.transcribe(buffer.samples())
    }
}

/// Normalize audio samples for Whisper
///
/// Ensures samples are in the correct range for transcription.
pub fn normalize_audio(samples: &[f32]) -> Vec<f32> {
    // Find the maximum absolute value
    let max_abs = samples
        .iter()
        .map(|s| s.abs())
        .fold(0.0f32, |a, b| a.max(b));

    if max_abs < 0.001 {
        // Audio is essentially silence
        return samples.to_vec();
    }

    // Normalize to -1.0..1.0 range if needed
    if max_abs > 1.0 {
        samples.iter().map(|s| s / max_abs).collect()
    } else {
        samples.to_vec()
    }
}

/// Apply a simple high-pass filter to remove DC offset
///
/// This helps improve transcription quality by removing low-frequency noise.
pub fn remove_dc_offset(samples: &[f32]) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }

    // Calculate mean (DC offset)
    let mean: f32 = samples.iter().sum::<f32>() / samples.len() as f32;

    // Subtract mean from all samples
    samples.iter().map(|s| s - mean).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_audio_already_normalized() {
        let samples = vec![0.5, -0.5, 0.3, -0.3];
        let normalized = normalize_audio(&samples);
        assert_eq!(normalized, samples);
    }

    #[test]
    fn test_normalize_audio_too_loud() {
        let samples = vec![2.0, -2.0, 1.0, -1.0];
        let normalized = normalize_audio(&samples);
        assert!((normalized[0] - 1.0).abs() < 0.001);
        assert!((normalized[1] - -1.0).abs() < 0.001);
    }

    #[test]
    fn test_normalize_audio_silence() {
        let samples = vec![0.0, 0.0, 0.0001, -0.0001];
        let normalized = normalize_audio(&samples);
        assert_eq!(normalized, samples);
    }

    #[test]
    fn test_remove_dc_offset() {
        let samples = vec![1.5, 1.0, 2.0, 1.5]; // Mean is 1.5
        let filtered = remove_dc_offset(&samples);

        // Check that mean is now ~0
        let mean: f32 = filtered.iter().sum::<f32>() / filtered.len() as f32;
        assert!(mean.abs() < 0.001);
    }

    #[test]
    fn test_remove_dc_offset_empty() {
        let samples: Vec<f32> = vec![];
        let filtered = remove_dc_offset(&samples);
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_audio_duration_limits() {
        // Check our constants are reasonable
        assert!(MIN_AUDIO_DURATION > 0.0);
        assert!(MAX_AUDIO_DURATION <= 30.0);
        assert!(MIN_AUDIO_DURATION < MAX_AUDIO_DURATION);
    }
}
