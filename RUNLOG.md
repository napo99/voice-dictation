# Voice-Dict Run Log

Session-by-session test results and environment snapshots.

---

## Session: 2026-01-13 (GPU + Bluetooth Diagnosis)

### Environment

| Component | Version/Value |
|-----------|---------------|
| OS | Windows 11 |
| GPU | NVIDIA GeForce RTX 4060 (8GB) |
| Driver | 591.74 |
| CUDA Toolkit | 13.1 |
| Whisper Model | ggml-small.en.bin (466MB) |
| Audio Device | AirPods Pro (Bluetooth HFP) |

### Changes Made

1. **GPU/CUDA acceleration enabled**
   - Patched `whisper-rs-sys` build.rs for compute arch 89
   - Copied CUDA DLLs to exe directory
   - Updated driver from 565.90 → 591.74

2. **Jitter tracking added**
   - `pipeline.rs`: Track chunk intervals, p95, max, gaps>50ms
   - Log gap_ratio percentage per recording
   - Warn when gap_ratio > 20%

3. **High-quality resampling**
   - Added `rubato` crate for sinc interpolation
   - Resample once before Whisper (not per-chunk)

### Test Results

| Timestamp | Audio | Gap | Gap% | p95 | Max | Proc | Conf | Notes |
|-----------|-------|-----|------|-----|-----|------|------|-------|
| 115027 | 11.46s | 6.77s | 59% | - | - | 32258ms | 0.74 | CPU, pre-driver |
| 162446 | 18.20s | 13.34s | 73% | - | - | 5152ms | 0.78 | GPU working! |
| 165607 | 14.42s | 9.62s | 67% | - | - | 865ms | 0.85 | GPU fast |
| 165914 | 13.91s | 8.91s | 64% | - | - | 482ms | 0.84 | GPU fast |
| 170422 | 17.70s | 12.68s | **71.6%** | 230ms | 501ms | 989ms | 0.84 | Full jitter stats |

### Key Findings

1. **GPU acceleration working**: 60x speedup (32s → 0.5s for same audio)
2. **Bluetooth is the bottleneck**: 71.6% of audio is silence (gaps)
3. **Jitter escalates over time**: 52ms → 501ms during recording
4. **Architecture is sound**: No refactor needed, just better audio input

### Debug Files

- WAV recordings: `%LOCALAPPDATA%\voice-dict\debug\recording_*.wav`
- Transcription log: `%LOCALAPPDATA%\voice-dict\debug\transcriptions.log`

### Decisions Made

- **No Python refactor**: Language doesn't affect C++/CUDA inference speed
- **No streaming refactor now**: Fix audio input first
- **USB mic recommended**: Fifine K669B (~$20) or similar

---

## Next Session Checklist

When testing with USB/wired mic:

- [ ] Note mic model and connection type
- [ ] Run 3-5 test recordings (15-20s each)
- [ ] Check gap_ratio in logs (target: <5%)
- [ ] Check jitter p95 (target: <20ms)
- [ ] Compare transcription quality
- [ ] Update this log with results

### Expected Results with USB Mic

| Metric | Bluetooth (current) | USB (expected) |
|--------|---------------------|----------------|
| gap_ratio | 71.6% | <5% |
| jitter p95 | 230ms | <20ms |
| jitter max | 501ms | <50ms |
| Transcription | Garbled | Accurate |

---

## Commands Reference

```powershell
# Run with debug logging
$env:RUST_LOG="info,vd_audio=debug,voice_dict_desktop::pipeline=debug"
.\target\release\voice-dict-desktop.exe

# Check latest transcription
Get-Content "$env:LOCALAPPDATA\voice-dict\debug\transcriptions.log" -Tail 5

# Play latest recording
Start-Process "$env:LOCALAPPDATA\voice-dict\debug\recording_*.wav"
```

---

*Last updated: 2026-01-13*
