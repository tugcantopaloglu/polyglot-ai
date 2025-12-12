#!/usr/bin/env bash
#
# Polyglot-AI Local - One-Line Installer
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/tugcantopaloglu/selfhosted-ai-code-platform/main/scripts/install.sh | bash
#
# Or with options:
#   curl -fsSL ... | bash -s -- --with-tools
#

set -euo pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Configuration
REPO_URL="https://github.com/tugcantopaloglu/selfhosted-ai-code-platform"
INSTALL_DIR="${POLYGLOT_INSTALL_DIR:-$HOME/.polyglot-ai}"
BIN_DIR="${POLYGLOT_BIN_DIR:-$HOME/.local/bin}"
VERSION="${POLYGLOT_VERSION:-latest}"

# Flags
INSTALL_TOOLS=false
SKIP_RUST=false
VERBOSE=false

print_banner() {
    echo -e "${CYAN}"
    echo "  ____       _             _       _        _    ___ "
    echo " |  _ \ ___ | |_   _  __ _| | ___ | |_     / \  |_ _|"
    echo " | |_) / _ \| | | | |/ _\` | |/ _ \| __|   / _ \  | | "
    echo " |  __/ (_) | | |_| | (_| | | (_) | |_   / ___ \ | | "
    echo " |_|   \___/|_|\__, |\__, |_|\___/ \__| /_/   \_\___|"
    echo "               |___/ |___/                           "
    echo -e "${NC}"
    echo -e "${GREEN}Polyglot-AI Local Installer${NC}"
    echo ""
}

info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

success() {
    echo -e "${GREEN}[OK]${NC} $1"
}

warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

error() {
    echo -e "${RED}[ERROR]${NC} $1"
    exit 1
}

# Parse arguments
parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            --with-tools)
                INSTALL_TOOLS=true
                shift
                ;;
            --skip-rust)
                SKIP_RUST=true
                shift
                ;;
            --install-dir)
                INSTALL_DIR="$2"
                shift 2
                ;;
            --bin-dir)
                BIN_DIR="$2"
                shift 2
                ;;
            --version)
                VERSION="$2"
                shift 2
                ;;
            -v|--verbose)
                VERBOSE=true
                shift
                ;;
            -h|--help)
                show_help
                exit 0
                ;;
            *)
                warn "Unknown option: $1"
                shift
                ;;
        esac
    done
}

show_help() {
    echo "Polyglot-AI Local Installer"
    echo ""
    echo "Usage: install.sh [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  --with-tools    Also install AI CLI tools (Claude, Gemini, etc.)"
    echo "  --skip-rust     Skip Rust installation (if already installed)"
    echo "  --install-dir   Installation directory (default: ~/.polyglot-ai)"
    echo "  --bin-dir       Binary directory (default: ~/.local/bin)"
    echo "  --version       Version to install (default: latest)"
    echo "  -v, --verbose   Verbose output"
    echo "  -h, --help      Show this help"
    echo ""
    echo "Environment variables:"
    echo "  POLYGLOT_INSTALL_DIR  Override install directory"
    echo "  POLYGLOT_BIN_DIR      Override bin directory"
    echo "  POLYGLOT_VERSION      Override version"
}

detect_os() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Linux*)
            OS="linux"
            ;;
        Darwin*)
            OS="macos"
            ;;
        MINGW*|MSYS*|CYGWIN*)
            OS="windows"
            ;;
        *)
            error "Unsupported operating system: $OS"
            ;;
    esac

    case "$ARCH" in
        x86_64|amd64)
            ARCH="x86_64"
            ;;
        aarch64|arm64)
            ARCH="aarch64"
            ;;
        armv7l)
            ARCH="armv7"
            ;;
        *)
            error "Unsupported architecture: $ARCH"
            ;;
    esac

    info "Detected: $OS ($ARCH)"
}

