use dashmap::DashMap;
use faasta_interface::{FunctionMetricsResponse, Metrics};
use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time;
use tracing::{debug, error, info};

use crate::storage;

pub static FUNCTION_METRICS: Lazy<DashMap<String, FunctionMetric>> = Lazy::new(DashMap::new);

#[derive(Debug)]
pub struct FunctionMetric {
    pub function_name: String,
    pub total_time: AtomicU64,
    pub call_count: AtomicU64,
    pub last_called: AtomicU64,
}

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
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_millis() as u64;

        let metric = if let Ok(Some((total_time, call_count, last_called))) =
            storage::get_metric(&function_name)
        {
            Self {
                function_name,
                total_time: AtomicU64::new(total_time),
                call_count: AtomicU64::new(call_count),
                last_called: AtomicU64::new(last_called),
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
        let prev_total = self.total_time.fetch_add(duration_ms, Ordering::Relaxed);
        let prev_count = self.call_count.fetch_add(1, Ordering::Relaxed);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_millis() as u64;
        self.last_called.store(now, Ordering::Relaxed);

        debug!(
            "Recorded metrics for function '{}': duration={}ms, prev_total={}ms, new_total={}ms, prev_calls={}, new_calls={}",
            self.function_name,
            duration_ms,
            prev_total,
            prev_total + duration_ms,
            prev_count,
            prev_count + 1
        );
    }

    pub fn flush_to_db(&self) {
        let (db_total, db_calls, db_last) =
            if let Ok(Some((t, c, l))) = storage::get_metric(&self.function_name) {
                info!(
                    "Found existing DB metrics for '{}': total={}ms, calls={}, last={}",
                    self.function_name, t, c, l
                );
                (t, c, l)
            } else {
                info!(
                    "No existing DB metrics for '{}', using zeros",
                    self.function_name
                );
                (0, 0, 0)
            };

        let mem_total = self.total_time.load(Ordering::Relaxed);
        let mem_calls = self.call_count.load(Ordering::Relaxed);
        let mem_last = self.last_called.load(Ordering::Relaxed);

        info!(
            "In-memory metrics for '{}': total={}ms, calls={}, last={}",
            self.function_name, mem_total, mem_calls, mem_last
        );

        let combined_total = db_total + mem_total;
        let combined_calls = db_calls + mem_calls;
        let combined_last = std::cmp::max(db_last, mem_last);

        info!(
            "Combined metrics for '{}': total={}ms, calls={}, last={}",
            self.function_name, combined_total, combined_calls, combined_last
        );

        match storage::upsert_metric(
            &self.function_name,
            combined_total,
            combined_calls,
            combined_last,
        ) {
            Ok(_) => info!(
                "Successfully persisted metrics for '{}'",
                self.function_name
            ),
            Err(e) => error!(
                "Failed to persist metrics for '{}': {}",
                self.function_name, e
            ),
        }
    }
}

fn function_artifact_exists(function_name: &str) -> bool {
    storage::artifact_exists(function_name).unwrap_or(false)
}

pub fn get_metrics() -> Metrics {
    info!("Retrieving metrics from database...");
    let mut function_metrics = Vec::new();
    let mut total_time = 0;
    let mut total_calls = 0;

    let metric_rows = storage::iter_metrics().unwrap_or_default();
    info!("Found {} entries in metrics database", metric_rows.len());

    for (function_name, db_total_time, db_call_count, db_last_called) in metric_rows {
        info!(
            "DB metrics for '{}': total={}ms, calls={}, last={}",
            function_name, db_total_time, db_call_count, db_last_called
        );

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

        let combined_total_time = db_total_time.saturating_add(mem_total_time);
        let combined_call_count = db_call_count.saturating_add(mem_call_count);
        let combined_last_called = std::cmp::max(db_last_called, mem_last_called);
        let last_called_time = UNIX_EPOCH + Duration::from_millis(combined_last_called);
        let last_called_str = chrono::DateTime::<chrono::Utc>::from(last_called_time).to_rfc3339();

        function_metrics.push(FunctionMetricsResponse {
            function_name: function_name.clone(),
            total_time_millis: combined_total_time,
            call_count: combined_call_count,
            last_called: last_called_str,
        });

        total_time += combined_total_time;
        total_calls += combined_call_count;
    }

    Metrics {
        total_time,
        total_calls,
        function_metrics,
    }
}

pub fn get_or_create_metric(function_name: &str) -> Option<FunctionMetric> {
    let entry = FUNCTION_METRICS.entry(function_name.to_string());

    match entry {
        dashmap::mapref::entry::Entry::Occupied(occupied) => {
            Some(FunctionMetric::new(occupied.key().clone()))
        }
        dashmap::mapref::entry::Entry::Vacant(vacant) => {
            if !function_artifact_exists(function_name) {
                return None;
            }

            let metric = FunctionMetric::new(function_name.to_string());
            vacant.insert(metric.clone());

            if !storage::metric_exists(function_name).unwrap_or(false) {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or(Duration::from_secs(0))
                    .as_millis() as u64;
                let _ = storage::upsert_metric(function_name, 0, 0, now);
            }

            Some(metric)
        }
    }
}

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
            let duration_ms = duration.as_millis() as u64;
            let rounded_duration = std::cmp::max(duration_ms, 1);
            metric.record_call(rounded_duration);
        }
    }
}

pub fn flush_metrics_to_db() {
    info!("Flushing metrics to database...");
    let mut flushed_count = 0;

    for entry in FUNCTION_METRICS.iter() {
        let metric = entry.value();
        let call_count = metric.call_count.load(Ordering::Relaxed);

        if call_count == 0 {
            continue;
        }

        metric.flush_to_db();
        metric.total_time.store(0, Ordering::Relaxed);
        metric.call_count.store(0, Ordering::Relaxed);
        flushed_count += 1;
    }

    if flushed_count > 0
        && let Err(err) = storage::flush_metrics()
    {
        error!("Failed to flush metrics DB: {}", err);
    }
}

pub fn spawn_periodic_flush(interval_secs: u64) {
    tokio::spawn(async move {
        let mut ticker = time::interval(Duration::from_secs(interval_secs));
        loop {
            ticker.tick().await;
            flush_metrics_to_db();
        }
    });
}
