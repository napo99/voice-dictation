# Windows Handoff: Voice-Dict Testing & Validation

**Date:** 2026-01-13
**From:** Claude Opus 4.5 (WSL2 development session)
**To:** Next Agent (Windows testing session)
**Branch:** `claude-impl`

---

## TL;DR

Voice-Dict is a local-first voice dictation app (Wispr Flow clone) built in Pure Rust + Tauri. The code is **complete and compiles**, but needs testing on **Windows native** because WSL2 has audio/hotkey limitations.

**Your mission:** Set up Windows dev environment, build, and validate the full pipeline works (speak → transcribe → inject text).

---

## 1. Project Status

### What's Done (Core Complete)
- [x] All Rust crates implemented and compiling
- [x] Tauri desktop app with React UI
- [x] VAD (Voice Activity Detection) with Silero ONNX
- [x] Whisper transcription (whisper-rs)
- [x] Text polishing (regex-based filler removal)
- [x] Text injection (enigo)
- [x] Global hotkey support (Alt+Shift+V)
- [x] Manual record button in UI (🎤)
- [x] All 42 tests passing
- [x] Comprehensive logging for debugging
- [x] Audio device listing command

### What's NOT Done - Testing (Priority 1)
- [ ] Windows build and testing
- [ ] Verify microphone capture works
- [ ] Verify full pipeline: record → transcribe → inject
- [ ] Test with Bluetooth microphone (AirPods)
- [ ] Measure end-to-end latency (<500ms target)

### What's NOT Done - Features (Priority 2)
- [ ] Microphone selection dropdown in settings UI
- [ ] Configurable hotkey (currently hardcoded Alt+Shift+V)
- [ ] Proper settings panel (currently just an alert popup)
- [ ] System tray icon with status
- [ ] First-run model download with progress UI

### Recommendation
**Continue development on Windows, not WSL2.** The code is cross-platform but testing requires native audio/hotkey access which WSL2 cannot provide.

---

## 2. Why Windows (Not WSL2)

WSL2 has fundamental limitations:
| Feature | WSL2 | Windows Native |
|---------|------|----------------|
| Microphone (AirPods) | ❌ Bluetooth on Windows side | ✅ Direct access |
| Global hotkeys | ❌ Can't capture Windows keys | ✅ System-wide |
| Audio capture | ❌ PulseAudio/WSLg issues | ✅ Native WASAPI |
| Text injection | ❌ Only Linux apps | ✅ All Windows apps |

---

## 3. Windows Setup Instructions

### Prerequisites

#### 1. Node.js and npm

**Option A - Direct Download (Recommended):**
1. Go to https://nodejs.org/
2. Download "LTS" version (e.g., 20.x.x)
3. Run installer, accept defaults
4. Verify in PowerShell:
   ```powershell
   node --version   # Should show v20.x.x
   npm --version    # Should show 10.x.x
   ```

**Option B - Via winget:**
```powershell
winget install OpenJS.NodeJS.LTS
# Restart PowerShell after install
node --version
npm --version
```

#### 2. Rust (with MSVC toolchain)

1. Go to https://rustup.rs/
2. Download `rustup-init.exe`
3. Run it, select "1) Proceed with installation (default)"
4. Restart PowerShell
5. Verify:
   ```powershell
   rustc --version   # Should show rustc 1.x.x
   cargo --version   # Should show cargo 1.x.x
   ```

**Or via winget:**
```powershell
winget install Rustlang.Rustup
# Restart PowerShell
rustc --version
cargo --version
```

#### 3. Visual Studio Build Tools (Required for Rust on Windows)

1. Go to https://visualstudio.microsoft.com/visual-cpp-build-tools/
2. Download "Build Tools for Visual Studio 2022"
3. Run installer
4. Select **"Desktop development with C++"** workload
5. Click Install (downloads ~2GB)

**Or via winget:**
```powershell
winget install Microsoft.VisualStudio.2022.BuildTools
# Then run Visual Studio Installer and add C++ workload
```

#### 4. Git
```powershell
winget install Git.Git
# Restart PowerShell
git --version
```

#### 5. Claude Code CLI

**Option A - npm (after Node.js installed):**
```powershell
npm install -g @anthropic-ai/claude-code
claude --version
```

