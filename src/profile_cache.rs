use crate::model_host::{ModelHost, ModelInfo};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::RwLock;

#[derive(Error, Debug)]
pub enum ProfileCacheError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Profile not found: {name}")]
    ProfileNotFound { name: String },
    #[error("Cache directory not accessible")]
    CacheDirectoryNotFound,
    #[error("Invalid profile data: {reason}")]
    InvalidProfileData { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelProfile {
    pub name: String,
    pub model_name: String,
    pub prompt_template: String,
    pub system_message: Option<String>,
    pub parameters: ModelParameters,
    pub created_at: u64,
    pub updated_at: u64,
    pub model_info: Option<ModelInfo>,
    pub usage_stats: ProfileUsageStats,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelParameters {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: Option<u32>,
    pub max_tokens: u32,
    pub repetition_penalty: f32,
    pub frequency_penalty: f32,
    pub presence_penalty: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProfileUsageStats {
    pub total_requests: u64,
    pub total_tokens: u64,
    pub average_latency_ms: f64,
    pub last_used: Option<u64>,
    pub success_rate: f64,
}

impl Default for ProfileUsageStats {
    fn default() -> Self {
        Self {
            total_requests: 0,
            total_tokens: 0,
            average_latency_ms: 0.0,
            last_used: None,
            success_rate: 1.0,
        }
    }
}

impl Default for ModelParameters {
    fn default() -> Self {
        Self {
            temperature: 0.7,
            top_p: 0.9,
            top_k: None,
            max_tokens: 2048,
            repetition_penalty: 1.0,
            frequency_penalty: 0.0,
            presence_penalty: 0.0,
        }
    }
}

#[derive(Debug, Clone)]
struct CacheEntry {
    profile: ModelProfile,
    last_accessed: Instant,
    access_count: u64,
}

pub struct ProfileCache {
    cache: Arc<RwLock<HashMap<String, CacheEntry>>>,
    cache_dir: PathBuf,
    max_size: usize,
    ttl: Duration,
}

impl ProfileCache {
    /// Create a new profile cache with the specified cache directory
    pub fn new(cache_dir: PathBuf, max_size: usize, ttl: Duration) -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            cache_dir,
            max_size,
            ttl,
        }
    }

    /// Initialize the cache by loading existing profiles from disk
    pub async fn initialize(&self) -> Result<(), ProfileCacheError> {
        if !self.cache_dir.exists() {
            std::fs::create_dir_all(&self.cache_dir)?;
        }

        // Load all profile files from cache directory
        let mut cache = self.cache.write().await;
        let mut entries = tokio::fs::read_dir(&self.cache_dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            if entry.path().extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(profile) = self.load_profile_from_file(&entry.path()).await {
                    let entry = CacheEntry {
                        profile,
                        last_accessed: Instant::now(),
                        access_count: 0,
                    };
                    cache.insert(entry.profile.name.clone(), entry);
                }
            }
        }

        Ok(())
    }

    /// Get a profile by name, loading from disk if not in cache
    pub async fn get_profile(&self, name: &str) -> Result<ModelProfile, ProfileCacheError> {
        let mut cache = self.cache.write().await;

        if let Some(entry) = cache.get_mut(name) {
            entry.last_accessed = Instant::now();
            entry.access_count += 1;
            return Ok(entry.profile.clone());
        }

        // Load from disk
        let profile_path = self.get_profile_path(name);
        let profile = self.load_profile_from_file(&profile_path).await?;
        let entry = CacheEntry {
            profile: profile.clone(),
            last_accessed: Instant::now(),
            access_count: 1,
        };
        cache.insert(name.to_string(), entry);

        // Evict if cache is full
        self.evict_if_needed(&mut cache).await;

        Ok(profile)
    }

    /// Store a profile in the cache and persist to disk
    pub async fn store_profile(&self, profile: ModelProfile) -> Result<(), ProfileCacheError> {
        let mut cache = self.cache.write().await;
        let now = Instant::now();

        let entry = CacheEntry {
            profile: profile.clone(),
            last_accessed: now,
            access_count: 1,
        };

        cache.insert(profile.name.clone(), entry);
        self.save_profile_to_file(&profile).await?;
        self.evict_if_needed(&mut cache).await;

        Ok(())
    }

    /// Update an existing profile
    pub async fn update_profile(
        &self,
        name: &str,
        updater: impl FnOnce(&mut ModelProfile),
    ) -> Result<(), ProfileCacheError> {
        let mut cache = self.cache.write().await;

        if let Some(entry) = cache.get_mut(name) {
            updater(&mut entry.profile);
            entry.last_accessed = Instant::now();
            entry.profile.updated_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();

            self.save_profile_to_file(&entry.profile).await?;
            Ok(())
        } else {
            Err(ProfileCacheError::ProfileNotFound {
                name: name.to_string(),
            })
        }
    }

    /// Delete a profile from cache and disk
    pub async fn delete_profile(&self, name: &str) -> Result<(), ProfileCacheError> {
        let mut cache = self.cache.write().await;
        cache.remove(name);

        let profile_path = self.get_profile_path(name);
        if profile_path.exists() {
            tokio::fs::remove_file(&profile_path).await?;
        }

        Ok(())
    }

    /// List all profile names in the cache
    pub async fn list_profiles(&self) -> Vec<String> {
        let cache = self.cache.read().await;
        cache.keys().cloned().collect()
    }

    /// Get cache statistics
    pub async fn get_stats(&self) -> HashMap<String, u64> {
        let cache = self.cache.read().await;
        let mut stats = HashMap::new();

        stats.insert("total_profiles".to_string(), cache.len() as u64);
        stats.insert(
            "total_accesses".to_string(),
            cache.values().map(|e| e.access_count).sum(),
        );

        stats
    }

    /// Clear expired entries from cache
    pub async fn cleanup_expired(&self) {
        let mut cache = self.cache.write().await;
        let now = Instant::now();

        cache.retain(|_, entry| now.duration_since(entry.last_accessed) < self.ttl);
    }

    /// Sync profile with model host information
    pub async fn sync_with_model_host(
        &self,
        model_host: &ModelHost,
    ) -> Result<(), ProfileCacheError> {
        let mut cache = self.cache.write().await;

        for entry in cache.values_mut() {
            if let Ok(model_info) = model_host.get_model_info(&entry.profile.model_name).await {
                entry.profile.model_info = Some(model_info);
            }
        }

        Ok(())
    }

    /// Update usage statistics for a profile
    pub async fn update_usage_stats(
        &self,
        name: &str,
        tokens_used: u32,
        latency_ms: f64,
        success: bool,
    ) -> Result<(), ProfileCacheError> {
        let mut cache = self.cache.write().await;

        if let Some(entry) = cache.get_mut(name) {
            let stats = &mut entry.profile.usage_stats;
            stats.total_requests += 1;
            stats.total_tokens += tokens_used as u64;

            // Update rolling average for latency
            let total_requests = stats.total_requests as f64;
            stats.average_latency_ms =
                (stats.average_latency_ms * (total_requests - 1.0) + latency_ms) / total_requests;

            // Update success rate
            let current_successes = (stats.success_rate * (total_requests - 1.0)) as u64;
            let new_successes = if success {
                current_successes + 1
            } else {
                current_successes
            };
            stats.success_rate = new_successes as f64 / total_requests;

            stats.last_used = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            );

            // Save updated profile
            self.save_profile_to_file(&entry.profile).await?;
        }

        Ok(())
    }

    /// Get profile recommendations based on usage patterns
    pub async fn get_recommendations(&self) -> Vec<String> {
        let cache = self.cache.read().await;
        let mut profiles: Vec<_> = cache.values().collect();

        // Sort by usage frequency and success rate
        profiles.sort_by(|a, b| {
            let a_score =
                a.profile.usage_stats.total_requests as f64 * a.profile.usage_stats.success_rate;
            let b_score =
                b.profile.usage_stats.total_requests as f64 * b.profile.usage_stats.success_rate;
            b_score
                .partial_cmp(&a_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        profiles
            .into_iter()
            .take(5)
            .map(|entry| entry.profile.name.clone())
            .collect()
    }

    /// Export profiles to a backup file
    pub async fn export_profiles(&self, export_path: &Path) -> Result<(), ProfileCacheError> {
        let cache = self.cache.read().await;
        let profiles: HashMap<String, &ModelProfile> = cache
            .iter()
            .map(|(name, entry)| (name.clone(), &entry.profile))
            .collect();

        let json = serde_json::to_string_pretty(&profiles)?;
        tokio::fs::write(export_path, json).await?;
        Ok(())
    }

    /// Import profiles from a backup file
    pub async fn import_profiles(&self, import_path: &Path) -> Result<(), ProfileCacheError> {
        let content = tokio::fs::read_to_string(import_path).await?;
        let imported_profiles: HashMap<String, ModelProfile> = serde_json::from_str(&content)?;

        let mut cache = self.cache.write().await;

        for (name, profile) in imported_profiles {
            let entry = CacheEntry {
                profile,
                last_accessed: Instant::now(),
                access_count: 0,
            };
            cache.insert(name, entry);
        }

        Ok(())
    }

    /// Get profiles by model type
    pub async fn get_profiles_by_model(&self, model_name: &str) -> Vec<ModelProfile> {
        let cache = self.cache.read().await;
        cache
            .values()
            .filter(|entry| entry.profile.model_name == model_name)
            .map(|entry| entry.profile.clone())
            .collect()
    }

    /// Optimize cache based on usage patterns
    pub async fn optimize_cache(&self) {
        let mut cache = self.cache.write().await;

        // Remove profiles with very low usage
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        cache.retain(|_, entry| {
            if let Some(last_used) = entry.profile.usage_stats.last_used {
                // Keep if used within last 30 days or has high usage
                now - last_used < 30 * 24 * 3600 || entry.profile.usage_stats.total_requests > 10
            } else {
                // Keep if never used but recently created
                if let Some(created_duration) = now.checked_sub(entry.profile.created_at) {
                    created_duration < 7 * 24 * 3600 // Keep for 7 days if never used
                } else {
                    false
                }
            }
        });
    }

    /// Get the file path for a profile
    fn get_profile_path(&self, name: &str) -> PathBuf {
        self.cache_dir.join(format!("{}.json", name))
    }

    /// Load a profile from disk
    async fn load_profile_from_file(&self, path: &Path) -> Result<ModelProfile, ProfileCacheError> {
        let content = tokio::fs::read_to_string(path).await?;
        let profile: ModelProfile = serde_json::from_str(&content)?;

        // Validate profile data
        if profile.name.is_empty() {
            return Err(ProfileCacheError::InvalidProfileData {
                reason: "Profile name cannot be empty".to_string(),
            });
        }

        if profile.model_name.is_empty() {
            return Err(ProfileCacheError::InvalidProfileData {
                reason: "Model name cannot be empty".to_string(),
            });
        }

        Ok(profile)
    }

    /// Save a profile to disk
    async fn save_profile_to_file(&self, profile: &ModelProfile) -> Result<(), ProfileCacheError> {
        let path = self.get_profile_path(&profile.name);
        let content = serde_json::to_string_pretty(profile)?;
        tokio::fs::write(&path, content).await?;
        Ok(())
    }

    /// Evict entries if cache exceeds maximum size (LRU policy)
    async fn evict_if_needed(&self, cache: &mut HashMap<String, CacheEntry>) {
        if cache.len() <= self.max_size {
            return;
        }

        // Find the least recently used entry
        let mut lru_name = None;
        let mut lru_time = Instant::now();

        for (name, entry) in cache.iter() {
            if entry.last_accessed < lru_time {
                lru_time = entry.last_accessed;
                lru_name = Some(name.clone());
            }
        }

        if let Some(name) = lru_name {
            cache.remove(&name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_profile_cache_basic_operations() {
        let temp_dir = TempDir::new().unwrap();
        let cache = ProfileCache::new(temp_dir.path().to_path_buf(), 10, Duration::from_secs(3600));
        cache.initialize().await.unwrap();

        // Create a test profile
        let profile = ModelProfile {
            name: "test_profile".to_string(),
            model_name: "mistral-7b".to_string(),
            prompt_template: "You are a helpful assistant. {prompt}".to_string(),
            system_message: Some("You are a helpful AI assistant.".to_string()),
            parameters: ModelParameters::default(),
            created_at: 1234567890,
            updated_at: 1234567890,
            model_info: None,
            usage_stats: ProfileUsageStats::default(),
        };

        // Store profile
        cache.store_profile(profile.clone()).await.unwrap();

        // Retrieve profile
        let retrieved = cache.get_profile("test_profile").await.unwrap();
        assert_eq!(retrieved, profile);

        // List profiles
        let profiles = cache.list_profiles().await;
        assert!(profiles.contains(&"test_profile".to_string()));
    }

    #[tokio::test]
    async fn test_profile_cache_update() {
        let temp_dir = TempDir::new().unwrap();
        let cache = ProfileCache::new(temp_dir.path().to_path_buf(), 10, Duration::from_secs(3600));
        cache.initialize().await.unwrap();

        let mut profile = ModelProfile {
            name: "update_test".to_string(),
            model_name: "mistral-7b".to_string(),
            prompt_template: "Initial template".to_string(),
            system_message: None,
            parameters: ModelParameters::default(),
            created_at: 1234567890,
            updated_at: 1234567890,
            model_info: None,
            usage_stats: ProfileUsageStats::default(),
        };

        cache.store_profile(profile.clone()).await.unwrap();

        // Update profile
        cache
            .update_profile("update_test", |p| {
                p.prompt_template = "Updated template".to_string();
            })
            .await
            .unwrap();

        let updated = cache.get_profile("update_test").await.unwrap();
        assert_eq!(updated.prompt_template, "Updated template");
    }

    #[tokio::test]
    async fn test_profile_cache_delete() {
        let temp_dir = TempDir::new().unwrap();
        let cache = ProfileCache::new(temp_dir.path().to_path_buf(), 10, Duration::from_secs(3600));
        cache.initialize().await.unwrap();

        let profile = ModelProfile {
            name: "delete_test".to_string(),
            model_name: "mistral-7b".to_string(),
            prompt_template: "Test template".to_string(),
            system_message: None,
            parameters: ModelParameters::default(),
            created_at: 1234567890,
            updated_at: 1234567890,
            model_info: None,
            usage_stats: ProfileUsageStats::default(),
        };

        cache.store_profile(profile).await.unwrap();
        assert!(cache
            .list_profiles()
            .await
            .contains(&"delete_test".to_string()));

        cache.delete_profile("delete_test").await.unwrap();
        assert!(!cache
            .list_profiles()
            .await
            .contains(&"delete_test".to_string()));
    }

    #[tokio::test]
    async fn test_profile_cache_eviction() {
        let temp_dir = TempDir::new().unwrap();
        let cache = ProfileCache::new(temp_dir.path().to_path_buf(), 2, Duration::from_secs(3600));
        cache.initialize().await.unwrap();

        // Add profiles up to limit
        for i in 0..3 {
            let profile = ModelProfile {
                name: format!("profile_{}", i),
                model_name: "mistral-7b".to_string(),
                prompt_template: "Template".to_string(),
                system_message: None,
                parameters: ModelParameters::default(),
                created_at: 1234567890,
                updated_at: 1234567890,
                model_info: None,
                usage_stats: ProfileUsageStats::default(),
            };
            cache.store_profile(profile).await.unwrap();
        }

        // Cache should have evicted the oldest entry
        let profiles = cache.list_profiles().await;
        assert_eq!(profiles.len(), 2);
    }

    #[tokio::test]
    async fn test_profile_validation() {
        let temp_dir = TempDir::new().unwrap();
        let cache = ProfileCache::new(temp_dir.path().to_path_buf(), 10, Duration::from_secs(3600));

        // Test invalid profile (empty name)
        let invalid_profile = ModelProfile {
            name: "".to_string(),
            model_name: "mistral-7b".to_string(),
            prompt_template: "Template".to_string(),
            system_message: None,
            parameters: ModelParameters::default(),
            created_at: 1234567890,
            updated_at: 1234567890,
            model_info: None,
            usage_stats: ProfileUsageStats::default(),
        };

        let result = cache.store_profile(invalid_profile).await;
        assert!(matches!(
            result,
            Err(ProfileCacheError::InvalidProfileData { .. })
        ));
    }
}
