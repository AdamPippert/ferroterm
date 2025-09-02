use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::net::UnixListener;
use tokio::sync::{mpsc, RwLock};
use tokio_stream::StreamExt;

#[derive(Error, Debug)]
pub enum AgentApiError {
    #[error("Plugin error: {0}")]
    Plugin(String),
    #[error("RPC error: {0}")]
    Rpc(String),
    #[error("Rate limit exceeded")]
    RateLimitExceeded,
    #[error("Capability denied: {capability}")]
    CapabilityDenied { capability: String },
    #[error("Plugin not found: {name}")]
    PluginNotFound { name: String },
}

/// Plugin capability manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub capabilities: Vec<PluginCapability>,
    pub rate_limit_per_second: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PluginCapability {
    SpawnPane,
    RenderOverlay,
    QueryContext,
    ExecuteCommand,
    FileSystemRead,
    NetworkAccess,
}

#[derive(Debug, Clone)]
pub struct PluginInstance {
    pub manifest: PluginManifest,
    pub socket_path: PathBuf,
    pub last_request: Instant,
    pub request_count: u32,
}

/// RPC message types
#[derive(Debug, Clone)]
pub enum RpcMessage {
    SpawnPane {
        command: String,
        title: String,
    },
    RenderOverlay {
        content: String,
        x: u32,
        y: u32,
    },
    QueryContext {
        query: String,
    },
    ExecuteCommand {
        command: String,
        args: Vec<String>,
    },
    Response {
        result: String,
        error: Option<String>,
    },
}

/// Agent API broker
pub struct AgentApiBroker {
    plugins: Arc<RwLock<HashMap<String, PluginInstance>>>,
    listener: UnixListener,
    request_tx: mpsc::UnboundedSender<RpcRequest>,
    request_rx: mpsc::UnboundedReceiver<RpcRequest>,
    rate_limiter: Arc<RateLimiter>,
}

#[derive(Debug)]
pub struct RpcRequest {
    pub plugin_name: String,
    pub message: RpcMessage,
    pub response_tx: mpsc::UnboundedSender<RpcMessage>,
}

#[derive(Debug)]
pub struct RateLimiter {
    requests: RwLock<HashMap<String, Vec<Instant>>>,
    max_per_second: u32,
}

impl RateLimiter {
    pub fn new(max_per_second: u32) -> Self {
        Self {
            requests: RwLock::new(HashMap::new()),
            max_per_second,
        }
    }

    pub async fn check_rate_limit(&self, plugin_name: &str) -> bool {
        let mut requests = self.requests.write().await;
        let now = Instant::now();
        let window_start = now - Duration::from_secs(1);

        let plugin_requests = requests
            .entry(plugin_name.to_string())
            .or_insert_with(Vec::new);
        plugin_requests.retain(|&time| time > window_start);

        if plugin_requests.len() >= self.max_per_second as usize {
            return false;
        }

        plugin_requests.push(now);
        true
    }
}

impl AgentApiBroker {
    /// Create a new broker
    pub async fn new(socket_path: PathBuf) -> Result<Self, AgentApiError> {
        let listener = UnixListener::bind(&socket_path)
            .map_err(|e| AgentApiError::Rpc(format!("Failed to bind socket: {}", e)))?;

        let (request_tx, request_rx) = mpsc::unbounded_channel();

        Ok(Self {
            plugins: Arc::new(RwLock::new(HashMap::new())),
            listener,
            request_tx,
            request_rx,
            rate_limiter: Arc::new(RateLimiter::new(30)), // 30 RPC/s limit
        })
    }

    /// Register a plugin
    pub async fn register_plugin(
        &self,
        manifest: PluginManifest,
        socket_path: PathBuf,
    ) -> Result<(), AgentApiError> {
        let instance = PluginInstance {
            manifest,
            socket_path,
            last_request: Instant::now(),
            request_count: 0,
        };

        let mut plugins = self.plugins.write().await;
        plugins.insert(instance.manifest.name.clone(), instance);
        Ok(())
    }

