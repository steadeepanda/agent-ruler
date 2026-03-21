use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::runners::RunnerKind;

pub const SESSIONS_FILE_NAME: &str = "sessions.json";
const DEFAULT_PAGE_LIMIT: usize = 10;
const MAX_PAGE_LIMIT: usize = 100;
const RECENT_ACTIVITY_DAYS: i64 = 7;
const AUTO_BIND_RECENT_WINDOW_HOURS: i64 = 24;
const MAX_TITLE_LENGTH: usize = 160;
const MAX_LABEL_LENGTH: usize = 120;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Archived,
}

impl SessionStatus {
    pub fn from_id(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "active" => Some(Self::Active),
            "archived" => Some(Self::Archived),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Archived => "archived",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SessionChannel {
    Telegram,
    Tui,
    Web,
    Api,
}

impl SessionChannel {
    pub fn from_id(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "telegram" => Some(Self::Telegram),
            "tui" => Some(Self::Tui),
            "web" => Some(Self::Web),
            "api" => Some(Self::Api),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Telegram => "telegram",
            Self::Tui => "tui",
            Self::Web => "web",
            Self::Api => "api",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Telegram => "Telegram",
            Self::Tui => "TUI",
            Self::Web => "Web",
            Self::Api => "API",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRecord {
    pub id: String,
    pub runner_kind: RunnerKind,
    pub created_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
    pub status: SessionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_session_key: Option<String>,
    #[serde(default)]
    pub channels: Vec<SessionChannel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telegram_chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telegram_thread_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telegram_message_anchor_id: Option<i64>,
}

impl SessionRecord {
    pub fn display_label(&self) -> String {
        if let Some(title) = self
            .title
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            return title.to_string();
        }
        if let Some(label) = self
            .label
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            return label.to_string();
        }
        if let Some(thread_id) = self.telegram_thread_id {
            return format!("Telegram thread {thread_id}");
        }
        if let Some(session_key) = self
            .runner_session_key
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            return format!(
                "{} session {}",
                self.runner_kind.display_name(),
                truncate_tail(session_key, 24)
            );
        }
        format!(
            "{} session {}",
            self.runner_kind.display_name(),
            truncate_tail(&self.id, 12)
        )
    }

    pub fn channel_ids(&self) -> Vec<String> {
        let mut channels = normalized_channels(&self.channels, self.telegram_chat_id.is_some());
        channels.sort();
        channels.dedup();
        channels
            .into_iter()
            .map(|channel| channel.id().to_string())
            .collect()
    }

    pub fn channel_labels(&self) -> Vec<String> {
        let mut channels = normalized_channels(&self.channels, self.telegram_chat_id.is_some());
        channels.sort();
        channels.dedup();
        channels
            .into_iter()
            .map(|channel| channel.label().to_string())
            .collect()
    }

    fn matches_channel(&self, channel: SessionChannel) -> bool {
        normalized_channels(&self.channels, self.telegram_chat_id.is_some()).contains(&channel)
    }

    fn matches_search(&self, query: &str) -> bool {
        let needle = query.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return true;
        }

        let mut haystack = vec![
            self.id.clone(),
            self.title.clone().unwrap_or_default(),
            self.label.clone().unwrap_or_default(),
            self.runner_session_key.clone().unwrap_or_default(),
            self.telegram_chat_id.clone().unwrap_or_default(),
            self.runner_kind.id().to_string(),
        ];
        if let Some(thread_id) = self.telegram_thread_id {
            haystack.push(thread_id.to_string());
        }

        haystack
            .into_iter()
            .any(|value| value.trim().to_ascii_lowercase().contains(needle.as_str()))
    }
}

