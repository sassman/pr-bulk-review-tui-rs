//! GitHub API response cache for development workflow
//!
//! Caches API responses to disk to avoid redundant API calls during
//! frequent app restarts (common during development). Responses are
//! cached with a 20-minute TTL and support ETags for efficient validation.

use ::log::{debug, warn};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// GitHub API response cache
#[derive(Debug)]
pub struct ApiCache {
    cache_file: PathBuf,
    ttl_seconds: u64,
    entries: HashMap<String, CacheEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    response_body: String,
    timestamp: u64, // Unix timestamp
    etag: Option<String>,
    status_code: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheFile {
    version: u8,
    entries: HashMap<String, CacheEntry>,
}

#[derive(Debug, Clone)]
pub struct CachedResponse {
    pub body: String,
    pub etag: Option<String>,
    pub status_code: u16,
}

#[derive(Debug, Clone)]
pub struct CacheStats {
    pub total_entries: usize,
    pub fresh_entries: usize,
    pub stale_entries: usize,
    pub ttl_seconds: u64,
}

impl ApiCache {
    /// Create cache with 20-minute TTL (hardcoded for development workflow)
    pub fn new() -> Result<Self> {
        let cache_dir = std::env::current_dir()?.join(".cache");
        std::fs::create_dir_all(&cache_dir)?;

        let cache_file = cache_dir.join("gh-api-cache.json");
        let ttl_seconds = 20 * 60; // 20 minutes

        let entries = if cache_file.exists() {
            Self::load_from_disk(&cache_file).unwrap_or_else(|e| {
                warn!(
                    "Failed to load cache file: {}, starting with empty cache",
                    e
                );
                HashMap::new()
            })
        } else {
            HashMap::new()
        };

        debug!(
            "API cache initialized with {} entries (TTL: {}min)",
            entries.len(),
            ttl_seconds / 60
        );

        Ok(Self {
            cache_file,
            ttl_seconds,
            entries,
        })
    }

    /// Get cached response if available and not stale
    ///
    /// Returns cached response with its ETag if entry is fresh (within TTL).
    /// Returns None if entry is missing or stale.
    pub fn get(&self, method: &str, url: &str, params: &[(&str, &str)]) -> Option<CachedResponse> {
        let key = self.cache_key(method, url, params);

        if let Some(entry) = self.entries.get(&key) {
            let age_seconds = self.current_timestamp() - entry.timestamp;

            if age_seconds < self.ttl_seconds {
                debug!(
                    "Cache HIT: {} (age: {}s, ttl: {}s)",
                    key, age_seconds, self.ttl_seconds
                );

                return Some(CachedResponse {
                    body: entry.response_body.clone(),
                    etag: entry.etag.clone(),
                    status_code: entry.status_code,
                });
            } else {
                debug!(
                    "Cache STALE: {} (age: {}s, ttl: {}s)",
                    key, age_seconds, self.ttl_seconds
                );

                // Return stale entry for ETag validation
                return Some(CachedResponse {
                    body: entry.response_body.clone(),
                    etag: entry.etag.clone(),
                    status_code: 200, // Treat stale as potential 304 candidate
                });
            }
        } else {
            debug!("Cache MISS: {}", key);
        }

        None
    }

    /// Store response in cache
    ///
    /// Persists the response body and ETag to disk for future requests.
    pub fn set(
        &mut self,
        method: &str,
        url: &str,
        params: &[(&str, &str)],
        response: &CachedResponse,
    ) -> Result<()> {
        let key = self.cache_key(method, url, params);

        let entry = CacheEntry {
            response_body: response.body.clone(),
            timestamp: self.current_timestamp(),
            etag: response.etag.clone(),
            status_code: response.status_code,
        };

        self.entries.insert(key.clone(), entry);

        debug!("Cache SET: {} (etag: {:?})", key, response.etag);

        // Persist to disk
        self.save_to_disk()?;

        Ok(())
    }

    /// Update timestamp for existing cache entry (after 304 Not Modified)
    ///
    /// When server returns 304, it means the cached content is still valid.
    /// We update the timestamp to extend the TTL without changing the body.
    pub fn touch(&mut self, method: &str, url: &str, params: &[(&str, &str)]) -> Result<()> {
        let key = self.cache_key(method, url, params);
        let timestamp = self.current_timestamp();

        if let Some(entry) = self.entries.get_mut(&key) {
            entry.timestamp = timestamp;
            debug!("Cache TOUCH: {} (TTL extended)", key);
            self.save_to_disk()?;
        }

        Ok(())
    }

