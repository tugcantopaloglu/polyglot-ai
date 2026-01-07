//! Feature utilities: rate limiting, caching, quotas, health checks, metrics

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::{Tool, ToolHealthInfo, ToolMetrics, CacheStats};

// =============================================================================
// Rate Limiting
// =============================================================================

/// Token bucket rate limiter for connection and request limiting
pub struct RateLimiter {
    buckets: RwLock<HashMap<String, TokenBucket>>,
    config: RateLimitConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Maximum requests per window
    pub max_requests: u32,
    /// Window duration in seconds
    pub window_seconds: u64,
    /// Maximum connections per IP
    pub max_connections_per_ip: u32,
    /// Cleanup interval in seconds
    pub cleanup_interval_seconds: u64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_requests: 100,
            window_seconds: 60,
            max_connections_per_ip: 10,
            cleanup_interval_seconds: 300,
        }
    }
}

struct TokenBucket {
    tokens: f64,
    last_update: Instant,
    max_tokens: f64,
    refill_rate: f64,
}

impl TokenBucket {
    fn new(max_tokens: u32, window_seconds: u64) -> Self {
        let max = max_tokens as f64;
        Self {
            tokens: max,
            last_update: Instant::now(),
            max_tokens: max,
            refill_rate: max / window_seconds as f64,
        }
    }

    fn try_consume(&mut self, tokens: f64) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_update).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.max_tokens);
        self.last_update = now;

        if self.tokens >= tokens {
            self.tokens -= tokens;
            true
        } else {
            false
        }
    }

    fn remaining(&self) -> u32 {
        self.tokens as u32
    }
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            buckets: RwLock::new(HashMap::new()),
            config,
        }
    }

    /// Check if a request is allowed for the given key
    pub fn check(&self, key: &str) -> RateLimitResult {
        let mut buckets = self.buckets.write();
        let bucket = buckets.entry(key.to_string()).or_insert_with(|| {
            TokenBucket::new(self.config.max_requests, self.config.window_seconds)
        });

        if bucket.try_consume(1.0) {
            RateLimitResult::Allowed {
                remaining: bucket.remaining(),
            }
        } else {
            RateLimitResult::Limited {
                retry_after_seconds: self.config.window_seconds,
            }
        }
    }

    /// Check connection rate limit for an IP
    pub fn check_connection(&self, ip: &str) -> RateLimitResult {
        let key = format!("conn:{}", ip);
        let mut buckets = self.buckets.write();
        let bucket = buckets.entry(key).or_insert_with(|| {
            TokenBucket::new(self.config.max_connections_per_ip, 60)
        });

        if bucket.try_consume(1.0) {
            RateLimitResult::Allowed {
                remaining: bucket.remaining(),
            }
        } else {
            RateLimitResult::Limited {
                retry_after_seconds: 60,
            }
        }
    }

    /// Clean up old buckets
    pub fn cleanup(&self) {
        let mut buckets = self.buckets.write();
        let threshold = Instant::now() - Duration::from_secs(self.config.cleanup_interval_seconds);
        buckets.retain(|_, b| b.last_update > threshold);
    }
}

#[derive(Debug, Clone)]
pub enum RateLimitResult {
    Allowed { remaining: u32 },
    Limited { retry_after_seconds: u64 },
}

impl RateLimitResult {
    pub fn is_allowed(&self) -> bool {
        matches!(self, RateLimitResult::Allowed { .. })
    }
}

// =============================================================================
// Response Caching
// =============================================================================

/// LRU cache for caching prompt responses
pub struct ResponseCache<K, V> {
    entries: RwLock<HashMap<K, CacheEntry<V>>>,
    config: CacheConfig,
    stats: CacheStatsInternal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    /// Maximum number of entries
    pub max_entries: usize,
    /// Entry TTL in seconds
    pub ttl_seconds: u64,
    /// Maximum memory in bytes (0 = unlimited)
    pub max_memory_bytes: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 1000,
            ttl_seconds: 3600,
            max_memory_bytes: 100 * 1024 * 1024, // 100 MB
        }
    }
}

