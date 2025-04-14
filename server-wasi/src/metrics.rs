use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use dashmap::DashMap;
use once_cell::sync::Lazy;
use tracing::{debug, info, error};
use faasta_interface::{FunctionMetricsResponse, Metrics, FunctionError, FunctionResult};
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
            
            // Decode the metrics data
            if let Ok(((total_time_ms, call_count, last_called), _)) =
                bincode::decode_from_slice::<(u64, u64, u64), bincode::config::Configuration>(&value, bincode::config::standard()) {
                
                // Convert timestamp to ISO string
                let last_called_time = UNIX_EPOCH + Duration::from_millis(last_called);
                let last_called_str = chrono::DateTime::<chrono::Utc>::from(last_called_time)
                    .to_rfc3339();
                
                function_metrics.push(FunctionMetricsResponse {
                    function_name: function_name.clone(),
                    total_time_millis: total_time_ms,
                    call_count,
                    last_called: last_called_str,
                });
                
                total_time += total_time_ms;
                total_calls += call_count;
            }
        }
    }
    
    info!("Returning metrics from sled: {} functions, {} total calls, {} total ms",
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

// User project management functions

/// Get the number of projects for a user
pub fn get_user_project_count(username: &str) -> u32 {
    let key = format!("user:{}", username);
    
    if let Ok(Some(data)) = METRICS_DB.get(key.as_bytes()) {
        // First convert bytes to string, then parse the string as u32
        if let Ok(s) = str::from_utf8(&data) {
            if let Ok(count) = s.parse::<u32>() {
                return count;
            }
        }
    }
    
    // Default to 0 if not found or error
    0
}

/// Increment the project count for a user
pub fn increment_user_project_count(username: &str) -> FunctionResult<u32> {
    let key = format!("user:{}", username);
    let new_count = get_user_project_count(username) + 1;
    
    // Check if user has reached the project limit
    if new_count > 10 {
        return Err(FunctionError::PermissionDenied(
            "User has reached the maximum limit of 10 projects".to_string()
        ));
    }
    
    // Store the new count
    match METRICS_DB.insert(key.as_bytes(), new_count.to_string().as_bytes()) {
        Ok(_) => {
            debug!("Incremented project count for user '{}' to {}", username, new_count);
            Ok(new_count)
        },
        Err(e) => {
            error!("Failed to increment project count for user '{}': {}", username, e);
            Err(FunctionError::InternalError(format!("Failed to update project count: {}", e)))
        }
    }
}

/// Decrement the project count for a user
pub fn decrement_user_project_count(username: &str) -> FunctionResult<u32> {
    let key = format!("user:{}", username);
    let current_count = get_user_project_count(username);
    
    // Don't go below zero
    let new_count = if current_count > 0 { current_count - 1 } else { 0 };
    
    // Store the new count
    match METRICS_DB.insert(key.as_bytes(), new_count.to_string().as_bytes()) {
        Ok(_) => {
            debug!("Decremented project count for user '{}' to {}", username, new_count);
            Ok(new_count)
        },
        Err(e) => {
            error!("Failed to decrement project count for user '{}': {}", username, e);
            Err(FunctionError::InternalError(format!("Failed to update project count: {}", e)))
        }
    }
}

/// Check if a user can create more projects
pub fn can_create_project(username: &str) -> bool {
    get_user_project_count(username) < 10
}