check_dependencies() {
    info "Checking dependencies..."

    # Check for git
    if ! command -v git &> /dev/null; then
        error "git is required but not installed. Please install git first."
    fi
    success "git found"

    # Check for curl or wget
    if command -v curl &> /dev/null; then
        DOWNLOADER="curl"
        success "curl found"
    elif command -v wget &> /dev/null; then
        DOWNLOADER="wget"
        success "wget found"
    else
        error "curl or wget is required but neither is installed."
    fi
}

install_rust() {
    if command -v cargo &> /dev/null; then
        RUST_VERSION=$(rustc --version)
        success "Rust already installed: $RUST_VERSION"
        return
    fi

    if [ "$SKIP_RUST" = true ]; then
        error "Rust is required but --skip-rust was specified"
    fi

    info "Installing Rust..."

    if [ "$DOWNLOADER" = "curl" ]; then
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    else
        wget -qO- https://sh.rustup.rs | sh -s -- -y
    fi

    # Source cargo environment
    source "$HOME/.cargo/env" 2>/dev/null || true

    if command -v cargo &> /dev/null; then
        success "Rust installed successfully"
    else
        error "Failed to install Rust"
    fi
}

install_nodejs() {
    if command -v node &> /dev/null; then
        NODE_VERSION=$(node --version)
        success "Node.js already installed: $NODE_VERSION"
        return
    fi

    info "Installing Node.js..."

    if [ "$OS" = "macos" ]; then
        if command -v brew &> /dev/null; then
            brew install node
        else
            warn "Homebrew not found. Installing Node.js via nvm..."
            install_nvm
        fi
    elif [ "$OS" = "linux" ]; then
        # Try to use package manager
        if command -v apt-get &> /dev/null; then
            sudo apt-get update
            sudo apt-get install -y nodejs npm
        elif command -v dnf &> /dev/null; then
            sudo dnf install -y nodejs npm
        elif command -v pacman &> /dev/null; then
            sudo pacman -S --noconfirm nodejs npm
        else
            install_nvm
        fi
    fi

    if command -v node &> /dev/null; then
        success "Node.js installed successfully"
    else
        warn "Node.js installation may have failed. AI tools may not work."
    fi
}

install_nvm() {
    info "Installing nvm (Node Version Manager)..."

    if [ "$DOWNLOADER" = "curl" ]; then
        curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.39.0/install.sh | bash
    else
        wget -qO- https://raw.githubusercontent.com/nvm-sh/nvm/v0.39.0/install.sh | bash
    fi

    export NVM_DIR="$HOME/.nvm"
    [ -s "$NVM_DIR/nvm.sh" ] && \. "$NVM_DIR/nvm.sh"

    nvm install --lts
    nvm use --lts
}

clone_and_build() {
    info "Creating installation directory..."
    mkdir -p "$INSTALL_DIR"
    mkdir -p "$BIN_DIR"

    if [ -d "$INSTALL_DIR/polyglot-ai" ]; then
        info "Updating existing installation..."
        cd "$INSTALL_DIR/polyglot-ai"
        git pull
    else
        info "Cloning repository..."
        cd "$INSTALL_DIR"
        git clone "$REPO_URL" polyglot-ai
        cd polyglot-ai
    fi

    if [ "$VERSION" != "latest" ]; then
        info "Checking out version: $VERSION"
        git checkout "$VERSION"
    fi

    info "Building polyglot-local (this may take a few minutes)..."
    cargo build --release -p polyglot-local

    # Copy binary to bin directory
    info "Installing binary..."
    cp target/release/polyglot-local "$BIN_DIR/"
    chmod +x "$BIN_DIR/polyglot-local"

    success "polyglot-local installed to $BIN_DIR/polyglot-local"
}

