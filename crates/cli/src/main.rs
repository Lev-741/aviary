mod render;

use std::io::Write as _;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use aviary_core::agent::{AgentEvent, AgentOptions};
use aviary_core::{ColibriAgent, ColibriClient, ColibriConfig, Memory, ToolRegistry, ToolSettings};
use clap::{Args, Parser, Subcommand};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

use crate::render::Style;

#[derive(Debug, Parser)]
#[command(
    name = "aviary",
    version,
    about = "Chat with Colibri from the terminal"
)]
struct Cli {
    #[command(flatten)]
    colibri: ColibriArgs,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Args)]
struct ColibriArgs {
    #[arg(
        long,
        global = true,
        env = "COLIBRI_BASE_URL",
        help = "Colibri API base URL"
    )]
    url: Option<String>,

    #[arg(
        long,
        global = true,
        env = "COLIBRI_MODEL",
        help = "Model id served by Colibri"
    )]
    model: Option<String>,

    #[arg(long, global = true, env = "COLIBRI_API_KEY", hide_env_values = true)]
    api_key: Option<String>,

    #[arg(
        long,
        global = true,
        env = "COLIBRI_KV_SLOTS",
        help = "KV slots configured with coli serve --kv-slots"
    )]
    kv_slots: Option<usize>,

    #[arg(
        long,
        global = true,
        env = "DATABASE_URL",
        help = "SQLite URL for conversation memory"
    )]
    db: Option<String>,

    #[arg(
        long,
        global = true,
        env = "AVIARY_USER",
        help = "Conversation id used for memory"
    )]
    user: Option<String>,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(about = "Interactive chat (default)")]
    Chat(ChatArgs),
    #[command(about = "Ask a single question, use - to read it from stdin")]
    Ask {
        #[command(flatten)]
        chat: ChatArgs,
        #[arg(required = true, num_args = 1..)]
        prompt: Vec<String>,
    },
    #[command(about = "Show Colibri health, RAM and NVMe expert streaming")]
    Status {
        #[arg(long, help = "Print raw JSON")]
        json: bool,
        #[arg(long, value_name = "SECS", help = "Refresh every SECS seconds")]
        watch: Option<u64>,
    },
    #[command(about = "List models served by Colibri")]
    Models,
    #[command(about = "Show or clear the stored conversation")]
    History {
        #[arg(long)]
        clear: bool,
    },
}

#[derive(Debug, Args, Default, Clone)]
struct ChatArgs {
    #[arg(long, help = "Disable tool calling")]
    no_tools: bool,
    #[arg(long, help = "Allow the model to write files in the workspace")]
    allow_write: bool,
    #[arg(long, help = "Allow the model to run shell commands in the workspace")]
    allow_shell: bool,
    #[arg(
        long,
        value_name = "DIR",
        help = "Directory the file and shell tools are limited to"
    )]
    workspace: Option<PathBuf>,
    #[arg(long, help = "Enable the model's reasoning mode")]
    thinking: bool,
    #[arg(long, help = "Print reasoning tokens")]
    show_reasoning: bool,
    #[arg(long, help = "Wait for the full answer instead of streaming")]
    no_stream: bool,
    #[arg(long, value_name = "N", env = "COLIBRI_MAX_TOKENS")]
    max_tokens: Option<u32>,
    #[arg(
        long,
        value_name = "TEXT",
        env = "AVIARY_INSTRUCTIONS",
        help = "Extra system instructions"
    )]
    instructions: Option<String>,
}

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    if let Err(err) = run(Cli::parse()).await {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<()> {
    let style = Style::detect();
    match cli.command.unwrap_or(Command::Chat(ChatArgs::default())) {
        Command::Chat(args) => {
            let agent = build_agent(&cli.colibri, &args).await?;
            repl(&agent, &user_id(&cli.colibri), &args, style).await
        }
        Command::Ask { chat, prompt } => {
            let agent = build_agent(&cli.colibri, &chat).await?;
            let prompt = read_prompt(prompt).await?;
            ask(&agent, &user_id(&cli.colibri), &prompt, &chat, style).await
        }
        Command::Status { json, watch } => status(&cli.colibri, json, watch, style).await,
        Command::Models => {
            let client = ColibriClient::new(config(&cli.colibri, &ChatArgs::default())?)?;
            for model in client.models().await? {
                println!("{}", model.id);
            }
            Ok(())
        }
        Command::History { clear } => {
            let memory = open_memory(&cli.colibri).await?;
            let user = user_id(&cli.colibri);
            if clear {
                let removed = memory.clear(&user).await?;
                println!("removed {removed} messages for {user}");
            } else {
                for message in memory.stored(&user).await? {
                    println!(
                        "{} {}",
                        style.bold(&format!("{}:", message.role.as_str())),
                        message.content
                    );
                }
            }
            Ok(())
        }
    }
}

fn config(args: &ColibriArgs, chat: &ChatArgs) -> Result<ColibriConfig> {
    let mut config = ColibriConfig::from_env()?;
    if let Some(url) = &args.url {
        config.base_url = url.clone();
    }
    if let Some(model) = &args.model {
        config.model = model.clone();
    }
    if let Some(key) = &args.api_key {
        config.api_key = Some(key.clone()).filter(|k| !k.is_empty());
    }
    if let Some(slots) = args.kv_slots {
        config.kv_slots = slots;
    }
    if let Some(max) = chat.max_tokens {
        config.max_tokens = max;
    }
    config.thinking |= chat.thinking;
    config.validate()?;
    Ok(config)
}

async fn open_memory(args: &ColibriArgs) -> Result<Memory> {
    let url = match &args.db {
        Some(url) => url.clone(),
        None => {
            let dir = data_dir();
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("cannot create {}", dir.display()))?;
            format!("sqlite://{}", dir.join("aviary.db").display())
        }
    };
    Memory::open(&url)
        .await
        .with_context(|| format!("cannot open memory database {url}"))
}

