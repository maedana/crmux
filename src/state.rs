use std::process::{Command, Stdio};
use std::time::Instant;
use tmux_claude_state::claude_state::ClaudeState;
use tmux_claude_state::monitor::{ClaudeSession, MonitorState};

/// Run `git -C <cwd> branch --show-current` and return the branch name.
fn resolve_git_branch(cwd: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["-C", cwd, "branch", "--show-current"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if output.status.success() {
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if branch.is_empty() {
            None
        } else {
            Some(branch)
        }
    } else {
        None
    }
}

/// A Claude Code session managed by crmux, tracked by PID.
#[derive(Debug, Clone)]
pub struct ManagedSession {
    /// PID of the Claude Code process (stable across pane moves).
    pub pid: u32,
    /// Current tmux pane ID (may change after join-pane).
    pub pane_id: String,
    /// Project name (basename of cwd).
    pub project_name: String,
    /// Current state.
    pub state: ClaudeState,
    /// When the current state was first observed.
    pub state_changed_at: Instant,
    /// Whether this session is marked for multi-preview.
    pub marked: bool,
    /// User-defined title label for this session.
    pub title: Option<String>,
    /// Claude Code `session_id` (from `SessionStart` hook).
    pub session_id: Option<String>,
    /// Model display name (from `statusLine` hook or `SessionStart` hook).
    pub model: Option<String>,
    /// Context window usage percentage (0–100).
    pub context_percent: Option<u8>,
    /// Current working directory.
    pub cwd: String,
    /// Current git branch name (if in a git repo).
    pub git_branch: Option<String>,
    /// Automatically resolved title from Claude Code session metadata.
    pub auto_title: Option<String>,
}

impl ManagedSession {
    /// Return the display title: manual title takes priority over auto title.
    pub fn display_title(&self) -> Option<&str> {
        self.title
            .as_deref()
            .or(self.auto_title.as_deref())
    }
}

/// Tab for filtering sessions by project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tab {
    All,
    Project(String),
}

/// State for tab-based session filtering.
pub struct TabState {
    pub tabs: Vec<Tab>,
    pub selected_tab: usize,
}

impl TabState {
    pub const fn new() -> Self {
        Self {
            tabs: Vec::new(),
            selected_tab: 0,
        }
    }

    /// Rebuild the tab list from sessions' project names.
    /// Maintains the currently selected tab value if it still exists.
    pub fn rebuild_tabs(&mut self, sessions: &[ManagedSession]) {
        let mut projects: Vec<String> = sessions
            .iter()
            .map(|s| s.project_name.clone())
            .collect();
        projects.sort();
        projects.dedup();

        let mut new_tabs = vec![Tab::All];
        for p in projects {
            new_tabs.push(Tab::Project(p));
        }

        // Maintain selection
        let current = self.current_tab().clone();
        self.tabs = new_tabs;
        if let Some(pos) = self.tabs.iter().position(|t| *t == current) {
            self.selected_tab = pos;
        } else {
            self.selected_tab = 0; // fallback to All
        }
    }

    /// Move to next tab (wraps around).
    pub const fn select_next_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.selected_tab = (self.selected_tab + 1) % self.tabs.len();
        }
    }

    /// Move to previous tab (wraps around).
    pub const fn select_prev_tab(&mut self) {
        if !self.tabs.is_empty() {
            if self.selected_tab == 0 {
                self.selected_tab = self.tabs.len() - 1;
            } else {
                self.selected_tab -= 1;
            }
        }
    }

    /// Get the currently selected tab.
    pub fn current_tab(&self) -> &Tab {
        self.tabs.get(self.selected_tab).unwrap_or(&Tab::All)
    }
}

/// Diff result from syncing with `MonitorState`.
#[derive(Debug, PartialEq, Eq)]
pub struct SyncDiff {
    /// PIDs of newly discovered sessions.
    pub added: Vec<u32>,
    /// PIDs of sessions that disappeared.
    pub removed: Vec<u32>,
    /// PIDs of sessions whose state changed.
    pub state_changed: Vec<u32>,
}

/// A single preview pane entry.
#[derive(Debug, Clone)]
pub struct PreviewEntry {
    /// Project name (basename of cwd).
    pub name: String,
    /// Tmux pane ID.
    pub pane_id: String,
    /// Optional session title label.
    pub title: Option<String>,
    /// Captured pane content.
    pub content: String,
}

/// Input mode for the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    /// Normal navigation mode.
    Normal,
    /// Text input mode for sending keys to a session.
    Input,
    /// Title input mode for setting session title label.
    Title,
    /// Broadcast mode for sending keys to all marked sessions.
    Broadcast,
    /// Scroll mode for navigating preview with j/k.
    Scroll,
}

/// Application state for the sidebar.
pub struct AppState {
    /// All managed sessions, ordered by discovery time.
    pub sessions: Vec<ManagedSession>,
    /// Index of the currently selected session.
    pub selected_index: usize,
    /// PID of our own sidebar pane's process (excluded from aggregation).
    pub own_pid: Option<u32>,
    /// Preview contents for each visible pane.
    pub preview_contents: Vec<PreviewEntry>,
    /// Current input mode.
    pub input_mode: InputMode,
    /// Buffer for text input in Input mode.
    pub input_buffer: String,
    /// Whether the help popup is currently shown.
    pub show_help: bool,
    /// Help popup scroll offset (in lines from the top).
    pub help_scroll: u16,
    /// Preview scroll offset (0=bottom, positive=scroll up).
    pub preview_scroll: u16,
    /// Preview area height (set during draw loop for scroll amount calculation).
    pub preview_height: u16,
    /// Whether `g` has been pressed once, waiting for the second `g` (vim `gg`).
    pub pending_g: bool,
    /// RPC messages that arrived before the matching session was discovered.
    pub pending_rpc: Vec<crate::rpc::RpcMessage>,
    /// Which mode Esc was pressed in (for Esc Esc cancel forwarding).
    pub esc_source_mode: Option<InputMode>,
    /// Whether the claudeye overlay is visible.
    pub claudeye_visible: bool,
    /// Tab state for project-based filtering.
    pub tab_state: TabState,
}