#[derive(Debug, Clone)]
pub struct SessionStore {
    path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct SessionListQuery {
    pub runner_kind: Option<RunnerKind>,
    pub channel: Option<SessionChannel>,
    pub status: Option<SessionStatus>,
    pub recent_only: bool,
    pub search: Option<String>,
    pub limit: usize,
    pub cursor: usize,
}

impl Default for SessionListQuery {
    fn default() -> Self {
        Self {
            runner_kind: None,
            channel: None,
            status: None,
            recent_only: false,
            search: None,
            limit: DEFAULT_PAGE_LIMIT,
            cursor: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionListResult {
    pub items: Vec<SessionRecord>,
    pub total: usize,
    pub limit: usize,
    pub cursor: usize,
    pub has_more: bool,
    pub next_cursor: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct SessionUpsertResult {
    pub session: SessionRecord,
    pub created: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionView {
    pub id: String,
    pub runner_kind: String,
    pub runner_label: String,
    pub created_at: String,
    pub last_active_at: String,
    pub status: String,
    pub title: Option<String>,
    pub label: Option<String>,
    pub display_label: String,
    pub runner_session_key: Option<String>,
    pub channels: Vec<String>,
    pub channel_labels: Vec<String>,
    pub telegram_chat_id: Option<String>,
    pub telegram_thread_id: Option<i64>,
    pub telegram_message_anchor_id: Option<i64>,
}

impl From<&SessionRecord> for SessionView {
    fn from(value: &SessionRecord) -> Self {
        Self {
            id: value.id.clone(),
            runner_kind: value.runner_kind.id().to_string(),
            runner_label: value.runner_kind.display_name().to_string(),
            created_at: value.created_at.to_rfc3339(),
            last_active_at: value.last_active_at.to_rfc3339(),
            status: value.status.id().to_string(),
            title: value.title.clone(),
            label: value.label.clone(),
            display_label: value.display_label(),
            runner_session_key: value.runner_session_key.clone(),
            channels: value.channel_ids(),
            channel_labels: value.channel_labels(),
            telegram_chat_id: value.telegram_chat_id.clone(),
            telegram_thread_id: value.telegram_thread_id,
            telegram_message_anchor_id: value.telegram_message_anchor_id,
        }
    }
}

impl SessionStore {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    pub fn default_path(state_dir: &Path) -> PathBuf {
        state_dir.join(SESSIONS_FILE_NAME)
    }

    pub fn list(&self) -> Result<Vec<SessionRecord>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let raw = fs::read_to_string(&self.path)
            .with_context(|| format!("read sessions file {}", self.path.display()))?;
        let mut items: Vec<SessionRecord> = serde_json::from_str(raw.trim())
            .or_else(|_| serde_json::from_str("[]"))
            .context("parse sessions json")?;

        for session in &mut items {
            session.title = normalize_optional_text(session.title.take(), MAX_TITLE_LENGTH);
            session.label = normalize_optional_text(session.label.take(), MAX_LABEL_LENGTH);
            session.runner_session_key =
                normalize_optional_text(session.runner_session_key.take(), 200);
            session.telegram_chat_id =
                normalize_optional_text(session.telegram_chat_id.take(), 120);
            session.channels =
                normalized_channels(&session.channels, session.telegram_chat_id.is_some());
        }

        sort_sessions(&mut items);
        Ok(items)
    }

    pub fn get(&self, id: &str) -> Result<Option<SessionRecord>> {
        let items = self.list()?;
        Ok(items.into_iter().find(|item| item.id == id.trim()))
    }

    pub fn page(&self, query: &SessionListQuery) -> Result<SessionListResult> {
        let mut items = self.list()?;
        let now = Utc::now();
        let recent_cutoff = now - Duration::days(RECENT_ACTIVITY_DAYS);
        let search = query
            .search
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_string();

        items.retain(|item| {
            if let Some(runner_kind) = query.runner_kind {
                if item.runner_kind != runner_kind {
                    return false;
                }
            }
            if let Some(channel) = query.channel {
                if !item.matches_channel(channel) {
                    return false;
                }
            }
            if let Some(status) = query.status {
                if item.status != status {
                    return false;
                }
            }
            if query.recent_only && item.last_active_at < recent_cutoff {
                return false;
            }
            item.matches_search(&search)
        });

        let total = items.len();
        let cursor = query.cursor.min(total);
        let limit = query.limit.clamp(1, MAX_PAGE_LIMIT);
        let end = (cursor + limit).min(total);
        let page_items = items[cursor..end].to_vec();
        let has_more = end < total;

        Ok(SessionListResult {
            items: page_items,
            total,
            limit,
            cursor,
            has_more,
            next_cursor: has_more.then_some(end),
        })
    }

    pub fn touch_runner_session(
        &self,
        runner_kind: RunnerKind,
        runner_session_key: &str,
        channel: SessionChannel,
        label: Option<&str>,
        title: Option<&str>,
    ) -> Result<SessionRecord> {
        let session_key = runner_session_key.trim();
        if session_key.is_empty() {
            return Err(anyhow!("runner_session_key must not be empty"));
        }

        let now = Utc::now();
        let mut items = self.list()?;
        if let Some(index) = items.iter().position(|item| {
            item.runner_kind == runner_kind
                && item
                    .runner_session_key
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    == Some(session_key)
        }) {
            let existing = &mut items[index];
            existing.last_active_at = now;
            existing.status = SessionStatus::Active;
            merge_channel(&mut existing.channels, channel);
            maybe_set_text(&mut existing.label, label, MAX_LABEL_LENGTH);
            maybe_set_text(&mut existing.title, title, MAX_TITLE_LENGTH);
            let updated = existing.clone();
            self.persist(&items)?;
            return Ok(updated);
        }

        let mut record = SessionRecord {
            id: uuid::Uuid::new_v4().to_string(),
            runner_kind,
            created_at: now,
            last_active_at: now,
            status: SessionStatus::Active,
            title: normalize_optional_text(title.map(ToOwned::to_owned), MAX_TITLE_LENGTH),
            label: normalize_optional_text(label.map(ToOwned::to_owned), MAX_LABEL_LENGTH),
            runner_session_key: Some(session_key.to_string()),
            channels: vec![channel],
            telegram_chat_id: None,
            telegram_thread_id: None,
            telegram_message_anchor_id: None,
        };
        record.channels = normalized_channels(&record.channels, false);
        items.push(record.clone());
        self.persist(&items)?;
        Ok(record)
    }

    pub fn bind_runner_session_key(
        &self,
        session_id: &str,
        runner_session_key: &str,
    ) -> Result<SessionRecord> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return Err(anyhow!("session_id must not be empty"));
        }
        let runner_session_key = runner_session_key.trim();
        if runner_session_key.is_empty() {
            return Err(anyhow!("runner_session_key must not be empty"));
        }

        let now = Utc::now();
        let mut items = self.list()?;
        let Some(index) = items.iter().position(|item| item.id == session_id) else {
            return Err(anyhow!("session `{session_id}` not found"));
        };

        let runner_kind = items[index].runner_kind;
        if let Some(existing) = items.iter().find(|item| {
            item.id != session_id
                && item.runner_kind == runner_kind
                && item
                    .runner_session_key
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    == Some(runner_session_key)
        }) {
            return Err(anyhow!(
                "runner session key `{runner_session_key}` is already bound to session `{}`",
                existing.id
            ));
        }

        let record = &mut items[index];
        record.runner_session_key = Some(runner_session_key.to_string());
        record.last_active_at = now;
        record.status = SessionStatus::Active;
        let updated = record.clone();
        self.persist(&items)?;
        Ok(updated)
    }

    pub fn resolve_telegram_thread(
        &self,
        runner_kind: RunnerKind,
        chat_id: &str,
        thread_id: i64,
        message_anchor_id: Option<i64>,
        title: Option<&str>,
        bind_session_id: Option<&str>,
        bind_runner_session_key: Option<&str>,
        prefer_existing_runner_session: bool,
    ) -> Result<SessionUpsertResult> {
        let chat_id = chat_id.trim();
        if chat_id.is_empty() {
            return Err(anyhow!("telegram chat_id must not be empty"));
        }
        if thread_id <= 0 {
            return Err(anyhow!("telegram thread_id must be greater than zero"));
        }

        let now = Utc::now();
        let mut items = self.list()?;
        if let Some(index) = items.iter().position(|item| {
            item.telegram_chat_id.as_deref() == Some(chat_id)
                && item.telegram_thread_id == Some(thread_id)
        }) {
            let existing = &mut items[index];
            if existing.runner_kind != runner_kind {
                return Err(anyhow!(
                    "telegram thread {}#{} is already bound to runner `{}`",
                    chat_id,
                    thread_id,
                    existing.runner_kind.id()
                ));
            }
            existing.last_active_at = now;
            existing.status = SessionStatus::Active;
            merge_channel(&mut existing.channels, SessionChannel::Telegram);
            if existing.telegram_message_anchor_id.is_none() {
                existing.telegram_message_anchor_id = normalize_positive_i64(message_anchor_id);
            }
            maybe_set_text(&mut existing.title, title, MAX_TITLE_LENGTH);
            let updated = existing.clone();
            self.persist(&items)?;
            return Ok(SessionUpsertResult {
                session: updated,
                created: false,
            });
        }

        let mut bind_index = None;
        if let Some(target_id) = bind_session_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let Some(index) = items.iter().position(|item| item.id == target_id) else {
                return Err(anyhow!("session `{target_id}` not found"));
            };
            bind_index = Some(index);
        } else if let Some(session_key) = bind_runner_session_key
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let Some(index) = items.iter().position(|item| {
                item.runner_kind == runner_kind
                    && item
                        .runner_session_key
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        == Some(session_key)
            }) else {
                return Err(anyhow!(
                    "runner session key `{session_key}` not found for runner `{}`",
                    runner_kind.id()
                ));
            };
            bind_index = Some(index);
        } else if prefer_existing_runner_session {
            bind_index = select_recent_runner_session_for_telegram_bind(&items, runner_kind, now);
        }

        if let Some(index) = bind_index {
            let existing = &mut items[index];
            if existing.runner_kind != runner_kind {
                return Err(anyhow!(
                    "session `{}` is bound to runner `{}`, expected `{}`",
                    existing.id,
                    existing.runner_kind.id(),
                    runner_kind.id()
                ));
            }
            if let (Some(existing_chat), Some(existing_thread)) = (
                existing.telegram_chat_id.as_deref(),
                existing.telegram_thread_id,
            ) {
                if existing_chat != chat_id || existing_thread != thread_id {
                    return Err(anyhow!(
                        "session `{}` is already bound to telegram thread {}#{}",
                        existing.id,
                        existing_chat,
                        existing_thread
                    ));
                }
            }
            existing.last_active_at = now;
            existing.status = SessionStatus::Active;
            merge_channel(&mut existing.channels, SessionChannel::Telegram);
            existing.telegram_chat_id = Some(chat_id.to_string());
            existing.telegram_thread_id = Some(thread_id);
            if existing.telegram_message_anchor_id.is_none() {
                existing.telegram_message_anchor_id = normalize_positive_i64(message_anchor_id);
            }
            maybe_set_text(&mut existing.title, title, MAX_TITLE_LENGTH);
            let updated = existing.clone();
            self.persist(&items)?;
            return Ok(SessionUpsertResult {
                session: updated,
                created: false,
            });
        }

        let mut record = SessionRecord {
            id: uuid::Uuid::new_v4().to_string(),
            runner_kind,
            created_at: now,
            last_active_at: now,
            status: SessionStatus::Active,
            title: normalize_optional_text(title.map(ToOwned::to_owned), MAX_TITLE_LENGTH),
            label: None,
            runner_session_key: None,
            channels: vec![SessionChannel::Telegram],
            telegram_chat_id: Some(chat_id.to_string()),
            telegram_thread_id: Some(thread_id),
            telegram_message_anchor_id: normalize_positive_i64(message_anchor_id),
        };
        record.channels = normalized_channels(&record.channels, true);
        items.push(record.clone());
        self.persist(&items)?;
        Ok(SessionUpsertResult {
            session: record,
            created: true,
        })
    }

