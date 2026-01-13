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
use crossbeam_queue::ArrayQueue;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
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
            buffer_size: 512, // 32ms at 16kHz (aligns with VAD chunking)
            device_name: None,
        }
    }
}

const AUDIO_RING_BUFFER_CHUNKS: usize = 256;
const AUDIO_QUEUE_DRAIN_INTERVAL_MS: u64 = 2;
const AUDIO_QUEUE_DROP_LOG_INTERVAL_MS: u64 = 1000;

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
        // Try default device first, fall back to first available device
        match host.default_input_device() {
            Some(device) => device,
            None => {
                warn!("No default input device, trying first available device...");
                host.input_devices()
                    .map_err(|e| AudioError::ConfigError(e.to_string()))?
                    .next()
                    .ok_or(AudioError::NoInputDevice)?
            }
        }
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
    let queue = Arc::new(ArrayQueue::new(AUDIO_RING_BUFFER_CHUNKS));
    let dropped_chunks = Arc::new(AtomicUsize::new(0));

    // LOG THE ACTUAL SAMPLE RATES - critical for debugging resampling
    info!(
        "Audio device: {}Hz {}ch -> will send at native rate (resample later)",
        sample_rate, channels
    );

    // Create the stream
    let running_cb = running.clone();
    let queue_cb = queue.clone();
    let dropped_chunks_cb = dropped_chunks.clone();
    let stream = device
        .build_input_stream(
            &stream_config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if !running_cb.load(Ordering::SeqCst) {
                    return;
                }

                let timestamp_ms = start_time.elapsed().as_millis() as u64;

                // Convert to mono if needed (but DON'T resample - send at native rate)
                let mono_samples: Vec<f32> = if channels > target_channels {
                    // Average channels to mono
                    data.chunks(channels)
                        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
                        .collect()
                } else {
                    data.to_vec()
                };

                // NO RESAMPLING HERE - send at native sample rate
                // Resampling will happen ONCE before Whisper with high-quality algorithm
                let chunk = AudioChunk {
                    samples: mono_samples,
                    sample_rate,  // Include actual sample rate so receiver knows
                    timestamp_ms,
                };

                // Push into ring buffer to avoid blocking the audio callback.
                if let Err(chunk) = queue_cb.push(chunk) {
                    dropped_chunks_cb.fetch_add(1, Ordering::Relaxed);
                    let _ = queue_cb.pop();
                    if queue_cb.push(chunk).is_err() {
                        dropped_chunks_cb.fetch_add(1, Ordering::Relaxed);
                    }
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

    // Keep the stream alive while running and drain queued audio
    let mut last_drop_log_at = std::time::Instant::now();
    while running.load(Ordering::SeqCst) {
        let mut drained_any = false;
        while let Some(chunk) = queue.pop() {
            drained_any = true;
            if sender.send(chunk).is_err() {
                warn!("Audio channel closed; stopping capture");
                running.store(false, Ordering::SeqCst);
                break;
            }
        }

        let dropped = dropped_chunks.swap(0, Ordering::Relaxed);
        if dropped > 0
            && last_drop_log_at.elapsed().as_millis() as u64 >= AUDIO_QUEUE_DROP_LOG_INTERVAL_MS
        {
            warn!(
                "Audio ring buffer overflow: dropped {} chunks (queue len={})",
                dropped,
                queue.len()
            );
            last_drop_log_at = std::time::Instant::now();
        }

        if !drained_any {
            thread::sleep(Duration::from_millis(AUDIO_QUEUE_DRAIN_INTERVAL_MS));
        }
    }

    // Stream is dropped here, stopping capture
    Ok(())
}

/// Find the best matching audio config from supported configs
fn find_best_config(
    mut supported: cpal::SupportedInputConfigs,
    config: &AudioCaptureConfig,
) -> Result<cpal::StreamConfig, AudioError> {
    let target_rate = cpal::SampleRate(config.sample_rate);
    let target_channels = config.channels;
    let mut fallback_config: Option<cpal::SupportedStreamConfig> = None;

    for supported_config in supported.by_ref() {
        if supported_config.min_sample_rate() <= target_rate
            && supported_config.max_sample_rate() >= target_rate
        {
            if supported_config.channels() == target_channels {
                let mut stream_config: cpal::StreamConfig =
                    supported_config.with_sample_rate(target_rate).into();
                stream_config.buffer_size = cpal::BufferSize::Fixed(config.buffer_size as u32);
                return Ok(stream_config);
            }

            if fallback_config.is_none() {
                fallback_config = Some(supported_config.with_sample_rate(target_rate));
            }
        }
    }

    if let Some(supported_config) = fallback_config {
        warn!(
            "No exact channel match for {}ch at {}Hz, using {}ch",
            target_channels,
            target_rate.0,
            supported_config.channels()
        );
        let mut stream_config: cpal::StreamConfig = supported_config.into();
        stream_config.buffer_size = cpal::BufferSize::Fixed(config.buffer_size as u32);
        return Ok(stream_config);
    }

    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or(AudioError::NoInputDevice)?;

    device
        .default_input_config()
        .map(|c| {
            let mut stream_config: cpal::StreamConfig = c.into();
            stream_config.buffer_size = cpal::BufferSize::Fixed(config.buffer_size as u32);
            stream_config
        })
        .map_err(|e| AudioError::ConfigError(e.to_string()))
}

/// High-quality resampling using rubato (sinc interpolation)
///
/// This provides much better quality than linear interpolation,
/// especially for downsampling (e.g., 48kHz → 16kHz).
pub fn resample_high_quality(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    use rubato::{FftFixedInOut, Resampler};

    if from_rate == to_rate || samples.is_empty() {
        return samples.to_vec();
    }

    info!(
        "Resampling {} samples from {}Hz to {}Hz (high-quality sinc)",
        samples.len(),
        from_rate,
        to_rate
    );

    // Create resampler - use FFT-based for best quality
    // chunk_size should be power of 2 for FFT efficiency
    let chunk_size = 1024;

    let mut resampler = match FftFixedInOut::<f32>::new(
        from_rate as usize,
        to_rate as usize,
        chunk_size,
        1, // mono
    ) {
        Ok(r) => r,
        Err(e) => {
            warn!("Failed to create resampler: {}, falling back to simple", e);
            return simple_resample(samples, from_rate, to_rate);
        }
    };

    let input_frames_needed = resampler.input_frames_next();
    let mut output = Vec::new();

    // Process in chunks
    let mut pos = 0;
    while pos + input_frames_needed <= samples.len() {
        let input_chunk: Vec<Vec<f32>> = vec![samples[pos..pos + input_frames_needed].to_vec()];

        match resampler.process(&input_chunk, None) {
            Ok(output_chunk) => {
                if !output_chunk.is_empty() && !output_chunk[0].is_empty() {
                    output.extend_from_slice(&output_chunk[0]);
                }
            }
            Err(e) => {
                warn!("Resampling error: {}", e);
            }
        }
        pos += input_frames_needed;
    }

    // Handle remaining samples with simple resampling (usually just a small tail)
    if pos < samples.len() {
        let remaining = &samples[pos..];
        let resampled_tail = simple_resample(remaining, from_rate, to_rate);
        output.extend_from_slice(&resampled_tail);
    }

    info!("Resampled to {} samples", output.len());
    output
}

/// Simple linear resampling (fallback for small chunks)
fn simple_resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || samples.is_empty() {
        return samples.to_vec();
    }

    let ratio = to_rate as f64 / from_rate as f64;
    let new_len = (samples.len() as f64 * ratio).ceil() as usize;
    let mut result = Vec::with_capacity(new_len);

    for i in 0..new_len {
        let src_idx = i as f64 / ratio;
        let src_idx_floor = src_idx.floor() as usize;
        let src_idx_ceil = (src_idx_floor + 1).min(samples.len().saturating_sub(1));
        let frac = src_idx - src_idx_floor as f64;

        if src_idx_floor < samples.len() {
            let sample = samples[src_idx_floor] * (1.0 - frac as f32)
                + samples.get(src_idx_ceil).copied().unwrap_or(0.0) * frac as f32;
            result.push(sample);
        }
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
        let result = resample_high_quality(&samples, 16000, 16000);
        assert_eq!(result, samples);
    }

    #[test]
    fn test_resample_downsampling() {
        // Test downsampling from 48kHz to 16kHz (common AirPods scenario)
        // Create a simple sine wave at 48kHz
        let samples: Vec<f32> = (0..4800)  // 100ms at 48kHz
            .map(|i| (i as f32 * 2.0 * std::f32::consts::PI * 440.0 / 48000.0).sin())
            .collect();
        let result = resample_high_quality(&samples, 48000, 16000);
        // Should have roughly 1/3 the samples (100ms at 16kHz = 1600 samples)
        assert!(result.len() >= 1500 && result.len() <= 1700);
    }

    #[test]
    fn test_audio_capture_config_default() {
        let config = AudioCaptureConfig::default();
        assert_eq!(config.sample_rate, SAMPLE_RATE);
        assert_eq!(config.channels, CHANNELS);
    }
}
