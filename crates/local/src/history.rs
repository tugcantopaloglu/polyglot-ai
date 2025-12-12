#![allow(dead_code)]

use std::path::PathBuf;
use std::fs;
use std::io::{BufReader, BufWriter};

use anyhow::{Context, Result};
use chrono::Utc;
use uuid::Uuid;

use polyglot_common::{
    Tool, ChatSession, Message, HistoryEntry,
    TransferContext, SummarizerConfig, CodeReference,
    create_transfer_context, summarize_messages,
};

pub struct HistoryManager {
    storage_dir: PathBuf,
    current_session: Option<ChatSession>,
    current_project: Option<String>,
    config: SummarizerConfig,
}

impl HistoryManager {
    pub fn new(storage_dir: Option<PathBuf>) -> Result<Self> {
        let storage_dir = storage_dir.unwrap_or_else(|| {
            dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("polyglot-ai")
                .join("history")
        });

        fs::create_dir_all(&storage_dir)
            .context("Failed to create history directory")?;

        Ok(Self {
            storage_dir,
            current_session: None,
            current_project: None,
            config: SummarizerConfig::default(),
        })
    }

    pub fn set_project(&mut self, project_path: Option<String>) {
        self.current_project = project_path;
    }

    pub fn current_project(&self) -> Option<&str> {
        self.current_project.as_deref()
    }

    pub fn new_session(&mut self) -> &mut ChatSession {
        if let Some(ref session) = self.current_session {
            if !session.messages.is_empty() {
                let _ = self.save_session(session);
            }
        }

        let session = ChatSession::new(self.current_project.clone());
        self.current_session = Some(session);
        self.current_session.as_mut().unwrap()
    }

    pub fn current_session(&mut self) -> &mut ChatSession {
        if self.current_session.is_none() {
            self.new_session();
        }
        self.current_session.as_mut().unwrap()
    }

    pub fn has_session(&self) -> bool {
        self.current_session.is_some()
    }

    pub fn add_user_message(&mut self, content: String) {
        let session = self.current_session();
        session.add_message(Message::user(content));
    }

    pub fn add_assistant_message(&mut self, content: String) {
        let session = self.current_session();
        session.add_message(Message::assistant(content));
    }

    pub fn set_tool(&mut self, tool: Tool) {
        let session = self.current_session();
        session.tool = Some(tool);
    }

    pub fn add_code_reference(&mut self, file_path: String, snippet: Option<String>, language: Option<String>) {
        let session = self.current_session();
        session.key_references.push(CodeReference {
            file_path,
            language,
            snippet,
            line_range: None,
        });
    }

    pub fn get_transfer_context(&self) -> Option<TransferContext> {
        self.current_session.as_ref()
            .map(|session| create_transfer_context(session, &self.config))
    }

    pub fn transfer_to_new_session(&mut self) -> Option<TransferContext> {
        let context = self.get_transfer_context();

        if context.is_some() {
            if let Some(ref session) = self.current_session {
                let _ = self.save_session(session);
            }

            let mut new_session = ChatSession::new(self.current_project.clone());
            if let Some(ref ctx) = context {
                if !ctx.summary.is_empty() {
                    new_session.summary = Some(ctx.summary.clone());
                }
            }
            self.current_session = Some(new_session);
        }

        context
    }

    pub fn get_context_prompt(&self) -> Option<String> {
        self.get_transfer_context().map(|ctx| ctx.as_prompt_prefix())
    }

    pub fn auto_summarize(&mut self) {
        if let Some(ref mut session) = self.current_session {
            if session.needs_summarization(self.config.summarize_threshold) && session.summary.is_none() {
                let messages_to_summarize = if session.messages.len() > self.config.keep_recent_messages {
                    &session.messages[..session.messages.len() - self.config.keep_recent_messages]
                } else {
                    &[]
                };

                if !messages_to_summarize.is_empty() {
                    session.summary = Some(summarize_messages(messages_to_summarize, &self.config));
                }
            }
        }
    }

    pub fn save_current(&self) -> Result<()> {
        if let Some(ref session) = self.current_session {
            self.save_session(session)?;
            let _ = self.auto_prune();
        }
        Ok(())
    }

