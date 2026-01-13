#!/bin/bash
# setup_migration.sh - Migration Script for Voice-Dict MVP
# Usage: ./setup_migration.sh
# Run this inside the ~/projects/voice-dict folder AFTER moving the files there.

set -e # Exit on error

echo "🚀 Starting Voice-Dict Migration & Setup..."

# 1. Initialize Git
if [ ! -d ".git" ]; then
    echo "📦 Initializing Git repository..."
    git init
    git branch -m main
else
    echo "✅ Git already initialized."
fi

# 2. Setup Python Backend with UV
echo "🐍 Setting up Python Backend (using uv)..."
cd backend

# Check for uv
if ! command -v uv &> /dev/null; then
    echo "❌ 'uv' not found. Installing uv..."
    curl -LsSf https://astral.sh/uv/install.sh | sh
    source $HOME/.cargo/env
fi

# Create venv and install
uv venv
source .venv/bin/activate
echo "📦 Installing Python dependencies..."
uv pip install -r requirements.txt
cd ..

# 3. Scaffold Frontend (Tauri) - Interactive workaround
echo "🎨 Setting up Tauri Frontend..."
if [ ! -f "package.json" ]; then
    echo "⚡ Creating Tauri App Scaffold..."
    # Create a fresh tauri app in a temp dir and move files over to avoid overwriting our custom code
    npm create tauri-app@latest temp-app -- --template react-ts --manager npm -y
    
    # Move boilerplate content (except what we already customized)
    cp -n temp-app/package.json .
    cp -n temp-app/tsconfig.json .
    cp -n temp-app/tauri.conf.json src-tauri/
    cp -rn temp-app/src/* src/
    cp -rn temp-app/src-tauri/* src-tauri/
    
    rm -rf temp-app
    
    # Install deps
    npm install
else
    echo "✅ Project files found."
fi

# 4. Final Instruction
echo "✅ Setup Complete!"
echo "------------------------------------------------"
echo "To run the app:"
echo "1. Terminal A (Backend):"
echo "   cd backend && source .venv/bin/activate && uv run python server.py"
echo ""
echo "2. Terminal B (Frontend):"
echo "   npm run tauri dev"
echo "------------------------------------------------"
