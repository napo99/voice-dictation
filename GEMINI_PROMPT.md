# COMPLETE GEMINI PROMPT - COPY EVERYTHING BELOW THIS LINE

---

You are Gemini 3.0, participating in an AI coding competition against Claude Opus 4.5.

## REPOSITORY & BRANCH STRATEGY

**Repo:** `/home/screener/projects/voice-dict`

**Branch Structure:**
```
voice-dict/
├── main           # Base branch (spec + docs only)
├── claude-impl    # Claude Opus 4.5 implementation (competitor)
└── gemini-impl    # YOUR branch - implement here
```

**Your workflow:**
```bash
cd /home/screener/projects/voice-dict
git checkout -b gemini-impl
# ... implement all code ...
git add -A && git commit -m "feat: description"
```

**Comparison after both complete:** `git diff claude-impl..gemini-impl`

---

## YOUR TASK
Build a local-first voice dictation app (Wispr Flow clone) in **Pure Rust** from scratch.

## CRITICAL CONSTRAINTS
- **NO PYTHON** - Zero Python code, no sidecar, no subprocess
- **NO OLLAMA** - HTTP overhead kills latency
- **NO CLOUD APIs** - All inference must be local
- **100% RUST** - Only Rust for core logic

## REQUIRED STACK
| Component | Technology |
|-----------|------------|
| Desktop | Tauri 2.x |
| Audio | cpal |
| VAD | silero-vad via ort (ONNX) |
| STT | whisper-rs |
| LLM | candle + Phi-3-mini |
| Key Injection | enigo 0.2+ |
| Async | tokio |
| Channels | crossbeam-channel |

## PERFORMANCE TARGET
- End-to-end latency: **<500ms**
- Memory (idle): <200MB
- Memory (active): <1GB

## THREADING MODEL (CRITICAL)
```
MAIN THREAD (Tokio)
├── Tauri UI
├── IPC Handler
└── State Management
        │
        │ crossbeam-channel
        ▼
AUDIO THREAD (std::thread, HIGH PRIORITY)  ← NOT async!
├── cpal Stream
└── Ring Buffer
        │
        ▼
INFERENCE THREAD (std::thread)
├── Silero VAD (ONNX)
├── Whisper-rs
└── Candle (Phi-3)
```

**CRITICAL:** Audio thread MUST use `std::thread` with high priority, NOT async.
Use `crossbeam-channel` for audio samples, not tokio channels.

## REQUIRED PROJECT STRUCTURE
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
│   ├── vd-polish/                # Candle + Phi-3 + regex filler removal
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   ├── vd-inject/                # OS key injection (enigo)
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   └── vd-engine/                # Orchestration layer
│       ├── Cargo.toml
│       └── src/lib.rs
├── apps/
│   └── desktop/
│       ├── src-tauri/
│       │   ├── Cargo.toml
│       │   └── src/main.rs
│       ├── src/
│       │   └── App.tsx
│       ├── package.json
│       └── index.html
├── scripts/
│   └── download-models.sh
└── README.md
```

## WORKSPACE CARGO.TOML
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
# Whisper
whisper-rs = "0.11"
# LLM
candle-core = "0.8"
candle-nn = "0.8"
candle-transformers = "0.8"
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
```

## FEATURES TO IMPLEMENT (P0 - Must Have)
1. Global hotkey to start/stop recording
2. Voice activity detection (auto-detect speech start/end)
3. Local Whisper transcription (base.en model)
4. Text polishing (regex filler removal + LLM grammar fix)
5. Text injection into active application
6. System tray icon with status
7. Microphone selection
8. First-run model download with progress

## FILLER WORD REMOVAL (vd-polish)
Implement two layers:
1. **Fast regex** (<1ms): Remove "um", "uh", "like", "you know", etc.
2. **LLM polish** (~150ms): Candle + Phi-3-mini for grammar cleanup

```rust
const FILLERS: &[&str] = &[
    "um", "uh", "er", "ah", "like", "you know",
    "basically", "actually", "literally", "so",
    "I mean", "kind of", "sort of", "right",
];
```

## MODEL MANAGEMENT
Models downloaded on first run, NOT bundled:
- silero_vad.onnx (~2MB)
- ggml-base.en.bin (~150MB)
- phi-3-mini-q4.gguf (~2GB)

Store in: `dirs::data_local_dir().join("voice-dict/models")`

## PRIVACY REQUIREMENT
**NEVER write audio to disk.** Memory-only buffers. No temp files.

## CODE QUALITY
- `cargo clippy` must pass with no warnings
- `cargo fmt` must be applied
- No `unwrap()` in library code - use `?` or proper error handling
- All public APIs documented with `///` doc comments

## EVALUATION (100 points)
- Functionality: 30 pts
- Performance: 25 pts (<500ms latency)
- Code Quality: 20 pts (idiomatic Rust)
- Architecture: 15 pts (clean separation)
- Testing: 10 pts

## OUTPUT FORMAT
Provide complete, working code for ALL files. Format:

```
// File: Cargo.toml
[workspace]
...

// File: crates/vd-core/Cargo.toml
[package]
...

// File: crates/vd-core/src/lib.rs
//! Core types and traits for voice-dict
...
```

## BEGIN
Implement the complete voice-dict application now. Output all files with complete, working code. Start with the workspace Cargo.toml, then each crate in order, then the Tauri app.
