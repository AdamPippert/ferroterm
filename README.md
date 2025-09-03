# Ferroterm

A cross-platform, GPU-accelerated terminal emulator with native AI agent integration, built in Rust for power users who demand speed and efficiency.

## Mission

Ferroterm addresses the gap in the terminal ecosystem where power users want ultra-fast, keyboard-first terminals with embedded AI assistance, without the latency, web-UI overhead, or cloud lock-in of current solutions. While Warp is proprietary and macOS-only, and open-source alternatives lack integrated agent hosting with sub-150ms cold-start times, Ferroterm delivers a cross-platform solution that combines performance with intelligence.

## Key Features

- **One-Keystroke AI**: Type `p` at the beginning of any line to turn the rest into an LLM prompt
- **Ultra-Fast Performance**: Sub-50ms time-to-first-frame, sub-300ms AI response times
- **GPU Acceleration**: wgpu-powered rendering with 144 FPS target
- **POSIX Compliant**: 100% compatibility with all shells and TUI applications
- **Local-First AI**: Support for local GGUF/MLC/vLLM models with remote fallback
- **Plain-Text Configuration**: Everything configurable via `ferroterm.toml`
- **Cross-Platform**: Native support for macOS and Linux

## Architecture

Ferroterm is built with a modular architecture consisting of:

### Core Runtime
- **TTY Engine**: Async PTY management with zero-copy I/O buffers
- **GPU Renderer**: Hardware-accelerated text rendering with ligature support
- **Input/Keymap**: Configurable keybindings that preserve native shell behavior

### Agent Subsystem
- **Command Parser**: Detects `p` prefix and routes to AI models
- **Model Host**: Manages local and remote AI models with warm pools
- **Streaming UI**: Real-time response display with markdown support

### Configuration & State
- **Plain-Text Config**: Live-reloading TOML configuration
- **Profile Cache**: Per-model settings with optional encryption

### Extensibility
- **Plugin SDK**: Rust/WASM plugin system with sandboxing

## Performance Targets

- Time-to-first-frame: ≤ 50ms
- AI response latency: ≤ 300ms (local 7B model, 4-bit quantized)
- Round-trip echo: < 1ms under load
- GPU rendering: 144 FPS target, < 5% CPU at idle

## Quick Start

### Prerequisites

- Rust 1.70+ with Cargo
- GPU drivers (for hardware acceleration)
- macOS 11+ or Linux with kernel 5.4+

### Installation

```bash
# From source
git clone https://github.com/AdamPippert/Ferroterm.git
cd Ferroterm
cargo build --release

# Add to PATH (choose one option)
# Option 1: Copy to system PATH
sudo cp target/release/ferroterm /usr/local/bin/

# Option 2: Add to your shell profile (recommended for development)
echo 'export PATH="$PATH:'$(pwd)'/target/release"' >> ~/.bashrc  # For bash
echo 'export PATH="$PATH:'$(pwd)'/target/release"' >> ~/.zshrc   # For zsh
source ~/.bashrc  # or source ~/.zshrc

# Verify installation
ferroterm --version

# Run examples
cargo run --example markdown_demo
```

### Configuration

Ferroterm creates a default configuration at `~/.config/ferroterm/ferroterm.toml`:

```toml
[ui]
font_size = 12
font_family = "JetBrains Mono"
theme = "dark"

[keymap]
prefix = "f"  # AI command prefix

[agent]
default_model = "mistral-7b-instruct"
temperature = 0.7

[models]
cache_dir = "~/.cache/ferroterm/models"

[[models.models]]
name = "mistral-7b-instruct"
path = "~/.cache/ferroterm/models/mistral-7b-instruct.gguf"
quantization = "q4_0"
```

## Usage

### Basic Terminal Usage

Ferroterm works like any standard terminal emulator. All your favorite shells (bash, zsh, fish) and TUI applications work without modification.

### AI Integration

Simply type `p` at the beginning of any line to activate the AI agent:

```bash
p how do I find large files in this directory?
p --model gpt-4 explain this error message
p what does this command do: find . -name "*.rs" -exec grep -l "async" {} \;
```

The AI agent has access to:
- Current terminal context
- Scrollback history (configurable)
- Environment variables
- Working directory

## Development Status

Ferroterm is currently in active development. Completed components:

- [x] Plain-Text Configuration System
- [x] TTY Engine with PTY management
- [ ] GPU Renderer (wgpu-based)
- [ ] Input/Keymap System
- [ ] Agent Subsystem
- [ ] Plugin SDK
- [ ] Distribution packages

## Testing

```bash
# Run all tests
cargo test

# Run specific component tests
cargo test config::tests
cargo test tty::tests

# Run benchmarks
cargo run --bin config_bench
cargo run --bin tty_bench
```

## Contributing

Ferroterm is built with security, performance, and reliability as top priorities. All contributions should maintain:

- Memory safety (no unsafe code without justification)
- Performance requirements (see benchmarks)
- Comprehensive test coverage
- Clear documentation

## License

[License to be determined]

## Roadmap

### Phase 1: Core Foundation (Weeks 1-6)
- [x] TTY Engine
- [x] Configuration System
- [ ] GPU Renderer
- [ ] Input/Keymap System

### Phase 2: AI Integration (Weeks 7-10)
- [ ] Command Prefix Parser
- [ ] Model Host
- [ ] Streaming UI
- [ ] Profile Cache

### Phase 3: Distribution & Polish (Weeks 11-14)
- [ ] Plugin SDK
- [ ] Installer packages
- [ ] Security hardening
- [ ] Performance optimization

## Support

For questions, issues, or contributions, please visit our [GitHub repository](https://github.com/AdamPippert/Ferroterm).