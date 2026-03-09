use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn open(base_path: &Path) -> Result<Self> {
        let db_path = sqlite_path(base_path, "faasta.sqlite3");
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create sqlite parent dir {:?}", parent))?;
        }

        let conn =
            Connection::open(&db_path).with_context(|| format!("failed to open {:?}", db_path))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        let db = Self {
            conn: Mutex::new(conn),
        };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<()> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS functions (
                name TEXT PRIMARY KEY,
                data BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS user_data (
                username TEXT PRIMARY KEY,
                data BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS metrics (
                function_name TEXT PRIMARY KEY,
                total_time INTEGER NOT NULL,
                call_count INTEGER NOT NULL,
                last_called INTEGER NOT NULL
            );",
        )?;
        Ok(())
    }

    pub fn get_function(&self, name: &str) -> Result<Option<Vec<u8>>> {
        self.get_blob("SELECT data FROM functions WHERE name = ?1", name)
    }

    pub fn put_function(&self, name: &str, data: &[u8]) -> Result<()> {
        self.put_blob(
            "INSERT INTO functions(name, data) VALUES (?1, ?2)
             ON CONFLICT(name) DO UPDATE SET data = excluded.data",
            name,
            data,
        )
    }

    pub fn delete_function(&self, name: &str) -> Result<()> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute("DELETE FROM functions WHERE name = ?1", params![name])?;
        Ok(())
    }

    pub fn put_user(&self, username: &str, data: &[u8]) -> Result<()> {
        self.put_blob(
            "INSERT INTO user_data(username, data) VALUES (?1, ?2)
             ON CONFLICT(username) DO UPDATE SET data = excluded.data",
            username,
            data,
        )
    }

    pub fn iter_users(&self) -> Result<Vec<(String, Vec<u8>)>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut stmt = conn.prepare("SELECT username, data FROM user_data")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
    }

    pub fn get_metric(&self, function_name: &str) -> Result<Option<(u64, u64, u64)>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.query_row(
            "SELECT total_time, call_count, last_called FROM metrics WHERE function_name = ?1",
            params![function_name],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn upsert_metric(
        &self,
        function_name: &str,
        total_time: u64,
        call_count: u64,
        last_called: u64,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            "INSERT INTO metrics(function_name, total_time, call_count, last_called)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(function_name) DO UPDATE SET
                total_time = excluded.total_time,
                call_count = excluded.call_count,
                last_called = excluded.last_called",
            params![function_name, total_time, call_count, last_called],
        )?;
        Ok(())
    }

    pub fn metric_exists(&self, function_name: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let exists = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM metrics WHERE function_name = ?1)",
            params![function_name],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(exists != 0)
    }

    pub fn iter_metrics(&self) -> Result<Vec<(String, u64, u64, u64)>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut stmt =
            conn.prepare("SELECT function_name, total_time, call_count, last_called FROM metrics")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
    }

    pub fn flush(&self) -> Result<()> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);")?;
        Ok(())
    }

    fn get_blob(&self, sql: &str, key: &str) -> Result<Option<Vec<u8>>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.query_row(sql, params![key], |row| row.get(0))
            .optional()
            .map_err(Into::into)
    }

    fn put_blob(&self, sql: &str, key: &str, data: &[u8]) -> Result<()> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(sql, params![key, data])?;
        Ok(())
    }
}

fn sqlite_path(base_path: &Path, default_name: &str) -> PathBuf {
    if base_path.extension().is_some() {
        base_path.to_path_buf()
    } else {
        base_path.join(default_name)
    }
}
