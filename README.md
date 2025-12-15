# Polyglot-AI - multiLLM "vibing" platform

Is developers done? or we are not coding anymore? Who cares. Vibe coding, hard coding (idk what that means but ok) or something else. As a developer and system engineer I always like to use new tools and experiences. So on every one or two weeks I am seeing a new model or new provider making better than previous ones. So i thought it's time to create a multi-platform fast adaptable AI coding tool. First I thought about VSCode extension but I don't like extensions or trust them. So for now this is the beautiful (arguably) solution with TUI. If you don't want to use any cloud LLM for now you can add your models with ollama. Soon I will be adding my solution to implement these local modals.

Btw it's written in rust because why not.

Feel free to open issues and pull requests for more feautres. I will be checking them every saturday.

So shortly a self-hosted platform that aggregates multiple AI coding assistants into a single interface. Query Claude, Gemini, Codex, GitHub Copilot, Cursor, Perplexity, and Ollama through one unified CLI with automatic failover, rate limit handling, and context preservation across tools.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Architecture](#architecture)
- [Installation](#installation)
  - [From Source](#from-source)
  - [From Releases](#from-releases)
  - [Docker](#docker)
- [Quick Start](#quick-start)
- [Configuration](#configuration)
- [Usage](#usage)
  - [Local Mode](#local-mode)
  - [Client-Server Mode](#client-server-mode)
  - [Multi-Model Mode](#multi-model-mode)
- [Supported Tools](#supported-tools)
- [Certificate Setup](#certificate-setup)
- [Troubleshooting](#troubleshooting)
- [License](#license)

## Overview

Polyglot-AI provides two modes of operation:

**Local Mode (`polyglot-local`)**: Standalone binary that runs AI tools directly on your machine. No server required. Best for individual developers.

**Client-Server Mode (`polyglot` + `polyglot-server`)**: Centralized server deployment where multiple clients connect over QUIC with mTLS authentication. Supports file synchronization, user management, and shared tool access. Best for teams or remote development.

Both modes share the same features: automatic tool rotation on rate limits, unified chat history, context transfer between sessions, and a terminal UI.

## Features

- **Multi-Tool Support**: Claude Code, Gemini CLI, OpenAI Codex, GitHub Copilot CLI, Cursor Agent, Perplexity, and Ollama
- **Automatic Failover**: When one tool hits a rate limit, automatically switches to the next available tool
- **Context Preservation**: Chat history and context transfer between tools and sessions
- **Multi-Model Queries**: Send the same prompt to multiple AI tools simultaneously and compare responses
- **Terminal UI**: Full-featured TUI with tabs, scrolling, and keyboard shortcuts
- **Simple CLI Mode**: Optional plain CLI mode without the TUI
- **Session History**: Searchable chat history with session resume
- **Plugin System**: Add custom AI tools via CLI commands, scripts, or HTTP APIs
- **File Sync**: Real-time or on-demand file synchronization in client-server mode
- **mTLS Security**: Mutual TLS authentication for client-server communication
- **Self-Updating**: Built-in update checker and installer

## Architecture

```
polyglot-ai/
├── crates/
│   ├── common/     # Shared types, protocol definitions, utilities
│   ├── local/      # Standalone local binary (polyglot-local)
│   ├── client/     # Client binary for server mode (polyglot)
│   └── server/     # Server binary (polyglot-server)
├── config/         # Example configuration files
├── docker/         # Dockerfiles for containerized deployment
└── scripts/        # Installation and setup scripts
```

Communication between client and server uses QUIC (UDP-based) with MessagePack serialization for efficiency.

## Installation

### Requirements

- Rust 1.75 or later (for building from source)
- One or more AI CLI tools installed (see [Supported Tools](#supported-tools))
- OpenSSL (for certificate generation)

### From Source

```bash
git clone https://github.com/tugcantopaloglu/polyglot-ai.git
cd polyglot-ai

# Build all binaries
cargo build --release

# Binaries are in target/release/
# - polyglot-local (standalone)
# - polyglot (client)
# - polyglot-server (server)
```

### From Releases

Download pre-built binaries from the [Releases](https://github.com/tugcantopaloglu/polyglot-ai/releases) page.

Available binaries:

- `polyglot-local-linux-amd64`
- `polyglot-local-linux-arm64`
- `polyglot-local-darwin-amd64`
- `polyglot-local-darwin-arm64`
- `polyglot-local-windows-amd64.exe`

### Docker

```bash
# Server
docker build -f docker/Dockerfile.server -t polyglot-server .
docker run -p 4433:4433/udp -v ./certs:/app/certs -v ./data:/app/data polyglot-server

# Client
docker build -f docker/Dockerfile.client -t polyglot-client .
docker run -it -v ./certs:/app/certs -v ./workspace:/app/workspace polyglot-client
```

Or use Docker Compose:

```bash
cd docker
docker-compose up -d
```

## Quick Start

### Local Mode (Recommended for Individual Use)

```bash
# Check which AI tools are available on your system
polyglot-local doctor

# Start the interactive TUI
polyglot-local

# Or run without TUI
polyglot-local --no-tui

# One-shot query
polyglot-local ask "explain this code" --tool claude
```

### Client-Server Mode

```bash
# 1. Generate certificates
./scripts/generate-certs.sh ./certs

# 2. Start the server
polyglot-server start -c config/server.toml

# 3. Connect with client
polyglot connect -c config/client.toml
```

## Configuration

### Local Mode Configuration

Default location: `~/.config/polyglot-ai/local.toml`

Generate a config file:

```bash
polyglot-local init
```

Example configuration:

```toml
[tools]
default_tool = "claude"
rotation_strategy = "on_limit"  # or "round_robin", "priority"
switch_delay = 3

[tools.claude]
enabled = true
path = "claude"
args = []

[tools.gemini]
enabled = true
path = "gemini"
args = []

[tools.copilot]
enabled = true
path = "gh"
args = ["copilot"]

[tools.ollama]
enabled = true
path = "ollama"
args = ["run", "codellama"]

[ui]
tui_enabled = true
show_timestamps = true
theme = "default"
```

### Server Configuration

See `config/server.example.toml` for all options.

Key settings:

```toml
[server]
bind_address = "0.0.0.0:4433"
max_connections = 100

[auth]
mode = "single_user"  # or "multi_user"
cert_path = "./certs/server.crt"
key_path = "./certs/server.key"
ca_path = "./certs/ca.crt"

[tools]
rotation_strategy = "on_limit"
default_tool = "claude"
```

### Client Configuration

See `config/client.example.toml` for all options.

```toml
[connection]
server_address = "localhost:4433"
cert_path = "./certs/client.crt"
key_path = "./certs/client.key"
ca_path = "./certs/ca.crt"

[sync]
default_mode = "on_demand"
ignore_patterns = [".git", "node_modules", "target"]
```

## Usage

### Local Mode

**Keyboard Shortcuts:**

| Key         | Action                           |
| ----------- | -------------------------------- |
| F1          | Chat view                        |
| F2          | Tools view                       |
| F3          | Usage statistics                 |
| F4          | Chat history                     |
| F5          | Help                             |
| F6          | Multi-model selection            |
| F7          | About                            |
| Ctrl+M      | Toggle multi-model mode          |
| Ctrl+N      | New chat (with context transfer) |
| Ctrl+Q      | Quit                             |
| PageUp/Down | Scroll                           |

**Commands:**

```
/tools          List available tools
/switch <tool>  Switch to a specific tool (claude, gemini, codex, copilot, cursor, ollama)
/usage          Show usage statistics
/history        Show chat history
/search <query> Search chat history
/new            Start new chat with context transfer
/multi          Open multi-model selection
/single         Return to single-tool mode
/clear          Clear chat output
/update         Check for updates
/help           Show help
/quit           Exit
```

### Client-Server Mode

Same keyboard shortcuts and commands as local mode, plus:

```
/sync [path]    Sync files with server
```

### Multi-Model Mode

Query multiple AI tools simultaneously:

1. Press F6 or type `/multi`
2. Select tools with number keys (1-7)
3. Press Enter to confirm
4. Send your prompt - all selected tools respond in parallel
5. Responses display side-by-side
6. Type `/single` to return to normal mode

Or specify tools directly:

```
/multi claude gemini codex
```

## Supported Tools

| Tool           | Command        | Installation                               |
| -------------- | -------------- | ------------------------------------------ |
| Claude Code    | `claude`       | `npm install -g @anthropic-ai/claude-code` |
| Gemini CLI     | `gemini`       | See Google Cloud documentation             |
| OpenAI Codex   | `codex`        | `npm install -g @openai/codex-cli`         |
| GitHub Copilot | `gh copilot`   | `gh extension install github/gh-copilot`   |
| Cursor Agent   | `cursor-agent` | Included with Cursor IDE                   |
| Perplexity     | `pplx`         | See Perplexity documentation               |
| Ollama         | `ollama`       | https://ollama.ai                          |

Run the installation script to install available tools:

```bash
# Linux/macOS
./scripts/install-tools.sh

# Windows (PowerShell)
.\scripts\install-tools.ps1
```

Verify installed tools:

```bash
polyglot-local doctor
```

## Certificate Setup

Client-server mode requires mTLS certificates.

### Using the Script

```bash
./scripts/generate-certs.sh ./certs
```

This generates:

- `ca.crt` / `ca.key` - Certificate Authority
- `server.crt` / `server.key` - Server certificate
- `client.crt` / `client.key` - Client certificate

### Using the CLI

```bash
# Generate CA and server certs
polyglot-server generate-certs -o ./certs

# Generate additional client certs
polyglot generate-certs -o ./client-certs --cn "client-2" --ca-cert ./certs/ca.crt --ca-key ./certs/ca.key
```

### Manual Setup with OpenSSL

See `scripts/generate-certs.sh` for the exact OpenSSL commands.

## Troubleshooting

### Double characters when typing (Windows)

This was a known issue with terminal echo on Windows. Update to the latest version which includes a fix for Windows console mode handling.

### Tool not found

1. Check if the tool is installed: `which claude` (or the tool name)
2. Run `polyglot-local doctor` to see tool status
3. Verify the path in your config matches the actual binary location

### Connection refused (client-server mode)

1. Verify the server is running: `polyglot-server start`
2. Check firewall allows UDP port 4433
3. Verify certificates are correctly configured
4. Check the CA certificate matches between client and server

### Rate limit errors

Polyglot-AI automatically handles rate limits by switching tools. If all tools are rate limited:

1. Wait a few minutes for limits to reset
2. Add more tools to your configuration
3. Consider using Ollama for unlimited local inference

### Build errors

```bash
# Clean build
cargo clean
cargo build --release

# Update dependencies
cargo update
```

## Plugin System

Add custom AI tools without modifying code. Add to your config file:

```toml
# CLI tool
[[plugins]]
name = "my-tool"
display_name = "My Custom Tool"
plugin_type = "cli"
enabled = true
command = "/path/to/my-tool"
args = ["--prompt", "{prompt}"]
timeout = 120

# HTTP API
[[plugins]]
name = "my-api"
display_name = "My API"
plugin_type = "http"
enabled = true
command = "https://api.example.com/chat"
http_method = "POST"
headers = { "Authorization" = "Bearer TOKEN" }
body_template = '{"prompt": "{prompt}"}'
timeout = 60
```

## Updates

Check for updates:

```bash
polyglot-local update --check-only
```

Install updates:

```bash
polyglot-local update
```

## License

MIT License. See [LICENSE](LICENSE) for details.

## Author

Tugcan Topaloglu ([@tugcantopaloglu](https://github.com/tugcantopaloglu))

## Contributing

Issues and pull requests welcome at https://github.com/tugcantopaloglu/polyglot-ai
