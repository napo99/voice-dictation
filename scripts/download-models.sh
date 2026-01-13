#!/bin/bash
# Voice-Dict Model Download Script
# Downloads required models for voice dictation

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Determine models directory
if [[ "$OSTYPE" == "linux-gnu"* ]]; then
    MODELS_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/voice-dict/models"
elif [[ "$OSTYPE" == "darwin"* ]]; then
    MODELS_DIR="$HOME/Library/Application Support/voice-dict/models"
elif [[ "$OSTYPE" == "msys" ]] || [[ "$OSTYPE" == "cygwin" ]]; then
    MODELS_DIR="$LOCALAPPDATA/voice-dict/models"
else
    MODELS_DIR="$HOME/.voice-dict/models"
fi

# Model URLs and sizes
SILERO_VAD_URL="https://github.com/snakers4/silero-vad/raw/master/files/silero_vad.onnx"
SILERO_VAD_FILE="silero_vad.onnx"
SILERO_VAD_SIZE="2MB"

WHISPER_BASE_URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin"
WHISPER_BASE_FILE="ggml-base.en.bin"
WHISPER_BASE_SIZE="74MB"

echo -e "${GREEN}Voice-Dict Model Downloader${NC}"
echo "================================"
echo ""
echo -e "Models directory: ${YELLOW}$MODELS_DIR${NC}"
echo ""

# Create models directory
mkdir -p "$MODELS_DIR"

# Function to download with progress
download_model() {
    local url="$1"
    local file="$2"
    local size="$3"
    local path="$MODELS_DIR/$file"

    if [[ -f "$path" ]]; then
        echo -e "${GREEN}✓${NC} $file already exists"
        return 0
    fi

    echo -e "${YELLOW}Downloading${NC} $file ($size)..."

    # Try curl first, fall back to wget
    if command -v curl &> /dev/null; then
        curl -L --progress-bar "$url" -o "$path"
    elif command -v wget &> /dev/null; then
        wget --progress=bar:force:noscroll "$url" -O "$path"
    else
        echo -e "${RED}Error: Neither curl nor wget found. Please install one of them.${NC}"
        exit 1
    fi

    if [[ -f "$path" ]]; then
        echo -e "${GREEN}✓${NC} Downloaded $file"
    else
        echo -e "${RED}✗${NC} Failed to download $file"
        exit 1
    fi
}

# Download models
echo "Downloading required models..."
echo ""

download_model "$SILERO_VAD_URL" "$SILERO_VAD_FILE" "$SILERO_VAD_SIZE"
download_model "$WHISPER_BASE_URL" "$WHISPER_BASE_FILE" "$WHISPER_BASE_SIZE"

echo ""
echo -e "${GREEN}All models downloaded successfully!${NC}"
echo ""
echo "Models are stored in:"
echo -e "  ${YELLOW}$MODELS_DIR${NC}"
echo ""
echo "You can now run Voice-Dict."