impl AppState {
    pub const fn new(own_pid: Option<u32>) -> Self {
        Self {
            sessions: Vec::new(),
            selected_index: 0,
            own_pid,
            preview_contents: Vec::new(),
            input_mode: InputMode::Normal,
            input_buffer: String::new(),
            show_help: false,
            help_scroll: 0,
            preview_scroll: 0,
            preview_height: 0,
            pending_g: false,
            pending_rpc: Vec::new(),
            esc_source_mode: None,
            claudeye_visible: false,
            tab_state: TabState::new(),
        }
    }

    /// Return sessions filtered by the current tab.
    pub fn filtered_sessions(&self) -> Vec<&ManagedSession> {
        match self.tab_state.current_tab() {
            Tab::All => self.sessions.iter().collect(),
            Tab::Project(name) => self
                .sessions
                .iter()
                .filter(|s| s.project_name == *name)
                .collect(),
        }
    }

    /// Sync with the latest `MonitorState`, returning what changed.
    pub fn sync_with_monitor(&mut self, monitor: &MonitorState) -> SyncDiff {
        let mut added = Vec::new();
        let mut removed = Vec::new();
        let mut state_changed = Vec::new();

        // Filter out our own sidebar pane
        let incoming: Vec<&ClaudeSession> = monitor
            .sessions
            .iter()
            .filter(|s| self.own_pid != Some(s.pane.pid))
            .collect();

        // Detect removed sessions
        self.sessions.retain(|managed| {
            let still_exists = incoming.iter().any(|s| s.pane.pid == managed.pid);
            if !still_exists {
                removed.push(managed.pid);
            }
            still_exists
        });

        // Detect new and state-changed sessions
        for session in &incoming {
            if let Some(existing) = self
                .sessions
                .iter_mut()
                .find(|m| m.pid == session.pane.pid)
            {
                // Update pane_id in case it changed after join-pane
                existing.pane_id.clone_from(&session.pane.id);
                if existing.state != session.state {
                    state_changed.push(existing.pid);
                    existing.state = session.state.clone();
                    existing.state_changed_at = session.state_changed_at;
                }
            } else {
                added.push(session.pane.pid);
                self.sessions.push(ManagedSession {
                    pid: session.pane.pid,
                    pane_id: session.pane.id.clone(),
                    project_name: session.pane.project_name.clone(),
                    state: session.state.clone(),
                    state_changed_at: session.state_changed_at,
                    marked: false,
                    title: None,
                    session_id: None,
                    model: None,
                    context_percent: None,
                    cwd: session.pane.cwd.clone(),
                    git_branch: None,
                    auto_title: None,
                });
            }
        }

        // Sort by project_name for stable directory-based ordering
        let selected_pid = self.selected_session().map(|s| s.pid);
        self.sessions.sort_by(|a, b| a.project_name.cmp(&b.project_name));

        // Rebuild tabs from current sessions
        self.tab_state.rebuild_tabs(&self.sessions);

        // Recalculate selected_index within filtered list
        let filtered = self.filtered_sessions();
        if let Some(pid) = selected_pid {
            if let Some(pos) = filtered.iter().position(|s| s.pid == pid) {
                self.selected_index = pos;
            } else if !filtered.is_empty() {
                self.selected_index = self.selected_index.min(filtered.len() - 1);
            } else {
                self.selected_index = 0;
            }
        } else if !filtered.is_empty() {
            self.selected_index = self.selected_index.min(filtered.len() - 1);
        } else {
            self.selected_index = 0;
        }

        // Apply buffered RPC messages to newly added sessions
        if !self.pending_rpc.is_empty() && !added.is_empty() {
            self.pending_rpc.retain(|msg| {
                let Some(pane_id) = msg.params.get("pane_id").and_then(|v| v.as_str()) else {
                    return false;
                };
                self.sessions.iter_mut().find(|s| s.pane_id == pane_id).is_none_or(|session| {
                    Self::apply_rpc_to_session(session, msg);
                    false // applied — remove from buffer
                })
            });
        }

        SyncDiff {
            added,
            removed,
            state_changed,
        }
    }

    /// Move selection down in the filtered list.
    pub fn select_next(&mut self) {
        let len = self.filtered_sessions().len();
        if len > 0 {
            self.selected_index = (self.selected_index + 1) % len;
        }
        self.preview_scroll = 0;
    }

    /// Move selection up in the filtered list.
    pub fn select_prev(&mut self) {
        let len = self.filtered_sessions().len();
        if len > 0 {
            if self.selected_index == 0 {
                self.selected_index = len - 1;
            } else {
                self.selected_index -= 1;
            }
        }
        self.preview_scroll = 0;
    }

    /// Scroll the preview up by `amount`, clamped to `max_scroll`.
    pub fn scroll_preview_up(&mut self, amount: u16, max_scroll: u16) {
        self.preview_scroll = self.preview_scroll.saturating_add(amount).min(max_scroll);
    }

    /// Scroll the preview down by `amount`, clamped to 0.
    pub const fn scroll_preview_down(&mut self, amount: u16) {
        self.preview_scroll = self.preview_scroll.saturating_sub(amount);
    }

    /// Reset preview scroll to bottom (0).
    pub const fn reset_preview_scroll(&mut self) {
        self.preview_scroll = 0;
    }

    /// Get the currently selected session from the filtered list.
    pub fn selected_session(&self) -> Option<&ManagedSession> {
        let filtered = self.filtered_sessions();
        filtered.get(self.selected_index).copied()
    }

    /// Get a mutable reference to the currently selected session (PID-based lookup).
    pub fn selected_session_mut(&mut self) -> Option<&mut ManagedSession> {
        let pid = self.selected_session().map(|s| s.pid)?;
        self.sessions.iter_mut().find(|s| s.pid == pid)
    }

    /// Get the pane ID of the currently selected session.
    pub fn selected_pane_id(&self) -> Option<&str> {
        self.selected_session().map(|s| s.pane_id.as_str())
    }