install_ai_tools() {
    if [ "$INSTALL_TOOLS" != true ]; then
        return
    fi

    info "Installing AI CLI tools..."

    # Check for npm
    if ! command -v npm &> /dev/null; then
        warn "npm not found. Skipping AI tool installation."
        return
    fi

    # Install Claude Code
    info "Installing Claude Code CLI..."
    npm install -g @anthropic-ai/claude-code 2>/dev/null || warn "Claude Code installation failed"

    # Install Gemini CLI
    info "Installing Gemini CLI..."
    npm install -g @google/gemini-cli 2>/dev/null || warn "Gemini CLI installation failed"

    # Install Codex CLI
    info "Installing Codex CLI..."
    npm install -g @openai/codex-cli 2>/dev/null || warn "Codex CLI installation failed"

    # Install GitHub Copilot CLI
    if command -v gh &> /dev/null; then
        info "Installing GitHub Copilot extension..."
        gh extension install github/gh-copilot 2>/dev/null || warn "Copilot installation failed"
    else
        warn "GitHub CLI (gh) not found. Skipping Copilot installation."
    fi

    success "AI tools installation completed"
}

setup_path() {
    info "Setting up PATH..."

    # Detect shell
    SHELL_NAME=$(basename "$SHELL")
    SHELL_RC=""

    case "$SHELL_NAME" in
        bash)
            if [ -f "$HOME/.bashrc" ]; then
                SHELL_RC="$HOME/.bashrc"
            elif [ -f "$HOME/.bash_profile" ]; then
                SHELL_RC="$HOME/.bash_profile"
            fi
            ;;
        zsh)
            SHELL_RC="$HOME/.zshrc"
            ;;
        fish)
            SHELL_RC="$HOME/.config/fish/config.fish"
            ;;
    esac

    if [ -n "$SHELL_RC" ]; then
        # Check if already in PATH
        if ! grep -q "POLYGLOT_AI" "$SHELL_RC" 2>/dev/null; then
            echo "" >> "$SHELL_RC"
            echo "# Polyglot-AI" >> "$SHELL_RC"
            echo "export PATH=\"\$PATH:$BIN_DIR\"" >> "$SHELL_RC"
            success "Added $BIN_DIR to PATH in $SHELL_RC"
        else
            success "PATH already configured"
        fi
    else
        warn "Could not detect shell configuration file"
        warn "Please add $BIN_DIR to your PATH manually"
    fi
}

verify_installation() {
    info "Verifying installation..."

    if [ -x "$BIN_DIR/polyglot-local" ]; then
        success "polyglot-local binary is installed"

        # Run doctor check
        echo ""
        info "Running system check..."
        "$BIN_DIR/polyglot-local" doctor || true
    else
        error "Installation verification failed"
    fi
}

print_success() {
    echo ""
    echo -e "${GREEN}============================================${NC}"
    echo -e "${GREEN}   Installation Complete!${NC}"
    echo -e "${GREEN}============================================${NC}"
    echo ""
    echo "To get started:"
    echo ""
    echo -e "  ${CYAN}# Reload your shell or run:${NC}"
    echo -e "  source ~/.bashrc  # or ~/.zshrc"
    echo ""
    echo -e "  ${CYAN}# Start Polyglot-AI:${NC}"
    echo -e "  polyglot-local"
    echo ""
    echo -e "  ${CYAN}# Or send a single prompt:${NC}"
    echo -e "  polyglot-local ask \"Write hello world in Python\""
    echo ""
    echo -e "  ${CYAN}# Check available tools:${NC}"
    echo -e "  polyglot-local doctor"
    echo ""

    if [ "$INSTALL_TOOLS" != true ]; then
        echo -e "${YELLOW}Note: AI tools were not installed.${NC}"
        echo "To install them, run:"
        echo "  $INSTALL_DIR/polyglot-ai/scripts/install-tools.sh"
        echo ""
    fi

    echo "For more information, visit:"
    echo "  https://github.com/tugcantopaloglu/selfhosted-ai-code-platform"
    echo ""
}

# Main installation
main() {
    print_banner
    parse_args "$@"
    detect_os
    check_dependencies
    install_rust

    if [ "$INSTALL_TOOLS" = true ]; then
        install_nodejs
    fi

    clone_and_build
    install_ai_tools
    setup_path
    verify_installation
    print_success
}

main "$@"
