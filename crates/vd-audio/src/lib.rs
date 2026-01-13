//! # vd-audio
//!
//! Audio capture for Voice-Dict using cpal.
//!
//! This crate provides low-latency audio capture from the system microphone.
//! It uses a dedicated high-priority thread for capture to avoid audio dropouts.
//!
//! ## Key Design Decisions
//!
//! - Uses `std::thread` (not async) for audio capture to ensure low latency
//! - Uses `crossbeam-channel` for lock-free communication
//! - Ring buffer prevents memory growth during long recording sessions
//! - Audio is never written to disk (privacy by design)
//!
//! ## Example
//!
//! ```ignore
//! use vd_audio::{AudioCapture, AudioCaptureConfig};
//! use crossbeam_channel::unbounded;
//!
//! let (tx, rx) = unbounded();
//! let config = AudioCaptureConfig::default();
//! let capture = AudioCapture::new(config, tx)?;
//! capture.start()?;
//!
//! // Receive audio samples
//! while let Ok(samples) = rx.recv() {
//!     // Process samples...
//! }
//! ```

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::Sender;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use tracing::{debug, error, info, warn};
use vd_core::{AudioChunk, AudioError, CHANNELS, SAMPLE_RATE};

/// Configuration for audio capture
#[derive(Debug, Clone)]
pub struct AudioCaptureConfig {
    /// Target sample rate (will be resampled if device doesn't support it)
    pub sample_rate: u32,
    /// Number of channels
    pub channels: u16,
    /// Buffer size in samples (per channel)
    pub buffer_size: usize,
    /// Device name (None for default)
    pub device_name: Option<String>,
}

impl Default for AudioCaptureConfig {
    fn default() -> Self {
        Self {
            sample_rate: SAMPLE_RATE,
            channels: CHANNELS,
            buffer_size: 480, // 30ms at 16kHz
            device_name: None,
        }
    }
}

/// Information about an audio input device
#[derive(Debug, Clone)]
pub struct AudioDevice {
    /// Device name
    pub name: String,
    /// Whether this is the default device
    pub is_default: bool,
}

/// Lists available audio input devices
pub fn list_input_devices() -> Result<Vec<AudioDevice>, AudioError> {
    let host = cpal::default_host();
    let default_device = host.default_input_device();
    let default_name = default_device.as_ref().and_then(|d| d.name().ok());

    let mut devices = Vec::new();

    for device in host
        .input_devices()
        .map_err(|e| AudioError::ConfigError(e.to_string()))?
    {
        if let Ok(name) = device.name() {
            let is_default = default_name.as_ref().map_or(false, |d| d == &name);
            devices.push(AudioDevice { name, is_default });
        }
    }

    Ok(devices)
}

/// Audio capture handle
///
/// Manages the audio capture thread and stream. Audio samples are sent
/// through the provided channel for processing.
pub struct AudioCapture {
    config: AudioCaptureConfig,
    sender: Sender<AudioChunk>,
    running: Arc<AtomicBool>,
    thread_handle: Option<JoinHandle<()>>,
}

impl AudioCapture {
    /// Create a new audio capture instance
    ///
    /// The provided sender will receive `AudioChunk` messages containing
    /// captured audio samples.
    pub fn new(config: AudioCaptureConfig, sender: Sender<AudioChunk>) -> Result<Self, AudioError> {
        Ok(Self {
            config,
            sender,
            running: Arc::new(AtomicBool::new(false)),
            thread_handle: None,
        })
    }

    /// Start capturing audio
    ///
    /// Spawns a dedicated high-priority thread for audio capture.
    /// Returns immediately; audio samples are sent through the channel.
    pub fn start(&mut self) -> Result<(), AudioError> {
        if self.running.load(Ordering::SeqCst) {
            return Ok(());
        }

        self.running.store(true, Ordering::SeqCst);

        let config = self.config.clone();
        let sender = self.sender.clone();
        let running = self.running.clone();

        let handle = thread::Builder::new()
            .name("vd-audio-capture".into())
            .spawn(move || {
                // Try to set high thread priority on Linux
                #[cfg(target_os = "linux")]
                {
                    // SAFETY: setpriority is safe to call with these arguments
                    unsafe {
                        let result = libc::setpriority(libc::PRIO_PROCESS, 0, -10);
                        if result != 0 {
                            warn!("Failed to set audio thread priority (requires root)");
                        } else {
                            debug!("Audio thread priority set to -10");
                        }
                    }
                }

                if let Err(e) = run_capture_loop(config, sender, running) {
                    error!("Audio capture error: {}", e);
                }
            })
            .map_err(|e| AudioError::StreamStartError(e.to_string()))?;

        self.thread_handle = Some(handle);
        info!("Audio capture started");

        Ok(())
    }

