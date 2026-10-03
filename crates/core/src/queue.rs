use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

#[derive(Debug, Default)]
pub struct TurnQueue {
    locks: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
}

impl TurnQueue {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock_for(&self, key: &str) -> Arc<AsyncMutex<()>> {
        let mut locks = self.locks.lock().unwrap_or_else(|p| p.into_inner());
        locks.retain(|k, lock| k == key || Arc::strong_count(lock) > 1);
        locks.entry(key.to_string()).or_default().clone()
    }

    pub fn is_busy(&self, key: &str) -> bool {
        self.lock_for(key).try_lock().is_err()
    }

    pub async fn acquire(&self, key: &str) -> OwnedMutexGuard<()> {
        self.lock_for(key).lock_owned().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn same_key_waits_other_keys_do_not() {
        let queue = Arc::new(TurnQueue::new());
        let guard = queue.acquire("a").await;
        assert!(queue.is_busy("a"));
        assert!(!queue.is_busy("b"));

        let waiter = {
            let queue = queue.clone();
            tokio::spawn(async move {
                let _g = queue.acquire("a").await;
            })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!waiter.is_finished());
        drop(guard);
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .unwrap()
            .unwrap();
        assert!(!queue.is_busy("a"));
    }

    #[tokio::test]
    async fn idle_entries_are_pruned() {
        let queue = TurnQueue::new();
        for i in 0..50 {
            let _g = queue.acquire(&format!("user{i}")).await;
        }
        assert!(queue.locks.lock().unwrap().len() <= 1);
    }
}
