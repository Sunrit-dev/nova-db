use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

/// Live server observability metrics.
#[derive(Debug)]
pub struct ServerMetrics {
    pub active_connections: AtomicU64,
    pub total_queries: AtomicU64,
    pub total_writes: AtomicU64,
    pub total_reads: AtomicU64,
    pub total_events_emitted: AtomicU64,
    pub start_time_micros: i64,
}

impl ServerMetrics {
    pub fn new() -> Self {
        Self {
            active_connections: AtomicU64::new(0),
            total_queries: AtomicU64::new(0),
            total_writes: AtomicU64::new(0),
            total_reads: AtomicU64::new(0),
            total_events_emitted: AtomicU64::new(0),
            start_time_micros: Utc::now().timestamp_micros(),
        }
    }

    pub fn inc_connections(&self) {
        self.active_connections.fetch_add(1, Ordering::Relaxed);
    }

    pub fn dec_connections(&self) {
        self.active_connections.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn inc_queries(&self) {
        self.total_queries.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_writes(&self) {
        self.total_writes.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_reads(&self) {
        self.total_reads.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_events(&self) {
        self.total_events_emitted.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        let now = Utc::now().timestamp_micros();
        let uptime_secs = ((now - self.start_time_micros) / 1_000_000).max(0) as u64;

        MetricsSnapshot {
            active_connections: self.active_connections.load(Ordering::Relaxed),
            total_queries: self.total_queries.load(Ordering::Relaxed),
            total_writes: self.total_writes.load(Ordering::Relaxed),
            total_reads: self.total_reads.load(Ordering::Relaxed),
            total_events_emitted: self.total_events_emitted.load(Ordering::Relaxed),
            uptime_seconds: uptime_secs,
        }
    }
}

impl Default for ServerMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Point-in-time snapshot of server metrics for reporting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    pub active_connections: u64,
    pub total_queries: u64,
    pub total_writes: u64,
    pub total_reads: u64,
    pub total_events_emitted: u64,
    pub uptime_seconds: u64,
}
