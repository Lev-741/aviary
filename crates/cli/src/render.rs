use std::fmt::Write;
use std::io::IsTerminal;

use aviary_core::agent::AgentReply;
use aviary_core::{CacheStats, ColibriStatus, ModelProfile, StreamingState};

#[derive(Debug, Clone, Copy)]
pub struct Style {
    color: bool,
}

impl Style {
    pub fn detect() -> Self {
        let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        Self { color }
    }

    #[cfg(test)]
    pub fn plain() -> Self {
        Self { color: false }
    }

    fn wrap(&self, code: &str, text: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    pub fn dim(&self, text: &str) -> String {
        self.wrap("2", text)
    }

    pub fn bold(&self, text: &str) -> String {
        self.wrap("1", text)
    }

    pub fn green(&self, text: &str) -> String {
        self.wrap("32", text)
    }

    pub fn red(&self, text: &str) -> String {
        self.wrap("31", text)
    }

    pub fn yellow(&self, text: &str) -> String {
        self.wrap("33", text)
    }

    pub fn cyan(&self, text: &str) -> String {
        self.wrap("36", text)
    }
}

pub fn percent(value: f64) -> String {
    format!("{:.0}%", value * 100.0)
}

pub fn thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

pub fn status(status: &ColibriStatus, cache: Option<&CacheStats>, style: Style) -> String {
    let mut out = String::new();
    let label = |out: &mut String, name: &str| {
        let _ = write!(out, "{}", style.bold(&format!("{name:<12}")));
    };

    label(&mut out, "Colibri");
    let state = if status.reachable {
        style.green("online")
    } else {
        style.red("offline")
    };
    let _ = writeln!(out, "{}  {state}", status.base_url);
    if !status.reachable {
        if let Some(err) = &status.error {
            label(&mut out, "Error");
            let _ = writeln!(out, "{err}");
        }
        return out;
    }

    let profile = ModelProfile::detect(&status.configured_model);
    label(&mut out, "Model");
    let loaded = if status.model_loaded() {
        style.green("served")
    } else if status.models.is_empty() {
        style.yellow("unknown")
    } else {
        style.yellow(&format!(
            "not served (server has {})",
            status
                .models
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    };
    let _ = writeln!(
        out,
        "{}  {loaded}  {}",
        status.configured_model,
        style.dim(&format!(
            "family {:?}, tools {}",
            profile.family,
            if profile.supports_tools { "yes" } else { "no" }
        ))
    );

    let health = status.health.as_ref();
    label(&mut out, "State");
    let state = match status.state {
        StreamingState::Idle => style.green("idle"),
        StreamingState::Generating => style.cyan("generating"),
        StreamingState::Queued => style.yellow("queued"),
        StreamingState::Unknown => style.dim("unknown (details need the API key)"),
    };
    match health.and_then(|h| h.scheduler.as_ref()) {
        Some(s) => {
            let _ = writeln!(
                out,
                "{state}  {}",
                style.dim(&format!(
                    "active {}/{}, queued {}/{}",
                    s.active, s.capacity, s.queued, s.max_queue
                ))
            );
        }
        None => {
            let _ = writeln!(out, "{state}");
        }
    }

    if let Some(hw) = health.and_then(|h| h.hwinfo.as_ref()) {
        label(&mut out, "Hardware");
        let gpu = if hw.gpus == 0 || hw.gpu.is_empty() {
            "no GPU".to_string()
        } else {
            format!("{} ({:.1} GB VRAM)", hw.gpu, hw.vram_total_gb)
        };
        let _ = writeln!(out, "{}, {} cores, {gpu}", hw.cpu, hw.cores);
        label(&mut out, "RAM");
        let used = hw.ram_used_gb();
        let ratio = if hw.ram_total_gb > 0.0 {
            used / hw.ram_total_gb
        } else {
            0.0
        };
        let _ = writeln!(
            out,
            "{} {:.1} / {:.1} GB used",
            bar(ratio, 24),
            used,
            hw.ram_total_gb
        );
    }

    if let Some(tiers) = health.and_then(|h| h.tiers.as_ref()) {
        label(&mut out, "Experts");
        let _ = writeln!(
            out,
            "VRAM {} ({:.1} GB)  RAM {} ({:.1} GB)  NVMe {}",
            thousands(tiers.vram),
            tiers.vram_gb,
            thousands(tiers.ram),
            tiers.ram_gb,
            thousands(tiers.disk)
        );
    }

    label(&mut out, "NVMe");
    let nvme = &status.nvme;
    let _ = writeln!(
        out,
        "{} {} of experts streamed from disk",
        bar(nvme.disk_ratio, 24),
        percent(nvme.disk_ratio)
    );

    if let Some(turn) = &status.last_turn {
        label(&mut out, "Last turn");
        let speed = turn
            .decode_tokens_per_sec()
            .map(|t| format!("{t:.2} tok/s"))
            .unwrap_or_else(|| "-".into());
        let _ = writeln!(
            out,
            "{} prompt + {} generated in {:.1}s, {speed}",
            turn.prompt_tokens, turn.completion_tokens, turn.wall_s
        );
        label(&mut out, "Disk I/O");
        let _ = writeln!(
            out,
            "read {:.1}s ({}), waiting on NVMe {}, matmul {:.1}s, attention {:.1}s",
            turn.expert_disk_s,
            percent(turn.disk_share()),
            percent(turn.io_wait_share()),
            turn.expert_matmul_s,
            turn.attention_s
        );
    }

    if let Some(routing) = &status.routing {
        label(&mut out, "Routing");
        match routing.hit_rate() {
            Some(rate) => {
                let _ = writeln!(
                    out,
                    "{} experts routed last turn, {} from RAM/VRAM, {} streamed from NVMe",
                    routing.routed_last_turn,
                    percent(rate),
                    routing.streamed_from_disk
                );
            }
            None => {
                let _ = writeln!(out, "no routed experts recorded yet");
            }
        }
    }

    if let Some(slots) = status.kv_slots() {
        label(&mut out, "KV slots");
        let _ = write!(out, "{slots}");
        if let Some(cache) = cache {
            if let Some(warm) = cache.warm_ratio() {
                let _ = write!(
                    out,
                    "  {}",
                    style.dim(&format!("{} of turns reused a warm slot", percent(warm)))
                );
            }
        }
        let _ = writeln!(out);
    }

    if let Some(s) = health.and_then(|h| h.scheduler.as_ref()) {
        label(&mut out, "Requests");
        let _ = writeln!(
            out,
            "{} admitted, {} completed, {} failed, {} rejected, {} timed out",
            s.admitted, s.completed, s.failed, s.rejected, s.timed_out
        );
    }

    if !health.is_some_and(|h| h.has_details()) {
        let _ = writeln!(
            out,
            "{}",
            style.dim("Colibri hides hardware and expert details without a valid API key.")
        );
    }
    out
}

pub fn footer(reply: &AgentReply, style: Style) -> String {
    let mut parts = vec![format!(
        "slot {}{}",
        reply.slot,
        if reply.warm_slot { " warm" } else { " cold" }
    )];
    parts.push(format!("{} tokens", reply.usage.completion_tokens));
    parts.push(format!("{:.1}s", reply.elapsed_ms as f64 / 1000.0));
    if let Some(speed) = reply.tokens_per_sec() {
        parts.push(format!("{speed:.2} tok/s"));
    }
    if let Some(wait) = reply.queue_wait_ms.filter(|w| *w > 0) {
        parts.push(format!("queued {:.1}s", wait as f64 / 1000.0));
    }
    if reply.tool_calls > 0 {
        parts.push(format!("{} tool calls", reply.tool_calls));
    }
    if let Some(rate) = reply.routing.and_then(|r| r.hit_rate()) {
        parts.push(format!("experts {} from memory", percent(rate)));
    }
    style.dim(&format!("[{}]", parts.join(" | ")))
}

fn bar(ratio: f64, width: usize) -> String {
    let filled = ((ratio.clamp(0.0, 1.0) * width as f64).round() as usize).min(width);
    format!("[{}{}]", "#".repeat(filled), ".".repeat(width - filled))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aviary_core::telemetry::{Health, HwInfo, Scheduler, Tiers};
    use aviary_core::types::ModelInfo;

    #[test]
    fn formats_thousands() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(19456), "19,456");
        assert_eq!(thousands(1234567), "1,234,567");
    }

    #[test]
    fn bar_is_bounded() {
        assert_eq!(bar(0.5, 4), "[##..]");
        assert_eq!(bar(2.0, 4), "[####]");
        assert_eq!(bar(-1.0, 4), "[....]");
    }

    #[test]
    fn renders_full_status() {
        let health = Health {
            status: "ok".into(),
            scheduler: Some(Scheduler {
                capacity: 1,
                max_queue: 8,
                ..Default::default()
            }),
            kv_slots: Some(2),
            tiers: Some(Tiers {
                ram: 1200,
                disk: 18000,
                ram_gb: 11.5,
                ..Default::default()
            }),
            hwinfo: Some(HwInfo {
                cores: 12,
                ram_total_gb: 32.0,
                ram_avail_gb: 8.0,
                cpu: "Apple M3 Pro".into(),
                ..Default::default()
            }),
        };
        let status = ColibriStatus::assemble(
            "http://localhost:8000/v1",
            "glm-5.2-colibri",
            health,
            vec![ModelInfo {
                id: "glm-5.2-colibri".into(),
                owned_by: None,
                created: None,
            }],
            None,
            None,
        );
        let text = super::status(&status, None, Style::plain());
        assert!(text.contains("online"));
        assert!(text.contains("served"));
        assert!(text.contains("Apple M3 Pro, 12 cores, no GPU"));
        assert!(text.contains("24.0 / 32.0 GB used"));
        assert!(text.contains("NVMe 18,000"));
        assert!(text.contains("94% of experts streamed from disk"));
        assert!(text.contains("idle"));
    }

    #[test]
    fn renders_offline() {
        let status = ColibriStatus::unreachable("http://x/v1", "glm", "connection refused".into());
        let text = super::status(&status, None, Style::plain());
        assert!(text.contains("offline"));
        assert!(text.contains("connection refused"));
        assert!(!text.contains("Model"));
    }
}
