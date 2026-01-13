# Voice-Dict Technical Notes

This document captures discoveries, trade-offs, and implementation decisions made during development.

---

## Table of Contents

1. [GPU/CUDA Acceleration](#gpucuda-acceleration)
2. [Bluetooth Audio Issues](#bluetooth-audio-issues)
3. [Architecture Decisions](#architecture-decisions)
4. [Performance Benchmarks](#performance-benchmarks)
5. [Future Improvements](#future-improvements)

---

## GPU/CUDA Acceleration

### Setup Requirements (Windows + RTX 40xx)

**Problem**: whisper-rs/whisper.cpp defaults to old CUDA architectures (52/61/70) which fail on modern GPUs.

**Solution**: Patch `whisper-rs-sys` build.rs to use correct architecture:

```rust
// In ~/.cargo/registry/src/.../whisper-rs-sys-0.9.0/build.rs
if cfg!(feature = "cuda") {
    config.define("WHISPER_CUBLAS", "ON");
    // RTX 30xx = 86, RTX 40xx = 89
    let cuda_arch = env::var("GGML_CUDA_ARCHITECTURES").unwrap_or_else(|_| "89".to_string());
    config.define("GGML_CUDA_ARCHITECTURES", &cuda_arch);
}
```

**Driver/Toolkit Compatibility**:

| CUDA Toolkit | Minimum Driver Version |
|--------------|------------------------|
| CUDA 12.0    | 525.60.13+             |
| CUDA 12.7    | 565.xx                 |
| CUDA 13.1    | 590.44+ (591.74 tested)|

**Required DLLs** (copy to exe directory):
- `cublas64_13.dll` (51MB)
- `cublasLt64_13.dll` (449MB)
- `cudart64_13.dll` (540KB)

From: `C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.1\bin\x64\`

### Verifying GPU Usage

Look for this in logs:
```
whisper_backend_init: using CUDA backend
```

If you see `using CPU backend` or CPU buffer messages, GPU is not being used.

---

## Bluetooth Audio Issues

### The Problem

Bluetooth audio (especially AirPods on Windows) suffers from severe jitter and dropouts due to:

1. **Windows uses generic HFP** (Hands-Free Profile) - low bandwidth, power-saving oriented
2. **macOS has Apple-specific optimizations** - H1/W1 chip integration, AAC codec
3. **Bluetooth inherently has variable latency** - chunks arrive in bursts, not steadily

### Diagnosis: Jitter Summary Logging

We added comprehensive jitter tracking in `pipeline.rs`:

```
=== AUDIO JITTER SUMMARY ===
chunks: 502
avg interval: 32.0ms (expected: ~10ms for 160 samples @ 16kHz)
p95 interval: 230.0ms
max interval: 501.0ms
gaps > 50ms: 42
gap_ratio: 71.6%
```

**Interpretation**:
- `gap_ratio > 20%` = Bluetooth/device dropout issue
- `gap_ratio < 5%` = Healthy audio capture
- `p95 > 50ms` = Significant jitter, will cause transcription issues

### Evidence: Bluetooth vs Expected

| Metric | Expected (USB) | Actual (AirPods/Windows) |
|--------|----------------|--------------------------|
| Avg interval | ~10ms | 32ms (3x worse) |
| p95 interval | ~15ms | 230ms (15x worse) |
| Max interval | ~20ms | 501ms (25x worse) |
| Gap ratio | <2% | 71.6% |

### Gap Escalation Pattern

Bluetooth dropouts typically escalate over time:
```
52ms → 83ms → 97ms → 114ms → 143ms → 156ms → 176ms → 205ms →
220ms → 238ms → 250ms → 281ms → 301ms → 315ms → 343ms → 360ms →
374ms → 406ms → 420ms → 501ms
```

This is characteristic of Bluetooth connection degradation.

### Solutions (Ranked)

1. **USB Microphone** (Best) - Fifine K669B (~$20), zero Bluetooth issues
2. **USB Headset** - Logitech H390 (~$30), reliable
3. **Wired 3.5mm** - Depends on PC sound card
4. **Bluetooth mitigations** (partial help):
   - Charge to 100%
   - Update Bluetooth drivers
   - Disable power saving in Device Manager
   - Keep device close (<1m)

---

## Architecture Decisions

### Current Design: Batch Processing

```
[Record] → [Stop] → [VAD] → [Whisper] → [Polish] → [Inject]
```

**Trade-offs**:

| Aspect | Batch (Current) | Streaming (Future) |
|--------|-----------------|-------------------|
| Complexity | Simple | Complex |
| Reliability | High | Medium |
| Latency | End-of-speech + processing | Near real-time |
| Accuracy | Best (full context) | Good (may need correction) |
| User feedback | None during recording | Partial transcripts |

**Why batch was chosen for MVP**:
- Simpler to implement and debug
- More reliable transcription (full context)
- Easier to add LLM polishing (final only)
- GPU makes batch processing fast enough (~1s for 20s audio)

### Real-Time Path (Future)

If real-time UI feedback is needed:

1. **Rolling buffer approach** (every 300-500ms):
   - Run Whisper on growing audio buffer
   - Show partial transcripts during recording
   - Stabilize text by committing only "stable prefix"
   - Keep injection at end (safe)

2. **Requirements**:
   - GPU acceleration (CPU too slow for rolling inference)
   - Stable audio capture (USB mic, not Bluetooth)
   - Diff algorithm for stable prefix detection

3. **What whisper-rs CAN'T do**:
   - True incremental decoding (re-runs full inference)
   - Continue from previous state (no streaming API)

### Key Insight

> "Streaming won't fix capture dropouts. If the device doesn't deliver audio,
> streaming just transcribes silence faster."

Fix audio capture reliability BEFORE adding real-time features.

### Decision: No Major Refactor Now (2026-01-13)

**Context:** Evaluated whether to refactor to streaming/real-time or switch to Python.

**Decision:** Keep current Rust batch architecture. No refactor.

**Reasoning:**

1. **Python would NOT improve performance:**
   - Whisper inference is C++/CUDA regardless of wrapper language
   - Python adds interpreter overhead and potential IPC latency
   - faster-whisper (CTranslate2) is faster but requires service boundary

2. **Current bottleneck is audio capture, not architecture:**
   - Bluetooth AirPods: 71.6% gap ratio (unusable)
   - USB mic expected: <5% gap ratio (usable)
   - Must fix input before optimizing processing

3. **GPU already provides acceptable latency:**
   - ~1s total latency for 20s audio
   - Real-time partials would add complexity for marginal gain

4. **Priority order:**
   ```
   1. ✅ GPU acceleration working
   2. ⏳ Get reliable audio input (USB mic)
   3. ⏳ Validate system with good audio
   4. ⏳ Re-evaluate if real-time needed
   ```

**Future path (if real-time needed):**
- Rolling buffer every 300-500ms
- Stable prefix algorithm
- Keep injection at end
- No language change required

**USB vs 3.5mm Audio Jack:**
- USB preferred: consistent quality, no interference, plug-and-play
- 3.5mm: depends on PC sound card quality, can have noise issues
- Both have ~1-5ms latency (negligible vs ~1s Whisper processing)

---

## Performance Benchmarks

### GPU vs CPU Transcription (RTX 4060)

| Audio Duration | CPU Time | GPU Time | Speedup |
|----------------|----------|----------|---------|
| 5s             | ~15s     | ~0.5s    | 30x     |
| 10s            | ~30s     | ~0.8s    | 37x     |
| 18s            | ~60s     | ~1.0s    | 60x     |

GPU enables real-time factor of **18x** (18s audio in 1s).

### Model Sizes

| Model | Size | Quality | GPU Memory |
|-------|------|---------|------------|
| tiny.en | 75MB | Fair | ~200MB |
| base.en | 142MB | Good | ~300MB |
| small.en | 466MB | Great | ~500MB |
| medium.en | 1.5GB | Excellent | ~1.5GB |

Currently using `small.en` - best balance of quality and speed.

---

## Future Improvements

### Priority Order

1. **Reliable audio capture** (USB mic) - CRITICAL
2. **Real-time UI feedback** (partial transcripts) - Nice to have
3. **Latency optimization** (shorter VAD timeout) - Polish
4. **LLM polishing** (grammar/punctuation) - Enhancement

### Potential Enhancements

- [ ] Rolling buffer transcription for real-time UI
- [ ] Configurable VAD sensitivity
- [ ] Multiple model size support in UI
- [ ] Audio device selection in UI
- [ ] Noise suppression preprocessing
- [ ] Custom vocabulary/hotwords

---

## Troubleshooting

### GPU Not Being Used

1. Check NVIDIA driver version: `nvidia-smi`
2. Verify CUDA DLLs in exe directory
3. Look for `using CUDA backend` in logs
4. Ensure `VD_WHISPER_GPU` env var is not `0`

### High Gap Ratio (>20%)

1. Check audio device (Bluetooth = bad)
2. Try USB microphone
3. Check for ring buffer overflow warnings
4. Verify sample rate matches (16kHz)

### Empty Transcription

1. Check audio level in logs (should be >0.01 when speaking)
2. Verify VAD is detecting speech
3. Check audio duration (min 0.5s, max 30s)
4. Listen to debug WAV file in `%LOCALAPPDATA%\voice-dict\debug\`

---

## Debug Files

Location: `%LOCALAPPDATA%\voice-dict\debug\`

- `recording_YYYYMMDD_HHMMSS.wav` - Raw audio capture
- `transcriptions.log` - All transcriptions with metrics

Log entry format:
```
[timestamp] audio=Xs gap=Xs gap_ratio=X% jitter_p95=Xms jitter_max=Xms gaps>50ms=N proc=Xms conf=X.XX wav=filename text="..."
```

---

*Last updated: 2026-01-13*