    /// Stop capturing audio
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);

        if let Some(handle) = self.thread_handle.take() {
            // Give the thread a chance to finish
            let _ = handle.join();
        }

        info!("Audio capture stopped");
    }

    /// Check if capture is currently running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Internal function that runs on the capture thread
fn run_capture_loop(
    config: AudioCaptureConfig,
    sender: Sender<AudioChunk>,
    running: Arc<AtomicBool>,
) -> Result<(), AudioError> {
    let host = cpal::default_host();

    // Get the input device
    let device = if let Some(ref name) = config.device_name {
        host.input_devices()
            .map_err(|e| AudioError::ConfigError(e.to_string()))?
            .find(|d| d.name().map_or(false, |n| &n == name))
            .ok_or_else(|| AudioError::NoInputDevice)?
    } else {
        host.default_input_device()
            .ok_or(AudioError::NoInputDevice)?
    };

    let device_name = device.name().unwrap_or_else(|_| "Unknown".to_string());
    info!("Using audio device: {}", device_name);

    // Get supported config
    let supported_configs = device
        .supported_input_configs()
        .map_err(|e| AudioError::ConfigError(e.to_string()))?;

    // Find the best matching config
    let stream_config = find_best_config(supported_configs, &config)?;
    info!(
        "Audio stream config: {} Hz, {} channels",
        stream_config.sample_rate.0, stream_config.channels
    );

    // Track timestamp for chunks
    let start_time = std::time::Instant::now();
    let sample_rate = stream_config.sample_rate.0;
    let channels = stream_config.channels as usize;
    let target_channels = config.channels as usize;
    let target_sample_rate = config.sample_rate;

    // Create the stream
    let running_cb = running.clone();
    let stream = device
        .build_input_stream(
            &stream_config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if !running_cb.load(Ordering::SeqCst) {
                    return;
                }

                let timestamp_ms = start_time.elapsed().as_millis() as u64;

                // Convert to mono if needed
                let mono_samples: Vec<f32> = if channels > target_channels {
                    // Average channels to mono
                    data.chunks(channels)
                        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
                        .collect()
                } else {
                    data.to_vec()
                };

                // Resample if needed
                let samples = if sample_rate != target_sample_rate {
                    resample(&mono_samples, sample_rate, target_sample_rate)
                } else {
                    mono_samples
                };

                let chunk = AudioChunk {
                    samples,
                    timestamp_ms,
                };

                // Non-blocking send - if the receiver is full, we drop the oldest samples
                if sender.try_send(chunk).is_err() {
                    warn!("Audio buffer full, dropping samples");
                }
            },
            move |err| {
                error!("Audio stream error: {}", err);
            },
            None, // No timeout
        )
        .map_err(|e| AudioError::StreamBuildError(e.to_string()))?;

    // Start the stream
    stream
        .play()
        .map_err(|e| AudioError::StreamStartError(e.to_string()))?;

    // Keep the stream alive while running
    while running.load(Ordering::SeqCst) {
        thread::sleep(std::time::Duration::from_millis(10));
    }

    // Stream is dropped here, stopping capture
    Ok(())
}

/// Find the best matching audio config from supported configs
fn find_best_config(
    mut supported: cpal::SupportedInputConfigs,
    config: &AudioCaptureConfig,
) -> Result<cpal::StreamConfig, AudioError> {
    // Try to find exact match first
    let target_rate = cpal::SampleRate(config.sample_rate);

    // Look for a config that supports our target sample rate
    for supported_config in supported.by_ref() {
        if supported_config.min_sample_rate() <= target_rate
            && supported_config.max_sample_rate() >= target_rate
        {
            return Ok(supported_config.with_sample_rate(target_rate).into());
        }
    }

    // Fall back to default config
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or(AudioError::NoInputDevice)?;

    device
        .default_input_config()
        .map(|c| c.into())
        .map_err(|e| AudioError::ConfigError(e.to_string()))
}

