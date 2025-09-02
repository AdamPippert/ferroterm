use rusqlite::{params, Connection, Result as SqlResult};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::Mutex;

#[derive(Error, Debug)]
pub enum TelemetryError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TelemetryCategory {
    Performance,
    Crash,
    Usage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryEvent {
    pub timestamp: u64,
    pub category: TelemetryCategory,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    pub enabled: bool,
    pub performance: bool,
    pub crash: bool,
    pub usage: bool,
    pub db_path: PathBuf,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            performance: false,
            crash: false,
            usage: false,
            db_path: dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".pachyterm")
                .join("telemetry.db"),
        }
    }
}

pub struct TelemetryManager {
    config: TelemetryConfig,
    conn: Arc<Mutex<Connection>>,
}

impl TelemetryManager {
    pub async fn new(config: TelemetryConfig) -> Result<Self, TelemetryError> {
        std::fs::create_dir_all(config.db_path.parent().unwrap())?;
        let conn = Connection::open(&config.db_path)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS events (
                id INTEGER PRIMARY KEY,
                ts INTEGER NOT NULL,
                category TEXT NOT NULL,
                payload TEXT NOT NULL
            )",
            [],
        )?;
        Ok(Self {
            config,
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub async fn log_event(
        &self,
        category: TelemetryCategory,
        payload: serde_json::Value,
    ) -> Result<(), TelemetryError> {
        if !self.config.enabled {
            return Ok(());
        }
        match category {
            TelemetryCategory::Performance if !self.config.performance => return Ok(()),
            TelemetryCategory::Crash if !self.config.crash => return Ok(()),
            TelemetryCategory::Usage if !self.config.usage => return Ok(()),
            _ => {}
        }
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let payload_str = serde_json::to_string(&payload)?;
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO events (ts, category, payload) VALUES (?1, ?2, ?3)",
            params![ts, serde_json::to_string(&category)?, payload_str],
        )?;
        Ok(())
    }

    pub async fn push_events(&self) -> Result<(), TelemetryError> {
        // Placeholder for pushing to remote server
        // In real implementation, this would send queued events
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare("SELECT id, ts, category, payload FROM events")?;
        let events = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        for event in events {
            let (_id, _ts, _category, _payload) = event?;
            // Send to server (not implemented)
        }
        // Clear events after push
        conn.execute("DELETE FROM events", [])?;
        Ok(())
    }

    pub async fn get_queued_count(&self) -> Result<i64, TelemetryError> {
        let conn = self.conn.lock().await;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_telemetry_disabled() {
        let dir = tempdir().unwrap();
        let config = TelemetryConfig {
            enabled: false,
            ..Default::default()
        };
        let manager = TelemetryManager::new(config).await.unwrap();
        manager
            .log_event(
                TelemetryCategory::Usage,
                serde_json::json!({"test": "data"}),
            )
            .await
            .unwrap();
        assert_eq!(manager.get_queued_count().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_telemetry_enabled() {
        let dir = tempdir().unwrap();
        let config = TelemetryConfig {
            enabled: true,
            usage: true,
            db_path: dir.path().join("test.db"),
            ..Default::default()
        };
        let manager = TelemetryManager::new(config).await.unwrap();
        manager
            .log_event(
                TelemetryCategory::Usage,
                serde_json::json!({"test": "data"}),
            )
            .await
            .unwrap();
        assert_eq!(manager.get_queued_count().await.unwrap(), 1);
    }
}
