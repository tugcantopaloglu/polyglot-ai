pub mod protocol;
pub mod models;
pub mod crypto;
pub mod context;
pub mod updater;

pub use protocol::{
    ClientMessage, ServerMessage, OutputType, ToolInfo, SwitchReason, ErrorCode,
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
    generate_title,
};

pub use updater::{
    UpdateConfig, UpdateInfo, UpdateStatus, UpdatePhase, BackupInfo,
    GitHubRelease, GitHubAsset,
    version_compare, get_platform_asset_name, get_backup_dir,
    create_backup, restore_backup, cleanup_old_backups,
    get_current_exe, verify_binary, format_bytes, print_status,
};
