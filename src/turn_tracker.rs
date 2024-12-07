use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

pub const HASH_RETENTION_PERIOD: Duration = Duration::from_secs(3600);

pub trait TurnTracking {
    fn tracked(&self, hash: u64) -> bool;
    fn processing(&self, hash: u64);
    fn processed(&self, hash: u64);
    fn cleanup(&self);
}

pub struct TurnTracker {
    processing_turns: Arc<Mutex<HashMap<u64, Instant>>>,
    processed_turns: Arc<Mutex<HashMap<u64, Instant>>>,
}

impl TurnTracker {
    pub fn new() -> Self {
        TurnTracker {
            processing_turns: Arc::new(Mutex::new(HashMap::new())),
            processed_turns: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn clone(&self) -> Self {
        TurnTracker {
            processing_turns: self.processing_turns.clone(),
            processed_turns: self.processed_turns.clone(),
        }
    }
}

impl TurnTracking for TurnTracker {
    fn tracked(&self, hash: u64) -> bool {
        let processing = self.processing_turns.blocking_lock();
        let processed = self.processed_turns.blocking_lock();
        processing.contains_key(&hash) || processed.contains_key(&hash)
    }

    fn processing(&self, hash: u64) {
        self.processing_turns.blocking_lock()
            .insert(hash, Instant::now());
    }

    fn processed(&self, hash: u64) {
        self.processing_turns.blocking_lock().remove(&hash);
        self.processed_turns.blocking_lock()
            .insert(hash, Instant::now());
    }

    fn cleanup(&self) {
        let now = Instant::now();
        let mut processed = self.processed_turns.blocking_lock();
        processed.retain(|_, timestamp| now.duration_since(*timestamp) < HASH_RETENTION_PERIOD);
    }
}