    fn persist(&self, items: &[SessionRecord]) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create sessions parent {}", parent.display()))?;
        }
        let payload = serde_json::to_string_pretty(items).context("serialize sessions")?;
        let temp = self.path.with_extension("json.tmp");
        fs::write(&temp, payload).with_context(|| format!("write {}", temp.display()))?;
        fs::rename(&temp, &self.path).with_context(|| {
            format!(
                "replace sessions file {} -> {}",
                temp.display(),
                self.path.display()
            )
        })?;
        Ok(())
    }
}

fn normalize_optional_text(value: Option<String>, max_len: usize) -> Option<String> {
    let value = value?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut normalized = trimmed.to_string();
    if normalized.len() > max_len {
        normalized.truncate(max_len);
        normalized = normalized.trim().to_string();
    }
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn maybe_set_text(target: &mut Option<String>, candidate: Option<&str>, max_len: usize) {
    if target.is_some() {
        return;
    }
    *target = normalize_optional_text(candidate.map(ToOwned::to_owned), max_len);
}

fn normalize_positive_i64(value: Option<i64>) -> Option<i64> {
    value.filter(|candidate| *candidate > 0)
}

fn truncate_tail(value: &str, max_len: usize) -> String {
    let trimmed = value.trim();
    if trimmed.len() <= max_len {
        return trimmed.to_string();
    }
    format!("{}...", &trimmed[..max_len])
}

fn merge_channel(channels: &mut Vec<SessionChannel>, channel: SessionChannel) {
    if !channels.contains(&channel) {
        channels.push(channel);
        channels.sort();
        channels.dedup();
    }
}

fn normalized_channels(channels: &[SessionChannel], include_telegram: bool) -> Vec<SessionChannel> {
    let mut output = channels.to_vec();
    if include_telegram {
        output.push(SessionChannel::Telegram);
    }
    output.sort();
    output.dedup();
    output
}

fn sort_sessions(items: &mut [SessionRecord]) {
    items.sort_by(|a, b| {
        b.last_active_at
            .cmp(&a.last_active_at)
            .then_with(|| b.created_at.cmp(&a.created_at))
            .then_with(|| a.id.cmp(&b.id))
    });
}

fn select_recent_runner_session_for_telegram_bind(
    items: &[SessionRecord],
    runner_kind: RunnerKind,
    now: DateTime<Utc>,
) -> Option<usize> {
    let recency_cutoff = now - Duration::hours(AUTO_BIND_RECENT_WINDOW_HOURS);
    items.iter().position(|item| {
        item.runner_kind == runner_kind
            && item.status == SessionStatus::Active
            && item.last_active_at >= recency_cutoff
            && item
                .runner_session_key
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_some()
            && item.telegram_chat_id.is_none()
            && item.telegram_thread_id.is_none()
    })
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use tempfile::tempdir;

    use super::{SessionChannel, SessionListQuery, SessionRecord, SessionStatus, SessionStore};
    use crate::runners::RunnerKind;

    fn store_for_temp() -> SessionStore {
        let dir = tempdir().expect("temp dir");
        let path = dir.path().join("sessions.json");
        // Keep tempdir alive for the test process by leaking it.
        std::mem::forget(dir);
        SessionStore::new(path)
    }

    #[test]
    fn telegram_thread_mapping_creates_and_reuses_session() {
        let store = store_for_temp();

        let first = store
            .resolve_telegram_thread(
                RunnerKind::Claudecode,
                "12345",
                17,
                Some(101),
                Some("Daily standup"),
                None,
                None,
                false,
            )
            .expect("create telegram session");
        assert!(first.created, "first resolve should create a session");
        assert_eq!(first.session.telegram_chat_id.as_deref(), Some("12345"));
        assert_eq!(first.session.telegram_thread_id, Some(17));
        assert_eq!(first.session.telegram_message_anchor_id, Some(101));
        assert_eq!(first.session.title.as_deref(), Some("Daily standup"));

        let second = store
            .resolve_telegram_thread(
                RunnerKind::Claudecode,
                "12345",
                17,
                Some(202),
                Some("Ignored title replacement"),
                None,
                None,
                false,
            )
            .expect("reuse telegram session");
        assert!(
            !second.created,
            "second resolve should reuse the same session"
        );
        assert_eq!(first.session.id, second.session.id);
        assert_eq!(second.session.telegram_message_anchor_id, Some(101));
        assert_eq!(second.session.title.as_deref(), Some("Daily standup"));
    }

    #[test]
    fn telegram_thread_binding_cannot_switch_runner_kind() {
        let store = store_for_temp();
        store
            .resolve_telegram_thread(
                RunnerKind::Claudecode,
                "12345",
                88,
                Some(1),
                None,
                None,
                None,
                false,
            )
            .expect("seed session");

        let err = store
            .resolve_telegram_thread(
                RunnerKind::Opencode,
                "12345",
                88,
                Some(2),
                None,
                None,
                None,
                false,
            )
            .expect_err("runner switch should be rejected");
        assert!(
            err.to_string()
                .contains("already bound to runner `claudecode`"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn pagination_and_filtering_support_runner_channel_recent_and_search() {
        let store = store_for_temp();
        let mut seeded = Vec::new();
        for idx in 0..6 {
            let record = SessionRecord {
                id: format!("session-{idx}"),
                runner_kind: if idx % 2 == 0 {
                    RunnerKind::Claudecode
                } else {
                    RunnerKind::Opencode
                },
                created_at: Utc::now() - Duration::minutes(idx as i64),
                last_active_at: if idx == 5 {
                    Utc::now() - Duration::days(10)
                } else {
                    Utc::now() - Duration::minutes(idx as i64)
                },
                status: if idx == 4 {
                    SessionStatus::Archived
                } else {
                    SessionStatus::Active
                },
                title: Some(format!("Thread {idx}")),
                label: None,
                runner_session_key: Some(format!("runner-session-{idx}")),
                channels: if idx % 2 == 0 {
                    vec![SessionChannel::Telegram]
                } else {
                    vec![SessionChannel::Tui]
                },
                telegram_chat_id: if idx % 2 == 0 {
                    Some("chat-1".to_string())
                } else {
                    None
                },
                telegram_thread_id: if idx % 2 == 0 {
                    Some(100 + idx as i64)
                } else {
                    None
                },
                telegram_message_anchor_id: None,
            };
            seeded.push(record);
        }
        store.persist(&seeded).expect("persist fixtures");

        let first_page = store
            .page(&SessionListQuery {
                runner_kind: Some(RunnerKind::Claudecode),
                channel: Some(SessionChannel::Telegram),
                status: Some(SessionStatus::Active),
                recent_only: true,
                search: Some("Thread".to_string()),
                limit: 2,
                cursor: 0,
            })
            .expect("page query");
        assert_eq!(first_page.total, 2);
        assert_eq!(first_page.items.len(), 2);
        assert!(first_page.has_more == false);
        assert!(first_page
            .items
            .iter()
            .all(|item| item.runner_kind == RunnerKind::Claudecode));
        assert!(first_page
            .items
            .iter()
            .all(|item| item.telegram_thread_id.is_some()));

        let paged = store
            .page(&SessionListQuery {
                limit: 2,
                cursor: 2,
                ..SessionListQuery::default()
            })
            .expect("paged query");
        assert_eq!(paged.items.len(), 2);
        assert_eq!(paged.cursor, 2);
        assert_eq!(paged.limit, 2);
    }

    #[test]
    fn explicit_session_bind_attaches_telegram_thread_to_existing_runner_session() {
        let store = store_for_temp();
        let seeded = store
            .touch_runner_session(
                RunnerKind::Claudecode,
                "runner-session-alpha",
                SessionChannel::Tui,
                Some("Agent A"),
                Some("Daily report"),
            )
            .expect("seed runner session");

        let resolved = store
            .resolve_telegram_thread(
                RunnerKind::Claudecode,
                "chat-1",
                901,
                Some(77),
                Some("Ignored title replacement"),
                Some(&seeded.id),
                None,
                false,
            )
            .expect("bind explicit session");

        assert!(!resolved.created);
        assert_eq!(resolved.session.id, seeded.id);
        assert_eq!(resolved.session.telegram_chat_id.as_deref(), Some("chat-1"));
        assert_eq!(resolved.session.telegram_thread_id, Some(901));
        assert_eq!(resolved.session.telegram_message_anchor_id, Some(77));
        assert_eq!(
            resolved.session.title.as_deref(),
            Some("Daily report"),
            "existing title should not be overwritten by bind call"
        );
    }

    #[test]
    fn auto_bind_prefers_recent_runner_session_without_telegram_mapping() {
        let store = store_for_temp();
        let seeded = store
            .touch_runner_session(
                RunnerKind::Claudecode,
                "runner-session-continue",
                SessionChannel::Tui,
                Some("Agent B"),
                Some("Computer started session"),
            )
            .expect("seed runner session");

        let resolved = store
            .resolve_telegram_thread(
                RunnerKind::Claudecode,
                "chat-2",
                777,
                Some(300),
                None,
                None,
                None,
                true,
            )
            .expect("auto-bind recent runner session");

        assert!(!resolved.created);
        assert_eq!(resolved.session.id, seeded.id);
        assert_eq!(resolved.session.telegram_chat_id.as_deref(), Some("chat-2"));
        assert_eq!(resolved.session.telegram_thread_id, Some(777));
    }
}