    fn save_session(&self, session: &ChatSession) -> Result<()> {
        if session.messages.is_empty() {
            return Ok(());
        }

        let mut session_clone = session.clone();
        session_clone.auto_title();

        let filename = format!("{}.json", session_clone.id);
        let path = self.storage_dir.join(&filename);

        let file = fs::File::create(&path)
            .context("Failed to create session file")?;
        let writer = BufWriter::new(file);

        serde_json::to_writer_pretty(writer, &session_clone)
            .context("Failed to serialize session")?;

        self.update_index(&session_clone)?;

        Ok(())
    }

    pub fn load_session(&self, session_id: Uuid) -> Result<ChatSession> {
        let filename = format!("{}.json", session_id);
        let path = self.storage_dir.join(&filename);

        let file = fs::File::open(&path)
            .context("Session not found")?;
        let reader = BufReader::new(file);

        let session: ChatSession = serde_json::from_reader(reader)
            .context("Failed to parse session")?;

        Ok(session)
    }

    pub fn resume_session(&mut self, session_id: Uuid) -> Result<&mut ChatSession> {
        if let Some(ref session) = self.current_session {
            if !session.messages.is_empty() {
                let _ = self.save_session(session);
            }
        }

        let session = self.load_session(session_id)?;
        self.current_session = Some(session);
        Ok(self.current_session.as_mut().unwrap())
    }

    pub fn get_recent_sessions(&self, limit: usize) -> Result<Vec<HistoryEntry>> {
        let index = self.load_index()?;
        let mut entries: Vec<_> = index.into_iter().collect();

        entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

        Ok(entries.into_iter().take(limit).collect())
    }

    pub fn get_project_sessions(&self) -> Result<Vec<HistoryEntry>> {
        let project = match &self.current_project {
            Some(p) => p,
            None => return Ok(Vec::new()),
        };

        let index = self.load_index()?;
        let mut entries: Vec<_> = index.into_iter()
            .filter(|e| e.project_path.as_ref() == Some(project))
            .collect();

        entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

        Ok(entries)
    }

    pub fn get_accessible_history(&self) -> Result<Vec<HistoryEntry>> {
        let mut result = Vec::new();
        let index = self.load_index()?;

        if let Some(ref project) = self.current_project {
            for entry in &index {
                if entry.project_path.as_ref() == Some(project) {
                    result.push(entry.clone());
                }
            }
        }

        let mut global_entries: Vec<_> = index.into_iter()
            .filter(|e| !result.iter().any(|r| r.session_id == e.session_id))
            .collect();
        global_entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

        result.extend(global_entries.into_iter().take(5));

        result.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

        Ok(result)
    }

    fn load_index(&self) -> Result<Vec<HistoryEntry>> {
        let index_path = self.storage_dir.join("index.json");

        if !index_path.exists() {
            return Ok(Vec::new());
        }

        let file = fs::File::open(&index_path)
            .context("Failed to open index")?;
        let reader = BufReader::new(file);

        let index: Vec<HistoryEntry> = serde_json::from_reader(reader)
            .context("Failed to parse index")?;

        Ok(index)
    }

    fn update_index(&self, session: &ChatSession) -> Result<()> {
        let index_path = self.storage_dir.join("index.json");
        let mut index = self.load_index().unwrap_or_default();

        let entry = HistoryEntry::from(session);
        if let Some(pos) = index.iter().position(|e| e.session_id == session.id) {
            index[pos] = entry;
        } else {
            index.push(entry);
        }

        if index.len() > 100 {
            index.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
            index.truncate(100);
        }

        let file = fs::File::create(&index_path)
            .context("Failed to create index file")?;
        let writer = BufWriter::new(file);

        serde_json::to_writer_pretty(writer, &index)
            .context("Failed to serialize index")?;

        Ok(())
    }

    pub fn delete_session(&self, session_id: Uuid) -> Result<()> {
        let filename = format!("{}.json", session_id);
        let path = self.storage_dir.join(&filename);

        if path.exists() {
            fs::remove_file(&path)?;
        }

        let index_path = self.storage_dir.join("index.json");
        let mut index = self.load_index().unwrap_or_default();
        index.retain(|e| e.session_id != session_id);

        let file = fs::File::create(&index_path)?;
        let writer = BufWriter::new(file);
        serde_json::to_writer_pretty(writer, &index)?;

        Ok(())
    }