/// Simple linear resampling
///
/// This is a basic implementation. For production, consider using a
/// proper resampling library like `rubato` for better quality.
fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate {
        return samples.to_vec();
    }

    let ratio = to_rate as f64 / from_rate as f64;
    let new_len = (samples.len() as f64 * ratio) as usize;
    let mut result = Vec::with_capacity(new_len);

    for i in 0..new_len {
        let src_idx = i as f64 / ratio;
        let src_idx_floor = src_idx.floor() as usize;
        let src_idx_ceil = (src_idx_floor + 1).min(samples.len() - 1);
        let frac = src_idx - src_idx_floor as f64;

        let sample = samples[src_idx_floor] * (1.0 - frac as f32)
            + samples[src_idx_ceil] * frac as f32;
        result.push(sample);
    }

    result
}

/// Ring buffer for accumulating audio samples
///
/// Fixed-size buffer that overwrites oldest data when full.
/// Used to accumulate audio while waiting for VAD decisions.
#[derive(Debug)]
pub struct RingBuffer {
    buffer: Vec<f32>,
    write_pos: usize,
    capacity: usize,
    filled: bool,
}

impl RingBuffer {
    /// Create a new ring buffer with the given capacity in samples
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: vec![0.0; capacity],
            write_pos: 0,
            capacity,
            filled: false,
        }
    }

    /// Write samples to the buffer
    pub fn write(&mut self, samples: &[f32]) {
        for &sample in samples {
            self.buffer[self.write_pos] = sample;
            self.write_pos = (self.write_pos + 1) % self.capacity;
            if self.write_pos == 0 {
                self.filled = true;
            }
        }
    }

    /// Get all samples in order (oldest first)
    pub fn read_all(&self) -> Vec<f32> {
        if !self.filled {
            // Buffer hasn't wrapped yet
            return self.buffer[..self.write_pos].to_vec();
        }

        // Buffer has wrapped, need to reorder
        let mut result = Vec::with_capacity(self.capacity);
        result.extend_from_slice(&self.buffer[self.write_pos..]);
        result.extend_from_slice(&self.buffer[..self.write_pos]);
        result
    }

    /// Clear the buffer
    pub fn clear(&mut self) {
        self.write_pos = 0;
        self.filled = false;
        // Don't need to zero out the buffer, just reset positions
    }

    /// Get the number of samples currently in the buffer
    pub fn len(&self) -> usize {
        if self.filled {
            self.capacity
        } else {
            self.write_pos
        }
    }

    /// Check if the buffer is empty
    pub fn is_empty(&self) -> bool {
        !self.filled && self.write_pos == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ring_buffer_basic() {
        let mut buffer = RingBuffer::new(10);
        assert!(buffer.is_empty());
        assert_eq!(buffer.len(), 0);

        buffer.write(&[1.0, 2.0, 3.0]);
        assert!(!buffer.is_empty());
        assert_eq!(buffer.len(), 3);
        assert_eq!(buffer.read_all(), vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_ring_buffer_wrap() {
        let mut buffer = RingBuffer::new(5);
        buffer.write(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(buffer.len(), 5);

        // Write more, should wrap
        buffer.write(&[6.0, 7.0]);
        assert_eq!(buffer.len(), 5);

        let data = buffer.read_all();
        assert_eq!(data, vec![3.0, 4.0, 5.0, 6.0, 7.0]);
    }

    #[test]
    fn test_ring_buffer_clear() {
        let mut buffer = RingBuffer::new(10);
        buffer.write(&[1.0, 2.0, 3.0]);
        buffer.clear();
        assert!(buffer.is_empty());
        assert_eq!(buffer.len(), 0);
    }

    #[test]
    fn test_resample_same_rate() {
        let samples = vec![1.0, 2.0, 3.0, 4.0];
        let result = resample(&samples, 16000, 16000);
        assert_eq!(result, samples);
    }

    #[test]
    fn test_resample_upsample() {
        let samples = vec![0.0, 1.0];
        let result = resample(&samples, 8000, 16000);
        assert_eq!(result.len(), 4);
        // Should interpolate between values
        assert!((result[0] - 0.0).abs() < 0.01);
        // Middle values should be between 0 and 1
        assert!(result[1] >= 0.0 && result[1] <= 1.0);
        assert!(result[2] >= 0.0 && result[2] <= 1.0);
    }

    #[test]
    fn test_audio_capture_config_default() {
        let config = AudioCaptureConfig::default();
        assert_eq!(config.sample_rate, SAMPLE_RATE);
        assert_eq!(config.channels, CHANNELS);
    }
}