struct CacheEntry<V> {
    value: V,
    created_at: Instant,
    last_accessed: Instant,
    size_bytes: usize,
}

struct CacheStatsInternal {
    hits: AtomicU64,
    misses: AtomicU64,
    memory_bytes: AtomicU64,
}

impl<K: Eq + Hash + Clone, V: Clone> ResponseCache<K, V> {
    pub fn new(config: CacheConfig) -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            config,
            stats: CacheStatsInternal {
                hits: AtomicU64::new(0),
                misses: AtomicU64::new(0),
                memory_bytes: AtomicU64::new(0),
            },
        }
    }

    pub fn get(&self, key: &K) -> Option<V> {
        let mut entries = self.entries.write();
        if let Some(entry) = entries.get_mut(key) {
            let now = Instant::now();
            if now.duration_since(entry.created_at).as_secs() < self.config.ttl_seconds {
                entry.last_accessed = now;
                self.stats.hits.fetch_add(1, Ordering::Relaxed);
                return Some(entry.value.clone());
            } else {
                // Expired
                self.stats.memory_bytes.fetch_sub(entry.size_bytes as u64, Ordering::Relaxed);
                entries.remove(key);
            }
        }
        self.stats.misses.fetch_add(1, Ordering::Relaxed);
        None
    }

    pub fn insert(&self, key: K, value: V, size_bytes: usize) {
        let mut entries = self.entries.write();

        // Evict if at capacity
        let max_entries = self.config.max_entries;
        while entries.len() >= max_entries {
            if let Some(oldest_key) = find_lru_key(&entries) {
                if let Some(entry) = entries.remove(&oldest_key) {
                    self.stats.memory_bytes.fetch_sub(entry.size_bytes as u64, Ordering::Relaxed);
                }
            } else {
                break;
            }
        }

        let now = Instant::now();
        entries.insert(key, CacheEntry {
            value,
            created_at: now,
            last_accessed: now,
            size_bytes,
        });
        self.stats.memory_bytes.fetch_add(size_bytes as u64, Ordering::Relaxed);
    }

    pub fn stats(&self) -> CacheStats {
        let entries = self.entries.read();
        let hits = self.stats.hits.load(Ordering::Relaxed);
        let misses = self.stats.misses.load(Ordering::Relaxed);
        let total = hits + misses;

        CacheStats {
            entries: entries.len() as u64,
            hits,
            misses,
            hit_rate: if total > 0 { hits as f32 / total as f32 } else { 0.0 },
            memory_bytes: self.stats.memory_bytes.load(Ordering::Relaxed),
        }
    }

    pub fn clear(&self) {
        let mut entries = self.entries.write();
        entries.clear();
        self.stats.memory_bytes.store(0, Ordering::Relaxed);
    }
}

fn find_lru_key<K: Eq + Hash + Clone, V>(entries: &HashMap<K, CacheEntry<V>>) -> Option<K> {
    entries.iter()
        .min_by_key(|(_, e)| e.last_accessed)
        .map(|(k, _)| k.clone())
}

// =============================================================================
// Usage Quotas
// =============================================================================

/// Usage quota tracker
pub struct QuotaTracker {
    quotas: RwLock<HashMap<String, UserQuota>>,
    config: QuotaConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaConfig {
    /// Daily request limit (None = unlimited)
    pub daily_limit: Option<u64>,
    /// Monthly request limit (None = unlimited)
    pub monthly_limit: Option<u64>,
    /// Daily token limit (None = unlimited)
    pub daily_token_limit: Option<u64>,
    /// Monthly token limit (None = unlimited)
    pub monthly_token_limit: Option<u64>,
}

impl Default for QuotaConfig {
    fn default() -> Self {
        Self {
            daily_limit: Some(1000),
            monthly_limit: Some(10000),
            daily_token_limit: Some(1_000_000),
            monthly_token_limit: Some(10_000_000),
        }
    }
}

#[derive(Debug, Clone)]
struct UserQuota {
    daily_requests: u64,
    monthly_requests: u64,
    daily_tokens: u64,
    monthly_tokens: u64,
    daily_reset: DateTime<Utc>,
    monthly_reset: DateTime<Utc>,
}

impl UserQuota {
    fn new() -> Self {
        let now = Utc::now();
        Self {
            daily_requests: 0,
            monthly_requests: 0,
            daily_tokens: 0,
            monthly_tokens: 0,
            daily_reset: now + chrono::Duration::days(1),
            monthly_reset: now + chrono::Duration::days(30),
        }
    }

