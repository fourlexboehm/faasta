use anyhow::{Context, Result, anyhow, bail};
use bincode::{Decode, Encode};
use std::path::Path;

use crate::kvm_guest;

const FUNCTIONS_DB_TREE: &str = "functions";
const USER_DB_TREE: &str = "user_data";
const STORAGE_BUFFER_SIZE: usize = 8 * 1024 * 1024;

#[derive(Debug, Encode, Decode)]
enum StorageRequest {
    GetFunction {
        name: String,
    },
    PutFunction {
        name: String,
        data: Vec<u8>,
    },
    DeleteFunction {
        name: String,
    },
    GetUser {
        username: String,
    },
    PutUser {
        username: String,
        data: Vec<u8>,
    },
    GetMetric {
        function_name: String,
    },
    UpsertMetric {
        function_name: String,
        total_time: u64,
        call_count: u64,
        last_called: u64,
    },
    MetricExists {
        function_name: String,
    },
    IterMetrics,
    FlushMetrics,
}

#[derive(Debug, Encode, Decode)]
enum StorageReply {
    Unit,
    Bytes(Option<Vec<u8>>),
    Metric(Option<(u64, u64, u64)>),
    MetricExists(bool),
    Metrics(Vec<(String, u64, u64, u64)>),
    Error(String),
}

pub fn get_function(name: &str) -> Result<Option<Vec<u8>>> {
    match call(StorageRequest::GetFunction {
        name: name.to_string(),
    })? {
        StorageReply::Bytes(value) => Ok(value),
        other => unexpected_reply("Bytes", other),
    }
}

pub fn put_function(name: &str, data: &[u8]) -> Result<()> {
    expect_unit(call(StorageRequest::PutFunction {
        name: name.to_string(),
        data: data.to_vec(),
    })?)
}

pub fn delete_function(name: &str) -> Result<()> {
    expect_unit(call(StorageRequest::DeleteFunction {
        name: name.to_string(),
    })?)
}

pub fn get_user(username: &str) -> Result<Option<Vec<u8>>> {
    match call(StorageRequest::GetUser {
        username: username.to_string(),
    })? {
        StorageReply::Bytes(value) => Ok(value),
        other => unexpected_reply("Bytes", other),
    }
}

pub fn put_user(username: &str, data: &[u8]) -> Result<()> {
    expect_unit(call(StorageRequest::PutUser {
        username: username.to_string(),
        data: data.to_vec(),
    })?)
}

pub fn get_metric(function_name: &str) -> Result<Option<(u64, u64, u64)>> {
    match call(StorageRequest::GetMetric {
        function_name: function_name.to_string(),
    })? {
        StorageReply::Metric(value) => Ok(value),
        other => unexpected_reply("Metric", other),
    }
}

pub fn upsert_metric(
    function_name: &str,
    total_time: u64,
    call_count: u64,
    last_called: u64,
) -> Result<()> {
    expect_unit(call(StorageRequest::UpsertMetric {
        function_name: function_name.to_string(),
        total_time,
        call_count,
        last_called,
    })?)
}

pub fn metric_exists(function_name: &str) -> Result<bool> {
    match call(StorageRequest::MetricExists {
        function_name: function_name.to_string(),
    })? {
        StorageReply::MetricExists(value) => Ok(value),
        other => unexpected_reply("MetricExists", other),
    }
}

pub fn iter_metrics() -> Result<Vec<(String, u64, u64, u64)>> {
    match call(StorageRequest::IterMetrics)? {
        StorageReply::Metrics(metrics) => Ok(metrics),
        other => unexpected_reply("Metrics", other),
    }
}

pub fn flush_metrics() -> Result<()> {
    expect_unit(call(StorageRequest::FlushMetrics)?)
}

fn call(request: StorageRequest) -> Result<StorageReply> {
    let payload = bincode::encode_to_vec(request, bincode::config::standard())
        .context("failed to encode storage request")?;
    if payload.len() > STORAGE_BUFFER_SIZE {
        bail!("storage request too large: {} bytes", payload.len());
    }

    let mut buffer = vec![0u8; STORAGE_BUFFER_SIZE];
    buffer[..payload.len()].copy_from_slice(&payload);

    let response_len =
        kvm_guest::remote_resume(&mut buffer, payload.len()).map_err(|code| anyhow!("{code}"))?;
    let (reply, _) = bincode::decode_from_slice::<StorageReply, _>(
        &buffer[..response_len],
        bincode::config::standard(),
    )
    .context("failed to decode storage response")?;

    if let StorageReply::Error(message) = &reply {
        bail!("{message}");
    }
    Ok(reply)
}

fn expect_unit(reply: StorageReply) -> Result<()> {
    match reply {
        StorageReply::Unit => Ok(()),
        other => unexpected_reply("Unit", other),
    }
}

fn unexpected_reply<T>(expected: &str, reply: StorageReply) -> Result<T> {
    Err(anyhow!(
        "unexpected storage reply: expected {expected}, got {reply:?}"
    ))
}

