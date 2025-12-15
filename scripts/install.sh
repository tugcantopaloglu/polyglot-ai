#!/usr/bin/env bash
set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

REPO="tugcantopaloglu/polyglot-ai"
INSTALL_DIR="${POLYGLOT_INSTALL_DIR:-$HOME/.polyglot-ai}"
BIN_DIR="${POLYGLOT_BIN_DIR:-$HOME/.local/bin}"
VERSION="${POLYGLOT_VERSION:-latest}"
FORCE="${POLYGLOT_FORCE:-0}"
INSTALL_TOOLS="${POLYGLOT_WITH_TOOLS:-0}"

print_banner() {
    echo -e "${CYAN}"
    echo "  ____       _             _       _        _    ___ "
    echo " |  _ \ ___ | |_   _  __ _| | ___ | |_     / \  |_ _|"
    echo " | |_) / _ \| | | | |/ _\` | |/ _ \| __|   / _ \  | | "
    echo " |  __/ (_) | | |_| | (_| | | (_) | |_   / ___ \ | | "
    echo " |_|   \___/|_|\__, |\__, |_|\___/ \__| /_/   \_\___|"
    echo "               |___/ |___/                           "
    echo -e "${NC}"
    echo -e "${GREEN}Polyglot-AI one-line installer (Linux/macOS)${NC}"
    echo ""
}

info()    { echo -e "${BLUE}[INFO]${NC} $1"; }
success() { echo -e "${GREEN}[OK]${NC} $1"; }
warn()    { echo -e "${YELLOW}[WARN]${NC} $1"; }
error()   { echo -e "${RED}[ERROR]${NC} $1"; exit 1; }

detect_os_arch() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Linux*)  OS="linux" ;;
        Darwin*) OS="macos" ;;
        *)       error "Unsupported operating system: $OS" ;;
    esac

    case "$ARCH" in
        x86_64|amd64) ARCH="x86_64" ;;
        aarch64|arm64) ARCH="aarch64" ;;
        *) error "Unsupported architecture: $ARCH" ;;
    esac

    case "$OS:$ARCH" in
        linux:x86_64)  TARGET_TRIPLE="x86_64-unknown-linux-gnu" ;;
        linux:aarch64) TARGET_TRIPLE="aarch64-unknown-linux-gnu" ;;
        macos:x86_64)  TARGET_TRIPLE="x86_64-apple-darwin" ;;
        macos:aarch64) TARGET_TRIPLE="aarch64-apple-darwin" ;;
        *) error "Unsupported platform combo: $OS/$ARCH" ;;
    esac

    EXT=""
    success "Detected platform: $OS ($ARCH) -> $TARGET_TRIPLE"
}

choose_downloader() {
    if command -v curl >/dev/null 2>&1; then
        DOWNLOADER="curl"
    elif command -v wget >/dev/null 2>&1; then
        DOWNLOADER="wget"
    else
        error "Need curl or wget to download releases"
    fi
}

fetch() {
    local url="$1"
    if [ "$DOWNLOADER" = "curl" ]; then
        curl -fsSL "$url"
    else
        wget -qO- "$url"
    fi
}

