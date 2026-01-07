pub mod protocol;
pub mod models;
pub mod crypto;
pub mod context;
pub mod updater;
pub mod features;
pub mod storage;

pub use protocol::{
    ClientMessage, ServerMessage, OutputType, ToolInfo, SwitchReason, ErrorCode,
    ExportFormat, ToolHealthInfo, ToolMetrics, CacheStats,
    encode_message, decode_message, frame_message,
    PROTOCOL_VERSION, MAX_MESSAGE_SIZE,
};

pub use models::{
    Tool, SyncMode, RotationStrategy, AuthMode,
    User, Session, ToolUsage, FileInfo, FileConflict,
    ConflictResolution, ToolConfig,
};

pub use context::{
    Message, MessageRole, ChatSession, CodeReference,
    TransferContext, HistoryEntry, SummarizerConfig,
    truncate_smart, summarize_messages, create_transfer_context,
    generate_title, export_session, export_sessions,
};

pub use features::{
    RateLimiter, RateLimitConfig, RateLimitResult,
    ResponseCache, CacheConfig,
    QuotaTracker, QuotaConfig, QuotaResult, QuotaStatus,
    HealthChecker, HealthCheckConfig,
    MetricsCollector, ServerMetrics,
    ContextWindowManager, ContextWindowConfig, TokenEstimationMethod, PromptValidation,
    PluginValidator, PluginValidationConfig, PluginValidationError,
    ApiKeyManager, ApiKeyError,
    WebhookEvent, WebhookPayload, WebhookConfig, compute_webhook_signature,
    StreamConfig, StreamBuffer, StreamChunk,
};

pub use updater::{
    UpdateConfig, UpdateInfo, UpdateStatus, UpdatePhase, BackupInfo,
    GitHubRelease, GitHubAsset,
    version_compare, get_platform_asset_name, get_backup_dir,
    create_backup, restore_backup, cleanup_old_backups,
    get_current_exe, verify_binary, format_bytes, print_status,
};

pub use storage::{
    Database, StorageError,
    StoredQuota, StoredSession, StoredApiKey, StoredWebhook,
    CachedResponse, AuditLogEntry,
};