    pub fn clear_all(&self) -> Result<()> {
        for entry in fs::read_dir(&self.storage_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                fs::remove_file(path)?;
            }
        }
        Ok(())
    }

    pub fn storage_dir(&self) -> &PathBuf {
        &self.storage_dir
    }

    pub fn config(&self) -> &SummarizerConfig {
        &self.config
    }

    pub fn set_config(&mut self, config: SummarizerConfig) {
        self.config = config;
    }

    pub fn search(&self, query: &str) -> Result<Vec<HistoryEntry>> {
        let index = self.load_index()?;
        let mut matches: Vec<_> = index.into_iter()
            .filter(|e| e.matches_search(query))
            .collect();

        matches.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(matches)
    }

    pub fn search_project(&self, query: &str) -> Result<Vec<HistoryEntry>> {
        let project = match &self.current_project {
            Some(p) => p,
            None => return self.search(query),
        };

        let index = self.load_index()?;
        let mut matches: Vec<_> = index.into_iter()
            .filter(|e| e.project_path.as_ref() == Some(project) && e.matches_search(query))
            .collect();

        matches.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(matches)
    }

    pub fn prune_old_sessions(&self, max_sessions: usize) -> Result<usize> {
        let mut index = self.load_index()?;

        if index.len() <= max_sessions {
            return Ok(0);
        }

        index.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

        let to_delete: Vec<_> = index.split_off(max_sessions);
        let deleted_count = to_delete.len();

        for entry in &to_delete {
            let filename = format!("{}.json", entry.session_id);
            let path = self.storage_dir.join(&filename);
            if path.exists() {
                let _ = fs::remove_file(&path);
            }
        }

        let index_path = self.storage_dir.join("index.json");
        let file = fs::File::create(&index_path)?;
        let writer = BufWriter::new(file);
        serde_json::to_writer_pretty(writer, &index)?;

        Ok(deleted_count)
    }

    pub fn auto_prune(&self) -> Result<usize> {
        self.prune_old_sessions(self.config.max_history_sessions)
    }

    pub fn set_session_title(&mut self, title: String) {
        if let Some(ref mut session) = self.current_session {
            session.set_title(title);
        }
    }

    pub fn auto_title_current(&mut self) {
        if let Some(ref mut session) = self.current_session {
            session.auto_title();
        }
    }
}

impl Drop for HistoryManager {
    fn drop(&mut self) {
        let _ = self.save_current();
    }
}

pub fn format_history_entry(entry: &HistoryEntry, compact: bool) -> String {
    let tool_name = entry.tool.map(|t| t.as_str()).unwrap_or("unknown");
    let time_ago = format_time_ago(entry.updated_at);

    if compact {
        format!(
            "[{}] {} - {} ({} msgs)",
            tool_name,
            polyglot_common::truncate_smart(&entry.preview, 50),
            time_ago,
            entry.message_count
        )
    } else {
        let project = entry.project_path.as_deref().unwrap_or("global");
        format!(
            "Session: {}\n  Tool: {}\n  Project: {}\n  Messages: {}\n  Updated: {}\n  Preview: {}",
            entry.session_id,
            tool_name,
            project,
            entry.message_count,
            time_ago,
            polyglot_common::truncate_smart(&entry.preview, 100)
        )
    }
}

fn format_time_ago(time: chrono::DateTime<chrono::Utc>) -> String {
    let now = Utc::now();
    let duration = now.signed_duration_since(time);

    if duration.num_seconds() < 60 {
        "just now".to_string()
    } else if duration.num_minutes() < 60 {
        format!("{}m ago", duration.num_minutes())
    } else if duration.num_hours() < 24 {
        format!("{}h ago", duration.num_hours())
    } else if duration.num_days() < 7 {
        format!("{}d ago", duration.num_days())
    } else {
        time.format("%Y-%m-%d").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_history_manager_new_session() {
        let temp_dir = tempdir().unwrap();
        let mut manager = HistoryManager::new(Some(temp_dir.path().to_path_buf())).unwrap();

        manager.add_user_message("Hello".to_string());
        manager.add_assistant_message("Hi there!".to_string());

        assert!(manager.has_session());
        assert_eq!(manager.current_session().messages.len(), 2);
    }

    #[test]
    fn test_transfer_context() {
        let temp_dir = tempdir().unwrap();
        let mut manager = HistoryManager::new(Some(temp_dir.path().to_path_buf())).unwrap();

        manager.add_user_message("Help me with Rust".to_string());
        manager.add_assistant_message("Sure! What do you need?".to_string());
        manager.add_user_message("How do I handle errors?".to_string());

        let context = manager.get_transfer_context().unwrap();
        assert_eq!(context.current_question, "How do I handle errors?");
    }
}