fn data_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/aviary")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("aviary")
    }
}

fn user_id(args: &ColibriArgs) -> String {
    let name = args
        .user
        .clone()
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "local".into());
    format!("cli:{name}")
}

async fn build_agent(args: &ColibriArgs, chat: &ChatArgs) -> Result<ColibriAgent> {
    let config = config(args, chat)?;
    let memory = open_memory(args).await?;
    let tools = if chat.no_tools {
        ToolRegistry::new()
    } else {
        let mut settings = ToolSettings {
            allow_write: chat.allow_write,
            allow_shell: chat.allow_shell,
            ..Default::default()
        };
        if let Some(dir) = &chat.workspace {
            settings.workspace = dir
                .canonicalize()
                .with_context(|| format!("workspace {} does not exist", dir.display()))?;
        }
        ToolRegistry::with_defaults(&settings)
    };
    let options = AgentOptions {
        streaming: !chat.no_stream,
        instructions: chat.instructions.clone(),
        ..Default::default()
    };
    Ok(ColibriAgent::builder(config)
        .memory(memory)
        .tools(tools)
        .options(options)
        .build()
        .await?)
}

async fn read_prompt(parts: Vec<String>) -> Result<String> {
    if parts.len() == 1 && parts[0] == "-" {
        let mut input = String::new();
        tokio::io::stdin().read_to_string(&mut input).await?;
        return Ok(input);
    }
    Ok(parts.join(" "))
}

async fn ask(
    agent: &ColibriAgent,
    user: &str,
    prompt: &str,
    args: &ChatArgs,
    style: Style,
) -> Result<()> {
    let reply = turn(agent, user, prompt, args, style).await?;
    eprintln!("{}", render::footer(&reply, style));
    Ok(())
}

