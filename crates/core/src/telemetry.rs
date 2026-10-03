use serde::{Deserialize, Serialize};

use crate::types::ModelInfo;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Health {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub scheduler: Option<Scheduler>,
    #[serde(default)]
    pub kv_slots: Option<usize>,
    #[serde(default)]
    pub tiers: Option<Tiers>,
    #[serde(default)]
    pub hwinfo: Option<HwInfo>,
}

impl Health {
    pub fn is_ok(&self) -> bool {
        self.status == "ok"
    }

    pub fn has_details(&self) -> bool {
        self.scheduler.is_some() || self.tiers.is_some() || self.hwinfo.is_some()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Scheduler {
    pub active: u64,
    pub queued: u64,
    pub capacity: u64,
    pub max_queue: u64,
    pub queue_timeout_seconds: f64,
    pub admitted: u64,
    pub completed: u64,
    pub failed: u64,
    pub rejected: u64,
    pub timed_out: u64,
    pub cancelled: u64,
}

impl Scheduler {
    pub fn is_busy(&self) -> bool {
        self.active >= self.capacity.max(1)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tiers {
    pub vram: u64,
    pub ram: u64,
    pub disk: u64,
    pub vram_gb: f64,
    pub ram_gb: f64,
}

impl Tiers {
    pub fn total(&self) -> u64 {
        self.vram + self.ram + self.disk
    }

    pub fn resident_ratio(&self) -> f64 {
        ratio(self.vram + self.ram, self.total())
    }

    pub fn disk_ratio(&self) -> f64 {
        ratio(self.disk, self.total())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HwInfo {
    pub cores: u32,
    pub ram_total_gb: f64,
    pub ram_avail_gb: f64,
    pub gpus: u32,
    pub vram_total_gb: f64,
    pub cpu: String,
    pub gpu: String,
}

impl HwInfo {
    pub fn ram_used_gb(&self) -> f64 {
        (self.ram_total_gb - self.ram_avail_gb).max(0.0)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExpertSnapshot {
    pub rows: usize,
    pub cols: usize,
    pub map: String,
    pub hits: String,
    pub seq: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Disk,
    Ram,
    Vram,
}

impl Tier {
    fn from_bits(bits: u8) -> Self {
        match bits {
            0 => Tier::Disk,
            1 => Tier::Ram,
            _ => Tier::Vram,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpertCell {
    pub tier: Tier,
    pub heat: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExpertMap {
    pub layers: usize,
    pub experts_per_layer: usize,
    pub cells: Vec<ExpertCell>,
    pub last_turn: Vec<bool>,
    pub seq: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Residency {
    pub disk: u64,
    pub ram: u64,
    pub vram: u64,
}

impl Residency {
    pub fn total(&self) -> u64 {
        self.disk + self.ram + self.vram
    }

    pub fn resident(&self) -> u64 {
        self.ram + self.vram
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct RoutingStats {
    pub residency: Residency,
    pub routed_last_turn: u64,
    pub served_from_memory: u64,
    pub streamed_from_disk: u64,
}

impl RoutingStats {
    pub fn hit_rate(&self) -> Option<f64> {
        (self.routed_last_turn > 0)
            .then(|| self.served_from_memory as f64 / self.routed_last_turn as f64)
    }
}

impl ExpertSnapshot {
    pub fn is_empty(&self) -> bool {
        self.rows == 0 || self.cols == 0 || self.map.is_empty()
    }

    pub fn decode(&self) -> Option<ExpertMap> {
        if self.is_empty() {
            return None;
        }
        let count = self.rows.checked_mul(self.cols)?;
        let bytes = decode_hex(&self.map)?;
        if bytes.len() != count {
            return None;
        }
        let cells = bytes
            .iter()
            .map(|b| ExpertCell {
                tier: Tier::from_bits(b >> 6),
                heat: b & 0x3f,
            })
            .collect();
        let bitmap = decode_hex(&self.hits).unwrap_or_default();
        let last_turn = (0..count)
            .map(|i| {
                bitmap
                    .get(i / 8)
                    .is_some_and(|byte| byte & (1 << (i % 8)) != 0)
            })
            .collect();
        Some(ExpertMap {
            layers: self.rows,
            experts_per_layer: self.cols,
            cells,
            last_turn,
            seq: self.seq,
        })
    }
}

impl ExpertMap {
    pub fn cell(&self, layer: usize, expert: usize) -> Option<ExpertCell> {
        if expert >= self.experts_per_layer {
            return None;
        }
        self.cells
            .get(layer * self.experts_per_layer + expert)
            .copied()
    }

    pub fn stats(&self) -> RoutingStats {
        let mut stats = RoutingStats::default();
        for (cell, &hit) in self.cells.iter().zip(&self.last_turn) {
            match cell.tier {
                Tier::Disk => stats.residency.disk += 1,
                Tier::Ram => stats.residency.ram += 1,
                Tier::Vram => stats.residency.vram += 1,
            }
            if hit {
                stats.routed_last_turn += 1;
                if cell.tier == Tier::Disk {
                    stats.streamed_from_disk += 1;
                } else {
                    stats.served_from_memory += 1;
                }
            }
        }
        stats
    }

    pub fn hottest(&self, limit: usize) -> Vec<(usize, usize, ExpertCell)> {
        let mut all: Vec<_> = self
            .cells
            .iter()
            .enumerate()
            .filter(|(_, c)| c.heat > 0)
            .map(|(i, c)| (i / self.experts_per_layer, i % self.experts_per_layer, *c))
            .collect();
        all.sort_by(|a, b| {
            b.2.heat
                .cmp(&a.2.heat)
                .then(a.0.cmp(&b.0))
                .then(a.1.cmp(&b.1))
        });
        all.truncate(limit);
        all
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfileTurn {
    pub wall_s: f64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub expert_disk_s: f64,
    pub expert_wait_s: f64,
    pub expert_matmul_s: f64,
    pub attention_s: f64,
    pub lm_head_s: f64,
    pub forwards: u64,
}

impl ProfileTurn {
    pub fn decode_tokens_per_sec(&self) -> Option<f64> {
        (self.wall_s > 0.0 && self.completion_tokens > 0)
            .then(|| self.completion_tokens as f64 / self.wall_s)
    }

    pub fn io_wait_share(&self) -> f64 {
        if self.wall_s > 0.0 {
            (self.expert_wait_s / self.wall_s).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    pub fn disk_share(&self) -> f64 {
        if self.wall_s > 0.0 {
            (self.expert_disk_s / self.wall_s).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub seq: u64,
    pub turns: Vec<ProfileTurn>,
}

impl Profile {
    pub fn last(&self) -> Option<&ProfileTurn> {
        self.turns.last()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamingState {
    Idle,
    Generating,
    Queued,
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct NvmeStatus {
    pub experts_on_disk: u64,
    pub disk_ratio: f64,
    pub last_turn_disk_s: f64,
    pub last_turn_io_wait_share: f64,
    pub last_turn_streamed_experts: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColibriStatus {
    pub reachable: bool,
    pub base_url: String,
    pub configured_model: String,
    pub models: Vec<ModelInfo>,
    pub health: Option<Health>,
    pub routing: Option<RoutingStats>,
    pub last_turn: Option<ProfileTurn>,
    pub recent_turns: Vec<ProfileTurn>,
    pub state: StreamingState,
    pub nvme: NvmeStatus,
    pub error: Option<String>,
}

impl ColibriStatus {
    pub fn unreachable(base_url: &str, model: &str, error: String) -> Self {
        Self {
            reachable: false,
            base_url: base_url.to_string(),
            configured_model: model.to_string(),
            models: Vec::new(),
            health: None,
            routing: None,
            last_turn: None,
            recent_turns: Vec::new(),
            state: StreamingState::Unknown,
            nvme: NvmeStatus::default(),
            error: Some(error),
        }
    }

    pub fn assemble(
        base_url: &str,
        model: &str,
        health: Health,
        models: Vec<ModelInfo>,
        experts: Option<ExpertSnapshot>,
        profile: Option<Profile>,
    ) -> Self {
        let routing = experts
            .as_ref()
            .and_then(ExpertSnapshot::decode)
            .map(|m| m.stats());
        let recent_turns: Vec<ProfileTurn> = profile
            .map(|p| {
                let skip = p.turns.len().saturating_sub(30);
                p.turns.into_iter().skip(skip).collect()
            })
            .unwrap_or_default();
        let last_turn = recent_turns.last().cloned();
        let state = match &health.scheduler {
            Some(s) if s.queued > 0 => StreamingState::Queued,
            Some(s) if s.active > 0 => StreamingState::Generating,
            Some(_) => StreamingState::Idle,
            None => StreamingState::Unknown,
        };
        let tiers = health.tiers.as_ref().filter(|t| t.total() > 0);
        let experts_on_disk = tiers
            .map(|t| t.disk)
            .or_else(|| routing.map(|r| r.residency.disk))
            .unwrap_or(0);
        let disk_ratio = tiers
            .map(Tiers::disk_ratio)
            .or_else(|| routing.map(|r| ratio(r.residency.disk, r.residency.total())))
            .unwrap_or(0.0);
        let nvme = NvmeStatus {
            experts_on_disk,
            disk_ratio,
            last_turn_disk_s: last_turn.as_ref().map(|t| t.expert_disk_s).unwrap_or(0.0),
            last_turn_io_wait_share: last_turn
                .as_ref()
                .map(ProfileTurn::io_wait_share)
                .unwrap_or(0.0),
            last_turn_streamed_experts: routing.map(|r| r.streamed_from_disk).unwrap_or(0),
        };
        Self {
            reachable: health.is_ok(),
            base_url: base_url.to_string(),
            configured_model: model.to_string(),
            models,
            health: Some(health),
            routing,
            last_turn,
            recent_turns,
            state,
            nvme,
            error: None,
        }
    }

    pub fn model_loaded(&self) -> bool {
        self.models.iter().any(|m| m.id == self.configured_model)
    }

    pub fn kv_slots(&self) -> Option<usize> {
        self.health.as_ref().and_then(|h| h.kv_slots)
    }
}

fn ratio(part: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        part as f64 / total as f64
    }
}

fn decode_hex(input: &str) -> Option<Vec<u8>> {
    let input = input.trim();
    if input.len() % 2 != 0 {
        return None;
    }
    input
        .as_bytes()
        .chunks(2)
        .map(|pair| {
            let hi = (pair[0] as char).to_digit(16)?;
            let lo = (pair[1] as char).to_digit(16)?;
            Some((hi * 16 + lo) as u8)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> ExpertSnapshot {
        ExpertSnapshot {
            rows: 2,
            cols: 4,
            map: "00418305c0000102".into(),
            hits: "11".into(),
            seq: 7,
        }
    }

    #[test]
    fn decodes_tiers_and_heat() {
        let map = snapshot().decode().unwrap();
        assert_eq!(map.cell(0, 0).unwrap().tier, Tier::Disk);
        assert_eq!(
            map.cell(0, 1).unwrap(),
            ExpertCell {
                tier: Tier::Ram,
                heat: 1
            }
        );
        assert_eq!(
            map.cell(0, 2).unwrap(),
            ExpertCell {
                tier: Tier::Vram,
                heat: 3
            }
        );
        assert_eq!(
            map.cell(1, 0).unwrap(),
            ExpertCell {
                tier: Tier::Vram,
                heat: 0
            }
        );
        assert!(map.cell(0, 4).is_none());
    }

    #[test]
    fn hit_bitmap_is_lsb_first() {
        let map = snapshot().decode().unwrap();
        let hits: Vec<usize> = map
            .last_turn
            .iter()
            .enumerate()
            .filter(|(_, h)| **h)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(hits, vec![0, 4]);
    }

    #[test]
    fn routing_stats_split_memory_and_disk() {
        let stats = snapshot().decode().unwrap().stats();
        assert_eq!(
            stats.residency,
            Residency {
                disk: 5,
                ram: 1,
                vram: 2
            }
        );
        assert_eq!(stats.routed_last_turn, 2);
        assert_eq!(stats.served_from_memory, 1);
        assert_eq!(stats.streamed_from_disk, 1);
        assert_eq!(stats.hit_rate(), Some(0.5));
    }

    #[test]
    fn hottest_orders_by_heat() {
        let hot = snapshot().decode().unwrap().hottest(2);
        assert_eq!(hot[0].0, 0);
        assert_eq!(hot[0].1, 3);
        assert_eq!(hot[0].2.heat, 5);
        assert_eq!(hot[1].1, 2);
    }

    #[test]
    fn rejects_mismatched_sizes() {
        let mut bad = snapshot();
        bad.rows = 3;
        assert!(bad.decode().is_none());
        bad.rows = 2;
        bad.map = "zz".into();
        assert!(bad.decode().is_none());
    }

    #[test]
    fn profile_shares() {
        let turn = ProfileTurn {
            wall_s: 10.0,
            completion_tokens: 20,
            expert_disk_s: 4.0,
            expert_wait_s: 2.5,
            ..Default::default()
        };
        assert_eq!(turn.decode_tokens_per_sec(), Some(2.0));
        assert_eq!(turn.disk_share(), 0.4);
        assert_eq!(turn.io_wait_share(), 0.25);
    }

    #[test]
    fn status_state_from_scheduler() {
        let health = Health {
            status: "ok".into(),
            scheduler: Some(Scheduler {
                active: 1,
                capacity: 1,
                ..Default::default()
            }),
            tiers: Some(Tiers {
                vram: 0,
                ram: 30,
                disk: 70,
                ..Default::default()
            }),
            ..Default::default()
        };
        let status =
            ColibriStatus::assemble("http://x/v1", "glm-5.2-colibri", health, vec![], None, None);
        assert_eq!(status.state, StreamingState::Generating);
        assert_eq!(status.nvme.experts_on_disk, 70);
        assert!((status.nvme.disk_ratio - 0.7).abs() < 1e-9);
        assert!(status.reachable);
    }
}
