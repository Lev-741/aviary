# Aviary - AI Agent System for Colibri

## Vision
Aviary is an innovative, high-performance AI agent system written in Rust – built **exclusively for Colibri**. It leverages Colibri's unique features (MoE streaming, NVMe hierarchy, expert routing) to deliver the best possible local AI experience.

## Why Colibri-Only?
- **Specialization > Generalization:** We optimize for ONE model, not many
- **Colibri's Unique Features:** MoE streaming, 744B+ models on consumer hardware
- **First-Mover Advantage:** First agent system built specifically for Colibri
- **Performance:** Fewer abstractions, more Colibri-native optimizations

## Supported Platforms
- **Desktop App:** macOS (10.15+) and Linux (Ubuntu 22.04+, Fedora 39+)
- **CLI:** macOS and Linux
- **Bots:** Platform-agnostic (Telegram, Discord)
- **NO Windows support** (keeps complexity low)

## Tech Stack

### Backend (Rust)
- **Language:** Rust 2024 Edition
- **Async Runtime:** Tokio (full features)
- **Web Framework:** Axum (API + Web UI backend)
- **Desktop:** Tauri v2 (macOS + Linux)
- **HTTP Client:** reqwest (for Colibri API)
- **Database:** SQLx + SQLite (MVP), PostgreSQL (production)
- **CLI:** clap v4
- **Telegram:** teloxide v0.13
- **Discord:** serenity v0.12

### Frontend (Desktop UI)
- **Framework:** React 18 + Vite
- **Styling:** Tailwind CSS
- **State:** Zustand or Jotai
- **Build:** npm + Vite

### Infrastructure
- **Containerization:** Docker + docker-compose
- **CI/CD:** GitHub Actions (build for macOS + Linux)
- **Logging:** tracing + tracing-subscriber

## Architecture
aviary/
├── Cargo.toml (workspace root)
├── crates/
│ ├── core/ # Colibri-specific agent logic
│ │ ├── src/
│ │ │ ├── lib.rs
│ │ │ ├── agent.rs # ColibriAgent struct
│ │ │ ├── colibri.rs # HTTP client to Colibri API
│ │ │ ├── tools/ # Tool system (file, shell, web search)
│ │ │ └── memory/ # SQLite memory (last N messages)
│ │ └── Cargo.toml
│ │
│ ├── cli/ # Terminal CLI
│ │ ├── src/
│ │ │ └── main.rs
│ │ └── Cargo.toml
│ │
│ ├── desktop/ # Tauri desktop app
│ │ ├── src/
│ │ │ ├── lib.rs
│ │ │ └── main.rs
│ │ ├── tauri.conf.json
│ │ └── Cargo.toml
│ │
│ ├── telegram/ # Telegram bot
│ │ ├── src/
│ │ │ └── lib.rs
│ │ └── Cargo.toml
│ │
│ └── discord/ # Discord bot
│ ├── src/
│ │ └── lib.rs
│ └── Cargo.toml
│
└── apps/
└── desktop-ui/ # React frontend for Tauri
├── src/
│ ├── App.tsx
│ ├── components/
│ └── pages/
├── package.json
└── vite.config.ts

text

## Colibri-Specific Features

1. **Expert Caching:** Cache routed experts for faster follow-up requests
2. **NVMe Monitoring:** Display streaming status, NVMe utilization
3. **Model-Aware Scheduling:** Understand Colibri's MoE architecture for better prompts
4. **Multi-Model Support:** Run different Colibri models in parallel (glm-5.2, etc.)
5. **Health Checks:** Monitor Colibri server status, RAM/VRAM/NVMe

## Design Principles

1. **Colibri-First:** Every decision optimized for Colibri
2. **Performance:** Rust for maximum speed
3. **Privacy:** Everything local, no external APIs
4. **Modular:** Each crate is independently usable
5. **Open Source:** MIT License
6. **macOS + Linux Only:** No Windows support (reduces complexity)

## Coding Conventions

- `cargo fmt` before every commit
- `cargo clippy -- -D warnings` must pass
- Async code with Tokio
- Error handling with thiserror/anyhow
- Tests for all core functions
- Conventional Commits for git messages

## MVP Goals

1. **Core:** Colibri client + agent loop with tool calling
2. **CLI:** Interactive chat with Colibri in terminal
3. **Desktop:** Tauri app with chat UI + Colibri monitoring
4. **Telegram Bot:** Interact with Colibri via Telegram
5. **Discord Bot:** Interact with Colibri via Discord
6. **Docker:** One-command deploy with docker-compose

## Colibri API

- **Base URL:** `http://localhost:8000/v1`
- **Chat Endpoint:** `/chat/completions` (OpenAI-compatible)
- **Messages Endpoint:** `/messages` (Anthropic-compatible)
- **Tool Calling:** OpenAI `tools` parameter
- **Models:** `glm-5.2-colibri`, etc.

## Environment Variables

```bash
# .env.example
COLIBRI_BASE_URL=http://localhost:8000/v1
COLIBRI_MODEL=glm-5.2-colibri
TELEGRAM_BOT_TOKEN=your_telegram_bot_token
DISCORD_BOT_TOKEN=your_discord_bot_token
DATABASE_URL=sqlite://aviary.db
RUST_LOG=info
```

## Build Commands

```bash
# Build all crates
cargo build --release

# Build desktop app (macOS)
cd crates/desktop
cargo tauri build

# Build desktop app (Linux)
cd crates/desktop
cargo tauri build --target x86_64-unknown-linux-gnu

# Build CLI
cd crates/cli
cargo build --release

# Run tests
cargo test --workspace

# Format code
cargo fmt --all

# Lint code
cargo clippy --workspace -- -D warnings
```

## Deployment

```bash
# Docker (all services)
docker-compose up -d

# Native (macOS/Linux)
# 1. Build desktop app
cd crates/desktop
cargo tauri build

# 2. Install CLI
cargo install --path crates/cli

# 3. Run bots
cd crates/telegram
cargo run --release
```
