use axum::Json;
use crate::LIB_CACHE;
use axum::response::IntoResponse;
use serde::Serialize;

#[derive(Debug, Serialize)]
struct Metrics {
    // pub total_requests: u64,
    // pub total_errors: u64,
    // pub total_success: u64,
    total_time: u64,
    function_metrics: Vec<FunctionMetrics>,
}

#[derive(Debug, Serialize)]
struct FunctionMetrics {
    function_name: String,
    total_time_millis: u64,
}

pub async fn get_metrics() -> impl IntoResponse {
    let (function_metrics, total_time) =
        LIB_CACHE
            .iter()
            .fold((Vec::new(), 0u64), |(mut metrics, sum), entry| {
                let time = entry.value().usage_count.load(std::sync::atomic::Ordering::Relaxed) as u64;
                metrics.push(FunctionMetrics {
                    function_name: entry.key().to_string(),
                    total_time_millis: time,
                });
                (metrics, sum + time)
            });

    Json(Metrics {
        total_time,
        function_metrics,
    })
}