pub fn run_storage_vm(db_path: &Path, metrics_db_path: &Path) -> Result<()> {
    ensure_dir(db_path).with_context(|| format!("failed to create db dir at {:?}", db_path))?;
    ensure_dir(metrics_db_path)
        .with_context(|| format!("failed to create metrics dir at {:?}", metrics_db_path))?;

    let metadata_db = sled::open(db_path).context("failed to open metadata sled db")?;
    let metrics_db = sled::open(metrics_db_path).context("failed to open metrics sled db")?;
    let functions_tree = metadata_db
        .open_tree(FUNCTIONS_DB_TREE)
        .context("failed to open functions tree")?;
    let user_tree = metadata_db
        .open_tree(USER_DB_TREE)
        .context("failed to open user tree")?;

    let mut return_value = 0isize;
    let mut storage = kvm_guest::storage().context("storage VM is unavailable")?;

    loop {
        return_value = match storage.wait_paused(return_value) {
            Err(code) => code,
            Ok(None) => 0,
            Ok(Some(buffer)) => {
                handle_storage_request(buffer, &functions_tree, &user_tree, &metrics_db)
            }
        };
    }
}

fn handle_storage_request(
    buffer: &mut [u8],
    functions_tree: &sled::Tree,
    user_tree: &sled::Tree,
    metrics_db: &sled::Db,
) -> isize {
    let request =
        bincode::decode_from_slice::<StorageRequest, _>(buffer, bincode::config::standard())
            .map(|(req, _)| req);

    let reply = match request {
        Ok(req) => process_request(req, functions_tree, user_tree, metrics_db),
        Err(err) => StorageReply::Error(format!("failed to decode storage request: {err}")),
    };

    let encoded = match bincode::encode_to_vec(reply, bincode::config::standard()) {
        Ok(encoded) => encoded,
        Err(err) => {
            let fallback = StorageReply::Error(format!("failed to encode storage response: {err}"));
            match bincode::encode_to_vec(fallback, bincode::config::standard()) {
                Ok(encoded) => encoded,
                Err(_) => return -1,
            }
        }
    };

    if encoded.len() > buffer.len() {
        return -1;
    }

    buffer[..encoded.len()].copy_from_slice(&encoded);
    encoded.len() as isize
}

fn process_request(
    request: StorageRequest,
    functions_tree: &sled::Tree,
    user_tree: &sled::Tree,
    metrics_db: &sled::Db,
) -> StorageReply {
    let result: Result<StorageReply> = match request {
        StorageRequest::GetFunction { name } => functions_tree
            .get(name.as_bytes())
            .context("failed to get function metadata")
            .map(|value| StorageReply::Bytes(value.map(|v| v.to_vec()))),
        StorageRequest::PutFunction { name, data } => functions_tree
            .insert(name.as_bytes(), data)
            .context("failed to persist function metadata")
            .map(|_| StorageReply::Unit),
        StorageRequest::DeleteFunction { name } => functions_tree
            .remove(name.as_bytes())
            .context("failed to delete function metadata")
            .map(|_| StorageReply::Unit),
        StorageRequest::GetUser { username } => user_tree
            .get(username.as_bytes())
            .context("failed to get user data")
            .map(|value| StorageReply::Bytes(value.map(|v| v.to_vec()))),
        StorageRequest::PutUser { username, data } => user_tree
            .insert(username.as_bytes(), data)
            .context("failed to persist user data")
            .map(|_| StorageReply::Unit),
        StorageRequest::GetMetric { function_name } => metrics_db
            .get(function_name.as_bytes())
            .context("failed to get metric")
            .map(|value| StorageReply::Metric(decode_metric_value(value))),
        StorageRequest::UpsertMetric {
            function_name,
            total_time,
            call_count,
            last_called,
        } => bincode::encode_to_vec(
            (total_time, call_count, last_called),
            bincode::config::standard(),
        )
        .context("failed to encode metric")
        .and_then(|value| {
            metrics_db
                .insert(function_name.as_bytes(), value)
                .map(|_| ())
                .context("failed to persist metric")
        })
        .map(|_| StorageReply::Unit),
        StorageRequest::MetricExists { function_name } => metrics_db
            .contains_key(function_name.as_bytes())
            .context("failed to check metric existence")
            .map(StorageReply::MetricExists),
        StorageRequest::IterMetrics => iter_metrics_from_db(metrics_db).map(StorageReply::Metrics),
        StorageRequest::FlushMetrics => metrics_db
            .flush()
            .context("failed to flush metrics db")
            .map(|_| StorageReply::Unit),
    };

    match result {
        Ok(reply) => reply,
        Err(err) => StorageReply::Error(err.to_string()),
    }
}

fn iter_metrics_from_db(db: &sled::Db) -> Result<Vec<(String, u64, u64, u64)>> {
    let mut metrics = Vec::new();
    for entry in db.iter() {
        let (key, value) = entry?;
        let function_name =
            String::from_utf8(key.to_vec()).context("metric key was not valid utf-8")?;
        if let Some((total_time, call_count, last_called)) = decode_metric_value(Some(value)) {
            metrics.push((function_name, total_time, call_count, last_called));
        }
    }
    Ok(metrics)
}

fn decode_metric_value(value: Option<sled::IVec>) -> Option<(u64, u64, u64)> {
    value.and_then(|encoded| {
        bincode::decode_from_slice::<(u64, u64, u64), _>(&encoded, bincode::config::standard())
            .ok()
            .map(|(metric, _)| metric)
    })
}

fn ensure_dir(path: &Path) -> Result<()> {
    match std::fs::create_dir_all(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(err) => Err(err.into()),
    }
}
