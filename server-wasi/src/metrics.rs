use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use dashmap::DashMap;
use once_cell::sync::Lazy;
use tracing::{debug, info, error};
use tokio::time::{interval, Duration as TokioDuration};
use bincode;
use faasta_interface::{FunctionMetricsResponse, Metrics};
use std::str;


// Global metrics storage using DashMap for concurrent access
pub static FUNCTION_METRICS: Lazy<DashMap<String, FunctionMetric>> = Lazy::new(DashMap::new);

// Sled database for persistent storage
pub static METRICS_DB: Lazy<sled::Db> = Lazy::new(|| {
    let db_path = std::env::var("METRICS_DB_PATH").unwrap_or_else(|_| "./data/metrics".to_string());
    sled::open(db_path).expect("Failed to open metrics database")
});

#[derive(Debug)]
pub struct FunctionMetric {
    pub function_name: String,
    pub total_time: AtomicU64,
    pub call_count: AtomicU64,
    pub last_called: AtomicU64,
}

impl FunctionMetric {
    pub fn new(function_name: String) -> Self {
        // Try to load from sled if it exists
        let metric = if let Ok(Some(data)) = METRICS_DB.get(function_name.as_bytes()) {
            if let Ok(((total_time, call_count, last_called), _)) = bincode::decode_from_slice::<(u64, u64, u64), bincode::config::Configuration>(&data, bincode::config::standard()) {
                Self {
                    function_name,
                    total_time: AtomicU64::new(total_time),
                    call_count: AtomicU64::new(call_count),
                    last_called: AtomicU64::new(last_called),
                }
            } else {
                Self::default(function_name)
            }
        } else {
            Self::default(function_name)
        };

        metric
    }

    fn default(function_name: String) -> Self {
        Self {
            function_name,
            total_time: AtomicU64::new(0),
            call_count: AtomicU64::new(0),
            last_called: AtomicU64::new(0),
        }
    }

    pub fn record_call(&self, duration_ms: u64) {
        self.total_time.fetch_add(duration_ms, Ordering::Relaxed);
        self.call_count.fetch_add(1, Ordering::Relaxed);
        
        // Update last called timestamp (milliseconds since epoch)
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_millis() as u64;
        self.last_called.store(now, Ordering::Relaxed);
        
        // Log the metrics update
        debug!(
            "Recorded metrics for function '{}': duration={}ms, total_calls={}, total_time={}ms",
            self.function_name,
            duration_ms,
            self.call_count.load(Ordering::Relaxed),
            self.total_time.load(Ordering::Relaxed)
        );
        
        // No immediate persistence; metrics will be flushed periodically
    }
    
}
pub fn get_metrics() -> Metrics {
    let mut function_metrics = Vec::new();
    let mut total_time = 0;
    let mut total_calls = 0;

    // Iterate through all entries in the sled database
    for result in METRICS_DB.iter() {
        if let Ok((key, value)) = result {
            // Skip user project count entries (they start with "user:")
            if key.starts_with(b"user:") {
                continue;
            }

            let function_name = match str::from_utf8(&key) {
                Ok(name) => name.to_string(),
                Err(_) => continue, // Skip invalid UTF-8 keys
            };

            // Decode the DB metrics data
            if let Ok(((db_total_time, db_call_count, db_last_called), _)) =
                bincode::decode_from_slice::<(u64, u64, u64), bincode::config::Configuration>(
                    &value,
                    bincode::config::standard(),
                )
            {
                // Load in-memory metrics
                let (mem_total_time, mem_call_count, mem_last_called) = FUNCTION_METRICS
                    .get(&function_name)
                    .map(|m| {
                        (
                            m.total_time.load(Ordering::Relaxed),
                            m.call_count.load(Ordering::Relaxed),
                            m.last_called.load(Ordering::Relaxed),
                        )
                    })
                    .unwrap_or((0, 0, 0));

                // Combine DB and in-memory metrics
                let combined_total_time = db_total_time.saturating_add(mem_total_time);
                let combined_call_count = db_call_count.saturating_add(mem_call_count);
                let combined_last_called = std::cmp::max(db_last_called, mem_last_called);

                // Convert timestamp to ISO string
                let last_called_time = UNIX_EPOCH + Duration::from_millis(combined_last_called);
                let last_called_str = chrono::DateTime::<chrono::Utc>::from(last_called_time)
                    .to_rfc3339();

                function_metrics.push(FunctionMetricsResponse {
                    function_name: function_name.clone(),
                    total_time_millis: combined_total_time,
                    call_count: combined_call_count,
                    last_called: last_called_str,
                });

                total_time += combined_total_time;
                total_calls += combined_call_count;
            }
        }
    }

    info!(
        "Returning metrics: {} functions, {} total calls, {} total ms",
        function_metrics.len(),
        total_calls,
        total_time
    );

    Metrics {
        total_time,
        total_calls,
        function_metrics,
    }
}