**Option B - Direct download:**
1. Go to https://claude.ai/download
2. Download Windows installer
3. Run installer
4. Verify in PowerShell:
   ```powershell
   claude --version
   ```

**First-time setup:**
```powershell
# Login to Claude (opens browser)
claude login

# Navigate to project and start
cd C:\projects\voice-dict
claude
```

#### 6. (Optional) Windows Terminal
Better terminal experience:
```powershell
winget install Microsoft.WindowsTerminal
```

### Get the Code

Option A - Copy from WSL2:
```powershell
# Access WSL files from Windows
cp -r \\wsl$\Ubuntu\home\screener\projects\voice-dict C:\projects\voice-dict
```

Option B - Clone fresh:
```powershell
git clone <repo-url> C:\projects\voice-dict
cd C:\projects\voice-dict
git checkout claude-impl
```

### Download Models

```powershell
# Create models directory
mkdir -p $env:LOCALAPPDATA\voice-dict\models

# Download Whisper model (~147MB)
Invoke-WebRequest -Uri "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin" -OutFile "$env:LOCALAPPDATA\voice-dict\models\ggml-base.en.bin"

# Download VAD model (~2.3MB)
Invoke-WebRequest -Uri "https://github.com/snakers4/silero-vad/raw/master/src/silero_vad/data/silero_vad.onnx" -OutFile "$env:LOCALAPPDATA\voice-dict\models\silero_vad.onnx"

# Verify
dir $env:LOCALAPPDATA\voice-dict\models
```

### Build

```powershell
cd C:\projects\voice-dict\apps\desktop

# Install npm dependencies
npm install

# Build Tauri app (this compiles Rust + bundles frontend)
npm run tauri build

# Binary will be at:
# .\target\release\voice-dict-desktop.exe
```

### Run

```powershell
.\target\release\voice-dict-desktop.exe
```

---

## 4. Expected Behavior

### Startup Logs (Terminal)
```
========================================
Starting Voice-Dict
========================================
Platform: windows
Arch: x86_64
Models directory: "C:\\Users\\<user>\\AppData\\Local\\voice-dict\\models"
=== Audio Input Devices ===
  🎤 Microphone (Your Device) [DEFAULT]
  🎤 AirPods (if connected)
============================
=== Model Check ===
VAD model: ... -> FOUND
Whisper model: ... -> FOUND
Registered global shortcut: Alt+Shift+V
```

### UI Elements
- Window title: "Voice-Dict" (400x120 pixels)
- Status text: "Press Alt+Shift+V"
- 🎤 Record button (red, hold to record)
- ⚙ Settings button (shows audio devices)

### Recording Flow
1. **Hold Alt+Shift+V** (or hold 🎤 button)
2. Terminal shows:
   ```
   >>> HOTKEY PRESSED - Starting recording <<<
   State -> Listening
   Audio capture STARTED
   !!! SPEECH DETECTED - VAD triggered !!!
   State -> Recording
   ```
3. **Speak** into microphone
4. **Release** hotkey/button
5. Terminal shows:
   ```
   >>> HOTKEY RELEASED - Stopping recording <<<
   State -> Processing
   Audio buffer: XXXXX samples (X.XXs)
   >>> Starting Whisper transcription...
   Transcribed XX chars in XXXms
   >>> Injecting text: "your transcribed words"
   State -> Injecting
   Text injected: XX chars
   State -> Idle
   ```
6. **Text appears** at cursor in active application

---

## 5. Project Structure

```
voice-dict/
├── Cargo.toml                 # Workspace root
├── crates/
│   ├── vd-core/              # Shared types, errors (AudioBuffer, VoiceDictError)
│   ├── vd-audio/             # cpal audio capture, device listing
│   ├── vd-vad/               # Silero VAD (ONNX via ort)
│   ├── vd-whisper/           # whisper-rs transcription
│   ├── vd-polish/            # Text cleanup (filler removal)
│   ├── vd-inject/            # enigo text injection
│   └── vd-engine/            # Pipeline orchestration
├── apps/desktop/
│   ├── src-tauri/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs       # Tauri app, commands, hotkey setup
│   │       └── pipeline.rs   # Recording pipeline state machine
│   ├── src/
│   │   ├── App.tsx           # React UI
│   │   └── App.css           # Styles with state colors
│   └── package.json
├── scripts/
│   └── download-models.sh    # Linux model download script
└── WINDOWS_HANDOFF.md        # THIS FILE
```

