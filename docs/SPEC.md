# Voice-Dict Specification v2.0 (Pure Rust)

## Overview
Local-first voice dictation app (Wispr Flow clone) built with Tauri + Pure Rust.

## Repository Structure
```
voice-dict/
├── main           # Clean base - winner merges here
├── claude-impl    # Claude Opus 4.5 implementation
└── gemini-impl    # Gemini 3.0 implementation
```

**Compare:** `git diff claude-impl..gemini-impl`

---

## Critical Constraint: NO PYTHON

> **MANDATORY:** Pure Rust build. No Python sidecar, no Ollama, no external processes.
> Rationale: <500ms latency is impossible with Python GIL + Ollama HTTP overhead.

---

## Product Requirements

### What We're Building
A **local-first voice dictation app** that:
1. Listens for voice input via global hotkey
2. Transcribes speech to text using local Whisper model
3. Polishes text (removes filler words, adds punctuation) using local LLM
4. Injects final text into any application at cursor position

### Target Platforms
- Linux (Ubuntu 22.04+) - PRIMARY
- macOS (14+) - SECONDARY
- Windows (11) - SECONDARY

### Performance Requirements
| Metric | Target | Hard Limit |
|--------|--------|------------|
| End-to-end latency | <500ms | <1000ms |
| Memory (idle) | <200MB | <500MB |
| Memory (active) | <1GB | <2GB |
| Binary size | <50MB | <100MB (excl. models) |

---

## Technical Stack (MANDATORY)

| Component | Required Technology | Rationale |
|-----------|---------------------|-----------|
| Desktop Framework | **Tauri 2.x** | Native, small binary |
| Core Language | **Rust (100%)** | No Python, no Node runtime |
| Audio Capture | **cpal** | Native Rust, low latency |
| Voice Activity Detection | **silero-vad via ort** | ONNX runtime, <10ms response |
| Speech Recognition | **whisper-rs** | C++ bindings, zero IPC overhead |
| LLM (text polish) | **candle + Phi-3-mini** | In-process, no cold start |
| Key Injection | **enigo 0.2+** | Cross-platform |
| Async Runtime | **tokio** | For IPC and UI events |
| Channels | **crossbeam-channel** | Audio thread communication |

### PROHIBITED
- NO Python (no sidecar, no subprocess)
- NO Ollama (HTTP overhead kills latency)
- NO cloud APIs for inference
- NO Electron / Node.js runtime
- NO Docker requirement for end users

---

## Threading Model

```
┌─────────────────────────────────────────────────────────────┐
│                    MAIN THREAD (Tokio)                      │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐  │
│  │ Tauri UI    │  │ IPC Handler │  │ State Management    │  │
│  └─────────────┘  └─────────────┘  └─────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
         ▲                                      │
         │ crossbeam-channel                    │
         │ (audio samples)                      ▼
┌─────────────────────────────────────────────────────────────┐
│              AUDIO THREAD (std::thread, HIGH PRIORITY)      │
│  ┌─────────────┐  ┌─────────────┐                          │
│  │ cpal Stream │  │ Ring Buffer │                          │
│  └─────────────┘  └─────────────┘                          │
└─────────────────────────────────────────────────────────────┘
         │
         ▼
┌─────────────────────────────────────────────────────────────┐
│              INFERENCE THREAD (std::thread)                 │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐  │
│  │ Silero VAD  │─►│ Whisper-rs  │─►│ Candle (Phi-3)      │  │
│  │ (ONNX)      │  │             │  │                     │  │
│  └─────────────┘  └─────────────┘  └─────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

**CRITICAL:**
- Audio thread: Use `std::thread` with high priority, NOT async
- Communication: Use `crossbeam-channel` (not tokio channels) for audio
- Inference: Can be async or dedicated thread, but must not block audio

---

## Required Project Structure

```
voice-dict/
├── Cargo.toml                    # Workspace root
├── crates/
│   ├── vd-core/                  # Shared types, traits, errors
│   ├── vd-audio/                 # Audio capture (cpal)
│   ├── vd-vad/                   # Silero VAD (ort/onnx)
│   ├── vd-whisper/               # whisper-rs bindings
│   ├── vd-polish/                # Candle + Phi-3 + regex filler removal
│   ├── vd-inject/                # OS key injection (enigo)
│   └── vd-engine/                # Orchestration layer
├── apps/
│   └── desktop/                  # Tauri application
│       ├── src-tauri/
│       └── src/                  # React frontend
├── tests/
│   ├── fixtures/                 # Audio samples
│   └── integration/
└── scripts/
    └── download-models.sh
```

---

## Feature Requirements (Phase 1)

### P0 - Must Have
- [ ] Global hotkey to start/stop recording (configurable)
- [ ] Voice activity detection (auto-detect speech start/end)
- [ ] Local Whisper transcription (base.en model minimum)
- [ ] Local LLM text polishing (filler word removal, punctuation)
- [ ] Text injection into active application
- [ ] System tray icon with status indicator
- [ ] Microphone selection
- [ ] First-run model download with progress UI

### P1 - Should Have
- [ ] Continuous dictation mode
- [ ] Visual feedback (floating widget)
- [ ] Configurable silence timeout
- [ ] Error handling with user feedback

---

## Evaluation Criteria (100 points)

| Category | Points | Criteria |
|----------|--------|----------|
| **Functionality** | 30 | Does it work? All P0 features? |
| **Performance** | 25 | Latency <500ms target |
| **Code Quality** | 20 | Idiomatic Rust, no unwrap() |
| **Architecture** | 15 | Clean separation, proper threading |
| **Testing** | 10 | Coverage, meaningful tests |

### Latency Budget
```
Audio Capture (cpal):           5ms
VAD Detection (Silero):        10ms
Whisper Inference (base.en):  300ms
LLM Polish (Phi-3 Q4):        150ms
Key Injection (enigo):          5ms
────────────────────────────────────
TOTAL TARGET:                 470ms  ✓
```