// Helper function to get or create a function metric
pub fn get_or_create_metric(function_name: &str) -> FunctionMetric {
    if !FUNCTION_METRICS.contains_key(function_name) {
        let metric = FunctionMetric::new(function_name.to_string());
        FUNCTION_METRICS.insert(function_name.to_string(), metric);
    }
    
    // Get a copy of the metric
    let entry = FUNCTION_METRICS.get(function_name).unwrap();
    FunctionMetric::new(entry.function_name.clone())
}

// Timer utility to measure function execution time
pub struct Timer {
    start: SystemTime,
    function_name: String,
}

impl Timer {
    pub fn new(function_name: String) -> Self {
        Self {
            start: SystemTime::now(),
            function_name,
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        let duration = SystemTime::now()
            .duration_since(self.start)
            .unwrap_or(Duration::from_secs(0));
        
        let metric = get_or_create_metric(&self.function_name);
        metric.record_call(duration.as_millis() as u64);
    }
}

/// Flush in-memory metrics to persistent DB and reset counters.
pub fn flush_metrics_to_db() {
    for entry in FUNCTION_METRICS.iter() {
        let function_name = entry.key();
        let metric = entry.value();
        let mem_total = metric.total_time.load(Ordering::Relaxed);
        let mem_calls = metric.call_count.load(Ordering::Relaxed);
        let mem_last = metric.last_called.load(Ordering::Relaxed);

        if mem_calls == 0 {
            continue; // Skip if no calls were made
        }

        // Load existing DB values
        let existing = METRICS_DB.get(function_name.as_bytes()).unwrap_or(None);
        let (db_total, db_calls, db_last) = if let Some(db_bytes) = existing {
            if let Ok(((t, c, l), _)) = bincode::decode_from_slice::<(u64, u64, u64), bincode::config::Configuration>(
                &db_bytes,
                bincode::config::standard(),
            ) {
                (t, c, l)
            } else {
                (0, 0, 0)
            }
        } else {
            (0, 0, 0)
        };

        let new_total = db_total.saturating_add(mem_total);
        let new_calls = db_calls.saturating_add(mem_calls);
        let new_last = std::cmp::max(db_last, mem_last);

        // Serialize and insert
        match bincode::encode_to_vec((new_total, new_calls, new_last), bincode::config::standard()) {
            Ok(data) => {
                if let Err(e) = METRICS_DB.insert(function_name.as_bytes(), data) {
                    error!("Failed to persist flushed metrics for {}: {}", function_name, e);
                }
            }
            Err(e) => {
                error!("Failed to encode flushed metrics for {}: {}", function_name, e);
            }
        }

        // Reset in-memory counters
        metric.total_time.store(0, Ordering::Relaxed);
        metric.call_count.store(0, Ordering::Relaxed);
        metric.last_called.store(0, Ordering::Relaxed);
    }
    // Ensure DB writes are durable
    if let Err(e) = METRICS_DB.flush() {
        error!("Failed to flush metrics DB: {}", e);
    }
}

/// Spawn a Tokio task to periodically flush metrics to DB every `interval_secs` seconds.
pub fn spawn_periodic_flush(interval_secs: u64) {
    tokio::spawn(async move {
        let mut ticker = interval(TokioDuration::from_secs(interval_secs));
        loop {
            ticker.tick().await;
            flush_metrics_to_db();
        }
    });
}