    fn check_and_reset(&mut self) {
        let now = Utc::now();
        if now >= self.daily_reset {
            self.daily_requests = 0;
            self.daily_tokens = 0;
            self.daily_reset = now + chrono::Duration::days(1);
        }
        if now >= self.monthly_reset {
            self.monthly_requests = 0;
            self.monthly_tokens = 0;
            self.monthly_reset = now + chrono::Duration::days(30);
        }
    }
}

impl QuotaTracker {
    pub fn new(config: QuotaConfig) -> Self {
        Self {
            quotas: RwLock::new(HashMap::new()),
            config,
        }
    }

    pub fn check(&self, user_id: &str) -> QuotaResult {
        let mut quotas = self.quotas.write();
        let quota = quotas.entry(user_id.to_string())
            .or_insert_with(UserQuota::new);
        quota.check_and_reset();

        // Check daily limit
        if let Some(limit) = self.config.daily_limit {
            if quota.daily_requests >= limit {
                return QuotaResult::Exceeded {
                    reason: "Daily request limit exceeded".to_string(),
                    reset_at: quota.daily_reset,
                };
            }
        }

        // Check monthly limit
        if let Some(limit) = self.config.monthly_limit {
            if quota.monthly_requests >= limit {
                return QuotaResult::Exceeded {
                    reason: "Monthly request limit exceeded".to_string(),
                    reset_at: quota.monthly_reset,
                };
            }
        }

        QuotaResult::Allowed {
            daily_remaining: self.config.daily_limit.map(|l| l.saturating_sub(quota.daily_requests)),
            monthly_remaining: self.config.monthly_limit.map(|l| l.saturating_sub(quota.monthly_requests)),
        }
    }

    pub fn record_usage(&self, user_id: &str, tokens: u64) {
        let mut quotas = self.quotas.write();
        let quota = quotas.entry(user_id.to_string())
            .or_insert_with(UserQuota::new);
        quota.check_and_reset();
        quota.daily_requests += 1;
        quota.monthly_requests += 1;
        quota.daily_tokens += tokens;
        quota.monthly_tokens += tokens;
    }

    pub fn get_status(&self, user_id: &str) -> QuotaStatus {
        let mut quotas = self.quotas.write();
        let quota = quotas.entry(user_id.to_string())
            .or_insert_with(UserQuota::new);
        quota.check_and_reset();

        QuotaStatus {
            daily_limit: self.config.daily_limit,
            daily_used: quota.daily_requests,
            monthly_limit: self.config.monthly_limit,
            monthly_used: quota.monthly_requests,
            daily_reset: quota.daily_reset,
            monthly_reset: quota.monthly_reset,
        }
    }
}

#[derive(Debug, Clone)]
pub enum QuotaResult {
    Allowed {
        daily_remaining: Option<u64>,
        monthly_remaining: Option<u64>,
    },
    Exceeded {
        reason: String,
        reset_at: DateTime<Utc>,
    },
}

impl QuotaResult {
    pub fn is_allowed(&self) -> bool {
        matches!(self, QuotaResult::Allowed { .. })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaStatus {
    pub daily_limit: Option<u64>,
    pub daily_used: u64,
    pub monthly_limit: Option<u64>,
    pub monthly_used: u64,
    pub daily_reset: DateTime<Utc>,
    pub monthly_reset: DateTime<Utc>,
}

// =============================================================================
// Health Checks
// =============================================================================

/// Tool health checker
pub struct HealthChecker {
    health: RwLock<HashMap<Tool, HealthState>>,
    config: HealthCheckConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckConfig {
    /// Check interval in seconds
    pub check_interval_seconds: u64,
    /// Failure threshold before marking unhealthy
    pub failure_threshold: u32,
    /// Recovery threshold to mark healthy again
    pub recovery_threshold: u32,
    /// Timeout for health check in milliseconds
    pub timeout_ms: u64,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            check_interval_seconds: 60,
            failure_threshold: 3,
            recovery_threshold: 2,
            timeout_ms: 5000,
        }
    }
}

#[derive(Debug, Clone)]
struct HealthState {
    healthy: bool,
    last_check: DateTime<Utc>,
    latency_ms: Option<u32>,
    consecutive_failures: u32,
    consecutive_successes: u32,
    total_checks: u64,
    failed_checks: u64,
}

impl Default for HealthState {
    fn default() -> Self {
        Self {
            healthy: true,
            last_check: Utc::now(),
            latency_ms: None,
            consecutive_failures: 0,
            consecutive_successes: 0,
            total_checks: 0,
            failed_checks: 0,
        }
    }
}

impl HealthChecker {
    pub fn new(config: HealthCheckConfig) -> Self {
        let mut health = HashMap::new();
        for tool in Tool::all() {
            health.insert(*tool, HealthState::default());
        }
        Self {
            health: RwLock::new(health),
            config,
        }
    }

