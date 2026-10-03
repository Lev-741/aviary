use std::sync::Mutex;
use std::time::Instant;

use serde::Serialize;

use crate::telemetry::RoutingStats;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotLease {
    pub slot: usize,
    pub warm: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SlotInfo {
    pub slot: usize,
    pub owner: Option<String>,
    pub turns: u64,
    pub idle_secs: Option<u64>,
    pub last_hit_rate: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CacheStats {
    pub slots: Vec<SlotInfo>,
    pub warm_turns: u64,
    pub cold_turns: u64,
}

impl CacheStats {
    pub fn warm_ratio(&self) -> Option<f64> {
        let total = self.warm_turns + self.cold_turns;
        (total > 0).then(|| self.warm_turns as f64 / total as f64)
    }
}

#[derive(Debug)]
struct Slot {
    owner: Option<String>,
    last_used: Option<Instant>,
    tick: u64,
    turns: u64,
    last_hit_rate: Option<f64>,
}

#[derive(Debug)]
struct Inner {
    slots: Vec<Slot>,
    clock: u64,
    warm_turns: u64,
    cold_turns: u64,
}

#[derive(Debug)]
pub struct ExpertCache {
    inner: Mutex<Inner>,
}

impl ExpertCache {
    pub fn new(kv_slots: usize) -> Self {
        let slots = (0..kv_slots.max(1))
            .map(|_| Slot {
                owner: None,
                last_used: None,
                tick: 0,
                turns: 0,
                last_hit_rate: None,
            })
            .collect();
        Self {
            inner: Mutex::new(Inner {
                slots,
                clock: 0,
                warm_turns: 0,
                cold_turns: 0,
            }),
        }
    }

    pub fn slot_count(&self) -> usize {
        self.lock().slots.len()
    }

    pub fn acquire(&self, key: &str) -> SlotLease {
        let mut inner = self.lock();
        inner.clock += 1;
        let tick = inner.clock;

        let owned = inner
            .slots
            .iter()
            .position(|s| s.owner.as_deref() == Some(key));
        let (index, warm) = match owned {
            Some(index) => (index, true),
            None => {
                let index = inner
                    .slots
                    .iter()
                    .position(|s| s.owner.is_none())
                    .unwrap_or_else(|| {
                        inner
                            .slots
                            .iter()
                            .enumerate()
                            .min_by_key(|(_, s)| s.tick)
                            .map(|(i, _)| i)
                            .unwrap_or(0)
                    });
                let slot = &mut inner.slots[index];
                slot.owner = Some(key.to_string());
                slot.turns = 0;
                slot.last_hit_rate = None;
                (index, false)
            }
        };

        let slot = &mut inner.slots[index];
        slot.tick = tick;
        slot.last_used = Some(Instant::now());
        slot.turns += 1;
        if warm {
            inner.warm_turns += 1;
        } else {
            inner.cold_turns += 1;
        }
        SlotLease { slot: index, warm }
    }

    pub fn record_routing(&self, slot: usize, routing: &RoutingStats) {
        if let Some(rate) = routing.hit_rate() {
            if let Some(entry) = self.lock().slots.get_mut(slot) {
                entry.last_hit_rate = Some(rate);
            }
        }
    }

    pub fn forget(&self, key: &str) {
        let mut inner = self.lock();
        for slot in inner
            .slots
            .iter_mut()
            .filter(|s| s.owner.as_deref() == Some(key))
        {
            slot.owner = None;
            slot.turns = 0;
            slot.tick = 0;
            slot.last_hit_rate = None;
        }
    }

    pub fn stats(&self) -> CacheStats {
        let inner = self.lock();
        CacheStats {
            slots: inner
                .slots
                .iter()
                .enumerate()
                .map(|(i, s)| SlotInfo {
                    slot: i,
                    owner: s.owner.clone(),
                    turns: s.turns,
                    idle_secs: s.last_used.map(|t| t.elapsed().as_secs()),
                    last_hit_rate: s.last_hit_rate,
                })
                .collect(),
            warm_turns: inner.warm_turns,
            cold_turns: inner.cold_turns,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_user_keeps_slot() {
        let cache = ExpertCache::new(2);
        assert_eq!(
            cache.acquire("a"),
            SlotLease {
                slot: 0,
                warm: false
            }
        );
        assert_eq!(
            cache.acquire("b"),
            SlotLease {
                slot: 1,
                warm: false
            }
        );
        assert_eq!(
            cache.acquire("a"),
            SlotLease {
                slot: 0,
                warm: true
            }
        );
        assert_eq!(
            cache.acquire("b"),
            SlotLease {
                slot: 1,
                warm: true
            }
        );
    }

    #[test]
    fn evicts_least_recently_used() {
        let cache = ExpertCache::new(2);
        cache.acquire("a");
        cache.acquire("b");
        cache.acquire("a");
        assert_eq!(
            cache.acquire("c"),
            SlotLease {
                slot: 1,
                warm: false
            }
        );
        assert_eq!(
            cache.acquire("b"),
            SlotLease {
                slot: 0,
                warm: false
            }
        );
    }

    #[test]
    fn single_slot_is_shared() {
        let cache = ExpertCache::new(1);
        assert!(!cache.acquire("a").warm);
        assert!(!cache.acquire("b").warm);
        assert!(!cache.acquire("a").warm);
        assert!(cache.acquire("a").warm);
        let stats = cache.stats();
        assert_eq!(stats.warm_turns, 1);
        assert_eq!(stats.cold_turns, 3);
    }

    #[test]
    fn forget_frees_slot() {
        let cache = ExpertCache::new(2);
        cache.acquire("a");
        cache.acquire("b");
        cache.forget("a");
        assert_eq!(
            cache.acquire("c"),
            SlotLease {
                slot: 0,
                warm: false
            }
        );
    }

    #[test]
    fn records_hit_rate() {
        let cache = ExpertCache::new(1);
        let lease = cache.acquire("a");
        let routing = RoutingStats {
            routed_last_turn: 4,
            served_from_memory: 3,
            streamed_from_disk: 1,
            ..Default::default()
        };
        cache.record_routing(lease.slot, &routing);
        assert_eq!(cache.stats().slots[0].last_hit_rate, Some(0.75));
    }
}