    /// Invalidate specific cache entry
    pub fn invalidate(&mut self, method: &str, url: &str, params: &[(&str, &str)]) {
        let key = self.cache_key(method, url, params);
        if self.entries.remove(&key).is_some() {
            debug!("Cache INVALIDATE: {}", key);
            let _ = self.save_to_disk();
        }
    }

    /// Invalidate all entries matching a pattern (e.g., all PRs from a repo)
    ///
    /// Pattern is matched against cache keys using contains().
    /// Example: "/repos/acme/widget" invalidates all entries for that repo.
    pub fn invalidate_pattern(&mut self, pattern: &str) {
        let keys_to_remove: Vec<_> = self
            .entries
            .keys()
            .filter(|k| k.contains(pattern))
            .cloned()
            .collect();

        for key in &keys_to_remove {
            self.entries.remove(key);
            debug!("Cache INVALIDATE (pattern '{}'): {}", pattern, key);
        }

        if !keys_to_remove.is_empty() {
            let _ = self.save_to_disk();
        }
    }

    /// Clear entire cache
    pub fn clear(&mut self) -> Result<()> {
        let count = self.entries.len();
        self.entries.clear();
        self.save_to_disk()?;
        debug!("Cache CLEARED ({} entries removed)", count);
        Ok(())
    }

    /// Get cache statistics for debugging
    pub fn stats(&self) -> CacheStats {
        let total_entries = self.entries.len();
        let fresh_entries = self
            .entries
            .values()
            .filter(|e| {
                let age = self.current_timestamp() - e.timestamp;
                age < self.ttl_seconds
            })
            .count();
        let stale_entries = total_entries - fresh_entries;

        CacheStats {
            total_entries,
            fresh_entries,
            stale_entries,
            ttl_seconds: self.ttl_seconds,
        }
    }

    /// Check if cache is enabled via environment variable
    pub fn is_enabled() -> bool {
        std::env::var("DISABLE_API_CACHE")
            .map(|v| v != "1" && v.to_lowercase() != "true")
            .unwrap_or(true)
    }

    // Private helpers

    fn cache_key(&self, method: &str, url: &str, params: &[(&str, &str)]) -> String {
        if params.is_empty() {
            format!("{}:{}", method, url)
        } else {
            // Sort params for deterministic key
            let mut sorted_params = params.to_vec();
            sorted_params.sort_by_key(|(k, _)| *k);

            let query = sorted_params
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join("&");

            format!("{}:{}?{}", method, url, query)
        }
    }

