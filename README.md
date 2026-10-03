# Aviary

Aviary is an agent system written in Rust for one inference engine: [Colibri](https://github.com/JustVugg/colibri), the C runtime that runs GLM-5.2 (744B MoE) and other sparse models on consumer hardware by streaming experts from NVMe.

It does not support other backends. Everything it does, from prompt building to monitoring, assumes a Colibri server: a single process that serves one generation at a time, keeps hot experts in RAM or VRAM, streams the rest from disk and exposes its internals over HTTP.

What you get:

- `aviary` CLI: streaming chat, one-shot questions, and a status view of RAM, expert placement and NVMe streaming
- Desktop app (Tauri v2, macOS and Linux): chat UI, live monitoring dashboard and settings
- Telegram and Discord bots
- `aviary-core`: the library the rest is built on

## Requirements

- macOS 10.15+ or Linux (Ubuntu 22.04+, Fedora 39+). Windows is not supported.
- A running Colibri server (`coli serve`), see below
- Rust 1.85 or newer
- Node.js 20+ and WebKitGTK 4.1 (Linux) for the desktop app only

## Start Colibri

Convert or download a model as described in the Colibri README, then serve it:

```bash
cd colibri/c
./coli serve --model /nvme/glm52_i4 --model-id glm-5.2-colibri --kv-slots 2 --port 8000
```

`--kv-slots` matters for Aviary, see [KV slots and expert reuse](#kv-slots-and-expert-reuse). If you set `COLI_API_KEY` on the server, give Aviary the same key with `COLIBRI_API_KEY`. Colibri only reports hardware, expert and profiling data to authenticated clients when a key is configured.

## CLI

```bash
cargo install --path crates/cli

aviary status
aviary chat
aviary ask "Summarize what MoE expert routing is in two sentences"
git diff | aviary ask -
aviary models
aviary history --clear
```

`aviary status` prints a snapshot of the server:

```
Colibri     http://localhost:8000/v1  online
Model       glm-5.2-colibri  served  family Glm, tools yes
State       idle  active 0/1, queued 0/8
Hardware    Apple M3 Pro, 12 cores, no GPU
RAM         [##################......] 24.5 / 32.0 GB used
Experts     VRAM 0 (0.0 GB)  RAM 1,200 (11.5 GB)  NVMe 18,000
NVMe        [#######################.] 94% of experts streamed from disk
Last turn   50 prompt + 10 generated in 20.0s, 0.50 tok/s
Disk I/O    read 8.0s (40%), waiting on NVMe 25%, matmul 4.0s, attention 2.0s
Routing     3 experts routed last turn, 33% from RAM/VRAM, 2 streamed from NVMe
KV slots    2
Requests    3 admitted, 3 completed, 0 failed, 0 rejected, 0 timed out
```

Use `--watch 5` to refresh it, or `--json` for scripts. The exit code is 2 when Colibri is unreachable.

In `aviary chat`, answers stream as they are generated. Every reply ends with a footer such as `[slot 0 warm | 42 tokens | 61.3s | 0.69 tok/s | experts 71% from memory]`. Commands: `/status`, `/history`, `/reset`, `/exit`. Ctrl-C stops the current answer and Ctrl-D quits.

Tools are on by default and limited to the current directory, which you can change with `--workspace`. `read_file` and `list_dir` are always available. `--allow-write` adds `write_file` and `--allow-shell` adds `run_shell`. Use `--no-tools` to send no tool definitions at all, which also shortens the prompt.

## Desktop app

```bash
cd apps/desktop-ui && npm install && cd ../..

cargo install tauri-cli --version "^2.11" --locked
cd crates/desktop
cargo tauri dev
cargo tauri build
```

On Linux, install the WebKitGTK build dependencies first. On Ubuntu:

```bash
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev librsvg2-dev libayatana-appindicator3-dev
```

`cargo tauri build` produces `.deb`, `.rpm` and AppImage on Linux, `.app` and `.dmg` on macOS.

The app has three pages:

- **Chats**: a Telegram Desktop style chat list and conversation view. Tokens stream into the bubble, tool calls show inline, and reasoning (when enabled) is behind a toggle. Each answer shows its KV slot, decode speed and expert hit rate.
- **Monitor**: decode speed, expert hit rate, NVMe wait share, KV slot reuse, RAM and VRAM meters, expert placement across VRAM/RAM/NVMe, a per-turn breakdown of where time went (NVMe wait, expert matmul, attention, LM head), the live expert map (one cell per expert, colored by tier, routed experts highlighted) and the KV slot table.
- **Settings**: Colibri URL, API key, model (probed from the server), KV slots, context size, reply length, reasoning, extra instructions and tool permissions.

Settings are stored in the app config directory, and conversations in `aviary.db` in the app data directory.

## Bots

Both bots read their configuration from the environment or a `.env` file (see `.env.example`).

Telegram:

```bash
TELEGRAM_BOT_TOKEN=... TELEGRAM_ALLOWED_USERS=123456789 cargo run --release -p aviary-telegram
```

The bot answers in private chats, and in groups when it is mentioned or replied to. It sends a placeholder and edits it as tokens arrive, and shows "typing" while Colibri loads experts. Commands: `/status`, `/reset`, `/help`.

Discord:

```bash
DISCORD_BOT_TOKEN=... DISCORD_ALLOWED_CHANNELS=... cargo run --release -p aviary-discord
```

Enable the Message Content intent for the bot in the Discord developer portal. The bot answers DMs, mentions and `!ask <question>`. Commands: `!status`, `!reset`, `!help`.

Set the allowlists. Without them, anyone who can reach the bot can use your hardware.

## Docker

```bash
cp .env.example .env
docker compose up -d
docker compose run --rm cli
```

`docker-compose.yml` builds Colibri from its upstream repository (`docker/Dockerfile.slim`), mounts `COLIBRI_MODEL_DIR` at `/model` and starts the Telegram and Discord bots once Colibri reports healthy. The model directory is mounted read-write because Colibri keeps its KV persistence files (`.coli_kv`) there. Colibri's port is published on `127.0.0.1:8000` only.

The Aviary image (`Dockerfile`) contains `aviary`, `aviary-telegram` and `aviary-discord`. Conversation memory lives in the `aviary-data` volume.

## How Aviary uses Colibri

### KV slots and expert reuse

On a disk-streaming engine the cost of a turn is dominated by prefill and by experts that have to come from NVMe. Colibri can keep up to 16 independent KV contexts (`--kv-slots`) and reuses the longest common token prefix within a slot, even across restarts.

Aviary's expert cache (`aviary_core::cache::ExpertCache`) assigns each conversation a sticky slot and sends it as `cache_slot`. When there are more conversations than slots, the least recently used one is evicted. Combined with a prompt prefix that never changes between turns (fixed system prompt, tool definitions sorted by name, history appended at the end), a follow-up turn only prefills the new message. When history has to be trimmed to fit the context, Aviary drops a larger block at once so the new prefix stays stable for the next several turns, instead of shifting it every turn.

After each answer Aviary reads `/experts`, compares the experts routed in that turn with the tier each one lives in, and records the hit rate for that slot. This is the "experts N% from memory" figure in the CLI footer and the desktop app.

### Telemetry

| Endpoint | Used for |
| --- | --- |
| `GET /health` | online state, scheduler (active, queued, completed, rejected), KV slot count, expert tier counts (VRAM, RAM, disk), hardware (cores, RAM, GPUs, VRAM) |
| `GET /experts` | per-expert tier and heat map, plus the bitmap of experts routed in the last turn |
| `GET /profile` | per-turn timings: wall time, expert disk reads, time blocked on NVMe, expert matmul, attention, LM head |
| `GET /v1/models` | which model id the server serves |
| `x-colibri-queue-wait-ms` | time a request waited in Colibri's admission queue |

### Model-aware prompts

The prompt builder detects the model family from the model id. Tool definitions are only sent to engines that support tool calling (GLM-5.2, DeepSeek V4, Kimi K3). Inkling, Qwen3.8 and OLMoE reject tool declarations with HTTP 400, so Aviary leaves them out. The system prompt is short on purpose, because every token is prefilled at a few tokens per second on the CPU-streaming path. History is trimmed against `COLIBRI_CONTEXT_TOKENS` minus the reply budget.

Reasoning (`COLIBRI_THINKING=true`) maps to Colibri's `enable_thinking` and is only sent to families that support it.

### Queueing

Colibri runs one generation at a time and queues the rest (`--max-queue`, default 8). Aviary keeps one in-flight turn per user and queues further messages from the same user in order. A full server queue (HTTP 429) surfaces as "Colibri is busy" instead of a raw error. Streaming requests tolerate Colibri's SSE keepalives during long prefills. The idle timeout is `COLIBRI_TIMEOUT_SECS`, which defaults to one hour.

## Configuration

| Variable | Default | Meaning |
| --- | --- | --- |
| `COLIBRI_BASE_URL` | `http://localhost:8000/v1` | Colibri API base URL |
| `COLIBRI_MODEL` | `glm-5.2-colibri` | model id given to `coli serve --model-id` |
| `COLIBRI_API_KEY` | | matches `COLI_API_KEY` on the server |
| `COLIBRI_KV_SLOTS` | `1` | matches `coli serve --kv-slots` |
| `COLIBRI_CONTEXT_TOKENS` | `4096` | KV context size used for history trimming |
| `COLIBRI_MAX_TOKENS` | `1024` | reply token limit |
| `COLIBRI_TEMPERATURE` | server default | sampling temperature |
| `COLIBRI_THINKING` | `false` | enable the reasoning block |
| `COLIBRI_TIMEOUT_SECS` | `3600` | request timeout and stream idle timeout |
| `DATABASE_URL` | `sqlite://aviary.db` | conversation memory, the last 10 messages per user |
| `AVIARY_TOOLS` | `off` | bots only: `off`, `read` or `write` |
| `AVIARY_WORKSPACE` | | bots only: directory the tools are limited to |
| `AVIARY_INSTRUCTIONS` | | extra system prompt text |
| `TELEGRAM_BOT_TOKEN` | | Telegram bot token |
| `TELEGRAM_ALLOWED_USERS` | | comma-separated Telegram user ids |
| `TELEGRAM_API_URL` | | self-hosted Bot API server |
| `DISCORD_BOT_TOKEN` | | Discord bot token |
| `DISCORD_ALLOWED_USERS` | | comma-separated Discord user ids |
| `DISCORD_ALLOWED_CHANNELS` | | comma-separated channel ids |
| `DISCORD_PREFIX` | `!` | command prefix |
| `RUST_LOG` | `info` | log filter |

The CLI also accepts most of these as flags. Run `aviary --help` to see them.

## Tools and safety

File tools resolve every path inside the workspace after following symlinks and refuse anything outside it. Files larger than 256 KB are not read and tool output is cut at 8 KB. `run_shell` runs `sh -c` in the workspace with a 30 second timeout and no confirmation step, so only enable it on machines and workspaces you are comfortable handing to the model. The bots never get a shell tool.

Everything runs locally. Aviary does not talk to any service other than your Colibri server and, for the bots, Telegram or Discord.

## Development

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`crates/core/tests/mock_colibri.rs` runs the agent against a mock Colibri server (wiremock): streaming, tool loops, KV slot assignment, memory limits, 429 handling and status assembly.

`cargo build` without `-p` builds core, CLI and both bots. The desktop crate is a workspace member but not a default member, so machines without WebKitGTK can still build everything else. Build it explicitly with `cargo build -p aviary-desktop` after `npm run build` in `apps/desktop-ui`.

## Layout

```
crates/core       Colibri client, telemetry, expert cache, prompts, tools, memory, agent loop
crates/cli        aviary binary
crates/desktop    Tauri v2 shell and commands
crates/telegram   Telegram bot (teloxide)
crates/discord    Discord bot (serenity)
apps/desktop-ui   React + Vite + Tailwind frontend
```

## License

MIT