    /// Toggle the mark on the currently selected session (PID-based).
    pub fn toggle_mark(&mut self) {
        let pid = self.selected_session().map(|s| s.pid);
        if let Some(pid) = pid
            && let Some(session) = self.sessions.iter_mut().find(|s| s.pid == pid)
        {
            session.marked = !session.marked;
        }
    }

    /// Return references to all marked sessions.
    pub fn marked_sessions(&self) -> Vec<&ManagedSession> {
        self.sessions.iter().filter(|s| s.marked).collect()
    }

    /// Return pane IDs of all marked sessions.
    pub fn marked_pane_ids(&self) -> Vec<String> {
        self.sessions
            .iter()
            .filter(|s| s.marked)
            .map(|s| s.pane_id.clone())
            .collect()
    }

    /// Refresh auto titles for sessions that have `session_id` and no manual title.
    pub fn refresh_auto_titles(&mut self) {
        for session in &mut self.sessions {
            if session.title.is_some() {
                continue;
            }
            if let (Some(session_id), cwd) = (&session.session_id, &session.cwd) {
                session.auto_title =
                    crate::auto_title::resolve_auto_title(cwd, session_id);
            }
        }
    }

    /// Refresh git branch names for all sessions by running `git branch --show-current`.
    pub fn refresh_git_branches(&mut self) {
        for session in &mut self.sessions {
            session.git_branch = resolve_git_branch(&session.cwd);
        }
    }

    /// Handle an incoming RPC message, updating session metadata.
    /// If the target session is not yet known, the message is buffered in `pending_rpc`.
    pub fn handle_rpc_message(&mut self, msg: &crate::rpc::RpcMessage) {
        let Some(pane_id) = msg.params.get("pane_id").and_then(|v| v.as_str()) else {
            return;
        };

        if let Some(session) = self.sessions.iter_mut().find(|s| s.pane_id == pane_id) {
            Self::apply_rpc_to_session(session, msg);
        } else {
            // Session not yet discovered — buffer for later
            const MAX_PENDING: usize = 20;
            if self.pending_rpc.len() < MAX_PENDING {
                self.pending_rpc.push(msg.clone());
            }
        }
    }

    /// Apply an RPC message to the matching session.
    fn apply_rpc_to_session(session: &mut ManagedSession, msg: &crate::rpc::RpcMessage) {
        match msg.method.as_str() {
            "session_start" => {
                session.session_id =
                    msg.params.get("session_id").and_then(|v| v.as_str()).map(String::from);
                session.model =
                    msg.params.get("model").and_then(|v| v.as_str()).map(String::from);
            }
            "status_update" => {
                // Extract session_id (fills in if not already set via session_start)
                if session.session_id.is_none() {
                    session.session_id = msg
                        .params
                        .get("session_id")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                }
                // Extract model.display_name from nested JSON
                if let Some(display_name) = msg
                    .params
                    .get("model")
                    .and_then(|m| m.get("display_name"))
                    .and_then(|v| v.as_str())
                {
                    session.model = Some(display_name.to_string());
                }
                // Extract context window used_percentage
                if let Some(pct) = msg
                    .params
                    .get("context_window")
                    .and_then(|c| c.get("used_percentage"))
                    .and_then(serde_json::Value::as_u64)
                {
                    // Context usage is 0–100, fits in u8.
                    #[allow(clippy::cast_possible_truncation)]
                    let pct = pct as u8;
                    session.context_percent = Some(pct);
                } else {
                    session.context_percent = Some(0);
                }
            }
            _ => {}
        }
    }

    /// Serialize all sessions and visibility state as a JSON value.
    pub fn serialize_sessions(&self) -> serde_json::Value {
        let sessions: Vec<serde_json::Value> = self
            .sessions
            .iter()
            .map(|s| {
                let state_name = match s.state {
                    ClaudeState::Idle => "Idle",
                    ClaudeState::Working => "Working",
                    ClaudeState::WaitingForApproval => "WaitingForApproval",
                };
                serde_json::json!({
                    "pane_id": s.pane_id,
                    "pid": s.pid,
                    "project_name": s.project_name,
                    "state": state_name,
                    "elapsed_secs": s.state_changed_at.elapsed().as_secs(),
                    "model": s.model,
                    "context_percent": s.context_percent,
                    "title": s.display_title(),
                    "session_id": s.session_id,
                    "git_branch": s.git_branch,
                })
            })
            .collect();
        serde_json::json!({
            "sessions": sessions,
            "visible": self.claudeye_visible,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tmux_claude_state::tmux::PaneInfo;

    fn make_session(pid: u32, pane_id: &str, project: &str, state: ClaudeState) -> ClaudeSession {
        ClaudeSession {
            pane: PaneInfo {
                id: pane_id.to_string(),
                pid,
                cwd: format!("/home/user/{project}"),
                project_name: project.to_string(),
            },
            state,
            state_changed_at: Instant::now(),
        }
    }

    fn make_monitor(sessions: Vec<ClaudeSession>) -> MonitorState {
        MonitorState {
            sessions,
            any_claude_focused: false,
        }
    }

    // --- New session detection ---

    #[test]
    fn test_sync_detects_new_sessions() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
            make_session(200, "%2", "project-b", ClaudeState::Working),
        ]);

        let diff = app.sync_with_monitor(&monitor);

        assert_eq!(diff.added, vec![100, 200]);
        assert!(diff.removed.is_empty());
        assert!(diff.state_changed.is_empty());
        assert_eq!(app.sessions.len(), 2);
    }

    // --- Session removal ---

