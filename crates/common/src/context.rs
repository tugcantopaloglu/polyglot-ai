use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Tool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: Uuid,
    pub role: MessageRole,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub token_estimate: u32,
}

impl Message {
    pub fn new(role: MessageRole, content: String) -> Self {
        let token_estimate = (content.len() / 4) as u32;
        Self {
            id: Uuid::new_v4(),
            role,
            content,
            timestamp: Utc::now(),
            token_estimate,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::new(MessageRole::User, content.into())
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(MessageRole::Assistant, content.into())
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::new(MessageRole::System, content.into())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSession {
    pub id: Uuid,
    pub title: Option<String>,
    pub project_path: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tool: Option<Tool>,
    pub messages: Vec<Message>,
    pub summary: Option<String>,
    pub key_references: Vec<CodeReference>,
    pub total_tokens: u32,
}

impl ChatSession {
    pub fn new(project_path: Option<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            title: None,
            project_path,
            created_at: now,
            updated_at: now,
            tool: None,
            messages: Vec::new(),
            summary: None,
            key_references: Vec::new(),
            total_tokens: 0,
        }
    }

    pub fn auto_title(&mut self) {
        if self.title.is_some() {
            return;
        }
        if let Some(msg) = self.messages.iter().find(|m| m.role == MessageRole::User) {
            self.title = Some(generate_title(&msg.content));
        }
    }

    pub fn set_title(&mut self, title: String) {
        self.title = Some(title);
    }

    pub fn display_title(&self) -> String {
        self.title.clone()
            .or_else(|| self.messages.first().map(|m| generate_title(&m.content)))
            .unwrap_or_else(|| "Untitled Chat".to_string())
    }

    pub fn add_message(&mut self, message: Message) {
        self.total_tokens += message.token_estimate;
        self.messages.push(message);
        self.updated_at = Utc::now();
    }

    pub fn last_messages(&self, n: usize) -> &[Message] {
        let start = self.messages.len().saturating_sub(n);
        &self.messages[start..]
    }

    pub fn last_user_message(&self) -> Option<&Message> {
        self.messages.iter().rev().find(|m| m.role == MessageRole::User)
    }

    pub fn is_project(&self, path: &str) -> bool {
        self.project_path.as_ref().map(|p| p == path).unwrap_or(false)
    }

    pub fn needs_summarization(&self, token_threshold: u32) -> bool {
        self.total_tokens > token_threshold
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeReference {
    pub file_path: String,
    pub language: Option<String>,
    pub snippet: Option<String>,
    pub line_range: Option<(u32, u32)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferContext {
    pub summary: String,
    pub current_question: String,
    pub key_points: Vec<String>,
    pub code_context: Vec<CodeReference>,
    pub project_path: Option<String>,
    pub token_estimate: u32,
}

impl TransferContext {
    pub fn as_prompt_prefix(&self) -> String {
        let mut parts = Vec::new();

        if let Some(ref path) = self.project_path {
            parts.push(format!("[Project: {}]", path));
        }

        if !self.summary.is_empty() {
            parts.push(format!("[Context: {}]", self.summary));
        }

        if !self.key_points.is_empty() {
            let points = self.key_points.join("; ");
            parts.push(format!("[Key decisions: {}]", points));
        }

        for code_ref in &self.code_context {
            if let Some(ref snippet) = code_ref.snippet {
                let lang = code_ref.language.as_deref().unwrap_or("code");
                parts.push(format!("[{}:{}]\n```{}\n{}\n```",
                    code_ref.file_path,
                    code_ref.line_range.map(|(s, e)| format!("L{}-{}", s, e)).unwrap_or_default(),
                    lang,
                    snippet
                ));
            }
        }

        if parts.is_empty() {
            self.current_question.clone()
        } else {
            format!("{}\n\n{}", parts.join("\n"), self.current_question)
        }
    }

    pub fn minimal(&self) -> String {
        if self.summary.is_empty() {
            self.current_question.clone()
        } else {
            format!("[Prior context: {}]\n\n{}",
                truncate_smart(&self.summary, 200),
                self.current_question
            )
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub session_id: Uuid,
    pub title: String,
    pub project_path: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tool: Option<Tool>,
    pub preview: String,
    pub message_count: u32,
}

impl From<&ChatSession> for HistoryEntry {
    fn from(session: &ChatSession) -> Self {
        let title = session.display_title();
        let preview = session.summary.clone()
            .or_else(|| session.messages.first().map(|m| truncate_smart(&m.content, 100)))
            .unwrap_or_else(|| "(empty session)".to_string());

        Self {
            session_id: session.id,
            title,
            project_path: session.project_path.clone(),
            created_at: session.created_at,
            updated_at: session.updated_at,
            tool: session.tool,
            preview,
            message_count: session.messages.len() as u32,
        }
    }
}

impl HistoryEntry {
    pub fn matches_search(&self, query: &str) -> bool {
        let query_lower = query.to_lowercase();
        self.title.to_lowercase().contains(&query_lower)
            || self.preview.to_lowercase().contains(&query_lower)
            || self.project_path.as_ref()
                .map(|p| p.to_lowercase().contains(&query_lower))
                .unwrap_or(false)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummarizerConfig {
    pub max_summary_tokens: u32,
    pub summarize_threshold: u32,
    pub keep_recent_messages: usize,
    pub max_snippet_length: usize,
    pub max_history_sessions: usize,
}

impl Default for SummarizerConfig {
    fn default() -> Self {
        Self {
            max_summary_tokens: 500,
            summarize_threshold: 4000,
            keep_recent_messages: 4,
            max_snippet_length: 500,
            max_history_sessions: 100,
        }
    }
}

pub fn generate_title(content: &str) -> String {
    let content = content.trim();

    let first_line = content.lines().next().unwrap_or(content);
    let first_sentence = first_line
        .split(|c| c == '.' || c == '?' || c == '!')
        .next()
        .unwrap_or(first_line);

    let cleaned: String = first_sentence
        .chars()
        .filter(|c| !c.is_control())
        .take(50)
        .collect();

    let title = cleaned.trim();

    if title.len() < 5 {
        "Chat Session".to_string()
    } else if title.len() >= 50 {
        format!("{}...", title)
    } else {
        title.to_string()
    }
}

pub fn truncate_smart(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }

    let truncated = &text[..max_chars];
    if let Some(pos) = truncated.rfind(|c| c == '.' || c == '!' || c == '?') {
        if pos > max_chars / 2 {
            return format!("{}", &text[..=pos]);
        }
    }

    if let Some(pos) = truncated.rfind(char::is_whitespace) {
        return format!("{}...", &text[..pos]);
    }

    format!("{}...", truncated)
}

pub fn extract_key_info(content: &str) -> Vec<String> {
    let mut key_info = Vec::new();

    let mut in_code_block = false;
    let mut current_lang = String::new();

    for line in content.lines() {
        if line.starts_with("```") {
            if !in_code_block {
                current_lang = line.trim_start_matches('`').to_string();
                in_code_block = true;
            } else {
                if !current_lang.is_empty() {
                    key_info.push(format!("Code: {}", current_lang));
                }
                in_code_block = false;
                current_lang.clear();
            }
        }
    }

    for word in content.split_whitespace() {
        if word.contains('/') && (word.ends_with(".rs") || word.ends_with(".py") ||
            word.ends_with(".js") || word.ends_with(".ts") || word.ends_with(".go")) {
            let path = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '/' && c != '.' && c != '_');
            if !path.is_empty() && path.len() < 100 {
                key_info.push(format!("File: {}", path));
            }
        }
    }

    let action_patterns = [
        ("implement", "Implementation"),
        ("fix", "Bug fix"),
        ("add", "Addition"),
        ("remove", "Removal"),
        ("refactor", "Refactoring"),
        ("create", "Creation"),
        ("update", "Update"),
        ("debug", "Debugging"),
    ];

    let lower_content = content.to_lowercase();
    for (pattern, label) in action_patterns {
        if lower_content.contains(pattern) {
            key_info.push(label.to_string());
            break;
        }
    }

    key_info
}

pub fn summarize_messages(messages: &[Message], config: &SummarizerConfig) -> String {
    if messages.is_empty() {
        return String::new();
    }

    let mut summary_parts = Vec::new();

    let user_count = messages.iter().filter(|m| m.role == MessageRole::User).count();
    let assistant_count = messages.iter().filter(|m| m.role == MessageRole::Assistant).count();

    if let Some(first_user) = messages.iter().find(|m| m.role == MessageRole::User) {
        let topic = truncate_smart(&first_user.content, 150);
        summary_parts.push(format!("Topic: {}", topic));
    }

    let mut all_key_info: Vec<String> = messages.iter()
        .flat_map(|m| extract_key_info(&m.content))
        .collect();
    all_key_info.sort();
    all_key_info.dedup();

    if !all_key_info.is_empty() {
        let limited = all_key_info.into_iter().take(5).collect::<Vec<_>>();
        summary_parts.push(format!("Involved: {}", limited.join(", ")));
    }

    summary_parts.push(format!("({} exchanges)", user_count.min(assistant_count)));

    if let Some(last_assistant) = messages.iter().rev().find(|m| m.role == MessageRole::Assistant) {
        let response_summary = truncate_smart(&last_assistant.content, 200);
        summary_parts.push(format!("Last response: {}", response_summary));
    }

    let full_summary = summary_parts.join(" | ");

    let max_chars = (config.max_summary_tokens * 4) as usize;
    truncate_smart(&full_summary, max_chars)
}

pub fn create_transfer_context(session: &ChatSession, config: &SummarizerConfig) -> TransferContext {
    let messages_to_summarize = if session.messages.len() > config.keep_recent_messages {
        &session.messages[..session.messages.len() - config.keep_recent_messages]
    } else {
        &[]
    };

    let summary = if !messages_to_summarize.is_empty() {
        session.summary.clone()
            .unwrap_or_else(|| summarize_messages(messages_to_summarize, config))
    } else {
        String::new()
    };

    let current_question = session.last_user_message()
        .map(|m| m.content.clone())
        .unwrap_or_default();

    let key_points: Vec<String> = session.last_messages(config.keep_recent_messages)
        .iter()
        .flat_map(|m| extract_key_info(&m.content))
        .take(5)
        .collect();

    let code_context = session.key_references.iter()
        .take(3)
        .cloned()
        .collect();

    let token_estimate = (summary.len() / 4
        + current_question.len() / 4
        + key_points.iter().map(|s| s.len()).sum::<usize>() / 4) as u32;

    TransferContext {
        summary,
        current_question,
        key_points,
        code_context,
        project_path: session.project_path.clone(),
        token_estimate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_smart() {
        let text = "This is a test sentence. And another one. Plus more.";
        let truncated = truncate_smart(text, 30);
        assert!(truncated.ends_with('.') || truncated.ends_with("..."));
        assert!(truncated.len() <= 33);
    }

    #[test]
    fn test_message_creation() {
        let msg = Message::user("Hello, world!");
        assert_eq!(msg.role, MessageRole::User);
        assert!(msg.token_estimate > 0);
    }

    #[test]
    fn test_transfer_context_minimal() {
        let ctx = TransferContext {
            summary: "Working on Rust CLI tool".to_string(),
            current_question: "How do I add error handling?".to_string(),
            key_points: vec![],
            code_context: vec![],
            project_path: Some("/my/project".to_string()),
            token_estimate: 50,
        };

        let minimal = ctx.minimal();
        assert!(minimal.contains("How do I add error handling?"));
    }
}
