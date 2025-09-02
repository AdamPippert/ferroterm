use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::process::{Child, Command as TokioCommand};
use tokio::sync::RwLock;
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

#[derive(Error, Debug)]
pub enum OciError {
    #[error("Container runtime error: {0}")]
    Runtime(String),
    #[error("Container creation failed: {0}")]
    Creation(String),
    #[error("Container execution failed: {0}")]
    Execution(String),
    #[error("Model loading failed: {0}")]
    ModelLoad(String),
    #[error("Resource allocation failed: {0}")]
    ResourceAllocation(String),
    #[error("Security violation: {0}")]
    Security(String),
    #[error("Timeout: operation took longer than {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("Container not found: {id}")]
    ContainerNotFound { id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerConfig {
    pub image: String,
    pub command: Vec<String>,
    pub env: HashMap<String, String>,
    pub working_dir: Option<String>,
    pub memory_limit: Option<String>,
    pub cpu_limit: Option<String>,
    pub gpu_enabled: bool,
    pub rootless: bool,
    pub seccomp_profile: Option<String>,
    pub volumes: Vec<VolumeMount>,
}

impl Default for ContainerConfig {
    fn default() -> Self {
        Self {
            image: "quay.io/pachyterm/ramalama:latest".to_string(),
            command: vec!["/bin/bash".to_string()],
            env: HashMap::new(),
            working_dir: Some("/workspace".to_string()),
            memory_limit: Some("2G".to_string()),
            cpu_limit: Some("1.0".to_string()),
            gpu_enabled: false,
            rootless: true,
            seccomp_profile: Some("default".to_string()),
            volumes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeMount {
    pub host_path: String,
    pub container_path: String,
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub name: String,
    pub size: String,         // e.g., "7B", "13B"
    pub quantization: String, // e.g., "q4_0", "q8_0"
    pub context_window: u32,
    pub gpu_layers: Option<u32>,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            name: "mistral-7b-instruct".to_string(),
            size: "7B".to_string(),
            quantization: "q4_0".to_string(),
            context_window: 4096,
            gpu_layers: Some(35),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    pub container: ContainerConfig,
    pub model: ModelConfig,
    pub session_id: String,
    pub log_directory: PathBuf,
    pub timeout_ms: u64,
}

impl SessionConfig {
    pub fn new(session_id: String) -> Self {
        let log_directory = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".pachyterm")
            .join("sessions")
            .join(&session_id);

        Self {
            container: ContainerConfig::default(),
            model: ModelConfig::default(),
            session_id,
            log_directory,
            timeout_ms: 30000, // 30 seconds
        }
    }
}

#[derive(Debug)]
pub struct ContainerSession {
    pub id: String,
    pub container_id: Option<String>,
    pub process: Option<Child>,
    pub config: SessionConfig,
    pub created_at: Instant,
    pub model_loaded: bool,
    pub model_load_time: Option<Duration>,
    pub logs: Vec<String>,
}

impl ContainerSession {
    pub fn new(config: SessionConfig) -> Self {
        Self {
            id: config.session_id.clone(),
            container_id: None,
            process: None,
            config,
            created_at: Instant::now(),
            model_loaded: false,
            model_load_time: None,
            logs: Vec::new(),
        }
    }

    pub fn is_running(&self) -> bool {
        if let Some(ref process) = self.process {
            process.id().is_some()
        } else {
            false
        }
    }

    pub fn uptime(&self) -> Duration {
        self.created_at.elapsed()
    }

    pub fn add_log(&mut self, log: String) {
        self.logs.push(log);
        // Keep only last 1000 log entries
        if self.logs.len() > 1000 {
            self.logs.remove(0);
        }
    }
}

pub struct OciLauncher {
    runtime_path: PathBuf,
    sessions: Arc<RwLock<HashMap<String, ContainerSession>>>,
    base_image_cache: Arc<RwLock<HashMap<String, String>>>,
}

impl OciLauncher {
    pub fn new() -> Result<Self, OciError> {
        let runtime_path = Self::find_runtime_path()?;

        Ok(Self {
            runtime_path,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            base_image_cache: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    fn find_runtime_path() -> Result<PathBuf, OciError> {
        // Try common runtime locations
        let candidates = [
            "/usr/bin/crun",
            "/usr/local/bin/crun",
            "/usr/bin/podman",
            "/usr/local/bin/podman",
            "/usr/bin/docker",
            "/usr/local/bin/docker",
        ];

        for candidate in &candidates {
            let path = PathBuf::from(candidate);
            if path.exists() && path.is_file() {
                return Ok(path);
            }
        }

        // Check PATH
        if let Ok(output) = Command::new("which").arg("crun").output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                return Ok(PathBuf::from(path));
            }
        }

        Err(OciError::Runtime(
            "No suitable container runtime found. Please install crun, podman, or docker."
                .to_string(),
        ))
    }

    pub async fn create_session(&self, config: SessionConfig) -> Result<String, OciError> {
        let session_id = config.session_id.clone();
        let mut session = ContainerSession::new(config);

        // Ensure log directory exists
        std::fs::create_dir_all(&session.config.log_directory)?;

        // Pull base image if not cached
        self.ensure_image_available(&session.config.container.image)
            .await?;

        // Create container
        let container_id = self.create_container(&mut session).await?;
        session.container_id = Some(container_id);

        // Start container
        self.start_container(&mut session).await?;

        // Wait for model to load
        self.wait_for_model_ready(&mut session).await?;

        // Store session
        self.sessions
            .write()
            .await
            .insert(session_id.clone(), session);

        info!("Created OCI session: {}", session_id);
        Ok(session_id)
    }

    async fn ensure_image_available(&self, image: &str) -> Result<(), OciError> {
        let cache = self.base_image_cache.read().await;
        if cache.contains_key(image) {
            return Ok(());
        }
        drop(cache);

        info!("Pulling container image: {}", image);

        let start = Instant::now();
        let output = TokioCommand::new(&self.runtime_path)
            .args(&["pull", image])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(OciError::Runtime(format!(
                "Failed to pull image {}: {}",
                image, stderr
            )));
        }

        let pull_time = start.elapsed();
        info!("Image {} pulled in {:?}", image, pull_time);

        // Cache the image
        self.base_image_cache
            .write()
            .await
            .insert(image.to_string(), image.to_string());

        Ok(())
    }

    async fn create_container(&self, session: &mut ContainerSession) -> Result<String, OciError> {
        let container_id = format!("pachyterm-{}", session.id);

        // Build container creation arguments
        let mut args = vec![
            "create".to_string(),
            "--name".to_string(),
            container_id.clone(),
        ];

        // Add rootless flag
        if session.config.container.rootless {
            args.push("--rootless".to_string());
        }

        // Add memory limit
        if let Some(ref memory) = session.config.container.memory_limit {
            args.extend_from_slice(&["--memory".to_string(), memory.clone()]);
        }

        // Add CPU limit
        if let Some(ref cpu) = session.config.container.cpu_limit {
            args.extend_from_slice(&["--cpus".to_string(), cpu.clone()]);
        }

        // Add GPU support
        if session.config.container.gpu_enabled {
            args.push("--device".to_string());
            args.push("/dev/nvidia0".to_string());
            args.push("--device".to_string());
            args.push("/dev/nvidiactl".to_string());
            args.push("--device".to_string());
            args.push("/dev/nvidia-uvm".to_string());
        }

        // Add volume mounts
        for volume in &session.config.container.volumes {
            let mount_spec = if volume.read_only {
                format!("{}:{}:ro", volume.host_path, volume.container_path)
            } else {
                format!("{}:{}", volume.host_path, volume.container_path)
            };
            args.extend_from_slice(&["--volume".to_string(), mount_spec]);
        }

        // Add working directory
        if let Some(ref workdir) = session.config.container.working_dir {
            args.extend_from_slice(&["--workdir".to_string(), workdir.clone()]);
        }

        // Add environment variables
        for (key, value) in &session.config.container.env {
            args.extend_from_slice(&["--env".to_string(), format!("{}={}", key, value)]);
        }

        // Add model-specific environment variables
        args.extend_from_slice(&[
            "--env".to_string(),
            format!("MODEL_NAME={}", session.config.model.name),
        ]);
        args.extend_from_slice(&[
            "--env".to_string(),
            format!("MODEL_SIZE={}", session.config.model.size),
        ]);
        args.extend_from_slice(&[
            "--env".to_string(),
            format!("QUANTIZATION={}", session.config.model.quantization),
        ]);
        args.extend_from_slice(&[
            "--env".to_string(),
            format!("CONTEXT_WINDOW={}", session.config.model.context_window),
        ]);

        if let Some(gpu_layers) = session.config.model.gpu_layers {
            args.extend_from_slice(&["--env".to_string(), format!("GPU_LAYERS={}", gpu_layers)]);
        }

        // Add image
        args.push(session.config.container.image.clone());

        // Add command
        args.extend(session.config.container.command.clone());

        debug!("Creating container with args: {:?}", args);

        let output = TokioCommand::new(&self.runtime_path)
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            session.add_log(format!("Container creation failed: {}", stderr));
            return Err(OciError::Creation(format!(
                "Failed to create container: {}",
                stderr
            )));
        }

        info!("Created container: {}", container_id);
        Ok(container_id)
    }

    async fn start_container(&self, session: &mut ContainerSession) -> Result<(), OciError> {
        let container_id = session
            .container_id
            .as_ref()
            .ok_or_else(|| OciError::ContainerNotFound {
                id: session.id.clone(),
            })?
            .clone();

        let output = TokioCommand::new(&self.runtime_path)
            .args(&["start", &container_id])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            session.add_log(format!("Container start failed: {}", stderr));
            return Err(OciError::Execution(format!(
                "Failed to start container {}: {}",
                container_id, stderr
            )));
        }

        // Get the container's PID for process monitoring
        let pid_output = TokioCommand::new(&self.runtime_path)
            .args(&["inspect", "--format", "{{.State.Pid}}", &container_id])
            .stdout(Stdio::piped())
            .output()
            .await?;

        if pid_output.status.success() {
            let pid_str_owned = String::from_utf8_lossy(&pid_output.stdout).to_string();
            let pid_str = pid_str_owned.trim();
            if let Ok(pid) = pid_str.parse::<u32>() {
                // In a real implementation, we'd monitor this process
                debug!("Container {} running with PID {}", container_id, pid);
            }
        }

        info!("Started container: {}", container_id);
        Ok(())
    }

    async fn wait_for_model_ready(&self, session: &mut ContainerSession) -> Result<(), OciError> {
        let container_id = session
            .container_id
            .as_ref()
            .ok_or_else(|| OciError::ContainerNotFound {
                id: session.id.clone(),
            })?
            .clone();

        info!("Waiting for model to load in container: {}", container_id);

        let start = Instant::now();
        let timeout_duration = Duration::from_millis(session.config.timeout_ms);

        // Poll container logs for model ready signal
        let ready_future = async {
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;

                let output = TokioCommand::new(&self.runtime_path)
                    .args(&["logs", &container_id])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .output()
                    .await;

                match output {
                    Ok(output) if output.status.success() => {
                        let logs = String::from_utf8_lossy(&output.stdout);
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        let all_logs = format!("{}{}", logs, stderr);

                        // Check for model ready indicators
                        if all_logs.contains("Model loaded successfully")
                            || all_logs.contains("vLLM model ready")
                            || all_logs.contains("RamaLama ready")
                        {
                            return Ok(());
                        }

                        // Check for errors
                        if all_logs.contains("Model loading failed")
                            || all_logs.contains("CUDA error")
                            || all_logs.contains("Out of memory")
                        {
                            return Err(OciError::ModelLoad("Model loading failed".to_string()));
                        }
                    }
                    Ok(_) => {} // Command failed, continue polling
                    Err(e) => {
                        warn!("Failed to get container logs: {}", e);
                    }
                }
            }
        };

        match timeout(timeout_duration, ready_future).await {
            Ok(result) => {
                result?;
                let load_time = start.elapsed();
                session.model_loaded = true;
                session.model_load_time = Some(load_time);
                session.add_log(format!("Model loaded in {:?}", load_time));
                info!(
                    "Model loaded in container {} in {:?}",
                    container_id, load_time
                );
                Ok(())
            }
            Err(_) => {
                let elapsed = start.elapsed();
                session.add_log(format!("Model loading timeout after {:?}", elapsed));
                Err(OciError::Timeout {
                    timeout_ms: session.config.timeout_ms,
                })
            }
        }
    }

    pub async fn destroy_session(&self, session_id: &str) -> Result<(), OciError> {
        let mut sessions = self.sessions.write().await;
        if let Some(mut session) = sessions.remove(session_id) {
            if let Some(ref container_id) = session.container_id {
                info!("Destroying container: {}", container_id);

                // Stop container
                let _ = TokioCommand::new(&self.runtime_path)
                    .args(&["stop", "--time", "5", container_id])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .await;

                // Remove container
                let _ = TokioCommand::new(&self.runtime_path)
                    .args(&["rm", "--force", container_id])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .await;
            }

            // Save logs
            self.save_session_logs(&session).await?;
        }

        Ok(())
    }

    async fn save_session_logs(&self, session: &ContainerSession) -> Result<(), OciError> {
        let log_file = session.config.log_directory.join("session.log");
        let log_content = session.logs.join("\n");

        std::fs::write(log_file, log_content)?;
        Ok(())
    }

    pub async fn list_sessions(&self) -> Vec<String> {
        self.sessions.read().await.keys().cloned().collect()
    }

    pub async fn get_session_info(&self, session_id: &str) -> Option<ContainerSession> {
        self.sessions
            .read()
            .await
            .get(session_id)
            .map(|s| ContainerSession {
                id: s.id.clone(),
                container_id: s.container_id.clone(),
                process: None, // Don't clone the process
                config: s.config.clone(),
                created_at: s.created_at,
                model_loaded: s.model_loaded,
                model_load_time: s.model_load_time,
                logs: s.logs.clone(),
            })
    }

    pub async fn execute_in_session(
        &self,
        session_id: &str,
        command: &str,
    ) -> Result<String, OciError> {
        let sessions = self.sessions.read().await;
        let session = sessions
            .get(session_id)
            .ok_or_else(|| OciError::ContainerNotFound {
                id: session_id.to_string(),
            })?;

        let container_id =
            session
                .container_id
                .as_ref()
                .ok_or_else(|| OciError::ContainerNotFound {
                    id: session_id.to_string(),
                })?;

        let output = TokioCommand::new(&self.runtime_path)
            .args(&["exec", container_id, "sh", "-c", command])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            Ok(stdout)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Err(OciError::Execution(format!("Command failed: {}", stderr)))
        }
    }

