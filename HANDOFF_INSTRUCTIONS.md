# Handoff Instructions: Voice-Dict MVP

**Current Status:** "One-Shot" code generation complete for Core Logic. Project Scaffolding (Tauri/React) needs to be initialized in the new location.

## 1. Architecture Overview (Hybrid Stack)
We chose **Option B (High Performance)**:
*   **Frontend (Rust/Tauri):** Handles OS Hooks (Key Injection), Window Management, and Global Hotkeys.
*   **Backend (Python Sidecar):** Handles AI Inference (`faster-whisper` + `Ollama`).
    *   *Why Python?* Uses `CTranslate2` engine which is often faster than raw C++ Whisper, and integrates easily with Ollama.
*   **Communication:** WebSocket (`ws://localhost:8765`) between Rust UI and Python Engine.

## 2. Migration Steps (How to move to `projects/voice-dict`)

1.  **Copy Files:** Copy the `dictation-app` folder from the Artifacts directory (where this file is) to your target:
    ```bash
    mkdir -p ~/projects/voice-dict
    cp -r dictation-app/* ~/projects/voice-dict/
    cp setup_migration.sh ~/projects/voice-dict/
    cd ~/projects/voice-dict
    ```

2.  **Run Setup Script:**
    Run the included `setup_migration.sh`. It will:
    *   Initialize Git.
    *   Setup Python using **`uv`** (as requested).
    *   Scaffold the Tauri/React project.
    *   Inject the AI code (`engine.py`, `main.rs`, etc).

## 3. Next Agent Prompt (Copy this to the new Agent)
> "Context: We are building a low-latency Dictation App (Wispr Flow clone) using Tauri (Rust) and Python (Faster-Whisper).
> The project structure is already set up in this directory using `uv` for Python and `npm` for frontend.
>
> **Your Job:**
> 1. Verify `uv` environment is active.
> 2. Check `backend/engine.py` logic.
> 3. Run the app (`npm run tauri dev`) and debug the WebSocket connection.
> 4. Performance constraint: Low Latency is critical."

## 4. Requirement: `uv` Usage
We use `uv` for ultra-fast Python package management.
*   Create venv: `uv venv`
*   Install deps: `uv pip install -r backend/requirements.txt`
*   Run engine: `uv run python backend/server.py`
