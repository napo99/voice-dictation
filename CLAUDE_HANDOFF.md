# Claude Opus Handoff: Voice-Dict Implementation

**Date:** 2025-01-13
**From:** Claude Opus 4.5 (planning session)
**To:** Claude Opus 4.5 (implementation session)
**Repo:** `/home/screener/projects/voice-dict`
**Branch:** Create `claude-impl` from `main`

---

## TL;DR - Start Here

```bash
cd /home/screener/projects/voice-dict
git checkout -b claude-impl
# Start implementing the Pure Rust voice dictation app
```

**Your mission:** Build a Wispr Flow clone in Pure Rust with <500ms latency.

---

## 1. Project Context

### What We're Building
A **local-first voice dictation app** that:
1. Captures audio via global hotkey
2. Detects speech (VAD)
3. Transcribes with Whisper (local)
4. Polishes text with LLM (local) - removes "um", "uh", fixes grammar
5. Injects text into any app at cursor position

### Why Pure Rust (Critical Decision)
Previous MVP used Python sidecar + Ollama. **REJECTED** because:
- Ollama HTTP cold start: 2-5 seconds
- Python GIL blocks audio thread
- WebSocket IPC adds latency
- **Impossible to achieve <500ms target**

New architecture: **100% Rust, in-process inference**

---

## 2. Technical Stack (MANDATORY)

| Component | Crate | Why |
|-----------|-------|-----|
| Desktop | `tauri 2.x` | Native, small binary |
| Audio | `cpal` | Low-latency, cross-platform |
| VAD | `ort` (ONNX) + Silero | <10ms detection |
| STT | `whisper-rs` | C++ bindings, fast |
| LLM | `candle` + Phi-3-mini | In-process, no cold start |
| Key Injection | `enigo 0.2+` | Cross-platform |
| Async | `tokio` | UI/IPC only |
| Channels | `crossbeam-channel` | Audio thread comm |

### PROHIBITED
- NO Python
- NO Ollama
- NO cloud APIs
- NO Electron

---

## 3. Architecture

### Threading Model (CRITICAL)

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
│        AUDIO THREAD (std::thread, HIGH PRIORITY)           │
│  ┌─────────────┐  ┌─────────────┐                          │
│  │ cpal Stream │  │ Ring Buffer │  ← NOT async!            │
│  └─────────────┘  └─────────────┘                          │
└─────────────────────────────────────────────────────────────┘
         │
         ▼
┌─────────────────────────────────────────────────────────────┐
│              INFERENCE THREAD (std::thread)                 │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐  │
│  │ Silero VAD  │─►│ Whisper-rs  │─►│ Candle (Phi-3)      │  │
│  └─────────────┘  └─────────────┘  └─────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

**CRITICAL RULES:**
1. Audio thread: `std::thread` with high priority, **NOT async**
2. Use `crossbeam-channel` for audio, **NOT tokio channels**
3. Never block audio thread with inference
4. **NEVER write audio to disk** - memory-only buffers

### Latency Budget

```
Audio Capture (cpal):           5ms
VAD Detection (Silero):        10ms
Whisper Inference (base.en):  300ms  (for 3s utterance)
LLM Polish (Phi-3 Q4):        150ms
Key Injection (enigo):          5ms
────────────────────────────────────
TOTAL TARGET:                 470ms  ✓ (under 500ms)
```

---

## 4. Project Structure

```
voice-dict/
├── Cargo.toml                    # Workspace root
├── crates/
│   ├── vd-core/                  # Shared types, traits, errors
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   ├── vd-audio/                 # Audio capture (cpal)
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   ├── vd-vad/                   # Silero VAD (ort/onnx)
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   ├── vd-whisper/               # whisper-rs bindings
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   ├── vd-polish/                # Regex + Candle/Phi-3
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   ├── vd-inject/                # OS key injection (enigo)
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   └── vd-engine/                # Orchestration layer
│       ├── Cargo.toml
│       └── src/lib.rs
├── apps/
│   └── desktop/                  # Tauri application
│       ├── src-tauri/
│       │   ├── Cargo.toml
│       │   └── src/main.rs
│       ├── src/
│       │   └── App.tsx
│       ├── package.json
│       └── index.html
├── scripts/
│   └── download-models.sh
└── tests/
    └── fixtures/
```

---

## 5. Workspace Cargo.toml

```toml
[workspace]
resolver = "2"
members = [
    "crates/*",
    "apps/desktop/src-tauri",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT"

[workspace.dependencies]
# Audio
cpal = "0.15"
# VAD
ort = "2.0"
ndarray = "0.16"
# Whisper
whisper-rs = "0.11"
# LLM
candle-core = "0.8"
candle-nn = "0.8"
candle-transformers = "0.8"
hf-hub = "0.3"
tokenizers = "0.20"
# Injection
enigo = "0.2"
# Async
tokio = { version = "1", features = ["full"] }
crossbeam-channel = "0.5"
# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
# Logging
tracing = "0.1"
tracing-subscriber = "0.3"
# Error handling
thiserror = "2"
anyhow = "1"
# Utils
dirs = "5"
regex = "1"
reqwest = { version = "0.12", features = ["blocking", "stream"] }
indicatif = "0.17"
```

---

## 6. Implementation Order

### Recommended Sequence (Sequential)