async fn turn(
    agent: &ColibriAgent,
    user: &str,
    prompt: &str,
    args: &ChatArgs,
    style: Style,
) -> Result<aviary_core::AgentReply> {
    let show_reasoning = args.show_reasoning;
    let mut in_reasoning = false;
    let mut printed = false;
    let reply = agent
        .run(user, prompt, |event| {
            let mut out = std::io::stdout().lock();
            match event {
                AgentEvent::Started { trimmed, .. } => {
                    if trimmed > 0 {
                        let note = format!(
                            "[{trimmed} older messages left out to fit the context window]"
                        );
                        let _ = writeln!(out, "{}", style.dim(&note));
                    }
                }
                AgentEvent::Reasoning(text) if show_reasoning => {
                    in_reasoning = true;
                    let _ = write!(out, "{}", style.dim(&text));
                }
                AgentEvent::Reasoning(_) => {}
                AgentEvent::Token(text) => {
                    if in_reasoning {
                        in_reasoning = false;
                        let _ = writeln!(out);
                    }
                    printed = true;
                    let _ = write!(out, "{text}");
                }
                AgentEvent::ToolCall { name, arguments } => {
                    if printed {
                        let _ = writeln!(out);
                        printed = false;
                    }
                    let _ = writeln!(out, "{}", style.cyan(&format!("> {name} {arguments}")));
                }
                AgentEvent::ToolResult { name, output, ok } => {
                    let first = output.lines().next().unwrap_or("");
                    let lines = output.lines().count();
                    let summary = if lines > 1 {
                        format!("< {name}: {first} (+{} lines)", lines - 1)
                    } else {
                        format!("< {name}: {first}")
                    };
                    let line = if ok {
                        style.dim(&summary)
                    } else {
                        style.red(&summary)
                    };
                    let _ = writeln!(out, "{line}");
                }
            }
            let _ = out.flush();
        })
        .await?;
    if !printed && !reply.content.is_empty() {
        print!("{}", reply.content);
    }
    println!();
    Ok(reply)
}

async fn repl(agent: &ColibriAgent, user: &str, args: &ChatArgs, style: Style) -> Result<()> {
    let config = agent.config();
    let status = agent.status().await;
    let state = if status.reachable {
        style.green("online")
    } else {
        style.red("offline")
    };
    println!(
        "{} {} at {} ({state})",
        style.bold("aviary"),
        config.model,
        config.base_url
    );
    if let Some(err) = status.error.as_ref().filter(|_| !status.reachable) {
        println!("{}", style.red(err));
    }
    let tools = agent.tools().names();
    if !tools.is_empty() && agent.profile().supports_tools {
        println!("{}", style.dim(&format!("tools: {}", tools.join(", "))));
    }
    println!(
        "{}",
        style.dim("Type /help for commands. Ctrl-C stops a running answer, Ctrl-D quits.")
    );

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    loop {
        print!("{} ", style.bold(">"));
        std::io::stdout().flush()?;
        let Some(line) = lines.next_line().await? else {
            println!();
            break;
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match line {
            "/exit" | "/quit" => break,
            "/help" => {
                println!("/status   Colibri health, RAM and NVMe streaming");
                println!("/history  show the stored conversation");
                println!("/reset    forget this conversation");
                println!("/exit     quit");
                continue;
            }
            "/status" => {
                let status = agent.status().await;
                print!(
                    "{}",
                    render::status(&status, Some(&agent.cache_stats()), style)
                );
                continue;
            }
            "/history" => {
                for message in agent.history(user).await? {
                    println!(
                        "{} {}",
                        style.bold(&format!("{}:", message.role.as_str())),
                        message.content
                    );
                }
                continue;
            }
            "/reset" => {
                let removed = agent.reset(user).await?;
                println!("{}", style.dim(&format!("forgot {removed} messages")));
                continue;
            }
            _ if line.starts_with('/') => {
                println!("{}", style.yellow("unknown command, try /help"));
                continue;
            }
            _ => {}
        }

        tokio::select! {
            result = turn(agent, user, line, args, style) => match result {
                Ok(reply) => println!("{}", render::footer(&reply, style)),
                Err(err) => println!("{}", style.red(&format!("error: {err:#}"))),
            },
            _ = tokio::signal::ctrl_c() => {
                println!();
                println!("{}", style.yellow("stopped"));
            }
        }
    }
    Ok(())
}

async fn status(args: &ColibriArgs, json: bool, watch: Option<u64>, style: Style) -> Result<()> {
    let client = ColibriClient::new(config(args, &ChatArgs::default())?)?;
    loop {
        let status = client.status().await;
        if json {
            println!("{}", serde_json::to_string_pretty(&status)?);
        } else {
            if watch.is_some() {
                print!("\x1b[2J\x1b[H");
            }
            print!("{}", render::status(&status, None, style));
        }
        let Some(secs) = watch else {
            if !status.reachable {
                std::process::exit(2);
            }
            return Ok(());
        };
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(secs.max(1))) => {}
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
    }
}
