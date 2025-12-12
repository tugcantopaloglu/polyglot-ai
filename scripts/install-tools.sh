#!/bin/bash
# Polyglot-AI Tool Installation Script
# This script installs the AI CLI tools required by Polyglot-AI

set -e

echo "==================================="
echo "Polyglot-AI Tool Installation"
echo "==================================="
echo

# Detect OS
OS="unknown"
if [[ "$OSTYPE" == "linux-gnu"* ]]; then
    OS="linux"
elif [[ "$OSTYPE" == "darwin"* ]]; then
    OS="macos"
elif [[ "$OSTYPE" == "msys" ]] || [[ "$OSTYPE" == "win32" ]]; then
    OS="windows"
fi

echo "Detected OS: $OS"
echo

# Function to check if a command exists
command_exists() {
    command -v "$1" >/dev/null 2>&1
}

# Function to install Node.js if not present
install_nodejs() {
    if command_exists node; then
        echo "Node.js is already installed: $(node --version)"
        return 0
    fi

    echo "Installing Node.js..."
    if [[ "$OS" == "linux" ]]; then
        curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash -
        sudo apt-get install -y nodejs
    elif [[ "$OS" == "macos" ]]; then
        brew install node
    else
        echo "Please install Node.js manually from https://nodejs.org/"
        exit 1
    fi
}

# Install Claude Code CLI
install_claude() {
    echo
    echo "Installing Claude Code CLI..."
    if command_exists claude; then
        echo "Claude Code is already installed"
        return 0
    fi

    npm install -g @anthropic-ai/claude-code || {
        echo "Warning: Failed to install Claude Code CLI"
        echo "You may need to install it manually or set up API key"
    }
}

# Install Gemini CLI
install_gemini() {
    echo
    echo "Installing Gemini CLI..."
    if command_exists gemini; then
        echo "Gemini CLI is already installed"
        return 0
    fi

    npm install -g @anthropic-ai/gemini-cli 2>/dev/null || {
        # Gemini CLI might have different package name or installation method
        echo "Note: Gemini CLI installation may require manual setup"
        echo "Visit: https://cloud.google.com/gemini for installation instructions"
    }
}

# Install Codex CLI
install_codex() {
    echo
    echo "Installing Codex CLI..."
    if command_exists codex; then
        echo "Codex CLI is already installed"
        return 0
    fi

    npm install -g @openai/codex-cli 2>/dev/null || {
        # Alternative installation
        pip install openai-codex 2>/dev/null || {
            echo "Note: Codex CLI installation may require manual setup"
            echo "Visit: https://openai.com/codex for installation instructions"
        }
    }
}

# Install GitHub Copilot CLI
install_copilot() {
    echo
    echo "Installing GitHub Copilot CLI..."

    # First check if gh is installed
    if ! command_exists gh; then
        echo "Installing GitHub CLI..."
        if [[ "$OS" == "linux" ]]; then
            curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg | sudo dd of=/usr/share/keyrings/githubcli-archive-keyring.gpg
            echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" | sudo tee /etc/apt/sources.list.d/github-cli.list > /dev/null
            sudo apt update
            sudo apt install gh -y
        elif [[ "$OS" == "macos" ]]; then
            brew install gh
        else
            echo "Please install GitHub CLI manually from https://cli.github.com/"
        fi
    fi

    # Install Copilot extension
    if command_exists gh; then
        gh extension install github/gh-copilot 2>/dev/null || {
            echo "GitHub Copilot CLI extension may already be installed or requires authentication"
            echo "Run 'gh auth login' first, then 'gh extension install github/gh-copilot'"
        }
    fi
}

# Main installation
main() {
    echo "Starting installation..."
    echo

    # Install Node.js first
    install_nodejs

    # Install AI tools
    install_claude
    install_gemini
    install_codex
    install_copilot

    echo
    echo "==================================="
    echo "Installation Complete!"
    echo "==================================="
    echo
    echo "Installed tools status:"
    echo -n "  Claude Code: "; command_exists claude && echo "OK" || echo "Not found"
    echo -n "  Gemini CLI:  "; command_exists gemini && echo "OK" || echo "Not found"
    echo -n "  Codex CLI:   "; command_exists codex && echo "OK" || echo "Not found"
    echo -n "  GitHub CLI:  "; command_exists gh && echo "OK" || echo "Not found"
    echo
    echo "Note: You may need to configure API keys for each tool."
    echo "Refer to each tool's documentation for setup instructions."
}

main "$@"
