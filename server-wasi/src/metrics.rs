use compio::runtime::spawn;
use compio::time::interval;
use dashmap::DashMap;
use faasta_interface::{FunctionMetricsResponse, Metrics};
use once_cell::sync::Lazy;
use std::path::Path;
use std::str;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{debug, error, info};

// Global metrics storage using DashMap for lock-free concurrent access
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

// Manual implementation of Clone for FunctionMetric
impl Clone for FunctionMetric {
    fn clone(&self) -> Self {
        Self {
            function_name: self.function_name.clone(),
            total_time: AtomicU64::new(self.total_time.load(Ordering::Relaxed)),
            call_count: AtomicU64::new(self.call_count.load(Ordering::Relaxed)),
            last_called: AtomicU64::new(self.last_called.load(Ordering::Relaxed)),
        }
    }
}

impl FunctionMetric {
    pub fn new(function_name: String) -> Self {
        // Initialize the last_called timestamp to current time
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_millis() as u64;

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
                Self::default(function_name, now)
            }
        } else {
            Self::default(function_name, now)
        };

        debug!(
            "Created or loaded metric for function: {}",
            metric.function_name
        );
        metric
    }

    fn default(function_name: String, now: u64) -> Self {
        Self {
            function_name,
            total_time: AtomicU64::new(0),
            call_count: AtomicU64::new(0),
            last_called: AtomicU64::new(now),
        }
    }

    pub fn record_call(&self, duration_ms: u64) {
        // Update in-memory metrics
        let prev_total = self.total_time.fetch_add(duration_ms, Ordering::Relaxed);
        let prev_count = self.call_count.fetch_add(1, Ordering::Relaxed);

        // Update last called timestamp (milliseconds since epoch)
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_millis() as u64;
        self.last_called.store(now, Ordering::Relaxed);

        // Log the metrics update with more detailed information
        debug!(
            "Recorded metrics for function '{}': duration={}ms, prev_total={}ms, new_total={}ms, prev_calls={}, new_calls={}",
            self.function_name,
            duration_ms,
            prev_total,
            prev_total + duration_ms,
            prev_count,
            prev_count + 1
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
                info!(
                    "Found existing DB metrics for '{}': total={}ms, calls={}, last={}",
                    self.function_name, t, c, l
                );
                (t, c, l)
            } else {
                info!(
                    "Failed to decode existing DB metrics for '{}', using zeros",
                    self.function_name
                );
                (0, 0, 0)
            }
        } else {
            info!(
                "No existing DB metrics for '{}', using zeros",
                self.function_name
            );
            (0, 0, 0)
        };

        // Add current in-memory values
        let mem_total = self.total_time.load(Ordering::Relaxed);
        let mem_calls = self.call_count.load(Ordering::Relaxed);
        let mem_last = self.last_called.load(Ordering::Relaxed);

        info!(
            "In-memory metrics for '{}': total={}ms, calls={}, last={}",
            self.function_name, mem_total, mem_calls, mem_last
        );

        // Calculate combined values
        let combined_total = db_total + mem_total;
        let combined_calls = db_calls + mem_calls;
        let combined_last = std::cmp::max(db_last, mem_last);

        info!(
            "Combined metrics for '{}': total={}ms, calls={}, last={}",
            self.function_name, combined_total, combined_calls, combined_last
        );

        // Combine and persist
        if let Ok(data) = bincode::encode_to_vec(
            (combined_total, combined_calls, combined_last),
            bincode::config::standard(),
        ) {
            match METRICS_DB.insert(self.function_name.as_bytes(), data) {
                Ok(_) => info!(
                    "Successfully persisted metrics for '{}'",
                    self.function_name
                ),
                Err(e) => error!(
                    "Failed to persist metrics for '{}': {}",
                    self.function_name, e
                ),
            }
        } else {
            error!("Failed to encode metrics for '{}'", self.function_name);
        }
    }
}

// Function to check if a function's WASM file exists
fn function_wasm_exists(function_name: &str) -> bool {
    // Get the functions directory from environment or use default
    let functions_dir =
        std::env::var("FUNCTIONS_PATH").unwrap_or_else(|_| "./functions".to_string());

    let wasm_filename = format!("{function_name}.wasm");
    let wasm_path = Path::new(&functions_dir).join(&wasm_filename);

    wasm_path.exists()
}

