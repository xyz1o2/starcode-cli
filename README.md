# StarCode CLI

<p align="center">
  <img src="https://img.shields.io/badge/version-0.3.0-blue" alt="version">
  <img src="https://img.shields.io/badge/rust-2021-orange" alt="rust edition">
  <img src="https://img.shields.io/badge/license-MIT-green" alt="license">
  <img src="https://img.shields.io/badge/platform-cross--platform-lightgrey" alt="platform">
</p>

<p align="center">
  <strong>🚀 A powerful conversational AI CLI tool with text editor capabilities, built in Rust</strong>
</p>

<p align="center">
  <a href="#features">Features</a> •
  <a href="#installation">Installation</a> •
  <a href="#quick-start">Quick Start</a> •
  <a href="#configuration">Configuration</a> •
  <a href="#commands">Commands</a> •
  <a href="#contributing">Contributing</a> •
  <a href="#license">License</a>
</p>

---

## ✨ Features

- **🤖 Multi-Provider AI Support** - Connect to OpenAI, Anthropic, and other OpenAI-compatible APIs
- **📝 Interactive TUI** - Beautiful terminal user interface with real-time streaming
- **🔧 Tool Integration** - Execute commands, edit files, and interact with your codebase
- **🌐 MCP Protocol** - Model Context Protocol support for extensible toolchains
- **📁 Git Integration** - AI-assisted git operations with intelligent suggestions
- **🔍 Smart Search** - Fast code search using ripgrep with AST-aware chunking
- **🎨 Syntax Highlighting** - Beautiful code highlighting with multiple themes
- **📊 Session Management** - Save, resume, and manage conversation sessions
- **🌍 Internationalization** - Multi-language support (English, Chinese)
- **🔒 Permission System** - Fine-grained control over tool execution permissions
- **⚡ Headless Mode** - Process prompts without interactive UI for scripting
- **📦 Cross-Platform** - Works on Linux, macOS, and Windows

## 🎯 What Can It Do?

StarCode CLI is your AI-powered coding assistant that can:

- **Read and understand** your entire codebase
- **Edit files** with intelligent suggestions and diff previews
- **Execute commands** in a sandboxed environment
- **Search code** using regex, glob patterns, or AST-aware search
- **Manage Git** operations with AI assistance
- **Work with MCP servers** for extensible functionality
- **Resume sessions** to continue where you left off
- **Process prompts** in headless mode for automation

## 📦 Installation

### From Source (Recommended)

```bash
# Clone the repository
git clone https://github.com/xyz1o2/starcode-cli.git
cd starcode-cli

# Build the project
cargo build --release

# The binary will be in target/release/starcode-cli
cp target/release/starcode-cli /usr/local/bin/
```

### Using Cargo

```bash
# Install directly from the repository
cargo install --git https://github.com/xyz1o2/starcode-cli.git starcode-cli
```

### Pre-built Binaries

