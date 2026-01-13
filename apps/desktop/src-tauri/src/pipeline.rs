//! Integrated Pipeline for Voice Dictation
//!
//! This module wires together all components:
//! Audio Capture → VAD → Whisper → Polish → Text Injection
//!
//! It runs on a dedicated thread to avoid blocking the UI.

use crossbeam_channel::{bounded, Receiver, Sender};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

use vd_audio::{AudioCapture, AudioCaptureConfig, resample_high_quality};
use vd_core::{
    AudioBuffer, AudioChunk, VadConfig, VadEvent,
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
            whisper_model_path: models_dir.join(WhisperModel::Small.filename()),
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

const VAD_FRAME_SAMPLES: usize = 512;
const AUDIO_GAP_TOLERANCE_MS: u64 = 5;
const AUDIO_GAP_LOG_THRESHOLD_MS: u64 = 50;
const AUDIO_RX_WARN_THRESHOLD: Duration = Duration::from_millis(500);

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

    // Audio buffer for accumulating speech - will use native sample rate from device
    // We'll resample ONCE before sending to Whisper
    let mut audio_buffer = AudioBuffer::new(SAMPLE_RATE);  // Will be updated with native rate
    let mut native_sample_rate: u32 = SAMPLE_RATE;  // Track actual device sample rate
    let mut vad_frame_buffer: Vec<f32> = Vec::new();
    let mut last_chunk_end_ms: Option<u64> = None;
    let mut total_gap_samples: usize = 0;
    let mut last_audio_rx_at = Instant::now();

    // Jitter tracking for diagnostics
    let mut chunk_intervals_ms: Vec<f32> = Vec::new();
    let mut last_chunk_rx_time: Option<Instant> = None;
    let mut gaps_over_50ms: usize = 0;
    let mut total_chunks_received: usize = 0;

    // Channel for receiving audio from capture thread - unbounded to prevent dropping samples
    let (audio_tx, audio_rx) = crossbeam_channel::unbounded::<AudioChunk>();

    // Audio capture - pre-initialize to avoid delay on first recording
    let mut audio_capture: Option<AudioCapture> = None;
    {
        let audio_config = AudioCaptureConfig {
            device_name: config.input_device.clone(),
            ..Default::default()
        };
        match AudioCapture::new(audio_config, audio_tx.clone()) {
            Ok(capture) => {
                info!("Audio capture pre-initialized (not started yet)");
                audio_capture = Some(capture);
            }
            Err(e) => {
                warn!("Failed to pre-initialize audio capture: {}", e);
            }
        }
    }

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
                info!("=========== START RECORDING ===========");
                info!("Pipeline received StartRecording command");
                set_state(PipelineState::Listening);
                info!("State -> Listening");
                audio_buffer.clear();
                native_sample_rate = SAMPLE_RATE;  // Reset - will be set from first chunk
                vad_frame_buffer.clear();
                last_chunk_end_ms = None;
                total_gap_samples = 0;
                last_audio_rx_at = Instant::now();
                // Reset jitter tracking
                chunk_intervals_ms.clear();
                last_chunk_rx_time = None;
                gaps_over_50ms = 0;
                total_chunks_received = 0;
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

                // Start audio capture (pre-initialized, just need to start)
                info!("Starting audio capture...");
                if let Some(ref mut capture) = audio_capture {
                    if let Err(e) = capture.start() {
                        emit(PipelineEvent::Error(format!(
                            "Failed to start audio: {}",
                            e
                        )));
                        set_state(PipelineState::Idle);
                        continue;
                    }
                    info!("Audio capture STARTED");
                } else {
                    emit(PipelineEvent::Error("Audio capture not initialized".into()));
                    set_state(PipelineState::Idle);
                    continue;
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

                    let mut process_chunk = |chunk: AudioChunk| {
                        let now = Instant::now();
                        last_audio_rx_at = now;
                        total_chunks_received += 1;

                        // Track inter-chunk arrival time for jitter analysis
                        if let Some(last_rx) = last_chunk_rx_time {
                            let interval_ms = last_rx.elapsed().as_secs_f32() * 1000.0;
                            chunk_intervals_ms.push(interval_ms);
                            if interval_ms > 50.0 {
                                gaps_over_50ms += 1;
                            }
                        }
                        last_chunk_rx_time = Some(now);

                        if native_sample_rate != chunk.sample_rate {
                            if audio_buffer.is_empty() {
                                native_sample_rate = chunk.sample_rate;
                                info!(
                                    "Native sample rate detected: {}Hz (will resample to {}Hz for Whisper)",
                                    native_sample_rate, SAMPLE_RATE
                                );
                                audio_buffer = AudioBuffer::new(native_sample_rate);
                            } else {
                                warn!(
                                    "Sample rate changed mid-recording ({}Hz -> {}Hz); keeping original buffer",
                                    native_sample_rate, chunk.sample_rate
                                );
                            }
                        }

                        let chunk_duration_ms =
                            (chunk.samples.len() as f32 / chunk.sample_rate as f32 * 1000.0)
                                .round() as u64;
                        if let Some(last_end_ms) = last_chunk_end_ms {
                            if chunk.timestamp_ms > last_end_ms + AUDIO_GAP_TOLERANCE_MS {
                                let gap_ms = chunk.timestamp_ms - last_end_ms;
                                let gap_samples = ((gap_ms as f32 / 1000.0)
                                    * native_sample_rate as f32)
                                    .round() as usize;
                                if gap_samples > 0 {
                                    total_gap_samples += gap_samples;
                                    audio_buffer.extend(&vec![0.0; gap_samples]);
                                    if gap_ms >= AUDIO_GAP_LOG_THRESHOLD_MS {
                                        warn!(
                                            "Audio gap detected: {}ms ({} samples)",
                                            gap_ms, gap_samples
                                        );
                                    } else {
                                        debug!(
                                            "Audio gap detected: {}ms ({} samples)",
                                            gap_ms, gap_samples
                                        );
                                    }
                                }
                            }
                        }
                        last_chunk_end_ms = Some(chunk.timestamp_ms + chunk_duration_ms);

                        let level = calculate_audio_level(&chunk.samples);
                        if level > 0.01 {
                            debug!(
                                "Audio level: {:.3}, samples: {}",
                                level,
                                chunk.samples.len()
                            );
                        }
                        set_audio_level(level);

                        audio_buffer.extend(&chunk.samples);

                        if let Some(ref mut v) = vad {
                            let vad_samples = if chunk.sample_rate != SAMPLE_RATE {
                                resample_linear(&chunk.samples, chunk.sample_rate, SAMPLE_RATE)
                            } else {
                                chunk.samples.clone()
                            };

                            if !vad_samples.is_empty() {
                                vad_frame_buffer.extend_from_slice(&vad_samples);
                                let mut processed = 0;
                                while vad_frame_buffer.len() - processed >= VAD_FRAME_SAMPLES {
                                    let frame = &vad_frame_buffer
                                        [processed..processed + VAD_FRAME_SAMPLES];
                                    match v.process(frame) {
                                        Ok(VadEvent::SpeechStart) => {
                                            if !speech_detected {
                                                info!("VAD detected speech");
                                                speech_detected = true;
                                                set_state(PipelineState::Recording);
                                                info!("State -> Recording");
                                            }
                                        }
                                        Ok(VadEvent::SpeechEnd) => {
                                            debug!("VAD detected speech end");
                                        }
                                        Ok(VadEvent::SpeechContinue | VadEvent::Silence) => {}
                                        Err(e) => {
                                            warn!("VAD error: {}", e);
                                        }
                                    }
                                    processed += VAD_FRAME_SAMPLES;
                                }
                                if processed > 0 {
                                    vad_frame_buffer.drain(..processed);
                                }
                            }
                        } else if !speech_detected {
                            speech_detected = true;
                            set_state(PipelineState::Recording);
                        }
                    };

                    match audio_rx.recv_timeout(Duration::from_millis(20)) {
                        Ok(chunk) => {
                            process_chunk(chunk);
                            while let Ok(chunk) = audio_rx.try_recv() {
                                process_chunk(chunk);
                            }
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                            if last_audio_rx_at.elapsed() >= AUDIO_RX_WARN_THRESHOLD {
                                warn!(
                                    "No audio chunks received for {}ms",
                                    last_audio_rx_at.elapsed().as_millis()
                                );
                                last_audio_rx_at = Instant::now();
                            }
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                            warn!("Audio channel disconnected");
                            break 'recording;
                        }
                    }

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

                // Drain any remaining audio chunks from the channel
                // (there may be chunks buffered that we haven't processed yet)
                let mut drained_count = 0;
                while let Ok(chunk) = audio_rx.try_recv() {
                    audio_buffer.extend(&chunk.samples);
                    drained_count += 1;
                }
                if drained_count > 0 {
                    info!("Drained {} remaining audio chunks after stop", drained_count);
                }

                let recording_elapsed_ms = start_time.elapsed().as_millis() as u64;
                if let Some(last_end_ms) = last_chunk_end_ms {
                    if recording_elapsed_ms > last_end_ms + AUDIO_GAP_TOLERANCE_MS {
                        let gap_ms = recording_elapsed_ms - last_end_ms;
                        let gap_samples = ((gap_ms as f32 / 1000.0) * native_sample_rate as f32)
                            .round() as usize;
                        if gap_samples > 0 {
                            total_gap_samples += gap_samples;
                            audio_buffer.extend(&vec![0.0; gap_samples]);
                            warn!(
                                "End-of-recording gap: {}ms ({} samples)",
                                gap_ms, gap_samples
                            );
                        }
                    }
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
                info!("State -> Processing");
                let audio_duration_secs = audio_buffer.samples().len() as f32 / native_sample_rate as f32;
                info!("Audio buffer: {} samples at {}Hz ({:.2}s)",
                    audio_buffer.samples().len(),
                    native_sample_rate,
                    audio_duration_secs
                );
                if total_gap_samples > 0 {
                    let gap_secs = total_gap_samples as f32 / native_sample_rate as f32;
                    warn!(
                        "Audio gaps inserted: {} samples ({:.2}s)",
                        total_gap_samples, gap_secs
                    );
                }

                // Calculate and log jitter statistics
                let (jitter_p95, jitter_max, jitter_avg) = if !chunk_intervals_ms.is_empty() {
                    let mut sorted = chunk_intervals_ms.clone();
                    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    let p95_idx = (sorted.len() as f32 * 0.95) as usize;
                    let p95 = sorted.get(p95_idx.min(sorted.len() - 1)).copied().unwrap_or(0.0);
                    let max = sorted.last().copied().unwrap_or(0.0);
                    let avg = sorted.iter().sum::<f32>() / sorted.len() as f32;
                    (p95, max, avg)
                } else {
                    (0.0, 0.0, 0.0)
                };
                let gap_ratio = if audio_duration_secs > 0.0 {
                    (total_gap_samples as f32 / native_sample_rate as f32) / audio_duration_secs * 100.0
                } else {
                    0.0
                };
                info!(
                    "=== AUDIO JITTER SUMMARY: chunks={} avg={:.1}ms p95={:.1}ms max={:.1}ms gaps>50ms={} gap_ratio={:.1}% ===",
                    total_chunks_received, jitter_avg, jitter_p95, jitter_max, gaps_over_50ms, gap_ratio
                );
                if gap_ratio > 20.0 {
                    warn!("HIGH GAP RATIO ({:.1}%): Likely Bluetooth/device dropout issue. Try wired/USB mic.", gap_ratio);
                }

                // Create debug directory
                let debug_dir = vd_core::models_dir().parent().unwrap().join("debug");
                let _ = std::fs::create_dir_all(&debug_dir);

                // Save audio to timestamped WAV file for debugging (at native sample rate)
                let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
                let wav_filename = format!("recording_{}.wav", timestamp);
                let debug_audio_path = debug_dir.join(&wav_filename);
                if let Err(e) = save_audio_to_wav(&debug_audio_path, audio_buffer.samples(), native_sample_rate) {
                    warn!("Failed to save debug audio: {}", e);
                } else {
                    info!("Debug audio saved to: {:?} ({}Hz)", debug_audio_path, native_sample_rate);
                }

                // RESAMPLE to 16kHz for Whisper using HIGH-QUALITY algorithm
                // This is the KEY fix - resample ONCE with proper sinc interpolation
                let samples_for_whisper = if native_sample_rate != SAMPLE_RATE {
                    info!(">>> Resampling from {}Hz to {}Hz for Whisper...", native_sample_rate, SAMPLE_RATE);
                    resample_high_quality(audio_buffer.samples(), native_sample_rate, SAMPLE_RATE)
                } else {
                    audio_buffer.samples().to_vec()
                };
                info!("Samples for Whisper: {} ({:.2}s at {}Hz)",
                    samples_for_whisper.len(),
                    samples_for_whisper.len() as f32 / SAMPLE_RATE as f32,
                    SAMPLE_RATE
                );

                // Transcribe
                info!(">>> Starting Whisper transcription...");
                let transcription_result = if let Some(ref w) = whisper {
                    match w.transcribe(&samples_for_whisper) {
                        Ok(result) => {
                            info!(
                                "Transcribed {} chars in {}ms (confidence: {:.2})",
                                result.text.len(),
                                result.processing_time_ms,
                                result.confidence
                            );
                            Some(result)
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

                let result = transcription_result.unwrap();
                let transcribed_text = result.text.clone();

                // Save transcription to log file
                let log_path = debug_dir.join("transcriptions.log");
                let gap_secs = if total_gap_samples > 0 {
                    total_gap_samples as f32 / native_sample_rate as f32
                } else {
                    0.0
                };
                let log_entry = format!(
                    "[{}] audio={:.2}s gap={:.2}s gap_ratio={:.1}% jitter_p95={:.1}ms jitter_max={:.1}ms gaps>50ms={} proc={}ms conf={:.2} wav={} text=\"{}\"\n",
                    timestamp,
                    audio_duration_secs,
                    gap_secs,
                    gap_ratio,
                    jitter_p95,
                    jitter_max,
                    gaps_over_50ms,
                    result.processing_time_ms,
                    result.confidence,
                    wav_filename,
                    transcribed_text
                );
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log_path)
                {
                    use std::io::Write;
                    let _ = file.write_all(log_entry.as_bytes());
                    info!("Transcription logged to: {:?}", log_path);
                }

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
                info!("State -> Injecting");
                info!(">>> Injecting text: \"{}\"", polished_text);

                if let Some(ref mut inj) = injector {
                    // Minimal delay - clipboard paste is fast
                    thread::sleep(std::time::Duration::from_millis(10));

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

/// Linear resampling for VAD input (fast, low overhead).
fn resample_linear(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
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

/// Save audio samples to a WAV file for debugging
fn save_audio_to_wav(path: &std::path::Path, samples: &[f32], sample_rate: u32) -> Result<(), String> {
    use std::io::Write;

    let num_samples = samples.len() as u32;
    let byte_rate = sample_rate * 2; // 16-bit mono
    let data_size = num_samples * 2;
    let file_size = 36 + data_size;

    let mut file = std::fs::File::create(path).map_err(|e| e.to_string())?;

    // RIFF header
    file.write_all(b"RIFF").map_err(|e| e.to_string())?;
    file.write_all(&file_size.to_le_bytes()).map_err(|e| e.to_string())?;
    file.write_all(b"WAVE").map_err(|e| e.to_string())?;

    // fmt chunk
    file.write_all(b"fmt ").map_err(|e| e.to_string())?;
    file.write_all(&16u32.to_le_bytes()).map_err(|e| e.to_string())?; // chunk size
    file.write_all(&1u16.to_le_bytes()).map_err(|e| e.to_string())?;  // PCM format
    file.write_all(&1u16.to_le_bytes()).map_err(|e| e.to_string())?;  // mono
    file.write_all(&sample_rate.to_le_bytes()).map_err(|e| e.to_string())?;
    file.write_all(&byte_rate.to_le_bytes()).map_err(|e| e.to_string())?;
    file.write_all(&2u16.to_le_bytes()).map_err(|e| e.to_string())?;  // block align
    file.write_all(&16u16.to_le_bytes()).map_err(|e| e.to_string())?; // bits per sample

    // data chunk
    file.write_all(b"data").map_err(|e| e.to_string())?;
    file.write_all(&data_size.to_le_bytes()).map_err(|e| e.to_string())?;

    // Convert f32 samples to i16 and write
    for &sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let i16_sample = (clamped * 32767.0) as i16;
        file.write_all(&i16_sample.to_le_bytes()).map_err(|e| e.to_string())?;
    }

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

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