pub fn get_metrics() -> Metrics {
    info!("Retrieving metrics from database...");
    let mut function_metrics = Vec::new();
    let mut total_time = 0;
    let mut total_calls = 0;

    // Log the number of entries in the metrics database
    let db_entries_count = METRICS_DB.iter().count();
    info!("Found {} entries in metrics database", db_entries_count);

    for (key, value) in METRICS_DB.iter().flatten() {
        if key.starts_with(b"user:") {
            continue;
        }

        let function_name = match str::from_utf8(&key) {
            Ok(name) => name.to_string(),
            Err(_) => {
                debug!("Skipping invalid UTF-8 key in metrics database");
                continue; // Skip invalid UTF-8 keys
            }
        };

        // Decode the DB metrics data
        if let Ok(((db_total_time, db_call_count, db_last_called), _)) =
            bincode::decode_from_slice::<(u64, u64, u64), bincode::config::Configuration>(
                &value,
                bincode::config::standard(),
            )
        {
            info!(
                "DB metrics for '{}': total={}ms, calls={}, last={}",
                function_name, db_total_time, db_call_count, db_last_called
            );

            // Load in-memory metrics
            let (mem_total_time, mem_call_count, mem_last_called) = FUNCTION_METRICS
                .get(&function_name)
                .map(|m| {
                    let total = m.total_time.load(Ordering::Relaxed);
                    let calls = m.call_count.load(Ordering::Relaxed);
                    let last = m.last_called.load(Ordering::Relaxed);

                    info!(
                        "In-memory metrics for '{}': total={}ms, calls={}, last={}",
                        function_name, total, calls, last
                    );

                    (total, calls, last)
                })
                .unwrap_or_else(|| {
                    info!("No in-memory metrics for '{}', using zeros", function_name);
                    (0, 0, 0)
                });

            // Combine DB and in-memory metrics
            let combined_total_time = db_total_time.saturating_add(mem_total_time);
            let combined_call_count = db_call_count.saturating_add(mem_call_count);
            let combined_last_called = std::cmp::max(db_last_called, mem_last_called);

            info!(
                "Combined metrics for '{}': total={}ms, calls={}, last={}",
                function_name, combined_total_time, combined_call_count, combined_last_called
            );

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
        } else {
            error!(
                "Failed to decode metrics data for function '{}'",
                function_name
            );
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
    // Use entry API to reduce lock contention
    let entry = FUNCTION_METRICS.entry(function_name.to_string());

    match entry {
        dashmap::mapref::entry::Entry::Occupied(occupied) => {
            // Return a clone of the existing metric
            Some(FunctionMetric::new(occupied.key().clone()))
        }
        dashmap::mapref::entry::Entry::Vacant(vacant) => {
            // First check if the function's WASM file exists
            if !function_wasm_exists(function_name) {
                return None;
            }

            debug!("Creating new metric for function: {}", function_name);

            // Create the new metric
            let metric = FunctionMetric::new(function_name.to_string());

            // Insert it into the map
            vacant.insert(metric.clone());

            // New function added - ensure it's recorded in Sled DB even if no calls happen
            if !METRICS_DB
                .contains_key(function_name.as_bytes())
                .unwrap_or(false)
            {
                // Get current time in milliseconds for initialization
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or(Duration::from_secs(0))
                    .as_millis() as u64;

                // Initialize with zeros
                let initial_data =
                    match bincode::encode_to_vec((0u64, 0u64, now), bincode::config::standard()) {
                        Ok(data) => data,
                        Err(e) => {
                            error!(
                                "Failed to encode initial metrics for new function {}: {}",
                                function_name, e
                            );
                            return None;
                        }
                    };
                let _ = METRICS_DB.insert(function_name.as_bytes(), initial_data);
                debug!("Added new function '{}' to metrics database", function_name);
            }

            Some(metric)
        }
    }
}

// Timer utility to measure function execution time
pub struct Timer {
    start: SystemTime,
    function_name: String,
}

impl Timer {
    #[tracing::instrument(level = "debug")]
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
        let function_name = &metric.function_name;
        let call_count = metric.call_count.load(Ordering::Relaxed);
        let total_time = metric.total_time.load(Ordering::Relaxed);

        // Skip if no calls were made since last flush
        if call_count == 0 {
            debug!(
                "Skipping flush for function '{}' - no calls since last flush",
                function_name
            );
            continue; // Skip if no calls were made
        }

        info!(
            "Flushing metrics for function '{}': calls={}, total_time={}ms",
            function_name, call_count, total_time
        );

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

/// Spawn a background task to periodically flush metrics to DB every `interval_secs` seconds.
pub fn spawn_periodic_flush(interval_secs: u64) {
    spawn(async move {
        let mut ticker = interval(Duration::from_secs(interval_secs));
        loop {
            ticker.tick().await;
            flush_metrics_to_db();
        }
    })
    .detach();
}