Download the latest release for your platform from the [Releases](https://github.com/xyz1o2/starcode-cli/releases) page.

## 🚀 Quick Start

### 1. Set up your API key

```bash
# Option 1: Environment variable
export STAR_API_KEY="your-api-key"

# Option 2: Create a .env file
echo "STAR_API_KEY=your-api-key" > .env
```

### 2. Start the interactive session

```bash
# Start interactive mode
starcode

# Or with an initial message
starcode "Explain the structure of this project"
```

### 3. Use headless mode for scripting

```bash
# Process a single prompt
starcode -p "What files are in the current directory?"

# With specific output format
starcode -p "List all Rust files" --output-format text
```

## ⚙️ Configuration

### Environment Variables

```bash
# API Configuration
STAR_API_KEY=your-api-key          # Required: Your API key
STAR_BASE_URL=https://api.openai.com/v1  # Optional: Custom API base URL

# Model Configuration
STAR_MODEL=gpt-4                   # Optional: Default model to use
```

### Configuration File

Create `~/.star/user-settings.json`:

```json
{
  "apiKey": "your-api-key",
  "model": "gpt-4",
  "baseUrl": "https://api.openai.com/v1",
  "temperature": 0.2,
  "maxTokens": 8192
}
```

### Project Configuration

StarCode looks for `STAR.md` in your project root for project-specific instructions:

```markdown
# Project: My Awesome Project

## Build Commands
- `cargo build` - Build the project
- `cargo test` - Run tests

## Code Style
- Use snake_case for variables
- Add doc comments for public functions

## Architecture
- Main entry point: src/main.rs
- Core logic: src/core/
```

## 📋 Commands

### Interactive Mode

```bash
# Start interactive session
starcode

# With working directory
starcode -d /path/to/project

# Resume a session
starcode --resume
starcode --resume <session-id>

# Skip permission prompts (use with caution!)
starcode --dangerously-skip-permissions
```

### Headless Mode

```bash
# Process a prompt and exit
starcode -p "Your prompt here"

# With specific output format
starcode -p "Your prompt" --output-format jsonl
starcode -p "Your prompt" --output-format text
```

### Subcommands

```bash
# Initialize a new project with STAR.md
starcode init

# MCP server management
starcode mcp add <server-name> <command>
starcode mcp remove <server-name>
starcode mcp list

# Git operations with AI assistance
starcode git commit
starcode git diff
starcode git status
```

### Permission Modes

```bash
# Default mode - asks for permission
starcode --permission-mode default

# Plan mode - read-only operations only
starcode --permission-mode plan

# YOLO mode - bypass all permissions (dangerous!)
starcode --permission-mode yolo
```

## 🔧 Available Tools

StarCode CLI comes with a comprehensive set of built-in tools:

- **Read** - Read file contents with line numbers
- **Write** - Create or overwrite files
- **Edit** - Make precise file edits
- **Bash** - Execute shell commands
- **Grep** - Search files using ripgrep
- **Glob** - Find files by pattern
- **Agent** - Spawn sub-agents for complex tasks
- **WebFetch** - Fetch and analyze web pages

## 🌐 MCP Support

StarCode supports the Model Context Protocol (Model Context Protocol) for extending its capabilities:

```bash
# Add an MCP server
starcode mcp add filesystem "npx -y @modelcontextprotocol/server-filesystem /path/to/directory"

# List configured servers
starcode mcp list

# Remove a server
starcode mcp remove filesystem
```

## 🧪 Development

### Building from Source

```bash
# Clone the repository
git clone https://github.com/xyz1o2/starcode-cli.git
cd starcode-cli

# Debug build
cargo build

# Release build
cargo build --release

# Run tests
cargo test

# Run with logging
RUST_LOG=debug cargo run
```

### Project Structure

```
starcode/
├── starcode-cli/
│   ├── src/
│   │   ├── main.rs          # Entry point
│   │   ├── agent/           # AI agent logic
│   │   ├── commands/        # CLI commands
│   │   ├── core/            # Core functionality
│   │   ├── llm/             # LLM integration
│   │   ├── runtime/         # Runtime environment
│   │   ├── tools/           # Built-in tools
│   │   ├── types/           # Type definitions
│   │   ├── ui/              # Terminal UI
│   │   └── utils/           # Utilities
│   ├── eval/                # Evaluation harness
│   ├── i18n/                # Internationalization
│   └── Cargo.toml           # Rust dependencies
├── README.md
└── LICENSE
```

### Contributing

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

## 📊 Evaluation

StarCode includes a built-in evaluation harness for testing AI capabilities:

```bash
# Run the evaluation suite
starcode eval --tasks eval/tasks.json

# Run with multiple trials
starcode eval --tasks eval/tasks.json --trials 3

# Generate markdown report
starcode eval --tasks eval/tasks.json --report-md eval-report.md

# Compare against baseline
starcode eval --baseline .star/eval-baseline.json
```

## 🐛 Troubleshooting

### Common Issues

**API Key not found**
```bash
# Check if environment variable is set
echo $STAR_API_KEY

# Or verify the config file exists
cat ~/.star/user-settings.json
```

**Build fails**
```bash
# Make sure you have Rust installed
rustc --version

# Update Rust toolchain
rustup update

# Clean and rebuild
cargo clean && cargo build --release
```

**Permission denied**
```bash
# Make sure the binary is executable
chmod +x target/release/starcode-cli

# Or install to a directory in your PATH
sudo cp target/release/starcode-cli /usr/local/bin/
```

## 📜 Changelog

### v0.3.0 (Latest)
- Added MCP (Model Context Protocol) support
- Improved session management
- Added Git integration commands
- Performance optimizations
- Bug fixes and stability improvements

### v0.2.0
- Added headless mode
- Multi-provider support
- Internationalization (i18n)
- Permission system

### v0.1.0
- Initial release
- Interactive TUI
- Basic tool integration
- File operations

## 🤝 Support

- 📖 [Documentation](https://github.com/xyz1o2/starcode-cli/wiki)
- 🐛 [Report Issues](https://github.com/xyz1o2/starcode-cli/issues)
- 💬 [Discussions](https://github.com/xyz1o2/starcode-cli/discussions)

## 🙏 Acknowledgments

- Built with [Rust](https://www.rust-lang.org/) 🦀
- Terminal UI powered by [Ratatui](https://github.com/fdehau/tui-rs)
- LLM integration via [Rig](https://github.com/0xPlaygrounds/rig)
- MCP support following [Model Context Protocol](https://modelcontextprotocol.io/)

---

<p align="center">
  Made with ❤️ by <a href="https://github.com/xyz1o2">xyz1o2</a>
</p>

<p align="center">
  <a href="https://github.com/xyz1o2/starcode-cli/stargazers">
    <img src="https://img.shields.io/github/stars/xyz1o2/starcode-cli?style=social" alt="Stars">
  </a>
  <a href="https://github.com/xyz1o2/starcode-cli/network/members">
    <img src="https://img.shields.io/github/forks/xyz1o2/starcode-cli?style=social" alt="Forks">
  </a>
</p>
