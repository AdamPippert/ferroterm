use serde::{Deserialize, Serialize};
use std::process::Command;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum OsAgentError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Command error: {0}")]
    Command(String),
    #[error("Parse error: {0}")]
    Parse(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsInfo {
    pub os: String,
    pub arch: String,
    pub hostname: String,
    pub username: String,
    pub memory_total: u64,
    pub memory_free: u64,
    pub cpu_count: usize,
    pub gpu_info: Option<String>,
}

impl OsInfo {
    pub fn collect() -> Result<Self, OsAgentError> {
        let os = std::env::consts::OS.to_string();
        let arch = std::env::consts::ARCH.to_string();
        let hostname = gethostname::gethostname()
            .to_str()
            .unwrap_or("unknown")
            .to_string();
        let username = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());
        let cpu_count = num_cpus::get();

        let (memory_total, memory_free) = Self::get_memory_info()?;
        let gpu_info = Self::get_gpu_info().ok();

        Ok(Self {
            os,
            arch,
            hostname,
            username,
            memory_total,
            memory_free,
            cpu_count,
            gpu_info,
        })
    }

    fn get_memory_info() -> Result<(u64, u64), OsAgentError> {
        if cfg!(target_os = "linux") {
            let output = Command::new("free")
                .arg("-b")
                .output()
                .map_err(|e| OsAgentError::Command(e.to_string()))?;
            let output_str =
                String::from_utf8(output.stdout).map_err(|e| OsAgentError::Parse(e.to_string()))?;
            let lines: Vec<&str> = output_str.lines().collect();
            if lines.len() > 1 {
                let mem_line = lines[1];
                let parts: Vec<&str> = mem_line.split_whitespace().collect();
                if parts.len() >= 4 {
                    let total = parts[1]
                        .parse()
                        .map_err(|_| OsAgentError::Parse("total".to_string()))?;
                    let free = parts[3]
                        .parse()
                        .map_err(|_| OsAgentError::Parse("free".to_string()))?;
                    return Ok((total, free));
                }
            }
        }
        // Fallback
        Ok((0, 0))
    }

    fn get_gpu_info() -> Result<String, OsAgentError> {
        if cfg!(target_os = "linux") {
            let output = Command::new("lspci")
                .output()
                .map_err(|e| OsAgentError::Command(e.to_string()))?;
            let output_str =
                String::from_utf8(output.stdout).map_err(|e| OsAgentError::Parse(e.to_string()))?;
            if output_str.contains("VGA") {
                return Ok("GPU detected".to_string());
            }
        }
        Ok("No GPU info".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collect_os_info() {
        let info = OsInfo::collect().unwrap();
        assert!(!info.os.is_empty());
        assert!(!info.arch.is_empty());
        assert!(!info.hostname.is_empty());
        assert!(info.cpu_count > 0);
    }
}
