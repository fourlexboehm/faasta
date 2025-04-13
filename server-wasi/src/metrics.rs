use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use dashmap::DashMap;
use once_cell::sync::Lazy;
use tracing::{debug, info};
use faasta_interface::{FunctionMetricsResponse, Metrics};
use sled;

// Global metrics storage using DashMap for concurrent access
pub static FUNCTION_METRICS: Lazy<DashMap<String, FunctionMetric>> = Lazy::new(|| DashMap::new());

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
        
        // Persist to sled
        let data = bincode::encode_to_vec(
            (
                self.total_time.load(Ordering::Relaxed),
                self.call_count.load(Ordering::Relaxed),
                now,
            ),
            bincode::config::standard()
        ).expect("Failed to serialize metrics");
        
        METRICS_DB.insert(self.function_name.as_bytes(), data)
            .expect("Failed to store metrics in database");
    }
    
    pub fn get_data(&self) -> (String, u64, u64, u64) {
        (
            self.function_name.clone(),
            self.total_time.load(Ordering::Relaxed),
            self.call_count.load(Ordering::Relaxed),
            self.last_called.load(Ordering::Relaxed),
        )
    }
}

pub fn get_metrics() -> Metrics {
    let mut function_metrics = Vec::new();
    let mut total_time = 0;
    let mut total_calls = 0;
    
    for entry in FUNCTION_METRICS.iter() {
        let (name, time, calls, last_called) = entry.get_data();
        
        // Convert timestamp to ISO string
        let last_called_time = UNIX_EPOCH + Duration::from_millis(last_called);
        let last_called_str = chrono::DateTime::<chrono::Utc>::from(last_called_time)
            .to_rfc3339();
        
        function_metrics.push(FunctionMetricsResponse {
            function_name: name,
            total_time_millis: time,
            call_count: calls,
            last_called: last_called_str,
        });
        
        total_time += time;
        total_calls += calls;
    }
    
    info!("Returning metrics: {} functions, {} total calls, {} total ms", 
          function_metrics.len(), total_calls, total_time);
    
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