    fn current_timestamp(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn load_from_disk(path: &PathBuf) -> Result<HashMap<String, CacheEntry>> {
        let content = std::fs::read_to_string(path)?;
        let cache_file: CacheFile = serde_json::from_str(&content)?;

        // Validate version
        if cache_file.version != 1 {
            warn!("Cache file version mismatch, clearing cache");
            return Ok(HashMap::new());
        }

        Ok(cache_file.entries)
    }

    fn save_to_disk(&self) -> Result<()> {
        let cache_file = CacheFile {
            version: 1,
            entries: self.entries.clone(),
        };

        let content = serde_json::to_string_pretty(&cache_file)?;
        std::fs::write(&self.cache_file, content)?;

        Ok(())
    }
}

impl Default for ApiCache {
    fn default() -> Self {
        Self::new().unwrap_or_else(|_| {
            // Fallback to in-memory only cache if disk fails
            warn!("Failed to create disk cache, using in-memory only");
            Self {
                cache_file: PathBuf::from(".cache/gh-api-cache.json"),
                ttl_seconds: 20 * 60,
                entries: HashMap::new(),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn test_cache_key_generation() {
        let cache = ApiCache::default();

        // No params
        let key1 = cache.cache_key("GET", "/repos/org/repo", &[]);
        assert_eq!(key1, "GET:/repos/org/repo");

        // With params
        let key2 = cache.cache_key("GET", "/repos/org/repo", &[("state", "open")]);
        assert_eq!(key2, "GET:/repos/org/repo?state=open");

        // Params sorted
        let key3 = cache.cache_key(
            "GET",
            "/repos/org/repo",
            &[("state", "open"), ("base", "main")],
        );
        assert_eq!(key3, "GET:/repos/org/repo?base=main&state=open");
    }

    #[test]
    fn test_cache_get_set() {
        let mut cache = ApiCache::default();

        let response = CachedResponse {
            body: r#"{"data": "test"}"#.into(),
            etag: Some("abc123".into()),
            status_code: 200,
        };

        // Set
        cache
            .set("GET", "/test", &[], &response)
            .expect("Failed to set cache");

        // Get
        let cached = cache.get("GET", "/test", &[]).expect("Cache miss");
        assert_eq!(cached.body, response.body);
        assert_eq!(cached.etag, response.etag);
        assert_eq!(cached.status_code, 200);
    }

    #[test]
    fn test_cache_ttl() {
        let cache_file = PathBuf::from(".cache/test-ttl.json");
        let mut cache = ApiCache {
            cache_file,
            ttl_seconds: 2, // 2 second TTL for test
            entries: HashMap::new(),
        };

        let response = CachedResponse {
            body: "test".into(),
            etag: Some("abc".into()),
            status_code: 200,
        };

        cache.set("GET", "/test", &[], &response).unwrap();

        // Immediate get - should hit
        assert!(cache.get("GET", "/test", &[]).is_some());

        // Wait 3 seconds
        sleep(Duration::from_secs(3));

        // Should return stale entry (for ETag validation)
        let stale = cache.get("GET", "/test", &[]);
        assert!(stale.is_some());
        assert_eq!(stale.unwrap().etag, Some("abc".into()));
    }

    #[test]
    fn test_cache_invalidate() {
        let mut cache = ApiCache::default();

        let response = CachedResponse {
            body: "test".into(),
            etag: None,
            status_code: 200,
        };

        cache.set("GET", "/test", &[], &response).unwrap();
        assert!(cache.get("GET", "/test", &[]).is_some());

        cache.invalidate("GET", "/test", &[]);
        assert!(cache.get("GET", "/test", &[]).is_none());
    }

    #[test]
    fn test_cache_invalidate_pattern() {
        let cache_file = PathBuf::from(".cache/test-pattern.json");
        let mut cache = ApiCache {
            cache_file,
            ttl_seconds: 20 * 60,
            entries: HashMap::new(),
        };

        let response = CachedResponse {
            body: "test".into(),
            etag: None,
            status_code: 200,
        };

        cache
            .set("GET", "/repos/acme/widget/pulls", &[], &response)
            .unwrap();
        cache
            .set("GET", "/repos/acme/widget/issues", &[], &response)
            .unwrap();
        cache
            .set("GET", "/repos/acme/other/pulls", &[], &response)
            .unwrap();

        assert_eq!(cache.entries.len(), 3);

        cache.invalidate_pattern("/repos/acme/widget");

        // Should have removed 2 entries, leaving 1
        assert_eq!(cache.entries.len(), 1);
        assert!(cache.get("GET", "/repos/acme/other/pulls", &[]).is_some());
    }

    #[test]
    fn test_cache_touch() {
        let cache_file = PathBuf::from(".cache/test-touch.json");
        let mut cache = ApiCache {
            cache_file,
            ttl_seconds: 2,
            entries: HashMap::new(),
        };

        let response = CachedResponse {
            body: "test".into(),
            etag: Some("abc".into()),
            status_code: 200,
        };

        cache.set("GET", "/test", &[], &response).unwrap();

        // Wait 1 second
        sleep(Duration::from_secs(1));

        // Touch to extend TTL
        cache.touch("GET", "/test", &[]).unwrap();

        // Wait another 1.5 seconds (total 2.5s from original, but only 1.5s from touch)
        sleep(Duration::from_secs_f32(1.5));

        // Should still be fresh because we touched it
        let cached = cache.get("GET", "/test", &[]);
        assert!(cached.is_some());
    }

    #[test]
    fn test_cache_stats() {
        let cache_file = PathBuf::from(".cache/test-stats.json");
        let mut cache = ApiCache {
            cache_file,
            ttl_seconds: 1,
            entries: HashMap::new(),
        };

        let response = CachedResponse {
            body: "test".into(),
            etag: None,
            status_code: 200,
        };

        cache.set("GET", "/fresh", &[], &response).unwrap();

        // Add stale entry by manipulating timestamp
        let stale_entry = CacheEntry {
            response_body: "stale".into(),
            timestamp: cache.current_timestamp() - 100, // 100s ago
            etag: None,
            status_code: 200,
        };
        cache.entries.insert("GET:/stale".into(), stale_entry);

        let stats = cache.stats();
        assert_eq!(stats.total_entries, 2);
        assert_eq!(stats.fresh_entries, 1);
        assert_eq!(stats.stale_entries, 1);
    }
}