fetch_latest_tag() {
    if [ "$VERSION" != "latest" ]; then
        TAG="$VERSION"
        return
    fi

    local url="https://api.github.com/repos/$REPO/releases/latest"
    info "Fetching latest release tag..."
    local json
    json=$(fetch "$url") || error "Failed to query GitHub API"
    TAG=$(printf "%s" "$json" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^" ]*\)".*/\1/p' | head -n1)
    [ -n "$TAG" ] || error "Could not determine latest release tag; set POLYGLOT_VERSION"
    success "Latest release: $TAG"
}

installed_version() {
    local bin="$1"
    [ -x "$bin" ] || return 1
    local out version
    out=$("$bin" --version 2>/dev/null) || true
    version=$(printf "%s" "$out" | sed -n 's/.*\([0-9]\+\.[0-9]\+\.[0-9]\+\).*/\1/p' | head -n1)
    [ -n "$version" ] || return 1
    printf "%s" "$version"
}

download_binary() {
    local name="$1"
    local asset="${name}-${TARGET_TRIPLE}${EXT}"
    local url="https://github.com/$REPO/releases/download/$TAG/$asset"
    local tmp
    tmp="$(mktemp)"

    info "Downloading $asset ($TAG)..."
    if [ "$DOWNLOADER" = "curl" ]; then
        curl -fsSL "$url" -o "$tmp" || error "Download failed: $asset"
    else
        wget -qO "$tmp" "$url" || error "Download failed: $asset"
    fi

    mkdir -p "$BIN_DIR"
    install -m 755 "$tmp" "$BIN_DIR/$name"
    rm -f "$tmp"
    success "$name installed to $BIN_DIR/$name"
}

ensure_path() {
    # Best-effort PATH update for bash/zsh
    local shell_rc=""
    if [ -n "${SHELL:-}" ]; then
        case "$(basename "$SHELL")" in
            bash)
                if [ -f "$HOME/.bashrc" ]; then shell_rc="$HOME/.bashrc"; fi
                ;;
            zsh)
                if [ -f "$HOME/.zshrc" ]; then shell_rc="$HOME/.zshrc"; fi
                ;;
        esac
    fi

    if [ -n "$shell_rc" ] && ! grep -q "polyglot-ai" "$shell_rc" 2>/dev/null; then
        {
            echo ""
            echo "# Polyglot-AI"
            echo "export PATH=\"$BIN_DIR:$PATH\""
        } >> "$shell_rc"
        success "Added $BIN_DIR to PATH in $shell_rc"
    elif echo "$PATH" | tr ':' '\n' | grep -Fxq "$BIN_DIR"; then
        success "PATH already contains $BIN_DIR"
    else
        warn "Could not update shell rc; ensure $BIN_DIR is on your PATH"
    fi
}

install_ai_tools() {
    if [ "$INSTALL_TOOLS" != "1" ]; then
        return
    fi

    info "Installing AI CLI tools (npm + gh required)..."
    if ! command -v npm >/dev/null 2>&1; then
        warn "npm not found; skipping AI tool installation"
        return
    fi

    npm install -g @anthropic-ai/claude-code 2>/dev/null && success "Claude Code installed" || warn "Claude Code install failed"
    npm install -g @google/gemini-cli 2>/dev/null && success "Gemini CLI installed" || warn "Gemini install failed"
    npm install -g @openai/codex-cli 2>/dev/null && success "Codex CLI installed" || warn "Codex install failed"

    if command -v gh >/dev/null 2>&1; then
        gh extension install github/gh-copilot 2>/dev/null && success "GitHub Copilot installed" || warn "Copilot install failed"
    else
        warn "GitHub CLI (gh) not found; skipping Copilot"
    fi
}

verify_installation() {
    if [ -x "$BIN_DIR/polyglot-local" ]; then
        success "polyglot-local installed"
        "$BIN_DIR/polyglot-local" --version || true
    else
        error "polyglot-local not found in $BIN_DIR"
    fi
}

main() {
    print_banner
    choose_downloader
    detect_os_arch
    fetch_latest_tag

    mkdir -p "$INSTALL_DIR" "$BIN_DIR"

    desired_version="${TAG#v}"
    for name in polyglot-local polyglot polyglot-server; do
        dest="$BIN_DIR/$name"
        current=$(installed_version "$dest" || true)
        if [ "$FORCE" != "1" ] && [ -n "$current" ] && [ -n "$desired_version" ] && [ "$current" = "$desired_version" ]; then
            info "$name already at $current; skipping download"
            continue
        fi
        download_binary "$name"
    done

    ensure_path
    install_ai_tools
    verify_installation
    success "Done! Open a new shell or source your rc file to use polyglot-* commands."
}

main "$@"