    pub fn record_success(&self, tool: Tool, latency_ms: u32) {
        let mut health = self.health.write();
        if let Some(state) = health.get_mut(&tool) {
            state.last_check = Utc::now();
            state.latency_ms = Some(latency_ms);
            state.consecutive_failures = 0;
            state.consecutive_successes += 1;
            state.total_checks += 1;

            if !state.healthy && state.consecutive_successes >= self.config.recovery_threshold {
                state.healthy = true;
            }
        }
    }

    pub fn record_failure(&self, tool: Tool) {
        let mut health = self.health.write();
        if let Some(state) = health.get_mut(&tool) {
            state.last_check = Utc::now();
            state.consecutive_successes = 0;
            state.consecutive_failures += 1;
            state.total_checks += 1;
            state.failed_checks += 1;

            if state.healthy && state.consecutive_failures >= self.config.failure_threshold {
                state.healthy = false;
            }
        }
    }

    pub fn is_healthy(&self, tool: Tool) -> bool {
        let health = self.health.read();
        health.get(&tool).map(|s| s.healthy).unwrap_or(false)
    }

    pub fn get_status(&self) -> Vec<ToolHealthInfo> {
        let health = self.health.read();
        health.iter().map(|(tool, state)| {
            let error_rate = if state.total_checks > 0 {
                state.failed_checks as f32 / state.total_checks as f32
            } else {
                0.0
            };
            ToolHealthInfo {
                tool: *tool,
                healthy: state.healthy,
                last_check: state.last_check,
                latency_ms: state.latency_ms,
                error_rate,
                consecutive_failures: state.consecutive_failures,
            }
        }).collect()
    }

    pub fn all_healthy(&self) -> bool {
        let health = self.health.read();
        health.values().all(|s| s.healthy)
    }
}

// =============================================================================
// Metrics Collection
// =============================================================================

/// Server metrics collector
pub struct MetricsCollector {
    tool_metrics: RwLock<HashMap<Tool, ToolMetricsInternal>>,
    request_times: RwLock<Vec<Instant>>,
    active_connections: AtomicU64,
    total_requests: AtomicU64,
    start_time: Instant,
}

struct ToolMetricsInternal {
    total_requests: u64,
    successful_requests: u64,
    failed_requests: u64,
    total_latency_ms: u64,
    rate_limit_hits: u64,
}

impl Default for ToolMetricsInternal {
    fn default() -> Self {
        Self {
            total_requests: 0,
            successful_requests: 0,
            failed_requests: 0,
            total_latency_ms: 0,
            rate_limit_hits: 0,
        }
    }
}

impl MetricsCollector {
    pub fn new() -> Self {
        let mut tool_metrics = HashMap::new();
        for tool in Tool::all() {
            tool_metrics.insert(*tool, ToolMetricsInternal::default());
        }
        Self {
            tool_metrics: RwLock::new(tool_metrics),
            request_times: RwLock::new(Vec::new()),
            active_connections: AtomicU64::new(0),
            total_requests: AtomicU64::new(0),
            start_time: Instant::now(),
        }
    }

