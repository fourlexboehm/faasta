use dashmap::DashMap;
use faasta_interface::{FunctionMetricsResponse, Metrics};
use once_cell::sync::Lazy;
use std::path::Path;
use std::str;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::{interval, Duration as TokioDuration};
use tracing::{debug, error, info};

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
            if let Ok(((total_time, call_count, last_called), _)) =
                bincode::decode_from_slice::<(u64, u64, u64), bincode::config::Configuration>(
                    &data,
                    bincode::config::standard(),
                )
            {
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
        // Update in-memory metrics
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

    // Method to flush this individual function's metrics to the database
    pub fn flush_to_db(&self) {
        // Load existing DB values
        let existing = METRICS_DB
            .get(self.function_name.as_bytes())
            .unwrap_or(None);
        let (db_total, db_calls, db_last) = if let Some(db_bytes) = existing {
            if let Ok(((t, c, l), _)) = bincode::decode_from_slice::<
                (u64, u64, u64),
                bincode::config::Configuration,
            >(&db_bytes, bincode::config::standard())
            {
                (t, c, l)
            } else {
                (0, 0, 0)
            }
        } else {
            (0, 0, 0)
        };

        // Add current in-memory values
        let mem_total = self.total_time.load(Ordering::Relaxed);
        let mem_calls = self.call_count.load(Ordering::Relaxed);
        let mem_last = self.last_called.load(Ordering::Relaxed);

        // Combine and persist
        if let Ok(data) = bincode::encode_to_vec(
            (
                db_total + mem_total,
                db_calls + mem_calls,
                std::cmp::max(db_last, mem_last),
            ),
            bincode::config::standard(),
        ) {
            let _ = METRICS_DB.insert(self.function_name.as_bytes(), data);
        }
    }
}

// Function to check if a function's WASM file exists
fn function_wasm_exists(function_name: &str) -> bool {
    // Get the functions directory from environment or use default
    let functions_dir =
        std::env::var("FUNCTIONS_PATH").unwrap_or_else(|_| "./functions".to_string());

    let wasm_filename = format!("{}.wasm", function_name);
    let wasm_path = Path::new(&functions_dir).join(&wasm_filename);

    wasm_path.exists()
}

pub fn get_metrics() -> Metrics {
    let mut function_metrics = Vec::new();
    let mut total_time = 0;
    let mut total_calls = 0;

    for (key, value) in METRICS_DB.iter().flatten() {
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
            let last_called_str =
                chrono::DateTime::<chrono::Utc>::from(last_called_time).to_rfc3339();

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
pub fn get_or_create_metric(function_name: &str) -> Option<FunctionMetric> {
    let is_new_function = !FUNCTION_METRICS.contains_key(function_name);

    if is_new_function {
        // First check if the function's WASM file exists
        if !function_wasm_exists(function_name) {
            return None;
        }
        // Create the new metric
        let metric = FunctionMetric::new(function_name.to_string());
        FUNCTION_METRICS.insert(function_name.to_string(), metric);

        // New function added - ensure it's recorded in Sled DB even if no calls happen
        // This is important for tracking deployed functions even before any calls
        if !METRICS_DB
            .contains_key(function_name.as_bytes())
            .unwrap_or(false)
        {
            // Initialize with zeros
            let initial_data =
                match bincode::encode_to_vec((0u64, 0u64, 0u64), bincode::config::standard()) {
                    Ok(data) => data,
                    Err(e) => {
                        error!(
                            "Failed to encode initial metrics for new function {}: {}",
                            function_name, e
                        );
                        return FunctionMetric::new(function_name.to_string()).into();
                    }
                };
            let _ = METRICS_DB.insert(function_name.as_bytes(), initial_data);
            debug!("Added new function '{}' to metrics database", function_name);
        }
    }
    // Get a copy of the metric
    let entry = FUNCTION_METRICS.get(function_name).unwrap();
    FunctionMetric::new(entry.function_name.clone()).into()
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

        if let Some(metric) = get_or_create_metric(&self.function_name) {
            // Round up any duration to at least 1 millisecond
            let duration_ms = duration.as_millis() as u64;
            // Ensure the minimum duration is 1ms, even if the actual duration was 0ms
            let rounded_duration = std::cmp::max(duration_ms, 1);

            metric.record_call(rounded_duration);
        }
    }
}

/// Flush in-memory metrics to persistent DB and reset counters.
pub fn flush_metrics_to_db() {
    info!("Flushing metrics to database...");
    let mut flushed_count = 0;

    for entry in FUNCTION_METRICS.iter() {
        let metric = entry.value(); // We only need the metric, not the key

        // Skip if no calls were made since last flush
        if metric.call_count.load(Ordering::Relaxed) == 0 {
            continue; // Skip if no calls were made
        }

        // First flush this function's current metrics to the database
        // using our helper method
        metric.flush_to_db();

        // Then reset the in-memory counters
        metric.total_time.store(0, Ordering::Relaxed);
        metric.call_count.store(0, Ordering::Relaxed);

        // Don't reset last_called timestamp
        // This preserves when the function was last used even after resetting counters

        flushed_count += 1;
    }

    if flushed_count > 0 {
        // Ensure DB writes are durable
        if let Err(e) = METRICS_DB.flush() {
            error!("Failed to flush metrics DB: {}", e);
        } else {
            info!(
                "Successfully flushed metrics for {} functions",
                flushed_count
            );
        }
    } else {
        // Log when no metrics were flushed (for monitoring)
        debug!("No metrics to flush - no functions were called since last flush");
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