    #[test]
    fn test_sync_detects_removed_sessions() {
        let mut app = AppState::new(None);
        let monitor1 = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
            make_session(200, "%2", "project-b", ClaudeState::Working),
        ]);
        app.sync_with_monitor(&monitor1);

        let monitor2 = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
        ]);
        let diff = app.sync_with_monitor(&monitor2);

        assert!(diff.added.is_empty());
        assert_eq!(diff.removed, vec![200]);
        assert_eq!(app.sessions.len(), 1);
    }

    // --- State change detection ---

    #[test]
    fn test_sync_detects_state_change() {
        let mut app = AppState::new(None);
        let monitor1 = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor1);

        let monitor2 = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Working),
        ]);
        let diff = app.sync_with_monitor(&monitor2);

        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());
        assert_eq!(diff.state_changed, vec![100]);
        assert_eq!(app.sessions[0].state, ClaudeState::Working);
    }

    // --- Sidebar exclusion ---

    #[test]
    fn test_sync_excludes_own_pid() {
        let mut app = AppState::new(Some(999));
        let monitor = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
            make_session(999, "%9", "sidebar", ClaudeState::Working),
        ]);

        let diff = app.sync_with_monitor(&monitor);

        assert_eq!(diff.added, vec![100]);
        assert_eq!(app.sessions.len(), 1);
        assert_eq!(app.sessions[0].pid, 100);
    }

    // --- Selection operations ---

    #[test]
    fn test_select_next_wraps_around() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "a", ClaudeState::Idle),
            make_session(200, "%2", "b", ClaudeState::Idle),
            make_session(300, "%3", "c", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);

        assert_eq!(app.selected_index, 0);
        app.select_next();
        assert_eq!(app.selected_index, 1);
        app.select_next();
        assert_eq!(app.selected_index, 2);
        app.select_next();
        assert_eq!(app.selected_index, 0); // wraps
    }

    #[test]
    fn test_select_prev_wraps_around() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "a", ClaudeState::Idle),
            make_session(200, "%2", "b", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);

        assert_eq!(app.selected_index, 0);
        app.select_prev();
        assert_eq!(app.selected_index, 1); // wraps to end
        app.select_prev();
        assert_eq!(app.selected_index, 0);
    }

    #[test]
    fn test_select_on_empty() {
        let mut app = AppState::new(None);
        app.select_next();
        assert_eq!(app.selected_index, 0);
        app.select_prev();
        assert_eq!(app.selected_index, 0);
    }

    #[test]
    fn test_selected_session() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "a", ClaudeState::Idle),
            make_session(200, "%2", "b", ClaudeState::Working),
        ]);
        app.sync_with_monitor(&monitor);

        assert_eq!(app.selected_session().unwrap().pid, 100);
        app.select_next();
        assert_eq!(app.selected_session().unwrap().pid, 200);
    }

    #[test]
    fn test_selected_pane_id() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);

        assert_eq!(app.selected_pane_id(), Some("%1"));
    }

    #[test]
    fn test_selected_pane_id_empty() {
        let app = AppState::new(None);
        assert_eq!(app.selected_pane_id(), None);
    }

    // --- selected_index fix on removal ---

    #[test]
    fn test_selected_index_fixed_on_removal() {
        let mut app = AppState::new(None);
        let monitor1 = make_monitor(vec![
            make_session(100, "%1", "a", ClaudeState::Idle),
            make_session(200, "%2", "b", ClaudeState::Idle),
            make_session(300, "%3", "c", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor1);
        app.selected_index = 2; // select last

        // Remove all but one
        let monitor2 = make_monitor(vec![
            make_session(100, "%1", "a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor2);

        assert_eq!(app.selected_index, 0);
    }

    // --- Pane ID update after join-pane ---

    // --- PreviewEntry ---

    #[test]
    fn test_preview_entry_with_title() {
        let entry = PreviewEntry {
            name: "crmux".to_string(),
            pane_id: "%1".to_string(),
            title: Some("development".to_string()),
            content: "hello".to_string(),
        };
        assert_eq!(entry.name, "crmux");
        assert_eq!(entry.title, Some("development".to_string()));
    }

    #[test]
    fn test_preview_entry_without_title() {
        let entry = PreviewEntry {
            name: "crmux".to_string(),
            pane_id: "%1".to_string(),
            title: None,
            content: "hello".to_string(),
        };
        assert_eq!(entry.title, None);
    }

    // --- Preview content ---

    #[test]
    fn test_preview_contents_default_empty() {
        let app = AppState::new(None);
        assert!(app.preview_contents.is_empty());
    }

    // --- Input mode ---

    #[test]
    fn test_initial_input_mode_is_normal() {
        let app = AppState::new(None);
        assert_eq!(app.input_mode, InputMode::Normal);
    }

    #[test]
    fn test_initial_input_buffer_is_empty() {
        let app = AppState::new(None);
        assert!(app.input_buffer.is_empty());
    }

    #[test]
    fn test_initial_show_help_is_false() {
        let app = AppState::new(None);
        assert!(!app.show_help);
    }

    // --- Mark operations ---

    #[test]
    fn test_new_session_is_not_marked() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);
        assert!(!app.sessions[0].marked);
    }

    #[test]
    fn test_toggle_mark_marks_selected() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "a", ClaudeState::Idle),
            make_session(200, "%2", "b", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);

        app.toggle_mark();
        assert!(app.sessions[0].marked);
        assert!(!app.sessions[1].marked);
    }

    #[test]
    fn test_toggle_mark_unmarks_marked() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);

        app.toggle_mark();
        assert!(app.sessions[0].marked);
        app.toggle_mark();
        assert!(!app.sessions[0].marked);
    }

    #[test]
    fn test_toggle_mark_on_empty_does_nothing() {
        let mut app = AppState::new(None);
        app.toggle_mark(); // should not panic
    }

    #[test]
    fn test_marked_sessions_returns_marked_only() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "a", ClaudeState::Idle),
            make_session(200, "%2", "b", ClaudeState::Idle),
            make_session(300, "%3", "c", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);

        app.selected_index = 0;
        app.toggle_mark();
        app.selected_index = 2;
        app.toggle_mark();

        let marked = app.marked_sessions();
        assert_eq!(marked.len(), 2);
        assert_eq!(marked[0].pid, 100);
        assert_eq!(marked[1].pid, 300);
    }

    #[test]
    fn test_marked_sessions_empty_when_none_marked() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);

        assert!(app.marked_sessions().is_empty());
    }

    #[test]
    fn test_marked_pane_ids_returns_marked_only() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "a", ClaudeState::Idle),
            make_session(200, "%2", "b", ClaudeState::Idle),
            make_session(300, "%3", "c", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);

        app.selected_index = 0;
        app.toggle_mark();
        app.selected_index = 2;
        app.toggle_mark();

        let ids = app.marked_pane_ids();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], "%1");
        assert_eq!(ids[1], "%3");
    }

    #[test]
    fn test_marked_pane_ids_empty_when_none_marked() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);

        assert!(app.marked_pane_ids().is_empty());
    }

    #[test]
    fn test_mark_preserved_on_sync() {
        let mut app = AppState::new(None);
        let monitor1 = make_monitor(vec![
            make_session(100, "%1", "a", ClaudeState::Idle),
            make_session(200, "%2", "b", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor1);
        app.toggle_mark(); // mark session 100

        // Re-sync with same sessions (state change)
        let monitor2 = make_monitor(vec![
            make_session(100, "%1", "a", ClaudeState::Working),
            make_session(200, "%2", "b", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor2);

        assert!(app.sessions[0].marked); // mark preserved
        assert!(!app.sessions[1].marked);
    }

    // --- Title field ---

    #[test]
    fn test_new_session_has_no_title() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);
        assert_eq!(app.sessions[0].title, None);
    }

    #[test]
    fn test_title_preserved_on_sync() {
        let mut app = AppState::new(None);
        let monitor1 = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor1);
        app.sessions[0].title = Some("refactoring auth".to_string());

        // Re-sync with state change
        let monitor2 = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Working),
        ]);
        app.sync_with_monitor(&monitor2);

        assert_eq!(app.sessions[0].title, Some("refactoring auth".to_string()));
    }

    // --- selected_session_mut ---

    #[test]
    fn test_selected_session_mut() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "a", ClaudeState::Idle),
            make_session(200, "%2", "b", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);

        app.selected_session_mut().unwrap().title = Some("testing".to_string());
        assert_eq!(app.sessions[0].title, Some("testing".to_string()));
        assert_eq!(app.sessions[1].title, None);
    }

    #[test]
    fn test_selected_session_mut_empty() {
        let mut app = AppState::new(None);
        assert!(app.selected_session_mut().is_none());
    }

    // --- Preview scroll tests ---

    #[test]
    fn test_initial_preview_scroll_is_zero() {
        let app = AppState::new(None);
        assert_eq!(app.preview_scroll, 0);
    }

    #[test]
    fn test_initial_preview_height_is_zero() {
        let app = AppState::new(None);
        assert_eq!(app.preview_height, 0);
    }

    #[test]
    fn test_initial_pending_g_is_false() {
        let app = AppState::new(None);
        assert!(!app.pending_g);
    }

    #[test]
    fn test_scroll_preview_up() {
        let mut app = AppState::new(None);
        app.scroll_preview_up(10, 90);
        assert_eq!(app.preview_scroll, 10);
    }

    #[test]
    fn test_scroll_preview_up_clamps_to_max() {
        let mut app = AppState::new(None);
        app.scroll_preview_up(100, 90);
        assert_eq!(app.preview_scroll, 90);
    }

    #[test]
    fn test_scroll_preview_up_saturating() {
        let mut app = AppState::new(None);
        app.preview_scroll = 80;
        app.scroll_preview_up(20, 90);
        assert_eq!(app.preview_scroll, 90);
    }

    #[test]
    fn test_scroll_preview_down() {
        let mut app = AppState::new(None);
        app.preview_scroll = 20;
        app.scroll_preview_down(10);
        assert_eq!(app.preview_scroll, 10);
    }

    #[test]
    fn test_scroll_preview_down_clamps_to_zero() {
        let mut app = AppState::new(None);
        app.preview_scroll = 5;
        app.scroll_preview_down(10);
        assert_eq!(app.preview_scroll, 0);
    }

    #[test]
    fn test_reset_preview_scroll() {
        let mut app = AppState::new(None);
        app.preview_scroll = 42;
        app.reset_preview_scroll();
        assert_eq!(app.preview_scroll, 0);
    }

    #[test]
    fn test_select_next_resets_scroll() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "a", ClaudeState::Idle),
            make_session(200, "%2", "b", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);
        app.preview_scroll = 20;
        app.select_next();
        assert_eq!(app.preview_scroll, 0);
    }

    #[test]
    fn test_select_prev_resets_scroll() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "a", ClaudeState::Idle),
            make_session(200, "%2", "b", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);
        app.selected_index = 1;
        app.preview_scroll = 15;
        app.select_prev();
        assert_eq!(app.preview_scroll, 0);
    }

    #[test]
    fn test_pane_id_updated_on_sync() {
        let mut app = AppState::new(None);
        let monitor1 = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor1);
        assert_eq!(app.sessions[0].pane_id, "%1");

        // Same PID, different pane_id (after join-pane)
        let monitor2 = make_monitor(vec![
            make_session(100, "%5", "project-a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor2);
        assert_eq!(app.sessions[0].pane_id, "%5");
    }

    // --- RPC message handling ---

    #[test]
    fn test_new_session_has_no_rpc_fields() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);
        assert_eq!(app.sessions[0].session_id, None);
        assert_eq!(app.sessions[0].model, None);
    }

    #[test]
    fn test_handle_rpc_session_start() {
        use crate::rpc::RpcMessage;

        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);

        app.handle_rpc_message(&RpcMessage {
            method: "session_start".to_string(),
            params: serde_json::json!({
                "pane_id": "%1",
                "session_id": "sess-abc",
                "model": "claude-sonnet-4-6",
            }),
        });

        assert_eq!(app.sessions[0].session_id, Some("sess-abc".to_string()));
        assert_eq!(app.sessions[0].model, Some("claude-sonnet-4-6".to_string()));
    }

    #[test]
    fn test_handle_rpc_session_start_unknown_pane() {
        use crate::rpc::RpcMessage;

        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);

        app.handle_rpc_message(&RpcMessage {
            method: "session_start".to_string(),
            params: serde_json::json!({
                "pane_id": "%99",
                "session_id": "sess-xyz",
            }),
        });

        // Should not crash, and existing session should be unchanged
        assert_eq!(app.sessions[0].session_id, None);
        // Unknown pane should be buffered for later
        assert_eq!(app.pending_rpc.len(), 1);
    }

    #[test]
    fn test_handle_rpc_missing_pane_id() {
        use crate::rpc::RpcMessage;

        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);

        app.handle_rpc_message(&RpcMessage {
            method: "session_start".to_string(),
            params: serde_json::json!({
                "session_id": "sess-abc",
            }),
        });

        // Without pane_id, nothing should be updated
        assert_eq!(app.sessions[0].session_id, None);
    }

    #[test]
    fn test_handle_rpc_unknown_method() {
        use crate::rpc::RpcMessage;

        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);

        app.handle_rpc_message(&RpcMessage {
            method: "unknown_method".to_string(),
            params: serde_json::json!({
                "pane_id": "%1",
            }),
        });

        // Unknown method should not change anything
        assert_eq!(app.sessions[0].session_id, None);
    }

    #[test]
    fn test_rpc_fields_preserved_on_sync() {
        use crate::rpc::RpcMessage;

        let mut app = AppState::new(None);
        let monitor1 = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor1);

        app.handle_rpc_message(&RpcMessage {
            method: "session_start".to_string(),
            params: serde_json::json!({
                "pane_id": "%1",
                "session_id": "sess-abc",
                "model": "opus",
            }),
        });

        // Re-sync with state change
        let monitor2 = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Working),
        ]);
        app.sync_with_monitor(&monitor2);

        assert_eq!(app.sessions[0].session_id, Some("sess-abc".to_string()));
        assert_eq!(app.sessions[0].model, Some("opus".to_string()));
    }

    // --- display_title ---

    #[test]
    fn test_display_title_manual_over_auto() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);
        app.sessions[0].title = Some("manual".to_string());
        app.sessions[0].auto_title = Some("auto".to_string());
        assert_eq!(app.sessions[0].display_title(), Some("manual"));
    }

    #[test]
    fn test_display_title_auto_when_no_manual() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);
        app.sessions[0].auto_title = Some("auto".to_string());
        assert_eq!(app.sessions[0].display_title(), Some("auto"));
    }

    #[test]
    fn test_display_title_none_when_both_empty() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);
        assert_eq!(app.sessions[0].display_title(), None);
    }

    // --- cwd and git_branch ---

    #[test]
    fn test_new_session_has_cwd() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);
        assert_eq!(app.sessions[0].cwd, "/home/user/project-a");
    }

    #[test]
    fn test_new_session_has_no_git_branch() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);
        assert_eq!(app.sessions[0].git_branch, None);
    }

    #[test]
    fn test_git_branch_preserved_on_sync() {
        let mut app = AppState::new(None);
        let monitor1 = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor1);
        app.sessions[0].git_branch = Some("feature-branch".to_string());

        let monitor2 = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Working),
        ]);
        app.sync_with_monitor(&monitor2);

        assert_eq!(app.sessions[0].git_branch, Some("feature-branch".to_string()));
    }

    #[test]
    fn test_refresh_git_branches_sets_branch_for_valid_repo() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "crmux", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);
        // Use this repo's own cwd for a known git repo
        app.sessions[0].cwd = env!("CARGO_MANIFEST_DIR").to_string();
        app.refresh_git_branches();
        assert!(app.sessions[0].git_branch.is_some());
    }

    #[test]
    fn test_refresh_git_branches_none_for_non_repo() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "tmp", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);
        app.sessions[0].cwd = "/tmp".to_string();
        app.refresh_git_branches();
        assert_eq!(app.sessions[0].git_branch, None);
    }

    // --- Pending RPC buffer ---

    #[test]
    fn test_rpc_before_session_is_buffered() {
        use crate::rpc::RpcMessage;

        let mut app = AppState::new(None);

        app.handle_rpc_message(&RpcMessage {
            method: "session_start".to_string(),
            params: serde_json::json!({
                "pane_id": "%1",
                "session_id": "sess-early",
                "model": "opus",
            }),
        });

        // Should be buffered in pending_rpc
        assert_eq!(app.pending_rpc.len(), 1);
        assert_eq!(app.pending_rpc[0].params["pane_id"], "%1");
    }

    #[test]
    fn test_pending_rpc_applied_after_sync() {
        use crate::rpc::RpcMessage;

        let mut app = AppState::new(None);

        app.handle_rpc_message(&RpcMessage {
            method: "session_start".to_string(),
            params: serde_json::json!({
                "pane_id": "%1",
                "session_id": "sess-early",
                "model": "opus",
            }),
        });

        // Now session appears via monitor
        let monitor = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);

        // Pending RPC should have been applied and removed
        assert!(app.pending_rpc.is_empty());
        assert_eq!(app.sessions[0].session_id, Some("sess-early".to_string()));
        assert_eq!(app.sessions[0].model, Some("opus".to_string()));
    }

    #[test]
    fn test_pending_rpc_unmatched_retained() {
        use crate::rpc::RpcMessage;

        let mut app = AppState::new(None);

        // Two RPCs for different panes
        for (pane, sid) in [("%1", "sess-1"), ("%2", "sess-2")] {
            app.handle_rpc_message(&RpcMessage {
                method: "session_start".to_string(),
                params: serde_json::json!({
                    "pane_id": pane,
                    "session_id": sid,
                }),
            });
        }
        assert_eq!(app.pending_rpc.len(), 2);

        // Only %1 session appears
        let monitor = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);

        // %1 applied and removed, %2 still pending
        assert_eq!(app.pending_rpc.len(), 1);
        assert_eq!(app.pending_rpc[0].params["pane_id"], "%2");
        assert_eq!(app.sessions[0].session_id, Some("sess-1".to_string()));
    }

    // --- status_update RPC ---

    #[test]
    fn test_handle_rpc_status_update_sets_model_display_name() {
        use crate::rpc::RpcMessage;

        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);

        app.handle_rpc_message(&RpcMessage {
            method: "status_update".to_string(),
            params: serde_json::json!({
                "pane_id": "%1",
                "model": { "display_name": "Opus" },
            }),
        });

        assert_eq!(app.sessions[0].model, Some("Opus".to_string()));
    }

    #[test]
    fn test_status_update_overwrites_session_start_model() {
        use crate::rpc::RpcMessage;

        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);

        // session_start sets model to ID format
        app.handle_rpc_message(&RpcMessage {
            method: "session_start".to_string(),
            params: serde_json::json!({
                "pane_id": "%1",
                "session_id": "sess-abc",
                "model": "claude-opus-4-6",
            }),
        });
        assert_eq!(app.sessions[0].model, Some("claude-opus-4-6".to_string()));

        // status_update overwrites with display_name
        app.handle_rpc_message(&RpcMessage {
            method: "status_update".to_string(),
            params: serde_json::json!({
                "pane_id": "%1",
                "model": { "display_name": "Opus" },
            }),
        });
        assert_eq!(app.sessions[0].model, Some("Opus".to_string()));
    }

    #[test]
    fn test_status_update_without_model_is_noop() {
        use crate::rpc::RpcMessage;

        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);
        app.sessions[0].model = Some("existing".to_string());

        app.handle_rpc_message(&RpcMessage {
            method: "status_update".to_string(),
            params: serde_json::json!({
                "pane_id": "%1",
            }),
        });

        // Model should remain unchanged
        assert_eq!(app.sessions[0].model, Some("existing".to_string()));
    }

    // --- context_percent ---

    #[test]
    fn test_status_update_sets_context_percent() {
        use crate::rpc::RpcMessage;

        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);

        app.handle_rpc_message(&RpcMessage {
            method: "status_update".to_string(),
            params: serde_json::json!({
                "pane_id": "%1",
                "model": { "display_name": "Opus" },
                "context_window": {
                    "used_percentage": 50,
                },
            }),
        });

        assert_eq!(app.sessions[0].context_percent, Some(50));
    }

    #[test]
    fn test_status_update_context_percent_zero_when_no_used_percentage() {
        use crate::rpc::RpcMessage;

        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);

        app.handle_rpc_message(&RpcMessage {
            method: "status_update".to_string(),
            params: serde_json::json!({
                "pane_id": "%1",
                "model": { "display_name": "Opus" },
                "context_window": {},
            }),
        });

        assert_eq!(app.sessions[0].context_percent, Some(0));
    }

    #[test]
    fn test_status_update_sets_session_id() {
        use crate::rpc::RpcMessage;
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Working),
        ]);
        app.sync_with_monitor(&monitor);
        assert_eq!(app.sessions[0].session_id, None);

        app.handle_rpc_message(&RpcMessage {
            method: "status_update".to_string(),
            params: serde_json::json!({
                "pane_id": "%1",
                "session_id": "sess-from-statusline",
                "model": { "display_name": "Opus" },
            }),
        });

        assert_eq!(app.sessions[0].session_id, Some("sess-from-statusline".to_string()));
    }

    #[test]
    fn test_status_update_does_not_overwrite_existing_session_id() {
        use crate::rpc::RpcMessage;
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Working),
        ]);
        app.sync_with_monitor(&monitor);

        // session_start sets session_id first
        app.handle_rpc_message(&RpcMessage {
            method: "session_start".to_string(),
            params: serde_json::json!({
                "pane_id": "%1",
                "session_id": "sess-original",
                "model": "opus",
            }),
        });
        assert_eq!(app.sessions[0].session_id, Some("sess-original".to_string()));

        // status_update should NOT overwrite existing session_id
        app.handle_rpc_message(&RpcMessage {
            method: "status_update".to_string(),
            params: serde_json::json!({
                "pane_id": "%1",
                "session_id": "sess-new",
                "model": { "display_name": "Opus" },
            }),
        });
        assert_eq!(app.sessions[0].session_id, Some("sess-original".to_string()));
    }

    #[test]
    fn test_new_session_has_no_context_percent() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "project-a", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);
        assert_eq!(app.sessions[0].context_percent, None);
    }

    // --- claudeye_visible ---

    #[test]
    fn test_claudeye_visible_default_false() {
        let app = AppState::new(None);
        assert!(!app.claudeye_visible);
    }

    #[test]
    fn test_claudeye_visible_toggle() {
        let mut app = AppState::new(None);
        app.claudeye_visible = false;
        assert!(!app.claudeye_visible);
        app.claudeye_visible = true;
        assert!(app.claudeye_visible);
    }

    // --- serialize_sessions ---

    #[test]
    fn test_serialize_sessions_empty() {
        let app = AppState::new(None);
        let result = app.serialize_sessions();
        assert_eq!(result["sessions"], serde_json::json!([]));
        assert_eq!(result["visible"], false);
    }

    #[test]
    fn test_serialize_sessions_one() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "crmux", ClaudeState::Working),
        ]);
        app.sync_with_monitor(&monitor);
        app.sessions[0].model = Some("Opus".to_string());
        app.sessions[0].context_percent = Some(23);
        app.sessions[0].title = Some("implementing feature X".to_string());
        app.sessions[0].session_id = Some("abc-123".to_string());
        app.sessions[0].git_branch = Some("main".to_string());

        let result = app.serialize_sessions();
        let sessions = result["sessions"].as_array().unwrap();
        assert_eq!(sessions.len(), 1);

        let s = &sessions[0];
        assert_eq!(s["pane_id"], "%1");
        assert_eq!(s["pid"], 100);
        assert_eq!(s["project_name"], "crmux");
        assert_eq!(s["state"], "Working");
        assert_eq!(s["model"], "Opus");
        assert_eq!(s["context_percent"], 23);
        assert_eq!(s["title"], "implementing feature X");
        assert_eq!(s["session_id"], "abc-123");
        assert_eq!(s["git_branch"], "main");
        // elapsed_secs should be a non-negative number
        assert!(s["elapsed_secs"].as_u64().is_some());
        assert_eq!(result["visible"], false);
    }

    #[test]
    fn test_serialize_sessions_waiting_for_approval_state() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "crmux", ClaudeState::WaitingForApproval),
        ]);
        app.sync_with_monitor(&monitor);

        let result = app.serialize_sessions();
        let sessions = result["sessions"].as_array().unwrap();
        assert_eq!(sessions[0]["state"], "WaitingForApproval");
    }

    #[test]
    fn test_serialize_sessions_visible_true() {
        let mut app = AppState::new(None);
        app.claudeye_visible = true;
        let result = app.serialize_sessions();
        assert_eq!(result["visible"], true);
    }

    // --- TabState tests ---

    #[test]
    fn test_tab_state_initial() {
        let ts = TabState::new();
        assert!(ts.tabs.is_empty());
        assert_eq!(ts.selected_tab, 0);
        assert_eq!(*ts.current_tab(), Tab::All);
    }

    #[test]
    fn test_tab_state_rebuild_from_sessions() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "crmux", ClaudeState::Idle),
            make_session(200, "%2", "aegis", ClaudeState::Working),
        ]);
        app.sync_with_monitor(&monitor);

        assert_eq!(app.tab_state.tabs, vec![
            Tab::All,
            Tab::Project("aegis".to_string()),
            Tab::Project("crmux".to_string()),
        ]);
    }

    #[test]
    fn test_tab_state_alphabetical_sort() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "zebra", ClaudeState::Idle),
            make_session(200, "%2", "alpha", ClaudeState::Idle),
            make_session(300, "%3", "middle", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);

        assert_eq!(app.tab_state.tabs, vec![
            Tab::All,
            Tab::Project("alpha".to_string()),
            Tab::Project("middle".to_string()),
            Tab::Project("zebra".to_string()),
        ]);
    }

    #[test]
    fn test_tab_state_dedup() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "crmux", ClaudeState::Idle),
            make_session(200, "%2", "crmux", ClaudeState::Working),
        ]);
        app.sync_with_monitor(&monitor);

        assert_eq!(app.tab_state.tabs, vec![
            Tab::All,
            Tab::Project("crmux".to_string()),
        ]);
    }

    #[test]
    fn test_tab_state_selection_maintained_on_rebuild() {
        let mut app = AppState::new(None);
        let monitor1 = make_monitor(vec![
            make_session(100, "%1", "aegis", ClaudeState::Idle),
            make_session(200, "%2", "crmux", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor1);
        app.tab_state.selected_tab = 2; // crmux tab

        // Re-sync with an additional project
        let monitor2 = make_monitor(vec![
            make_session(100, "%1", "aegis", ClaudeState::Idle),
            make_session(200, "%2", "crmux", ClaudeState::Working),
            make_session(300, "%3", "zeta", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor2);

        // crmux tab should still be selected
        assert_eq!(*app.tab_state.current_tab(), Tab::Project("crmux".to_string()));
    }

    #[test]
    fn test_tab_state_fallback_on_project_disappear() {
        let mut app = AppState::new(None);
        let monitor1 = make_monitor(vec![
            make_session(100, "%1", "aegis", ClaudeState::Idle),
            make_session(200, "%2", "crmux", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor1);
        app.tab_state.selected_tab = 2; // crmux tab

        // crmux session disappears
        let monitor2 = make_monitor(vec![
            make_session(100, "%1", "aegis", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor2);

        // Should fallback to All
        assert_eq!(*app.tab_state.current_tab(), Tab::All);
    }

    #[test]
    fn test_tab_state_next_prev_wrap() {
        let mut ts = TabState::new();
        ts.tabs = vec![Tab::All, Tab::Project("a".into()), Tab::Project("b".into())];
        ts.selected_tab = 0;

        ts.select_next_tab();
        assert_eq!(ts.selected_tab, 1);
        ts.select_next_tab();
        assert_eq!(ts.selected_tab, 2);
        ts.select_next_tab();
        assert_eq!(ts.selected_tab, 0); // wraps

        ts.select_prev_tab();
        assert_eq!(ts.selected_tab, 2); // wraps back
        ts.select_prev_tab();
        assert_eq!(ts.selected_tab, 1);
    }

    // --- Filtering tests ---

    #[test]
    fn test_filtered_sessions_all_tab() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "aegis", ClaudeState::Idle),
            make_session(200, "%2", "crmux", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor);

        let filtered = app.filtered_sessions();
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filtered_sessions_project_tab() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "aegis", ClaudeState::Idle),
            make_session(200, "%2", "crmux", ClaudeState::Idle),
            make_session(300, "%3", "crmux", ClaudeState::Working),
        ]);
        app.sync_with_monitor(&monitor);
        app.tab_state.selected_tab = 2; // crmux tab

        let filtered = app.filtered_sessions();
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|s| s.project_name == "crmux"));
    }

    #[test]
    fn test_selected_index_adjusted_on_filter() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "aegis", ClaudeState::Idle),
            make_session(200, "%2", "crmux", ClaudeState::Idle),
            make_session(300, "%3", "crmux", ClaudeState::Working),
        ]);
        app.sync_with_monitor(&monitor);
        app.selected_index = 2; // third item in All tab

        // Switch to aegis tab (only 1 session)
        app.tab_state.selected_tab = 1; // aegis
        let filtered = app.filtered_sessions();
        // selected_index should not exceed filtered len
        let adjusted = app.selected_index.min(filtered.len().saturating_sub(1));
        assert!(adjusted < filtered.len());
    }

    #[test]
    fn test_select_next_on_filtered() {
        let mut app = AppState::new(None);
        let monitor = make_monitor(vec![
            make_session(100, "%1", "aegis", ClaudeState::Idle),
            make_session(200, "%2", "crmux", ClaudeState::Idle),
            make_session(300, "%3", "crmux", ClaudeState::Working),
        ]);
        app.sync_with_monitor(&monitor);

        // Switch to crmux tab
        app.tab_state.selected_tab = 2;
        app.selected_index = 0;

        app.select_next();
        assert_eq!(app.selected_index, 1);
        app.select_next();
        assert_eq!(app.selected_index, 0); // wraps at 2, not 3
    }

    #[test]
    fn test_sync_rebuilds_tabs() {
        let mut app = AppState::new(None);
        let monitor1 = make_monitor(vec![
            make_session(100, "%1", "aegis", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor1);
        assert_eq!(app.tab_state.tabs.len(), 2); // All + aegis

        let monitor2 = make_monitor(vec![
            make_session(100, "%1", "aegis", ClaudeState::Idle),
            make_session(200, "%2", "crmux", ClaudeState::Idle),
        ]);
        app.sync_with_monitor(&monitor2);
        assert_eq!(app.tab_state.tabs.len(), 3); // All + aegis + crmux
    }
}