```
1. vd-core       (30 min)  - Types, errors, traits
2. vd-audio      (1 hr)    - cpal capture, ring buffer
3. vd-vad        (1 hr)    - Silero ONNX integration
4. vd-whisper    (1 hr)    - whisper-rs wrapper
5. vd-polish     (1.5 hr)  - Regex + Candle/Phi-3
6. vd-inject     (30 min)  - enigo wrapper
7. vd-engine     (1 hr)    - Orchestration, state machine
8. desktop app   (1.5 hr)  - Tauri + React UI
9. scripts       (30 min)  - Model download
10. tests        (1 hr)    - Integration tests
```

### Parallel Strategy (With Multiple Agents)

```
Agent 1: vd-core → vd-audio → vd-engine
Agent 2: vd-vad → vd-whisper
Agent 3: vd-polish (regex + candle)
Agent 4: vd-inject → desktop app → scripts
```

**Sync points:**
- After vd-core (shared types)
- Before vd-engine (needs all inference crates)
- Before desktop (needs vd-engine)

---

## 7. Feature Checklist (P0 - Must Have)

- [ ] Global hotkey to start/stop recording
- [ ] Voice activity detection (auto-detect speech)
- [ ] Local Whisper transcription (base.en)
- [ ] Text polishing (filler removal + grammar)
- [ ] Text injection into active application
- [ ] System tray icon with status
- [ ] Microphone selection
- [ ] First-run model download with progress UI

---

## 8. Key Implementation Details

### vd-audio: High-Priority Thread

```rust
use std::thread::{self, JoinHandle};
use crossbeam_channel::{bounded, Sender};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub fn start_capture(tx: Sender<Vec<f32>>) -> JoinHandle<()> {
    thread::Builder::new()
        .name("audio-capture".into())
        .spawn(move || {
            // Set thread priority HIGH (platform-specific)
            #[cfg(target_os = "linux")]
            unsafe {
                libc::setpriority(libc::PRIO_PROCESS, 0, -10);
            }

            let host = cpal::default_host();
            let device = host.default_input_device().expect("No input device");
            // ... cpal stream setup
        })
        .expect("Failed to spawn audio thread")
}
```

### vd-polish: Two-Layer Approach

```rust
// Layer 1: Fast regex (<1ms)
const FILLERS: &[&str] = &[
    "um", "uh", "er", "ah", "like", "you know",
    "basically", "actually", "literally", "so",
    "I mean", "kind of", "sort of", "right",
];

pub fn remove_fillers(text: &str) -> String {
    let mut result = text.to_string();
    for filler in FILLERS {
        let pattern = format!(r"(?i)\b{}\b,?\s*", regex::escape(filler));
        let re = regex::Regex::new(&pattern).unwrap();
        result = re.replace_all(&result, "").to_string();
    }
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

// Layer 2: LLM polish (~150ms) - only for longer text
pub fn llm_polish(text: &str) -> Result<String> {
    // candle + Phi-3-mini inference
}
```

### Model Management

```rust
// Models downloaded on first run, NOT bundled
const MODELS: &[(&str, &str, u64)] = &[
    ("silero_vad.onnx",
     "https://github.com/snakers4/silero-vad/raw/master/files/silero_vad.onnx",
     2_000_000),
    ("ggml-base.en.bin",
     "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin",
     150_000_000),
];

pub fn models_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("voice-dict")
        .join("models")
}
```

### Privacy: Memory-Only

```rust
// NEVER write audio to disk
pub struct AudioBuffer {
    samples: Vec<f32>,  // In-memory only
    sample_rate: u32,
}

// NO: File, PathBuf, tempfile, etc.
```

---

## 9. Code Quality Rules

- `cargo clippy` must pass with **no warnings**
- `cargo fmt` must be applied
- **No `unwrap()` in library code** - use `?` or proper error handling
- All public APIs documented with `///` doc comments
- Minimum 60% test coverage on core modules

---

## 10. Competition Context

This is a competition against **Gemini 3.0** (on `gemini-impl` branch).

**Evaluation (100 points):**
- Functionality: 30 pts
- Performance: 25 pts (<500ms latency)
- Code Quality: 20 pts
- Architecture: 15 pts
- Testing: 10 pts

**You're on `claude-impl` branch. Build to win.**

---

## 11. Files in Repo

```
voice-dict/
├── README.md           # Project overview
├── SPEC.md             # Full specification
├── GEMINI_PROMPT.md    # Competitor's prompt (don't read)
├── CLAUDE_HANDOFF.md   # THIS FILE
└── archive/            # Legacy Python MVP (reference only)
```

---

## 12. Quick Start Commands

```bash
# 1. Go to repo
cd /home/screener/projects/voice-dict

# 2. Create your branch
git checkout -b claude-impl

# 3. Create workspace structure
mkdir -p crates/{vd-core,vd-audio,vd-vad,vd-whisper,vd-polish,vd-inject,vd-engine}/src
mkdir -p apps/desktop/{src-tauri/src,src}
mkdir -p scripts tests/fixtures

# 4. Start with Cargo.toml, then vd-core, then work through the list

# 5. Commit often
git add -A && git commit -m "feat(vd-core): initial types and errors"
```

---

## 13. Success Criteria

1. **`cargo build --release`** succeeds
2. **App launches** and shows system tray icon
3. **Press hotkey** → recording starts
4. **Speak** → text appears in active app
5. **Latency** < 500ms measured
6. **No crashes** during normal use

---

## BEGIN IMPLEMENTATION

You have full context. Create `claude-impl` branch and start building.

**Recommended first commit:**
1. Workspace `Cargo.toml`
2. `crates/vd-core/Cargo.toml` + `src/lib.rs`
3. Basic types: `AudioSample`, `TranscriptionResult`, `VoiceDictError`

**Go.**