    /// Send RPC request to plugin
    pub async fn send_rpc(
        &self,
        plugin_name: &str,
        message: RpcMessage,
    ) -> Result<RpcMessage, AgentApiError> {
        // Check rate limit
        if !self.rate_limiter.check_rate_limit(plugin_name).await {
            return Err(AgentApiError::RateLimitExceeded);
        }

        // Check capabilities
        let plugins = self.plugins.read().await;
        let plugin = plugins
            .get(plugin_name)
            .ok_or_else(|| AgentApiError::PluginNotFound {
                name: plugin_name.to_string(),
            })?;

        self.check_capability(&plugin.manifest, &message)?;

        // In practice, this would send over UDS
        // For now, simulate response
        let response = match message {
            RpcMessage::SpawnPane { .. } => RpcMessage::Response {
                result: "Pane spawned".to_string(),
                error: None,
            },
            RpcMessage::RenderOverlay { .. } => RpcMessage::Response {
                result: "Overlay rendered".to_string(),
                error: None,
            },
            RpcMessage::QueryContext { query } => RpcMessage::Response {
                result: format!("Context for: {}", query),
                error: None,
            },
            RpcMessage::ExecuteCommand { command, .. } => RpcMessage::Response {
                result: format!("Executed: {}", command),
                error: None,
            },
            _ => RpcMessage::Response {
                result: "OK".to_string(),
                error: None,
            },
        };

        Ok(response)
    }

    /// Check if plugin has required capability
    fn check_capability(
        &self,
        manifest: &PluginManifest,
        message: &RpcMessage,
    ) -> Result<(), AgentApiError> {
        let required_cap = match message {
            RpcMessage::SpawnPane { .. } => PluginCapability::SpawnPane,
            RpcMessage::RenderOverlay { .. } => PluginCapability::RenderOverlay,
            RpcMessage::QueryContext { .. } => PluginCapability::QueryContext,
            RpcMessage::ExecuteCommand { .. } => PluginCapability::ExecuteCommand,
            _ => return Ok(()),
        };

        if !manifest.capabilities.contains(&required_cap) {
            return Err(AgentApiError::CapabilityDenied {
                capability: format!("{:?}", required_cap),
            });
        }

        Ok(())
    }

    /// List registered plugins
    pub async fn list_plugins(&self) -> Vec<PluginManifest> {
        let plugins = self.plugins.read().await;
        plugins.values().map(|p| p.manifest.clone()).collect()
    }

    /// Unregister plugin
    pub async fn unregister_plugin(&self, name: &str) -> Result<(), AgentApiError> {
        let mut plugins = self.plugins.write().await;
        plugins
            .remove(name)
            .ok_or_else(|| AgentApiError::PluginNotFound {
                name: name.to_string(),
            })?;
        Ok(())
    }

    /// Get plugin stats
    pub async fn get_plugin_stats(&self, name: &str) -> Result<PluginStats, AgentApiError> {
        let plugins = self.plugins.read().await;
        let plugin = plugins
            .get(name)
            .ok_or_else(|| AgentApiError::PluginNotFound {
                name: name.to_string(),
            })?;

        Ok(PluginStats {
            name: name.to_string(),
            request_count: plugin.request_count,
            last_request: plugin.last_request,
        })
    }
}

#[derive(Debug, Clone)]
pub struct PluginStats {
    pub name: String,
    pub request_count: u32,
    pub last_request: Instant,
}

/// Plugin trait for implementing plugins
#[async_trait::async_trait]
pub trait Plugin: Send + Sync {
    async fn handle_rpc(&self, message: RpcMessage) -> Result<RpcMessage, AgentApiError>;
    fn get_manifest(&self) -> PluginManifest;
}

/// Example plugin implementation
pub struct ExampleChartPlugin;