    pub fn record_request(&self, tool: Tool, success: bool, latency_ms: u32) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);

        let mut metrics = self.tool_metrics.write();
        if let Some(m) = metrics.get_mut(&tool) {
            m.total_requests += 1;
            m.total_latency_ms += latency_ms as u64;
            if success {
                m.successful_requests += 1;
            } else {
                m.failed_requests += 1;
            }
        }

        let mut times = self.request_times.write();
        times.push(Instant::now());
        // Keep only last 5 minutes of requests
        let cutoff = Instant::now() - Duration::from_secs(300);
        times.retain(|t| *t > cutoff);
    }

    pub fn record_rate_limit(&self, tool: Tool) {
        let mut metrics = self.tool_metrics.write();
        if let Some(m) = metrics.get_mut(&tool) {
            m.rate_limit_hits += 1;
        }
    }

    pub fn connection_opened(&self) {
        self.active_connections.fetch_add(1, Ordering::Relaxed);
    }

    pub fn connection_closed(&self) {
        self.active_connections.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn get_metrics(&self, cache_stats: CacheStats) -> ServerMetrics {
        let metrics = self.tool_metrics.read();
        let times = self.request_times.read();

        let requests_per_minute = if times.is_empty() {
            0.0
        } else {
            let now = Instant::now();
            let one_minute_ago = now - Duration::from_secs(60);
            let recent = times.iter().filter(|t| **t > one_minute_ago).count();
            recent as f64
        };

        let tool_stats: Vec<ToolMetrics> = metrics.iter().map(|(tool, m)| {
            ToolMetrics {
                tool: *tool,
                total_requests: m.total_requests,
                successful_requests: m.successful_requests,
                failed_requests: m.failed_requests,
                avg_latency_ms: if m.total_requests > 0 {
                    (m.total_latency_ms / m.total_requests) as u32
                } else {
                    0
                },
                rate_limit_hits: m.rate_limit_hits,
            }
        }).collect();

        ServerMetrics {
            active_connections: self.active_connections.load(Ordering::Relaxed) as u32,
            total_requests: self.total_requests.load(Ordering::Relaxed),
            requests_per_minute,
            tool_stats,
            cache_stats,
            uptime_seconds: self.start_time.elapsed().as_secs(),
        }
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerMetrics {
    pub active_connections: u32,
    pub total_requests: u64,
    pub requests_per_minute: f64,
    pub tool_stats: Vec<ToolMetrics>,
    pub cache_stats: CacheStats,
    pub uptime_seconds: u64,
}

// =============================================================================
// Context Window Management
// =============================================================================

/// Context window manager for handling token limits
pub struct ContextWindowManager {
    config: ContextWindowConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextWindowConfig {
    /// Maximum tokens per request
    pub max_tokens: u32,
    /// Reserve tokens for response
    pub response_reserve: u32,
    /// Token estimation method
    pub estimation_method: TokenEstimationMethod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenEstimationMethod {
    /// Approximate: chars / 4
    CharDivide4,
    /// Approximate: words * 1.3
    WordMultiply,
    /// More accurate but slower
    Tiktoken,
}

impl Default for ContextWindowConfig {
    fn default() -> Self {
        Self {
            max_tokens: 128000,
            response_reserve: 4000,
            estimation_method: TokenEstimationMethod::CharDivide4,
        }
    }
}

impl ContextWindowManager {
    pub fn new(config: ContextWindowConfig) -> Self {
        Self { config }
    }

    pub fn estimate_tokens(&self, text: &str) -> u32 {
        match self.config.estimation_method {
            TokenEstimationMethod::CharDivide4 => (text.len() / 4) as u32,
            TokenEstimationMethod::WordMultiply => {
                let words = text.split_whitespace().count();
                (words as f64 * 1.3) as u32
            }
            TokenEstimationMethod::Tiktoken => {
                // Fallback to char/4 - tiktoken requires external lib
                (text.len() / 4) as u32
            }
        }
    }

    pub fn available_tokens(&self) -> u32 {
        self.config.max_tokens.saturating_sub(self.config.response_reserve)
    }

    pub fn fits(&self, text: &str) -> bool {
        self.estimate_tokens(text) <= self.available_tokens()
    }

    pub fn truncate_to_fit(&self, text: &str) -> String {
        let available = self.available_tokens();
        let estimated = self.estimate_tokens(text);

        if estimated <= available {
            return text.to_string();
        }

        // Calculate approximate character limit
        let ratio = available as f64 / estimated as f64;
        let char_limit = (text.len() as f64 * ratio * 0.95) as usize; // 5% safety margin

        crate::truncate_smart(text, char_limit)
    }

    pub fn validate_prompt(&self, prompt: &str) -> PromptValidation {
        let tokens = self.estimate_tokens(prompt);
        let max = self.config.max_tokens;

        if tokens > max {
            PromptValidation::TooLong {
                tokens,
                max_tokens: max,
                excess: tokens - max,
            }
        } else if tokens > max * 90 / 100 {
            PromptValidation::Warning {
                tokens,
                max_tokens: max,
                usage_percent: (tokens as f64 / max as f64 * 100.0) as u8,
            }
        } else {
            PromptValidation::Valid {
                tokens,
                remaining: max - tokens,
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum PromptValidation {
    Valid { tokens: u32, remaining: u32 },
    Warning { tokens: u32, max_tokens: u32, usage_percent: u8 },
    TooLong { tokens: u32, max_tokens: u32, excess: u32 },
}

impl PromptValidation {
    pub fn is_valid(&self) -> bool {
        !matches!(self, PromptValidation::TooLong { .. })
    }
}

// =============================================================================
// Plugin Validation
// =============================================================================

/// Plugin configuration validator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginValidationConfig {
    /// Maximum command length
    pub max_command_length: usize,
    /// Maximum number of arguments
    pub max_args: usize,
    /// Allowed interpreters
    pub allowed_interpreters: Vec<String>,
    /// Maximum timeout in seconds
    pub max_timeout_seconds: u64,
    /// Forbidden command patterns
    pub forbidden_patterns: Vec<String>,
}

impl Default for PluginValidationConfig {
    fn default() -> Self {
        Self {
            max_command_length: 1024,
            max_args: 50,
            allowed_interpreters: vec![
                "python".to_string(),
                "python3".to_string(),
                "node".to_string(),
                "bash".to_string(),
                "sh".to_string(),
                "powershell".to_string(),
            ],
            max_timeout_seconds: 300,
            forbidden_patterns: vec![
                "rm -rf".to_string(),
                "sudo".to_string(),
                "chmod 777".to_string(),
                "curl | sh".to_string(),
                "wget | sh".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone)]
pub struct PluginValidator {
    config: PluginValidationConfig,
}

impl PluginValidator {
    pub fn new(config: PluginValidationConfig) -> Self {
        Self { config }
    }

    pub fn validate_command(&self, command: &str) -> Result<(), PluginValidationError> {
        if command.len() > self.config.max_command_length {
            return Err(PluginValidationError::CommandTooLong {
                length: command.len(),
                max: self.config.max_command_length,
            });
        }

        let lower = command.to_lowercase();
        for pattern in &self.config.forbidden_patterns {
            if lower.contains(&pattern.to_lowercase()) {
                return Err(PluginValidationError::ForbiddenPattern {
                    pattern: pattern.clone(),
                });
            }
        }

        Ok(())
    }

    pub fn validate_interpreter(&self, interpreter: &str) -> Result<(), PluginValidationError> {
        let name = std::path::Path::new(interpreter)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(interpreter);

        if !self.config.allowed_interpreters.iter().any(|i| i == name) {
            return Err(PluginValidationError::DisallowedInterpreter {
                interpreter: interpreter.to_string(),
                allowed: self.config.allowed_interpreters.clone(),
            });
        }

        Ok(())
    }

    pub fn validate_args(&self, args: &[String]) -> Result<(), PluginValidationError> {
        if args.len() > self.config.max_args {
            return Err(PluginValidationError::TooManyArgs {
                count: args.len(),
                max: self.config.max_args,
            });
        }

        // Check for injection patterns in args
        for arg in args {
            if arg.contains('`') || arg.contains("$(") || arg.contains('\0') {
                return Err(PluginValidationError::SuspiciousArg {
                    arg: arg.clone(),
                });
            }
        }

        Ok(())
    }

    pub fn validate_timeout(&self, timeout_seconds: u64) -> Result<(), PluginValidationError> {
        if timeout_seconds > self.config.max_timeout_seconds {
            return Err(PluginValidationError::TimeoutTooLong {
                timeout: timeout_seconds,
                max: self.config.max_timeout_seconds,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum PluginValidationError {
    CommandTooLong { length: usize, max: usize },
    ForbiddenPattern { pattern: String },
    DisallowedInterpreter { interpreter: String, allowed: Vec<String> },
    TooManyArgs { count: usize, max: usize },
    SuspiciousArg { arg: String },
    TimeoutTooLong { timeout: u64, max: u64 },
}

impl std::fmt::Display for PluginValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CommandTooLong { length, max } => {
                write!(f, "Command too long: {} chars (max {})", length, max)
            }
            Self::ForbiddenPattern { pattern } => {
                write!(f, "Forbidden pattern detected: {}", pattern)
            }
            Self::DisallowedInterpreter { interpreter, allowed } => {
                write!(f, "Interpreter '{}' not allowed. Allowed: {:?}", interpreter, allowed)
            }
            Self::TooManyArgs { count, max } => {
                write!(f, "Too many arguments: {} (max {})", count, max)
            }
            Self::SuspiciousArg { arg } => {
                write!(f, "Suspicious argument detected: {}", arg)
            }
            Self::TimeoutTooLong { timeout, max } => {
                write!(f, "Timeout too long: {}s (max {}s)", timeout, max)
            }
        }
    }
}

impl std::error::Error for PluginValidationError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limiter() {
        let config = RateLimitConfig {
            max_requests: 5,
            window_seconds: 60,
            max_connections_per_ip: 3,
            cleanup_interval_seconds: 300,
        };
        let limiter = RateLimiter::new(config);

        for _ in 0..5 {
            assert!(limiter.check("user1").is_allowed());
        }
        assert!(!limiter.check("user1").is_allowed());
    }

    #[test]
    fn test_response_cache() {
        let config = CacheConfig {
            max_entries: 2,
            ttl_seconds: 3600,
            max_memory_bytes: 1024,
        };
        let cache: ResponseCache<String, String> = ResponseCache::new(config);

        cache.insert("key1".to_string(), "value1".to_string(), 10);
        cache.insert("key2".to_string(), "value2".to_string(), 10);

        assert_eq!(cache.get(&"key1".to_string()), Some("value1".to_string()));
        assert_eq!(cache.get(&"key2".to_string()), Some("value2".to_string()));

        // Third insert should evict oldest
        cache.insert("key3".to_string(), "value3".to_string(), 10);
        assert!(cache.get(&"key1".to_string()).is_none() || cache.get(&"key2".to_string()).is_none());
    }

    #[test]
    fn test_context_window_manager() {
        let config = ContextWindowConfig {
            max_tokens: 1000,
            response_reserve: 100,
            estimation_method: TokenEstimationMethod::CharDivide4,
        };
        let manager = ContextWindowManager::new(config);

        assert_eq!(manager.available_tokens(), 900);
        assert!(manager.fits("short text"));

        let long_text = "x".repeat(5000);
        assert!(!manager.fits(&long_text));

        let truncated = manager.truncate_to_fit(&long_text);
        assert!(manager.fits(&truncated));
    }

    #[test]
    fn test_plugin_validator() {
        let validator = PluginValidator::new(PluginValidationConfig::default());

        assert!(validator.validate_command("python script.py").is_ok());
        assert!(validator.validate_command("rm -rf /").is_err());
        assert!(validator.validate_interpreter("python").is_ok());
        assert!(validator.validate_interpreter("evil-binary").is_err());
    }
}
