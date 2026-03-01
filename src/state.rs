use std::time::Instant;
use tmux_claude_state::claude_state::ClaudeState;
use tmux_claude_state::monitor::{ClaudeSession, MonitorState};

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

/// Input mode for the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    /// Normal navigation mode.
    Normal,
    /// Text input mode for sending keys to a session.
    Input,
    /// Title input mode for setting session title label.
    Title,
}

/// Application state for the sidebar.
pub struct AppState {
    /// All managed sessions, ordered by discovery time.
    pub sessions: Vec<ManagedSession>,
    /// Index of the currently selected session.
    pub selected_index: usize,
    /// PID of our own sidebar pane's process (excluded from aggregation).
    pub own_pid: Option<u32>,
    /// Preview contents: Vec of (`project_name`, `pane_content`) pairs.
    pub preview_contents: Vec<(String, String)>,
    /// Current input mode.
    pub input_mode: InputMode,
    /// Buffer for text input in Input mode.
    pub input_buffer: String,
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
                });
            }
        }

        // Sort by project_name for stable directory-based ordering
        let selected_pid = self.selected_session().map(|s| s.pid);
        self.sessions.sort_by(|a, b| a.project_name.cmp(&b.project_name));
        if let Some(pid) = selected_pid
            && let Some(pos) = self.sessions.iter().position(|s| s.pid == pid)
        {
            self.selected_index = pos;
        }

        // Fix selected_index if out of bounds
        if !self.sessions.is_empty() && self.selected_index >= self.sessions.len() {
            self.selected_index = self.sessions.len() - 1;
        }

        SyncDiff {
            added,
            removed,
            state_changed,
        }
    }

    /// Move selection down.
    pub const fn select_next(&mut self) {
        if !self.sessions.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.sessions.len();
        }
    }

    /// Move selection up.
    pub const fn select_prev(&mut self) {
        if !self.sessions.is_empty() {
            if self.selected_index == 0 {
                self.selected_index = self.sessions.len() - 1;
            } else {
                self.selected_index -= 1;
            }
        }
    }

    /// Get the currently selected session, if any.
    pub fn selected_session(&self) -> Option<&ManagedSession> {
        self.sessions.get(self.selected_index)
    }

    /// Get a mutable reference to the currently selected session.
    pub fn selected_session_mut(&mut self) -> Option<&mut ManagedSession> {
        self.sessions.get_mut(self.selected_index)
    }

    /// Get the pane ID of the currently selected session.
    pub fn selected_pane_id(&self) -> Option<&str> {
        self.selected_session().map(|s| s.pane_id.as_str())
    }

    /// Toggle the mark on the currently selected session.
    pub fn toggle_mark(&mut self) {
        if let Some(session) = self.sessions.get_mut(self.selected_index) {
            session.marked = !session.marked;
        }
    }

    /// Return references to all marked sessions.
    pub fn marked_sessions(&self) -> Vec<&ManagedSession> {
        self.sessions.iter().filter(|s| s.marked).collect()
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
}