#[async_trait::async_trait]
impl Plugin for ExampleChartPlugin {
    async fn handle_rpc(&self, message: RpcMessage) -> Result<RpcMessage, AgentApiError> {
        match message {
            RpcMessage::RenderOverlay { content, x, y } => {
                // Simulate rendering ASCII chart
                let chart = format!("ASCII Chart at ({},{}): {}", x, y, content);
                Ok(RpcMessage::Response {
                    result: chart,
                    error: None,
                })
            }
            _ => Err(AgentApiError::Plugin("Unsupported RPC".to_string())),
        }
    }

    fn get_manifest(&self) -> PluginManifest {
        PluginManifest {
            name: "ascii-chart".to_string(),
            version: "1.0.0".to_string(),
            description: "Renders ASCII charts".to_string(),
            capabilities: vec![PluginCapability::RenderOverlay],
            rate_limit_per_second: 10,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_plugin_registration() {
        let temp_dir = tempdir().unwrap();
        let socket_path = temp_dir.path().join("test.sock");

        let broker = AgentApiBroker::new(socket_path).await.unwrap();

        let manifest = PluginManifest {
            name: "test-plugin".to_string(),
            version: "1.0.0".to_string(),
            description: "Test plugin".to_string(),
            capabilities: vec![PluginCapability::SpawnPane],
            rate_limit_per_second: 5,
        };

        let plugin_socket = temp_dir.path().join("plugin.sock");
        broker
            .register_plugin(manifest.clone(), plugin_socket)
            .await
            .unwrap();

        let plugins = broker.list_plugins().await;
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].name, "test-plugin");
    }

    #[tokio::test]
    async fn test_capability_check() {
        let temp_dir = tempdir().unwrap();
        let socket_path = temp_dir.path().join("test.sock");

        let broker = AgentApiBroker::new(socket_path).await.unwrap();

        let manifest = PluginManifest {
            name: "test-plugin".to_string(),
            version: "1.0.0".to_string(),
            description: "Test plugin".to_string(),
            capabilities: vec![PluginCapability::SpawnPane],
            rate_limit_per_second: 5,
        };

        let plugin_socket = temp_dir.path().join("plugin.sock");
        broker
            .register_plugin(manifest, plugin_socket)
            .await
            .unwrap();

        // Should succeed
        let result = broker
            .send_rpc(
                "test-plugin",
                RpcMessage::SpawnPane {
                    command: "echo hello".to_string(),
                    title: "Test".to_string(),
                },
            )
            .await;
        assert!(result.is_ok());

        // Should fail - capability not granted
        let result = broker
            .send_rpc(
                "test-plugin",
                RpcMessage::RenderOverlay {
                    content: "test".to_string(),
                    x: 0,
                    y: 0,
                },
            )
            .await;
        assert!(matches!(
            result,
            Err(AgentApiError::CapabilityDenied { .. })
        ));
    }

    #[tokio::test]
    async fn test_rate_limiting() {
        let temp_dir = tempdir().unwrap();
        let socket_path = temp_dir.path().join("test.sock");

        let broker = AgentApiBroker::new(socket_path).await.unwrap();

        let manifest = PluginManifest {
            name: "test-plugin".to_string(),
            version: "1.0.0".to_string(),
            description: "Test plugin".to_string(),
            capabilities: vec![PluginCapability::SpawnPane],
            rate_limit_per_second: 2, // Low limit for test
        };

        let plugin_socket = temp_dir.path().join("plugin.sock");
        broker
            .register_plugin(manifest, plugin_socket)
            .await
            .unwrap();

        // First two should succeed
        for i in 0..2 {
            let result = broker
                .send_rpc(
                    "test-plugin",
                    RpcMessage::SpawnPane {
                        command: format!("echo {}", i),
                        title: format!("Test {}", i),
                    },
                )
                .await;
            assert!(result.is_ok());
        }

        // Third should be rate limited
        let result = broker
            .send_rpc(
                "test-plugin",
                RpcMessage::SpawnPane {
                    command: "echo rate-limited".to_string(),
                    title: "Rate Limited".to_string(),
                },
            )
            .await;
        assert!(matches!(result, Err(AgentApiError::RateLimitExceeded)));
    }
}