    pub async fn get_session_stats(&self, session_id: &str) -> Result<SessionStats, OciError> {
        let sessions = self.sessions.read().await;
        let session = sessions
            .get(session_id)
            .ok_or_else(|| OciError::ContainerNotFound {
                id: session_id.to_string(),
            })?;

        let container_id =
            session
                .container_id
                .as_ref()
                .ok_or_else(|| OciError::ContainerNotFound {
                    id: session_id.to_string(),
                })?;

        // Get container stats
        let output = TokioCommand::new(&self.runtime_path)
            .args(&["stats", "--format", "json", container_id])
            .stdout(Stdio::piped())
            .output()
            .await?;

        let stats = if output.status.success() {
            let json_str = String::from_utf8_lossy(&output.stdout);
            serde_json::from_str(&json_str).unwrap_or_default()
        } else {
            SessionStats::default()
        };

        Ok(stats)
    }

    pub async fn cleanup_orphaned_containers(&self) -> Result<(), OciError> {
        info!("Cleaning up orphaned containers");

        let output = TokioCommand::new(&self.runtime_path)
            .args(&[
                "ps",
                "--filter",
                "name=pachyterm-",
                "--format",
                "{{.Names}}",
            ])
            .stdout(Stdio::piped())
            .output()
            .await?;

        if output.status.success() {
            let containers = String::from_utf8_lossy(&output.stdout);
            for container_name in containers.lines() {
                let container_name = container_name.trim();
                if !container_name.is_empty() {
                    // Check if we have a session for this container
                    let session_id = container_name
                        .strip_prefix("pachyterm-")
                        .unwrap_or(container_name);

                    let sessions = self.sessions.read().await;
                    if !sessions.contains_key(session_id) {
                        warn!("Found orphaned container: {}", container_name);

                        // Remove orphaned container
                        let _ = TokioCommand::new(&self.runtime_path)
                            .args(&["rm", "--force", container_name])
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .status()
                            .await;
                    }
                }
            }
        }

        Ok(())
    }

