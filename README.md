# Voice-Dict

A privacy-first, local voice dictation application for Windows. Speak naturally and have your words transcribed and inserted into any application.

**No cloud. No subscriptions. Your voice stays on your machine.**

---

## Features

- **Local-only transcription** - Uses OpenAI Whisper running entirely on your machine
- **GPU accelerated** - NVIDIA CUDA support for 60x faster transcription
- **Global hotkey** - `Alt+Shift+V` to start/stop recording from anywhere
- **Auto text injection** - Transcribed text is automatically pasted into the active window
- **Voice Activity Detection** - Smart detection of speech start/end using Silero VAD
- **High-quality audio** - Sinc interpolation resampling for optimal Whisper input
- **Debug logging** - Jitter tracking and audio diagnostics for troubleshooting

---

## System Requirements

### Minimum
- Windows 10/11 (64-bit)
- 8GB RAM
- Any microphone (USB recommended, see [Audio Notes](#audio-notes))

### Recommended (for GPU acceleration)
- NVIDIA GPU with CUDA support (RTX 20xx or newer)
- 16GB RAM
- USB microphone for reliable audio capture

---

## Quick Start

### 1. Install Prerequisites

```powershell
# Install Rust
winget install Rustlang.Rust.MSVC

# Install Node.js (for Tauri frontend)
winget install OpenJS.NodeJS.LTS

# Install Visual Studio Build Tools (C++ workload)
winget install Microsoft.VisualStudio.2022.BuildTools
```

For GPU support, install CUDA Toolkit:
- Download from: https://developer.nvidia.com/cuda-downloads

### 2. Clone and Build

```powershell
git clone https://github.com/yourusername/voice-dict.git
cd voice-dict

# Install frontend dependencies
cd apps/desktop
npm install
cd ../..

# Build release (CPU only)
cargo build --release -p voice-dict-desktop

# Build with GPU/CUDA support
cargo build --release -p voice-dict-desktop --features cuda
```

### 3. Download Models

```powershell
# Create models directory
$modelsDir = "$env:LOCALAPPDATA\voice-dict\models"
New-Item -ItemType Directory -Force -Path $modelsDir

# Download Whisper model (small.en recommended)
Invoke-WebRequest -Uri "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin" `
    -OutFile "$modelsDir\ggml-small.en.bin"

# Download Silero VAD model
Invoke-WebRequest -Uri "https://github.com/snakers4/silero-vad/raw/master/files/silero_vad.onnx" `
    -OutFile "$modelsDir\silero_vad.onnx"
```

### 4. Run

```powershell
.\target\release\voice-dict-desktop.exe
```

Press `Alt+Shift+V` to start recording, speak, press `Alt+Shift+V` again to stop and transcribe.

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        Tauri Desktop App                         │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐  │
│  │   React UI  │  │   Hotkey    │  │     Pipeline Thread     │  │
│  │  (frontend) │  │  Handler    │  │                         │  │
│  └─────────────┘  └─────────────┘  │  ┌─────┐ ┌─────┐ ┌───┐  │  │
│                                     │  │Audio│→│ VAD │→│STT│  │  │
│                                     │  │Capt.│ │     │ │   │  │  │
│                                     │  └─────┘ └─────┘ └───┘  │  │
│                                     │            ↓            │  │
│                                     │  ┌──────┐ ┌─────────┐   │  │
│                                     │  │Inject│←│ Polish  │   │  │
│                                     │  └──────┘ └─────────┘   │  │
│                                     └─────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

### Crates

| Crate | Purpose |
|-------|---------|
| `vd-core` | Shared types, config, constants |
| `vd-audio` | Audio capture via cpal, resampling via rubato |
| `vd-vad` | Voice Activity Detection using Silero ONNX |
| `vd-whisper` | Whisper transcription via whisper-rs |
| `vd-polish` | Text cleanup (regex-based, optional LLM) |
| `vd-inject` | Text injection via clipboard + Ctrl+V |
| `voice-dict-desktop` | Tauri app, pipeline orchestration |

### Data Flow

1. **Audio Capture** → cpal captures from microphone at native sample rate
2. **Resampling** → rubato resamples to 16kHz for Whisper (sinc interpolation)
3. **VAD** → Silero detects speech boundaries
4. **Transcription** → Whisper converts audio to text (GPU or CPU)
5. **Polish** → Regex cleanup (capitalization, punctuation)
6. **Injection** → Text copied to clipboard, Ctrl+V sent to active window

---

## GPU Setup (NVIDIA CUDA)

GPU acceleration provides ~60x speedup (30s → 0.5s for 20s audio).

### Requirements

- NVIDIA GPU (Compute Capability 7.0+ recommended)
- CUDA Toolkit 12.x or 13.x
- Compatible driver (see table below)

| CUDA Toolkit | Minimum Driver |
|--------------|----------------|
| CUDA 12.0 | 525.60+ |
| CUDA 12.7 | 565.xx+ |
| CUDA 13.1 | 590.44+ |

### Setup Steps

1. **Install CUDA Toolkit** from https://developer.nvidia.com/cuda-downloads

2. **Copy CUDA DLLs to exe directory**
   ```powershell
   $cudaBin = "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.1\bin"
   $target = ".\target\release"
   Copy-Item "$cudaBin\cublas64_*.dll" $target
   Copy-Item "$cudaBin\cublasLt64_*.dll" $target
   Copy-Item "$cudaBin\cudart64_*.dll" $target
   ```

3. **For RTX 40xx GPUs**: You may need to patch whisper-rs-sys build.rs:
   ```rust
   // Add: config.define("GGML_CUDA_ARCHITECTURES", "89");
   ```

4. **Verify GPU usage** - Look for `"whisper_backend_init: using CUDA backend"` in logs

---

## Audio Notes

### Recommended: USB Microphone

USB microphones provide the most reliable audio capture:
- Consistent quality (built-in DAC)
- No driver issues
- Zero Bluetooth jitter

**Suggested models:**
- Fifine K669B (~$20) - Best value for voice
- Blue Snowball iCE (~$50) - Studio quality
- Logitech H390 USB Headset (~$30) - Headset option

### Not Recommended: Bluetooth

Bluetooth audio on Windows has known issues:
- High jitter (50-500ms gaps between audio chunks)
- HFP profile is low-bandwidth
- Can result in 50-70% audio gaps

### Audio Diagnostics

Check audio quality in logs:
```
=== AUDIO JITTER SUMMARY ===
chunks=502 avg=10.0ms p95=15.0ms max=25.0ms gaps>50ms=0 gap_ratio=0.5%
```

- `gap_ratio < 5%` = Good
- `gap_ratio > 20%` = Audio device issues, try USB mic

---

## Configuration

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `RUST_LOG` | `info` | Log level (debug, info, warn, error) |
| `VD_WHISPER_GPU` | `1` | Set to `0` to force CPU |

### Whisper Models

| Model | Size | Quality | Speed |
|-------|------|---------|-------|
| tiny.en | 75MB | Fair | Fastest |
| base.en | 142MB | Good | Fast |
| small.en | 466MB | Great | Medium |
| medium.en | 1.5GB | Excellent | Slow |

Default: `small.en` (best balance of quality and speed)

---

## Development

### Project Structure

```
voice-dict/
├── apps/
│   └── desktop/           # Tauri desktop app
│       ├── src/           # React frontend
│       └── src-tauri/     # Rust backend
├── crates/
│   ├── vd-core/          # Shared types
│   ├── vd-audio/         # Audio capture
│   ├── vd-vad/           # Voice activity detection
│   ├── vd-whisper/       # Transcription
│   ├── vd-polish/        # Text cleanup
│   └── vd-inject/        # Text injection
├── docs/
│   └── TECHNICAL_NOTES.md # Architecture decisions
├── RUNLOG.md             # Test session logs
└── README.md
```

### Running in Development

```powershell
# Run with hot reload
cd apps/desktop
npm run tauri dev

# Run tests
cargo test --workspace

# Build release
cargo build --release -p voice-dict-desktop
```

### Debug Logging

```powershell
$env:RUST_LOG = "info,vd_audio=debug,vd_whisper=debug"
.\target\release\voice-dict-desktop.exe
```

Debug files saved to: `%LOCALAPPDATA%\voice-dict\debug\`

---

## Troubleshooting

| Issue | Solution |
|-------|----------|
| GPU not used | Check CUDA DLLs in exe dir, verify driver version |
| High gap ratio (>20%) | Switch to USB microphone |
| Empty transcription | Check audio levels, verify VAD detecting speech |
| Build errors | Install VS Build Tools, run from Developer PowerShell |

See [docs/TECHNICAL_NOTES.md](docs/TECHNICAL_NOTES.md) for detailed troubleshooting.

---

## Documentation

- [TECHNICAL_NOTES.md](docs/TECHNICAL_NOTES.md) - Architecture, trade-offs, troubleshooting
- [RUNLOG.md](RUNLOG.md) - Test session results and environment snapshots

---

## License

MIT License

---

## Acknowledgments

- [whisper.cpp](https://github.com/ggerganov/whisper.cpp) - Whisper C++ implementation
- [whisper-rs](https://github.com/tazz4843/whisper-rs) - Rust bindings for whisper.cpp
- [Silero VAD](https://github.com/snakers4/silero-vad) - Voice Activity Detection
- [Tauri](https://tauri.app/) - Desktop app framework
- [cpal](https://github.com/RustAudio/cpal) - Cross-platform audio
- [rubato](https://github.com/HEnquist/rubato) - High-quality audio resampling
