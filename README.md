# Voice-Dict

Local-first voice dictation app (Wispr Flow clone) built with Tauri + Pure Rust.

## AI Competition

This project is being built as a competition between:
- **Claude Opus 4.5** → `claude-impl` branch
- **Gemini 2.5 Pro** → `gemini-impl` branch

Winner merges to `main`.

## Quick Start

```bash
# Download models
./scripts/download-models.sh

# Build
cargo build --release

# Run
cargo run --release -p desktop
```

## Architecture

Pure Rust, no Python sidecar:

```
┌─────────────────────────────────────────────────────────────┐
│                    MAIN THREAD (Tokio)                      │
│  Tauri UI │ IPC Handler │ State Management                  │
└─────────────────────────────────────────────────────────────┘
         ▲ crossbeam-channel
┌─────────────────────────────────────────────────────────────┐
│              AUDIO THREAD (std::thread)                     │
│  cpal Stream │ Ring Buffer                                  │
└─────────────────────────────────────────────────────────────┘
         │
┌─────────────────────────────────────────────────────────────┐
│              INFERENCE THREAD                               │
│  Silero VAD → Whisper-rs → Candle/Phi-3                     │
└─────────────────────────────────────────────────────────────┘
```

## Stack

| Component | Technology |
|-----------|------------|
| Desktop | Tauri 2.x |
| Audio | cpal |
| VAD | Silero (ONNX via ort) |
| STT | whisper-rs |
| LLM | candle + Phi-3-mini |
| Key Injection | enigo |

## Performance Target

- End-to-end latency: **<500ms**
- Memory (idle): <200MB

## Docs

- [SPEC.md](./SPEC.md) - Full specification
- [GEMINI_PROMPT.md](./GEMINI_PROMPT.md) - Prompt for Gemini competitor

## License

MIT