    pub async fn shutdown(&self) -> Result<(), OciError> {
        info!("Shutting down OCI launcher");

        // Destroy all sessions
        let session_ids: Vec<String> = self.sessions.read().await.keys().cloned().collect();
        for session_id in session_ids {
            if let Err(e) = self.destroy_session(&session_id).await {
                error!("Failed to destroy session {}: {}", session_id, e);
            }
        }

        // Cleanup orphaned containers
        if let Err(e) = self.cleanup_orphaned_containers().await {
            error!("Failed to cleanup orphaned containers: {}", e);
        }

        info!("OCI launcher shutdown complete");
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionStats {
    pub cpu_percent: f64,
    pub memory_usage: u64,
    pub memory_limit: u64,
    pub network_rx: u64,
    pub network_tx: u64,
    pub block_read: u64,
    pub block_write: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_session_config_creation() {
        let config = SessionConfig::new("test-session".to_string());
        assert_eq!(config.session_id, "test-session");
        assert!(config.container.rootless);
        assert!(!config.container.gpu_enabled);
    }

    #[test]
    fn test_container_config_default() {
        let config = ContainerConfig::default();
        assert_eq!(config.image, "quay.io/pachyterm/ramalama:latest");
        assert_eq!(config.command, vec!["/bin/bash"]);
        assert!(config.rootless);
    }

    #[test]
    fn test_model_config_default() {
        let config = ModelConfig::default();
        assert_eq!(config.name, "mistral-7b-instruct");
        assert_eq!(config.size, "7B");
        assert_eq!(config.quantization, "q4_0");
        assert_eq!(config.context_window, 4096);
    }

    #[tokio::test]
    async fn test_session_creation_mock() {
        // This test would require mocking the container runtime
        // For now, just test the data structures
        let config = SessionConfig::new("test-session".to_string());
        let session = ContainerSession::new(config);
        assert_eq!(session.id, "test-session");
        assert!(!session.model_loaded);
        assert!(session.logs.is_empty());
    }

    #[test]
    fn test_volume_mount_formatting() {
        let mount = VolumeMount {
            host_path: "/host/path".to_string(),
            container_path: "/container/path".to_string(),
            read_only: true,
        };

        let expected = "/host/path:/container/path:ro";
        assert_eq!(
            format!("{}:{}:ro", mount.host_path, mount.container_path),
            expected
        );
    }

    #[test]
    fn test_session_stats_default() {
        let stats = SessionStats::default();
        assert_eq!(stats.cpu_percent, 0.0);
        assert_eq!(stats.memory_usage, 0);
        assert_eq!(stats.memory_limit, 0);
    }
}