---

## 6. Key Files to Understand

### `apps/desktop/src-tauri/src/main.rs`
- Tauri app setup
- Global hotkey registration (Alt+Shift+V)
- IPC commands: `start_recording`, `stop_recording`, `list_audio_devices`
- Model checking at startup

### `apps/desktop/src-tauri/src/pipeline.rs`
- Pipeline state machine: Idle → Listening → Recording → Processing → Injecting
- Audio capture initialization
- VAD, Whisper, Polish, Inject orchestration
- Comprehensive logging

### `crates/vd-audio/src/lib.rs`
- `list_input_devices()` - enumerate microphones
- `AudioCapture` - cpal stream management
- Device selection by name

---

## 7. Configuration

### Hotkey
Currently hardcoded in `main.rs:240`:
```rust
let shortcut: Shortcut = "Alt+Shift+V".parse().unwrap();
```

### Audio Device
Currently uses system default. To use specific device, modify `PipelineConfig` in `pipeline.rs`:
```rust
input_device: Some("AirPods".to_string()),
```

### Models Location
Windows: `%LOCALAPPDATA%\voice-dict\models\`
- `ggml-base.en.bin` (Whisper)
- `silero_vad.onnx` (VAD)

---

## 8. Troubleshooting

### "No audio input devices found"
- Check Windows sound settings
- Ensure microphone permissions for the app
- Try: Settings → Privacy → Microphone → Allow apps

### "VAD model: Protobuf parsing failed"
- Model file corrupted, re-download:
```powershell
Invoke-WebRequest -Uri "https://github.com/snakers4/silero-vad/raw/master/src/silero_vad/data/silero_vad.onnx" -OutFile "$env:LOCALAPPDATA\voice-dict\models\silero_vad.onnx"
```

### "Hotkey already registered"
- Another app using Alt+Shift+V
- Kill other instances: `taskkill /f /im voice-dict-desktop.exe`
- Or change hotkey in `main.rs`

### "No audio recorded"
- Check microphone is set as default in Windows
- Check audio level is showing in UI when speaking
- Look for VAD triggering in logs

### Build errors
- Ensure Visual Studio Build Tools installed with C++ workload
- Run from "Developer PowerShell for VS 2022"

---

## 9. Validation Checklist

- [ ] App starts without errors
- [ ] Audio devices listed in terminal at startup
- [ ] Settings button (⚙) shows device list
- [ ] Holding 🎤 button changes UI to yellow (Listening)
- [ ] Speaking triggers red state (Recording) - VAD working
- [ ] Releasing button shows "Processing" then transcription
- [ ] Text appears in a text editor (Notepad, VS Code, etc.)
- [ ] Global hotkey (Alt+Shift+V) works same as button
- [ ] Works with Bluetooth microphone (AirPods)

---

## 10. Success Criteria

1. **`npm run tauri build`** succeeds on Windows
2. **App launches** and shows audio devices
3. **Hold hotkey/button + speak** → text transcribed
4. **Text injected** into active application (Notepad, browser, etc.)
5. **Latency** feels responsive (<1s from release to injection)
6. **No crashes** during normal use

---

## 11. If Issues Persist

### Debug Mode
Run with verbose logging:
```powershell
$env:RUST_LOG="debug,voice_dict_desktop=trace,vd_audio=debug,vd_vad=debug,vd_whisper=debug"
.\target\release\voice-dict-desktop.exe
```

### Check Audio Subsystem
```powershell
# List Windows audio devices
Get-PnpDevice -Class AudioEndpoint | Where-Object Status -eq "OK"
```

### Test Whisper Standalone
```powershell
cd C:\projects\voice-dict
cargo test -p vd-whisper
```

---

## 12. Contact Points

- **Spec:** `SPEC.md` - full requirements
- **Original handoff:** `CLAUDE_HANDOFF.md` - initial implementation guide
- **This file:** `WINDOWS_HANDOFF.md` - Windows testing guide

---

## BEGIN TESTING

You have the complete codebase. Set up Windows environment and validate the full voice dictation pipeline.

**First steps:**
1. Install prerequisites (Rust, Node, VS Build Tools)
2. Copy code to Windows filesystem
3. Download models
4. Build with `npm run tauri build`
5. Run and test with your microphone

**Go.**
