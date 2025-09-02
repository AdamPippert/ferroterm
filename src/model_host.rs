use async_trait::async_trait;
use futures::stream::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::{broadcast, mpsc, Mutex, RwLock};
use tokio::time::timeout;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_stream::Stream;
use tracing::{debug, error, info, warn};

#[derive(Error, Debug)]
pub enum ModelHostError {
    #[error("Model loading error: {0}")]
    ModelLoad(String),
    #[error("Inference error: {0}")]
    Inference(String),
    #[error("API error: {0}")]
    Api(#[from] reqwest::Error),
    #[error("Configuration error: {0}")]
    Config(String),
    #[error("Model not found: {name}")]
    ModelNotFound { name: String },
    #[error("Resource exhausted: {resource}")]
    ResourceExhausted { resource: String },
    #[error("Timeout: operation took longer than {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },
    #[error("VRAM exhausted: required {required}MB, available {available}MB")]
    VramExhausted { required: u64, available: u64 },
    #[error("Hot-swap failed: {reason}")]
    HotSwapFailed { reason: String },
    #[error("Batch processing error: {0}")]
    BatchProcessing(String),
    #[error("Stream error: {0}")]
    Stream(String),
    #[error("Authentication error: {0}")]
    Authentication(String),
    #[error("Model corrupted: {name}")]
    ModelCorrupted { name: String },
    #[error("Pool exhausted: all {count} workers are busy")]
    PoolExhausted { count: usize },
    #[error("Fallback chain exhausted: all {count} models failed")]
    FallbackExhausted { count: usize },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceRequest {
    pub prompt: String,
    pub model_name: String,
    pub parameters: InferenceParameters,
    pub context: Option<Vec<String>>,
    pub stream: bool,
    pub batch_id: Option<String>,
    pub priority: InferencePriority,
    pub fallback_chain: Option<Vec<String>>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum InferencePriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceParameters {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: Option<u32>,
    pub max_tokens: u32,
    pub stop_sequences: Vec<String>,
    pub repetition_penalty: f32,
    pub frequency_penalty: f32,
    pub presence_penalty: f32,
}

impl Default for InferenceParameters {
    fn default() -> Self {
        Self {
            temperature: 0.7,
            top_p: 0.9,
            top_k: None,
            max_tokens: 2048,
            stop_sequences: vec![],
            repetition_penalty: 1.0,
            frequency_penalty: 0.0,
            presence_penalty: 0.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct InferenceResponse {
    pub text: String,
    pub tokens_generated: u32,
    pub total_tokens: u32,
    pub finish_reason: FinishReason,
    pub timing: InferenceTiming,
    pub model_used: String,
    pub is_fallback: bool,
}

#[derive(Debug, Clone)]
pub struct StreamToken {
    pub token: String,
    pub is_final: bool,
    pub token_index: u32,
    pub timestamp: Instant,
}

pub type TokenStream = Pin<Box<dyn Stream<Item = Result<StreamToken, ModelHostError>> + Send>>;

#[derive(Debug, Clone)]
pub struct BatchInferenceRequest {
    pub requests: Vec<InferenceRequest>,
    pub batch_id: String,
    pub max_parallel: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct InferenceTiming {
    pub prompt_eval_time: Duration,
    pub eval_time: Duration,
    pub total_time: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FinishReason {
    Stop,
    Length,
    Error(String),
}

#[async_trait]
pub trait ModelAdapter: Send + Sync {
    async fn load(&mut self) -> Result<(), ModelHostError>;
    async fn unload(&mut self) -> Result<(), ModelHostError>;
    async fn infer(&self, request: InferenceRequest) -> Result<InferenceResponse, ModelHostError>;
    async fn infer_stream(&self, request: InferenceRequest) -> Result<TokenStream, ModelHostError>;
    async fn batch_infer(&self, requests: Vec<InferenceRequest>) -> Result<Vec<InferenceResponse>, ModelHostError>;
    fn is_loaded(&self) -> bool;
    fn get_model_info(&self) -> ModelInfo;
    fn supports_streaming(&self) -> bool;
    fn supports_batch(&self) -> bool;
    async fn health_check(&self) -> Result<(), ModelHostError>;
    async fn warmup(&self) -> Result<(), ModelHostError>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelInfo {
    pub name: String,
    pub model_type: ModelType,
    pub context_window: u32,
    pub supports_streaming: bool,
    pub loaded_at: Option<u64>, // Changed to u64 for serialization
    pub vram_required_mb: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ModelType {
    LocalGGUF,
    RemoteAPI,
    MLC,
    VLLM,
    OpenAI,
    Gemini,
    Anthropic,
    Ollama,
}

#[derive(Debug)]
pub struct VramStats {
    pub total_mb: u64,
    pub used_mb: AtomicU64,
    pub available_mb: AtomicU64,
    pub last_updated: Instant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub name: String,
    pub model_type: ModelType,
    pub model_path: Option<PathBuf>,
    pub api_endpoint: Option<String>,
    pub api_key_env: Option<String>, // Environment variable name for API key
    pub context_window: u32,
    pub vram_required_mb: u64,
    pub default_parameters: InferenceParameters,
    pub fallback_models: Vec<String>,
    pub warm_pool_size: usize,
    pub max_concurrent: usize,
}

#[derive(Debug, Clone)]
pub struct SecureApiKey {
    inner: String,
}

impl SecureApiKey {
    pub fn new(key: String) -> Self {
        Self { inner: key }
    }

    pub fn from_env(env_var: &str) -> Result<Self, ModelHostError> {
        std::env::var(env_var)
            .map(|key| Self::new(key))
            .map_err(|_| ModelHostError::Authentication(format!("Environment variable {} not found", env_var)))
    }

    pub fn get(&self) -> &str {
        &self.inner
    }
}

// Prevent accidental logging of API keys
impl std::fmt::Debug for SecureApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecureApiKey")
            .field("inner", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug)]
pub struct ModelWorker {
    pub id: String,
    pub adapter: Arc<Mutex<Box<dyn ModelAdapter>>>,
    pub is_busy: AtomicBool,
    pub last_used: Arc<Mutex<Instant>>,
    pub requests_processed: AtomicU64,
}

#[derive(Debug)]
pub struct FallbackChain {
    pub models: Vec<String>,
    pub current_index: usize,
    pub failed_models: Vec<String>,
}

impl FallbackChain {
    pub fn new(models: Vec<String>) -> Self {
        Self {
            models,
            current_index: 0,
            failed_models: Vec::new(),
        }
    }

    pub fn next_model(&mut self) -> Option<String> {
        if self.current_index < self.models.len() {
            let model = self.models[self.current_index].clone();
            self.current_index += 1;
            Some(model)
        } else {
            None
        }
    }

    pub fn mark_failed(&mut self, model: String) {
        self.failed_models.push(model);
    }

    pub fn has_more(&self) -> bool {
        self.current_index < self.models.len()
    }
}

impl VramStats {
    pub fn new(total_mb: u64) -> Self {
        Self {
            total_mb,
            used_mb: AtomicU64::new(0),
            available_mb: AtomicU64::new(total_mb),
            last_updated: Instant::now(),
        }
    }

    pub fn allocate(&self, size_mb: u64) -> Result<(), ModelHostError> {
        let current_used = self.used_mb.load(Ordering::SeqCst);
        let new_used = current_used + size_mb;

        if new_used > self.total_mb {
            return Err(ModelHostError::VramExhausted {
                required: size_mb,
                available: self.total_mb - current_used,
            });
        }

        self.used_mb.store(new_used, Ordering::SeqCst);
        self.available_mb
            .store(self.total_mb - new_used, Ordering::SeqCst);
        Ok(())
    }

    pub fn deallocate(&self, size_mb: u64) {
        let current_used = self.used_mb.load(Ordering::SeqCst);
        let new_used = current_used.saturating_sub(size_mb);
        self.used_mb.store(new_used, Ordering::SeqCst);
        self.available_mb
            .store(self.total_mb - new_used, Ordering::SeqCst);
    }

    pub fn get_usage_percent(&self) -> f32 {
        let used = self.used_mb.load(Ordering::SeqCst) as f32;
        let total = self.total_mb as f32;
        (used / total) * 100.0
    }
}

pub struct LocalGGUFAdapter {
    model_path: PathBuf,
    model_info: ModelInfo,
    loaded: AtomicBool,
    config: ModelConfig,
    // In a real implementation, this would hold the GGUF model instance
    // For now, we'll simulate the interface
}

impl LocalGGUFAdapter {
    pub fn new(config: ModelConfig) -> Self {
        Self {
            model_path: config.model_path.clone().unwrap_or_default(),
            model_info: ModelInfo {
                name: config.name.clone(),
                model_type: ModelType::LocalGGUF,
                context_window: config.context_window,
                supports_streaming: true,
                loaded_at: None,
                vram_required_mb: config.vram_required_mb,
            },
            loaded: AtomicBool::new(false),
            config,
        }
    }
}

#[async_trait]
impl ModelAdapter for LocalGGUFAdapter {
    async fn load(&mut self) -> Result<(), ModelHostError> {
        debug!("Loading GGUF model from: {:?}", self.model_path);
        
        // Check if model file exists
        if !self.model_path.exists() {
            return Err(ModelHostError::ModelLoad(format!(
                "Model file not found: {:?}", self.model_path
            )));
        }

        // Simulate model loading with realistic timing
        let start = Instant::now();
        tokio::time::sleep(Duration::from_millis(120)).await;
        
        // Validate model file (simulate)
        if self.model_path.extension().and_then(|s| s.to_str()) != Some("gguf") {
            return Err(ModelHostError::ModelCorrupted { 
                name: self.model_info.name.clone() 
            });
        }

        self.loaded.store(true, Ordering::SeqCst);
        self.model_info.loaded_at = Some(start.elapsed().as_secs());
        
        info!("GGUF model loaded successfully: {}", self.model_info.name);
        Ok(())
    }

    async fn unload(&mut self) -> Result<(), ModelHostError> {
        debug!("Unloading GGUF model: {}", self.model_info.name);
        self.loaded.store(false, Ordering::SeqCst);
        self.model_info.loaded_at = None;
        Ok(())
    }

    async fn infer(&self, request: InferenceRequest) -> Result<InferenceResponse, ModelHostError> {
        if !self.loaded.load(Ordering::SeqCst) {
            return Err(ModelHostError::ModelLoad("Model not loaded".to_string()));
        }

        let start_time = Instant::now();
        let timeout_duration = Duration::from_millis(request.timeout_ms.unwrap_or(30000));

        // Simulate realistic inference timing based on prompt length and parameters
        let prompt_eval_time = Duration::from_millis(5 + request.prompt.len() as u64 / 20);
        let tokens_to_generate = request.parameters.max_tokens.min(512);
        let eval_time = Duration::from_millis(
            (tokens_to_generate as u64 * 15) / (request.parameters.temperature * 10.0) as u64
        );

        // Apply timeout
        let inference_result = timeout(timeout_duration, async {
            tokio::time::sleep(prompt_eval_time).await;
            tokio::time::sleep(eval_time).await;
            
            // Generate contextual response
            let response_text = format!(
                "GGUF model {} response to: {} [temp: {}, max_tokens: {}]",
                self.model_info.name,
                request.prompt,
                request.parameters.temperature,
                request.parameters.max_tokens
            );
            
            Ok(response_text)
        }).await;

        match inference_result {
            Ok(Ok(response_text)) => {
                let tokens_generated = response_text.split_whitespace().count() as u32;
                let total_time = start_time.elapsed();

                Ok(InferenceResponse {
                    text: response_text,
                    tokens_generated,
                    total_tokens: request.prompt.split_whitespace().count() as u32 + tokens_generated,
                    finish_reason: FinishReason::Stop,
                    timing: InferenceTiming {
                        prompt_eval_time,
                        eval_time,
                        total_time,
                    },
                    model_used: self.model_info.name.clone(),
                    is_fallback: false,
                })
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(ModelHostError::Timeout { 
                timeout_ms: timeout_duration.as_millis() as u64 
            }),
        }
    }

    async fn infer_stream(&self, request: InferenceRequest) -> Result<TokenStream, ModelHostError> {
        if !self.loaded.load(Ordering::SeqCst) {
            return Err(ModelHostError::ModelLoad("Model not loaded".to_string()));
        }

        let (tx, rx) = mpsc::unbounded_channel();
        let model_name = self.model_info.name.clone();
        let start_time = Instant::now();

        tokio::spawn(async move {
            // Simulate streaming token generation
            let tokens = vec![
                "GGUF", "streaming", "response", "to", "your", "prompt:", 
                &request.prompt, "with", "realistic", "timing"
            ];

            for (i, token) in tokens.iter().enumerate() {
                // Simulate realistic token generation timing
                tokio::time::sleep(Duration::from_millis(50 + (i as u64 * 10))).await;
                
                let stream_token = StreamToken {
                    token: token.to_string(),
                    is_final: i == tokens.len() - 1,
                    token_index: i as u32,
                    timestamp: start_time,
                };

                if tx.send(Ok(stream_token)).is_err() {
                    break;
                }
            }
        });

        let stream = UnboundedReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn batch_infer(&self, requests: Vec<InferenceRequest>) -> Result<Vec<InferenceResponse>, ModelHostError> {
        if !self.loaded.load(Ordering::SeqCst) {
            return Err(ModelHostError::ModelLoad("Model not loaded".to_string()));
        }

        let mut responses = Vec::new();
        
        // Process in parallel with controlled concurrency
        let max_parallel = self.config.max_concurrent.min(requests.len());
        let mut tasks = Vec::new();

        for chunk in requests.chunks(max_parallel) {
            let chunk_tasks: Vec<_> = chunk.iter().map(|req| {
                let req_clone = req.clone();
                async move { self.infer(req_clone).await }
            }).collect();

            let chunk_results = futures::future::join_all(chunk_tasks).await;
            for result in chunk_results {
                responses.push(result?);
            }
        }

        Ok(responses)
    }

    fn is_loaded(&self) -> bool {
        self.loaded.load(Ordering::SeqCst)
    }

    fn get_model_info(&self) -> ModelInfo {
        self.model_info.clone()
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_batch(&self) -> bool {
        true
    }

    async fn health_check(&self) -> Result<(), ModelHostError> {
        if !self.loaded.load(Ordering::SeqCst) {
            return Err(ModelHostError::ModelLoad("Model not loaded".to_string()));
        }

        // Quick inference test
        let test_request = InferenceRequest {
            prompt: "test".to_string(),
            model_name: self.model_info.name.clone(),
            parameters: InferenceParameters {
                max_tokens: 1,
                ..Default::default()
            },
            context: None,
            stream: false,
            batch_id: None,
            priority: InferencePriority::Normal,
            fallback_chain: None,
            timeout_ms: Some(5000),
        };

        self.infer(test_request).await.map(|_| ())
    }

    async fn warmup(&self) -> Result<(), ModelHostError> {
        if !self.loaded.load(Ordering::SeqCst) {
            return Err(ModelHostError::ModelLoad("Model not loaded".to_string()));
        }

        debug!("Warming up GGUF model: {}", self.model_info.name);
        
        // Warmup inference
        let warmup_request = InferenceRequest {
            prompt: "warmup".to_string(),
            model_name: self.model_info.name.clone(),
            parameters: InferenceParameters {
                max_tokens: 5,
                ..Default::default()
            },
            context: None,
            stream: false,
            batch_id: None,
            priority: InferencePriority::Normal,
            fallback_chain: None,
            timeout_ms: Some(10000),
        };

        self.infer(warmup_request).await.map(|_| ())
    }
}

pub struct MLCAdapter {
    config: ModelConfig,
    model_info: ModelInfo,
    loaded: AtomicBool,
}

impl MLCAdapter {
    pub fn new(config: ModelConfig) -> Self {
        Self {
            model_info: ModelInfo {
                name: config.name.clone(),
                model_type: ModelType::MLC,
                context_window: config.context_window,
                supports_streaming: true,
                loaded_at: None,
                vram_required_mb: config.vram_required_mb,
            },
            loaded: AtomicBool::new(false),
            config,
        }
    }
}

#[async_trait]
impl ModelAdapter for MLCAdapter {
    async fn load(&mut self) -> Result<(), ModelHostError> {
        debug!("Loading MLC model: {}", self.model_info.name);
        
        let start = Instant::now();
        // Simulate MLC model loading
        tokio::time::sleep(Duration::from_millis(150)).await;
        
        self.loaded.store(true, Ordering::SeqCst);
        self.model_info.loaded_at = Some(start.elapsed().as_secs());
        
        info!("MLC model loaded successfully: {}", self.model_info.name);
        Ok(())
    }

    async fn unload(&mut self) -> Result<(), ModelHostError> {
        debug!("Unloading MLC model: {}", self.model_info.name);
        self.loaded.store(false, Ordering::SeqCst);
        self.model_info.loaded_at = None;
        Ok(())
    }

    async fn infer(&self, request: InferenceRequest) -> Result<InferenceResponse, ModelHostError> {
        if !self.loaded.load(Ordering::SeqCst) {
            return Err(ModelHostError::ModelLoad("MLC model not loaded".to_string()));
        }

        let start_time = Instant::now();
        let eval_time = Duration::from_millis(30 + (request.prompt.len() as u64 / 15));
        
        tokio::time::sleep(eval_time).await;

        let response_text = format!(
            "MLC {} optimized response: {} [GPU accelerated]",
            self.model_info.name,
            request.prompt
        );
        let tokens_generated = response_text.split_whitespace().count() as u32;

        Ok(InferenceResponse {
            text: response_text,
            tokens_generated,
            total_tokens: request.prompt.split_whitespace().count() as u32 + tokens_generated,
            finish_reason: FinishReason::Stop,
            timing: InferenceTiming {
                prompt_eval_time: Duration::from_millis(5),
                eval_time,
                total_time: start_time.elapsed(),
            },
            model_used: self.model_info.name.clone(),
            is_fallback: false,
        })
    }

    async fn infer_stream(&self, request: InferenceRequest) -> Result<TokenStream, ModelHostError> {
        if !self.loaded.load(Ordering::SeqCst) {
            return Err(ModelHostError::ModelLoad("MLC model not loaded".to_string()));
        }

        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let tokens = vec!["MLC", "fast", "streaming", "tokens", "here"];
            for (i, token) in tokens.iter().enumerate() {
                tokio::time::sleep(Duration::from_millis(30)).await;
                let stream_token = StreamToken {
                    token: token.to_string(),
                    is_final: i == tokens.len() - 1,
                    token_index: i as u32,
                    timestamp: Instant::now(),
                };
                if tx.send(Ok(stream_token)).is_err() {
                    break;
                }
            }
        });

        Ok(Box::pin(UnboundedReceiverStream::new(rx)))
    }

    async fn batch_infer(&self, requests: Vec<InferenceRequest>) -> Result<Vec<InferenceResponse>, ModelHostError> {
        // MLC optimized batch processing
        let mut responses = Vec::new();
        for request in requests {
            responses.push(self.infer(request).await?);
        }
        Ok(responses)
    }

    fn is_loaded(&self) -> bool {
        self.loaded.load(Ordering::SeqCst)
    }

    fn get_model_info(&self) -> ModelInfo {
        self.model_info.clone()
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_batch(&self) -> bool {
        true
    }

    async fn health_check(&self) -> Result<(), ModelHostError> {
        if self.loaded.load(Ordering::SeqCst) { Ok(()) } else { 
            Err(ModelHostError::ModelLoad("MLC model not loaded".to_string())) 
        }
    }

    async fn warmup(&self) -> Result<(), ModelHostError> {
        Ok(()) // MLC models are typically fast to warm up
    }
}

pub struct VLLMAdapter {
    config: ModelConfig,
    model_info: ModelInfo,
    loaded: AtomicBool,
    process_handle: Arc<Mutex<Option<tokio::process::Child>>>,
}

impl VLLMAdapter {
    pub fn new(config: ModelConfig) -> Self {
        Self {
            model_info: ModelInfo {
                name: config.name.clone(),
                model_type: ModelType::VLLM,
                context_window: config.context_window,
                supports_streaming: true,
                loaded_at: None,
                vram_required_mb: config.vram_required_mb,
            },
            loaded: AtomicBool::new(false),
            config,
            process_handle: Arc::new(Mutex::new(None)),
        }
    }
}

#[async_trait]
impl ModelAdapter for VLLMAdapter {
    async fn load(&mut self) -> Result<(), ModelHostError> {
        debug!("Starting vLLM server for model: {}", self.model_info.name);
        
        let start = Instant::now();
        
        // In a real implementation, this would start the vLLM server process
        // For now, simulate the startup time
        tokio::time::sleep(Duration::from_millis(200)).await;
        
        self.loaded.store(true, Ordering::SeqCst);
        self.model_info.loaded_at = Some(start.elapsed().as_secs());
        
        info!("vLLM server started successfully: {}", self.model_info.name);
        Ok(())
    }

    async fn unload(&mut self) -> Result<(), ModelHostError> {
        debug!("Stopping vLLM server: {}", self.model_info.name);
        
        // Stop the vLLM process if running
        let mut handle = self.process_handle.lock().await;
        if let Some(mut child) = handle.take() {
            let _ = child.kill().await;
        }
        
        self.loaded.store(false, Ordering::SeqCst);
        self.model_info.loaded_at = None;
        Ok(())
    }

    async fn infer(&self, request: InferenceRequest) -> Result<InferenceResponse, ModelHostError> {
        if !self.loaded.load(Ordering::SeqCst) {
            return Err(ModelHostError::ModelLoad("vLLM server not running".to_string()));
        }

        let start_time = Instant::now();
        // vLLM is typically faster due to optimizations
        let eval_time = Duration::from_millis(20 + (request.prompt.len() as u64 / 25));
        
        tokio::time::sleep(eval_time).await;

        let response_text = format!(
            "vLLM {} high-throughput response: {} [PagedAttention optimized]",
            self.model_info.name,
            request.prompt
        );
        let tokens_generated = response_text.split_whitespace().count() as u32;

        Ok(InferenceResponse {
            text: response_text,
            tokens_generated,
            total_tokens: request.prompt.split_whitespace().count() as u32 + tokens_generated,
            finish_reason: FinishReason::Stop,
            timing: InferenceTiming {
                prompt_eval_time: Duration::from_millis(3),
                eval_time,
                total_time: start_time.elapsed(),
            },
            model_used: self.model_info.name.clone(),
            is_fallback: false,
        })
    }

    async fn infer_stream(&self, request: InferenceRequest) -> Result<TokenStream, ModelHostError> {
        if !self.loaded.load(Ordering::SeqCst) {
            return Err(ModelHostError::ModelLoad("vLLM server not running".to_string()));
        }

        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let tokens = vec!["vLLM", "high", "throughput", "streaming", "optimized"];
            for (i, token) in tokens.iter().enumerate() {
                tokio::time::sleep(Duration::from_millis(20)).await;
                let stream_token = StreamToken {
                    token: token.to_string(),
                    is_final: i == tokens.len() - 1,
                    token_index: i as u32,
                    timestamp: Instant::now(),
                };
                if tx.send(Ok(stream_token)).is_err() {
                    break;
                }
            }
        });

        Ok(Box::pin(UnboundedReceiverStream::new(rx)))
    }

    async fn batch_infer(&self, requests: Vec<InferenceRequest>) -> Result<Vec<InferenceResponse>, ModelHostError> {
        // vLLM excels at batch processing
        let start_time = Instant::now();
        let mut responses = Vec::new();
        
        // Simulate optimized batch processing
        let batch_time = Duration::from_millis(requests.len() as u64 * 15); // Very efficient
        tokio::time::sleep(batch_time).await;
        
        for request in requests {
            let response_text = format!("vLLM batch response: {}", request.prompt);
            responses.push(InferenceResponse {
                text: response_text.clone(),
                tokens_generated: response_text.split_whitespace().count() as u32,
                total_tokens: request.prompt.split_whitespace().count() as u32 + response_text.split_whitespace().count() as u32,
                finish_reason: FinishReason::Stop,
                timing: InferenceTiming {
                    prompt_eval_time: Duration::from_millis(2),
                    eval_time: Duration::from_millis(15),
                    total_time: start_time.elapsed(),
                },
                model_used: self.model_info.name.clone(),
                is_fallback: false,
            });
        }
        
        Ok(responses)
    }

    fn is_loaded(&self) -> bool {
        self.loaded.load(Ordering::SeqCst)
    }

    fn get_model_info(&self) -> ModelInfo {
        self.model_info.clone()
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_batch(&self) -> bool {
        true
    }

    async fn health_check(&self) -> Result<(), ModelHostError> {
        if self.loaded.load(Ordering::SeqCst) { Ok(()) } else { 
            Err(ModelHostError::ModelLoad("vLLM server not running".to_string())) 
        }
    }

    async fn warmup(&self) -> Result<(), ModelHostError> {
        // vLLM servers benefit from warmup requests
        debug!("Warming up vLLM server: {}", self.model_info.name);
        Ok(())
    }
}

pub struct RemoteAPIAdapter {
    config: ModelConfig,
    api_key: Option<SecureApiKey>,
    model_info: ModelInfo,
    client: Client,
    loaded: AtomicBool,
}

impl RemoteAPIAdapter {
    pub fn new(config: ModelConfig) -> Result<Self, ModelHostError> {
        let api_key = if let Some(env_var) = &config.api_key_env {
            Some(SecureApiKey::from_env(env_var)?)
        } else {
            None
        };

        let model_type = match config.api_endpoint.as_ref().map(|s| s.as_str()) {
            Some(url) if url.contains("openai") => ModelType::OpenAI,
            Some(url) if url.contains("gemini") => ModelType::Gemini,
            Some(url) if url.contains("anthropic") => ModelType::Anthropic,
            Some(url) if url.contains("ollama") => ModelType::Ollama,
            _ => ModelType::RemoteAPI,
        };

        Ok(Self {
            model_info: ModelInfo {
                name: config.name.clone(),
                model_type,
                context_window: config.context_window,
                supports_streaming: true,
                loaded_at: None,
                vram_required_mb: 0, // Remote APIs don't use local VRAM
            },
            api_key,
            client: Client::new(),
            loaded: AtomicBool::new(false),
            config,
        })
    }
}

#[async_trait]
impl ModelAdapter for RemoteAPIAdapter {
    async fn load(&mut self) -> Result<(), ModelHostError> {
        debug!("Connecting to remote API: {:?}", self.config.api_endpoint);
        
        let api_endpoint = self.config.api_endpoint.as_ref()
            .ok_or_else(|| ModelHostError::Config("No API endpoint specified".to_string()))?;
        
        // Validate connection with health check
        let mut request_builder = self.client
            .get(api_endpoint)
            .timeout(Duration::from_secs(10));

        if let Some(key) = &self.api_key {
            request_builder = request_builder.header("Authorization", format!("Bearer {}", key.get()));
        }

        match request_builder.send().await {
            Ok(response) => {
                if response.status().is_success() || response.status() == 404 {
                    // 404 is acceptable for some API endpoints that don't support GET
                    self.loaded.store(true, Ordering::SeqCst);
                    self.model_info.loaded_at = Some(Instant::now().elapsed().as_secs());
                    info!("Connected to remote API: {}", self.model_info.name);
                    Ok(())
                } else {
                    Err(ModelHostError::Api(reqwest::Error::from(response.error_for_status().unwrap_err())))
                }
            }
            Err(e) => {
                error!("Failed to connect to API {}: {}", api_endpoint, e);
                Err(ModelHostError::Api(e))
            }
        }
    }

    async fn unload(&mut self) -> Result<(), ModelHostError> {
        debug!("Disconnecting from remote API: {}", self.model_info.name);
        self.loaded.store(false, Ordering::SeqCst);
        self.model_info.loaded_at = None;
        Ok(())
    }

    async fn infer(&self, request: InferenceRequest) -> Result<InferenceResponse, ModelHostError> {
        if !self.loaded.load(Ordering::SeqCst) {
            return Err(ModelHostError::ModelLoad("API not connected".to_string()));
        }

        let api_endpoint = self.config.api_endpoint.as_ref()
            .ok_or_else(|| ModelHostError::Config("No API endpoint specified".to_string()))?;

        let start_time = Instant::now();
        let timeout_duration = Duration::from_millis(request.timeout_ms.unwrap_or(60000));

        // Build API request based on provider type
        let (api_request, endpoint) = match self.model_info.model_type {
            ModelType::OpenAI => {
                #[derive(Serialize)]
                struct OpenAIRequest {
                    model: String,
                    messages: Vec<OpenAIMessage>,
                    temperature: f32,
                    top_p: f32,
                    max_tokens: u32,
                    stop: Option<Vec<String>>,
                    stream: bool,
                }

                #[derive(Serialize)]
                struct OpenAIMessage {
                    role: String,
                    content: String,
                }

                let messages = vec![OpenAIMessage {
                    role: "user".to_string(),
                    content: request.prompt.clone(),
                }];

                let req = OpenAIRequest {
                    model: self.model_info.name.clone(),
                    messages,
                    temperature: request.parameters.temperature,
                    top_p: request.parameters.top_p,
                    max_tokens: request.parameters.max_tokens,
                    stop: if request.parameters.stop_sequences.is_empty() { 
                        None 
                    } else { 
                        Some(request.parameters.stop_sequences.clone()) 
                    },
                    stream: request.stream,
                };

                (serde_json::to_value(req)?, format!("{}/chat/completions", api_endpoint))
            }
            _ => {
                // Generic API format
                #[derive(Serialize)]
                struct GenericRequest {
                    model: String,
                    prompt: String,
                    temperature: f32,
                    top_p: f32,
                    max_tokens: u32,
                    stop: Vec<String>,
                }

                let req = GenericRequest {
                    model: self.model_info.name.clone(),
                    prompt: request.prompt.clone(),
                    temperature: request.parameters.temperature,
                    top_p: request.parameters.top_p,
                    max_tokens: request.parameters.max_tokens,
                    stop: request.parameters.stop_sequences.clone(),
                };

                (serde_json::to_value(req)?, api_endpoint.clone())
            }
        };

        let response_result = timeout(timeout_duration, async {
            let mut request_builder = self.client
                .post(&endpoint)
                .json(&api_request)
                .header("Content-Type", "application/json");

            if let Some(key) = &self.api_key {
                request_builder = request_builder.header("Authorization", format!("Bearer {}", key.get()));
            }

            request_builder.send().await
        }).await;

        match response_result {
            Ok(Ok(response)) => {
                let total_time = start_time.elapsed();
                
                if !response.status().is_success() {
                    return Err(ModelHostError::Api(response.error_for_status().unwrap_err()));
                }

                // Parse response based on provider type
                let (text, tokens_generated, total_tokens) = match self.model_info.model_type {
                    ModelType::OpenAI => {
                        #[derive(Deserialize)]
                        struct OpenAIResponse {
                            choices: Vec<OpenAIChoice>,
                            usage: Option<OpenAIUsage>,
                        }

                        #[derive(Deserialize)]
                        struct OpenAIChoice {
                            message: OpenAIMessage,
                            finish_reason: Option<String>,
                        }

                        #[derive(Deserialize)]
                        struct OpenAIMessage {
                            content: String,
                        }

                        #[derive(Deserialize)]
                        struct OpenAIUsage {
                            prompt_tokens: u32,
                            completion_tokens: u32,
                            total_tokens: u32,
                        }

                        let api_response: OpenAIResponse = response.json().await?;
                        let text = api_response.choices.first()
                            .map(|c| c.message.content.clone())
                            .unwrap_or_default();

                        let (tokens_generated, total_tokens) = if let Some(usage) = api_response.usage {
                            (usage.completion_tokens, usage.total_tokens)
                        } else {
                            let gen = text.split_whitespace().count() as u32;
                            let total = request.prompt.split_whitespace().count() as u32 + gen;
                            (gen, total)
                        };

                        (text, tokens_generated, total_tokens)
                    }
                    _ => {
                        #[derive(Deserialize)]
                        struct GenericResponse {
                            text: Option<String>,
                            content: Option<String>,
                            response: Option<String>,
                            usage: Option<GenericUsage>,
                        }

                        #[derive(Deserialize)]
                        struct GenericUsage {
                            prompt_tokens: Option<u32>,
                            completion_tokens: Option<u32>,
                            total_tokens: Option<u32>,
                        }

                        let api_response: GenericResponse = response.json().await?;
                        let text = api_response.text
                            .or(api_response.content)
                            .or(api_response.response)
                            .unwrap_or_default();

                        let (tokens_generated, total_tokens) = if let Some(usage) = api_response.usage {
                            (
                                usage.completion_tokens.unwrap_or(text.split_whitespace().count() as u32),
                                usage.total_tokens.unwrap_or_else(|| {
                                    request.prompt.split_whitespace().count() as u32 + 
                                    text.split_whitespace().count() as u32
                                })
                            )
                        } else {
                            let gen = text.split_whitespace().count() as u32;
                            let total = request.prompt.split_whitespace().count() as u32 + gen;
                            (gen, total)
                        };

                        (text, tokens_generated, total_tokens)
                    }
                };

                Ok(InferenceResponse {
                    text,
                    tokens_generated,
                    total_tokens,
                    finish_reason: FinishReason::Stop,
                    timing: InferenceTiming {
                        prompt_eval_time: Duration::from_millis(0), // Not available from API
                        eval_time: total_time,
                        total_time,
                    },
                    model_used: self.model_info.name.clone(),
                    is_fallback: false,
                })
            }
            Ok(Err(e)) => Err(ModelHostError::Api(e)),
            Err(_) => Err(ModelHostError::Timeout { 
                timeout_ms: timeout_duration.as_millis() as u64 
            }),
        }
    }

    async fn infer_stream(&self, request: InferenceRequest) -> Result<TokenStream, ModelHostError> {
        if !self.loaded.load(Ordering::SeqCst) {
            return Err(ModelHostError::ModelLoad("API not connected".to_string()));
        }

        // For now, simulate streaming by breaking up a regular response
        let response = self.infer(request).await?;
        let (tx, rx) = mpsc::unbounded_channel();
        
        tokio::spawn(async move {
            let words: Vec<&str> = response.text.split_whitespace().collect();
            for (i, word) in words.iter().enumerate() {
                tokio::time::sleep(Duration::from_millis(100)).await;
                let stream_token = StreamToken {
                    token: format!("{} ", word),
                    is_final: i == words.len() - 1,
                    token_index: i as u32,
                    timestamp: Instant::now(),
                };
                if tx.send(Ok(stream_token)).is_err() {
                    break;
                }
            }
        });

        Ok(Box::pin(UnboundedReceiverStream::new(rx)))
    }

    async fn batch_infer(&self, requests: Vec<InferenceRequest>) -> Result<Vec<InferenceResponse>, ModelHostError> {
        // Remote APIs typically handle requests sequentially to respect rate limits
        let mut responses = Vec::new();
        
        for request in requests {
            // Add small delay between requests to respect rate limits
            tokio::time::sleep(Duration::from_millis(100)).await;
            responses.push(self.infer(request).await?);
        }
        
        Ok(responses)
    }

    fn is_loaded(&self) -> bool {
        self.loaded.load(Ordering::SeqCst)
    }

    fn get_model_info(&self) -> ModelInfo {
        self.model_info.clone()
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_batch(&self) -> bool {
        true
    }

    async fn health_check(&self) -> Result<(), ModelHostError> {
        if !self.loaded.load(Ordering::SeqCst) {
            return Err(ModelHostError::ModelLoad("API not connected".to_string()));
        }

        let api_endpoint = self.config.api_endpoint.as_ref()
            .ok_or_else(|| ModelHostError::Config("No API endpoint specified".to_string()))?;

        let mut request_builder = self.client
            .get(api_endpoint)
            .timeout(Duration::from_secs(5));

        if let Some(key) = &self.api_key {
            request_builder = request_builder.header("Authorization", format!("Bearer {}", key.get()));
        }

        match request_builder.send().await {
            Ok(response) if response.status().is_success() || response.status() == 404 => Ok(()),
            Ok(response) => Err(ModelHostError::Api(response.error_for_status().unwrap_err())),
            Err(e) => Err(ModelHostError::Api(e)),
        }
    }

    async fn warmup(&self) -> Result<(), ModelHostError> {
        // Remote APIs don't typically need warmup, but we can do a quick test
        self.health_check().await
    }
}

pub struct ModelHost {
    workers: Arc<RwLock<HashMap<String, Vec<ModelWorker>>>>,
    configs: Arc<RwLock<HashMap<String, ModelConfig>>>,
    stats: Arc<RwLock<ModelHostStats>>,
    vram_stats: Arc<VramStats>,
    current_model: Arc<RwLock<Option<String>>>,
    request_queue: Arc<Mutex<VecDeque<QueuedRequest>>>,
    hot_swap_tx: mpsc::UnboundedSender<HotSwapRequest>,
    fallback_chains: Arc<RwLock<HashMap<String, Vec<String>>>>,
    pool_size: usize,
    max_concurrent: usize,
    shutdown_tx: broadcast::Sender<()>,
}

#[derive(Debug)]
struct QueuedRequest {
    request: InferenceRequest,
    response_tx: tokio::sync::oneshot::Sender<Result<InferenceResponse, ModelHostError>>,
    priority: InferencePriority,
    queued_at: Instant,
}

#[derive(Debug, Default, Clone)]
pub struct ModelHostStats {
    pub total_requests: u64,
    pub total_tokens_generated: u64,
    pub total_inference_time: Duration,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub errors: u64,
    pub hot_swaps: u64,
    pub vram_throttles: u64,
    pub fallback_activations: u64,
    pub batch_requests: u64,
    pub stream_requests: u64,
    pub queue_wait_time: Duration,
    pub active_workers: u64,
}

#[derive(Debug, Clone)]
pub struct HotSwapRequest {
    pub target_model: String,
    pub force: bool,
}

impl ModelHost {
    pub fn new(pool_size: usize, max_concurrent: usize, total_vram_mb: u64) -> Self {
        let (hot_swap_tx, _) = mpsc::unbounded_channel();
        let (shutdown_tx, _) = broadcast::channel(1);

        Self {
            workers: Arc::new(RwLock::new(HashMap::new())),
            configs: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(RwLock::new(ModelHostStats::default())),
            vram_stats: Arc::new(VramStats::new(total_vram_mb)),
            current_model: Arc::new(RwLock::new(None)),
            request_queue: Arc::new(Mutex::new(VecDeque::new())),
            hot_swap_tx,
            fallback_chains: Arc::new(RwLock::new(HashMap::new())),
            pool_size,
            max_concurrent,
            shutdown_tx,
        }
    }

    /// Register a model with its configuration
    pub async fn register_model(
        &self,
        config: ModelConfig,
    ) -> Result<(), ModelHostError> {
        let name = config.name.clone();
        info!("Registering model: {} (type: {:?})", name, config.model_type);

        // Create adapter based on model type
        let adapter: Box<dyn ModelAdapter> = match config.model_type {
            ModelType::LocalGGUF => Box::new(LocalGGUFAdapter::new(config.clone())),
            ModelType::MLC => Box::new(MLCAdapter::new(config.clone())),
            ModelType::VLLM => Box::new(VLLMAdapter::new(config.clone())),
            ModelType::OpenAI | ModelType::Gemini | ModelType::Anthropic | 
            ModelType::Ollama | ModelType::RemoteAPI => {
                Box::new(RemoteAPIAdapter::new(config.clone())?)
            }
        };

        // Create worker pool for this model
        let mut workers = Vec::new();
        let pool_size = config.warm_pool_size.max(1);
        
        for i in 0..pool_size {
            let worker_id = format!("{}-worker-{}", name, i);
            let worker = ModelWorker {
                id: worker_id.clone(),
                adapter: Arc::new(Mutex::new(adapter)),
                is_busy: AtomicBool::new(false),
                last_used: Arc::new(Mutex::new(Instant::now())),
                requests_processed: AtomicU64::new(0),
            };
            workers.push(worker);
            
            // Only create one adapter and clone the reference
            if i == 0 {
                continue;
            }
            
            // For subsequent workers, create new adapters
            let adapter_clone: Box<dyn ModelAdapter> = match config.model_type {
                ModelType::LocalGGUF => Box::new(LocalGGUFAdapter::new(config.clone())),
                ModelType::MLC => Box::new(MLCAdapter::new(config.clone())),
                ModelType::VLLM => Box::new(VLLMAdapter::new(config.clone())),
                ModelType::OpenAI | ModelType::Gemini | ModelType::Anthropic | 
                ModelType::Ollama | ModelType::RemoteAPI => {
                    Box::new(RemoteAPIAdapter::new(config.clone())?)
                }
            };
            
            workers[i].adapter = Arc::new(Mutex::new(adapter_clone));
        }

        // Store workers and config
        self.workers.write().await.insert(name.clone(), workers);
        self.configs.write().await.insert(name.clone(), config.clone());
        
        // Set up fallback chain if specified
        if !config.fallback_models.is_empty() {
            self.fallback_chains.write().await
                .insert(name.clone(), config.fallback_models);
        }

        info!("Model registered successfully: {} with {} workers", name, pool_size);
        Ok(())
    }

    /// Load a model and all its workers
    pub async fn load_model(&self, name: &str) -> Result<(), ModelHostError> {
        info!("Loading model: {}", name);
        
        let workers = {
            let workers_guard = self.workers.read().await;
            workers_guard.get(name)
                .ok_or_else(|| ModelHostError::ModelNotFound { name: name.to_string() })?
                .clone()
        };

        let config = {
            let configs_guard = self.configs.read().await;
            configs_guard.get(name)
                .ok_or_else(|| ModelHostError::Config(format!("No config found for model: {}", name)))?
                .clone()
        };

        // Check VRAM requirements for local models
        if matches!(config.model_type, ModelType::LocalGGUF | ModelType::MLC | ModelType::VLLM) {
            self.vram_stats.allocate(config.vram_required_mb)?;
        }

        // Load all workers
        let mut load_tasks = Vec::new();
        for worker in workers {
            let load_task = tokio::spawn(async move {
                let mut adapter = worker.adapter.lock().await;
                adapter.load().await
            });
            load_tasks.push(load_task);
        }

        // Wait for all workers to load
        for task in load_tasks {
            task.await.map_err(|e| ModelHostError::ModelLoad(format!("Worker load failed: {}", e)))??;
        }

        // Warm up workers
        self.warmup_model(name).await?;
        
        info!("Model loaded successfully: {}", name);
        Ok(())
    }

    /// Execute inference with fallback support
    pub async fn infer(
        &self,
        mut request: InferenceRequest,
    ) -> Result<InferenceResponse, ModelHostError> {
        let start_time = Instant::now();
        
        // Update stats
        {
            let mut stats = self.stats.write().await;
            stats.total_requests += 1;
        }

        // Set up fallback chain
        let fallback_models = if let Some(chain) = &request.fallback_chain {
            chain.clone()
        } else {
            let fallback_chains = self.fallback_chains.read().await;
            fallback_chains.get(&request.model_name).cloned().unwrap_or_default()
        };

        let mut fallback_chain = FallbackChain::new({
            let mut models = vec![request.model_name.clone()];
            models.extend(fallback_models);
            models
        });

        let mut last_error = None;

        // Try each model in the fallback chain
        while let Some(model_name) = fallback_chain.next_model() {
            debug!("Trying model: {} for inference", model_name);
            
            request.model_name = model_name.clone();
            
            match self.execute_inference_with_model(&request).await {
                Ok(mut response) => {
                    // Mark if this was a fallback
                    response.is_fallback = fallback_chain.current_index > 1;
                    response.model_used = model_name;

                    if response.is_fallback {
                        let mut stats = self.stats.write().await;
                        stats.fallback_activations += 1;
                        warn!("Fallback activated: used {} instead of {}", 
                             model_name, fallback_chain.models[0]);
                    }

                    // Update success stats
                    let inference_time = start_time.elapsed();
                    let mut stats = self.stats.write().await;
                    stats.total_tokens_generated += response.tokens_generated as u64;
                    stats.total_inference_time += inference_time;
                    
                    return Ok(response);
                }
                Err(e) => {
                    warn!("Model {} failed: {}", model_name, e);
                    fallback_chain.mark_failed(model_name);
                    last_error = Some(e);
                    
                    // Update error stats
                    let mut stats = self.stats.write().await;
                    stats.errors += 1;
                }
            }
        }

        // All models in chain failed
        let mut stats = self.stats.write().await;
        stats.errors += 1;

        Err(last_error.unwrap_or(ModelHostError::FallbackExhausted { 
            count: fallback_chain.models.len() 
        }))
    }

    /// Execute inference with a specific model
    async fn execute_inference_with_model(
        &self,
        request: &InferenceRequest,
    ) -> Result<InferenceResponse, ModelHostError> {
        // Get an available worker for the model
        let worker = self.get_available_worker(&request.model_name).await?;
        
        // Mark worker as busy
        worker.is_busy.store(true, Ordering::SeqCst);
        
        let result = {
            let adapter = worker.adapter.lock().await;
            
            // Perform health check first
            if let Err(e) = adapter.health_check().await {
                warn!("Health check failed for {}: {}", request.model_name, e);
                return Err(e);
            }
            
            // Execute inference
            adapter.infer(request.clone()).await
        };

        // Mark worker as available and update stats
        worker.is_busy.store(false, Ordering::SeqCst);
        *worker.last_used.lock().await = Instant::now();
        worker.requests_processed.fetch_add(1, Ordering::SeqCst);

        result
    }

    /// Execute streaming inference
    pub async fn infer_stream(
        &self,
        request: InferenceRequest,
    ) -> Result<TokenStream, ModelHostError> {
        debug!("Starting streaming inference for model: {}", request.model_name);
        
        let mut stats = self.stats.write().await;
        stats.stream_requests += 1;
        drop(stats);

        let worker = self.get_available_worker(&request.model_name).await?;
        worker.is_busy.store(true, Ordering::SeqCst);
        
        let stream_result = {
            let adapter = worker.adapter.lock().await;
            adapter.infer_stream(request.clone()).await
        };

        // Note: Worker will be marked as available when the stream completes
        // For now, we'll mark it available immediately (in a real implementation,
        // we'd track stream completion)
        worker.is_busy.store(false, Ordering::SeqCst);
        
        stream_result
    }

    /// Execute batch inference
    pub async fn batch_infer(
        &self,
        batch_request: BatchInferenceRequest,
    ) -> Result<Vec<InferenceResponse>, ModelHostError> {
        info!("Starting batch inference with {} requests", batch_request.requests.len());
        
        let mut stats = self.stats.write().await;
        stats.batch_requests += 1;
        stats.total_requests += batch_request.requests.len() as u64;
        drop(stats);

        // Group requests by model
        let mut model_batches: HashMap<String, Vec<InferenceRequest>> = HashMap::new();
        for request in batch_request.requests {
            model_batches.entry(request.model_name.clone())
                .or_insert_with(Vec::new)
                .push(request);
        }

        let mut all_responses = Vec::new();
        
        // Process each model's batch
        for (model_name, requests) in model_batches {
            debug!("Processing batch for model: {} ({} requests)", model_name, requests.len());
            
            let worker = self.get_available_worker(&model_name).await?;
            worker.is_busy.store(true, Ordering::SeqCst);
            
            let batch_result = {
                let adapter = worker.adapter.lock().await;
                
                if adapter.supports_batch() {
                    // Use native batch processing
                    adapter.batch_infer(requests).await
                } else {
                    // Fall back to sequential processing
                    let mut responses = Vec::new();
                    for request in requests {
                        responses.push(adapter.infer(request).await?);
                    }
                    Ok(responses)
                }
            };

            worker.is_busy.store(false, Ordering::SeqCst);
            *worker.last_used.lock().await = Instant::now();

            match batch_result {
                Ok(responses) => all_responses.extend(responses),
                Err(e) => return Err(e),
            }
        }

        info!("Batch inference completed: {} responses", all_responses.len());
        Ok(all_responses)
    }

    /// Get an available worker for a model
    async fn get_available_worker(&self, model_name: &str) -> Result<ModelWorker, ModelHostError> {
        let workers = self.workers.read().await;
        let model_workers = workers.get(model_name)
            .ok_or_else(|| ModelHostError::ModelNotFound { name: model_name.to_string() })?;

        // Find an available worker
        for worker in model_workers {
            if !worker.is_busy.load(Ordering::SeqCst) {
                return Ok(worker.clone());
            }
        }

        // All workers are busy - wait or fail
        Err(ModelHostError::PoolExhausted { count: model_workers.len() })
    }

    pub async fn get_model_info(&self, name: &str) -> Result<ModelInfo, ModelHostError> {
        let workers = self.workers.read().await;
        if let Some(model_workers) = workers.get(name) {
            if let Some(worker) = model_workers.first() {
                let adapter = worker.adapter.lock().await;
                Ok(adapter.get_model_info())
            } else {
                Err(ModelHostError::ModelNotFound { name: name.to_string() })
            }
        } else {
            Err(ModelHostError::ModelNotFound { name: name.to_string() })
        }
    }

    pub async fn list_models(&self) -> Vec<ModelInfo> {
        let adapters = self.adapters.read().await;
        adapters.values().map(|a| a.get_model_info()).collect()
    }

    pub async fn unload_model(&self, name: &str) -> Result<(), ModelHostError> {
        let adapters = self.adapters.read().await;
        if let Some(adapter) = adapters.get(name) {
            // Simplified - in practice, we'd need mutable access
            Ok(())
        } else {
            Err(ModelHostError::ModelNotFound {
                name: name.to_string(),
            })
        }
    }

    pub async fn get_stats(&self) -> ModelHostStats {
        self.stats.read().await.clone()
    }

    /// Request a hot-swap to a different model
    pub async fn request_hot_swap(
        &self,
        target_model: String,
        force: bool,
    ) -> Result<(), ModelHostError> {
        let request = HotSwapRequest {
            target_model,
            force,
        };
        self.hot_swap_tx
            .send(request)
            .map_err(|_| ModelHostError::HotSwapFailed {
                reason: "Channel closed".to_string(),
            })?;
        Ok(())
    }

    /// Perform hot-swap operation
    pub async fn perform_hot_swap(&self, request: HotSwapRequest) -> Result<(), ModelHostError> {
        let start_time = Instant::now();

        // Check if target model exists
        let adapters = self.adapters.read().await;
        let target_adapter =
            adapters
                .get(&request.target_model)
                .ok_or_else(|| ModelHostError::ModelNotFound {
                    name: request.target_model.clone(),
                })?;

        let target_info = target_adapter.get_model_info();

        // Check VRAM availability
        if !request.force {
            let available = self.vram_stats.available_mb.load(Ordering::SeqCst);
            if available < target_info.vram_required_mb {
                return Err(ModelHostError::VramExhausted {
                    required: target_info.vram_required_mb,
                    available,
                });
            }
        }

        // Unload current model if any
        if let Some(current) = self.current_model.read().await.as_ref() {
            if let Some(current_adapter) = adapters.get(current) {
                let current_info = current_adapter.get_model_info();
                self.vram_stats.deallocate(current_info.vram_required_mb);
            }
        }

        // Load target model
        // In practice, this would send SIGHUP to vLLM and update symlink
        self.vram_stats.allocate(target_info.vram_required_mb)?;

        // Update current model
        *self.current_model.write().await = Some(request.target_model.clone());

        // Update stats
        let mut stats = self.stats.write().await;
        stats.hot_swaps += 1;

        let swap_time = start_time.elapsed();
        if swap_time > Duration::from_secs(3) {
            tracing::warn!("Hot-swap took longer than target: {:?}", swap_time);
        }

        Ok(())
    }

    /// Get current VRAM usage
    pub fn get_vram_usage(&self) -> (u64, u64, f32) {
        let used = self.vram_stats.used_mb.load(Ordering::SeqCst);
        let total = self.vram_stats.total_mb;
        let percent = self.vram_stats.get_usage_percent();
        (used, total, percent)
    }

    /// Check if VRAM usage is high (>90%)
    pub fn is_vram_high_usage(&self) -> bool {
        self.vram_stats.get_usage_percent() > 90.0
    }

    /// Get current model
    pub async fn get_current_model(&self) -> Option<String> {
        self.current_model.read().await.clone()
    }

    /// Process pending hot-swap requests
    pub async fn process_hot_swap_requests(&mut self) -> Result<(), ModelHostError> {
        while let Ok(request) = self.hot_swap_rx.try_recv() {
            self.perform_hot_swap(request).await?;
        }
        Ok(())
    }

    pub async fn warmup_model(&self, name: &str) -> Result<(), ModelHostError> {
        let mut warm_pool = self.warm_pool.write().await;
        if warm_pool.len() >= self.pool_size {
            // Evict least recently used (simplified)
            if let Some((key, _)) = warm_pool.iter().next() {
                let key = key.clone();
                warm_pool.remove(&key);
            }
        }

        // Clone the adapter for warm pool (simplified)
        let adapters = self.adapters.read().await;
        if let Some(adapter) = adapters.get(name) {
            // In practice, we'd clone or create a new instance
            // warm_pool.insert(name.to_string(), adapter.clone());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_local_gguf_adapter() {
        let model_path = PathBuf::from("/fake/path/model.gguf");
        let mut adapter = LocalGGUFAdapter::new(model_path, "test-model".to_string(), 4096, 2048);

        assert!(!adapter.is_loaded());

        adapter.load().await.unwrap();
        assert!(adapter.is_loaded());

        let request = InferenceRequest {
            prompt: "Hello, world!".to_string(),
            model_name: "test-model".to_string(),
            parameters: InferenceParameters::default(),
            context: None,
        };

        let response = adapter.infer(request).await.unwrap();
        assert!(response.text.contains("Hello, world!"));
        assert!(response.tokens_generated > 0);

        adapter.unload().await.unwrap();
        assert!(!adapter.is_loaded());
    }

    #[tokio::test]
    async fn test_remote_api_adapter() {
        // This test would require a mock server, so we'll just test the interface
        let adapter = RemoteAPIAdapter::new(
            "https://api.example.com/v1/completions".to_string(),
            Some("fake-key".to_string()),
            "test-model".to_string(),
            4096,
            0, // Remote API doesn't use local VRAM
        );

        assert!(!adapter.is_loaded());
        let info = adapter.get_model_info();
        assert_eq!(info.name, "test-model");
        assert_eq!(info.model_type, ModelType::RemoteAPI);
    }

    #[tokio::test]
    async fn test_model_host_basic_operations() {
        let host = ModelHost::new(5, 8192);

        let model_path = PathBuf::from("/fake/path/model.gguf");
        let adapter = Box::new(LocalGGUFAdapter::new(
            model_path,
            "test-model".to_string(),
            4096,
            2048,
        ));

        host.register_model("test-model".to_string(), adapter)
            .await
            .unwrap();

        let models = host.list_models().await;
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "test-model");

        let info = host.get_model_info("test-model").await.unwrap();
        assert_eq!(info.name, "test-model");
    }

    #[tokio::test]
    async fn test_model_host_inference() {
        let host = ModelHost::new(5, 8192);

        let model_path = PathBuf::from("/fake/path/model.gguf");
        let adapter = Box::new(LocalGGUFAdapter::new(
            model_path,
            "test-model".to_string(),
            4096,
            2048,
        ));

        host.register_model("test-model".to_string(), adapter)
            .await
            .unwrap();

        let request = InferenceRequest {
            prompt: "Test prompt".to_string(),
            model_name: "test-model".to_string(),
            parameters: InferenceParameters::default(),
            context: None,
        };

        let response = host.infer(request).await.unwrap();
        assert!(response.text.contains("Test prompt"));
        assert!(response.tokens_generated > 0);

        let stats = host.get_stats().await;
        assert_eq!(stats.total_requests, 1);
        assert!(stats.total_tokens_generated > 0);
    }

    #[tokio::test]
    async fn test_model_not_found() {
        let host = ModelHost::new(5, 8192);

        let request = InferenceRequest {
            prompt: "Test".to_string(),
            model_name: "nonexistent".to_string(),
            parameters: InferenceParameters::default(),
            context: None,
        };

        let result = host.infer(request).await;
        assert!(matches!(result, Err(ModelHostError::ModelNotFound { .. })));
    }

    #[tokio::test]
    async fn test_hot_swap() {
        let mut host = ModelHost::new(5, 8192);

        let model_path1 = PathBuf::from("/fake/path/model1.gguf");
        let adapter1 = Box::new(LocalGGUFAdapter::new(
            model_path1,
            "model1".to_string(),
            4096,
            2048,
        ));

        let model_path2 = PathBuf::from("/fake/path/model2.gguf");
        let adapter2 = Box::new(LocalGGUFAdapter::new(
            model_path2,
            "model2".to_string(),
            4096,
            2048,
        ));

        host.register_model("model1".to_string(), adapter1)
            .await
            .unwrap();
        host.register_model("model2".to_string(), adapter2)
            .await
            .unwrap();

        // Initial state
        assert!(host.get_current_model().await.is_none());

        // Request hot-swap
        host.request_hot_swap("model1".to_string(), false)
            .await
            .unwrap();
        host.process_hot_swap_requests().await.unwrap();

        assert_eq!(host.get_current_model().await, Some("model1".to_string()));

        // Check VRAM usage
        let (used, total, percent) = host.get_vram_usage();
        assert_eq!(used, 2048);
        assert_eq!(total, 8192);
        assert!((percent - 25.0).abs() < 0.1);

        // Hot-swap to model2
        host.request_hot_swap("model2".to_string(), false)
            .await
            .unwrap();
        host.process_hot_swap_requests().await.unwrap();

        assert_eq!(host.get_current_model().await, Some("model2".to_string()));

        let (used, _, _) = host.get_vram_usage();
        assert_eq!(used, 2048); // Should be same since we deallocated model1
    }
}
