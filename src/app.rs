use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tmux_claude_state::monitor::MonitorState;

use crate::event_handler::{self, Action};
use crate::state::{AppState, PreviewEntry};
use crate::ui;

/// Run the TUI application.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let own_pid = std::process::id();

    // Start monitor polling
    let monitor_state = Arc::new(Mutex::new(MonitorState::default()));
    tmux_claude_state::monitor::start_polling(Arc::clone(&monitor_state));

    // Terminal setup
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

    // Enable Kitty keyboard protocol for Ctrl+Enter detection
    let keyboard_enhancement = supports_keyboard_enhancement().unwrap_or(false);
    if keyboard_enhancement {
        execute!(
            stdout,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
    }

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_event_loop(&mut terminal, &monitor_state, own_pid);

    // Terminal cleanup
    if keyboard_enhancement {
        execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags)?;
    }
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result?;
    Ok(())
}

fn run_event_loop<B: ratatui::backend::Backend<Error = io::Error>>(
    terminal: &mut Terminal<B>,
    monitor_state: &Arc<Mutex<MonitorState>>,
    own_pid: u32,
) -> io::Result<()> {
    let mut app_state = AppState::new(Some(own_pid));

    loop {
        // Sync with monitor state
        if let Ok(monitor) = monitor_state.lock() {
            app_state.sync_with_monitor(&monitor);
        }

        // Update preview contents
        let marked = app_state.marked_sessions();
        if marked.is_empty() {
            // No marked sessions: show the selected session
            if let Some(session) = app_state.selected_session() {
                let content = tmux_claude_state::tmux::capture_pane_with_ansi(&session.pane_id);
                app_state.preview_contents = vec![PreviewEntry {
                    name: session.project_name.clone(),
                    pane_id: session.pane_id.clone(),
                    title: session.title.clone(),
                    content,
                }];
            } else {
                app_state.preview_contents.clear();
            }
        } else {
            // Show all marked sessions
            let entries: Vec<PreviewEntry> = marked
                .iter()
                .map(|s| {
                    let content = tmux_claude_state::tmux::capture_pane_with_ansi(&s.pane_id);
                    PreviewEntry {
                        name: s.project_name.clone(),
                        pane_id: s.pane_id.clone(),
                        title: s.title.clone(),
                        content,
                    }
                })
                .collect();
            app_state.preview_contents = entries;
        }

        // Draw TUI
        terminal.draw(|f| {
            ui::draw(
                f,
                &app_state.sessions,
                app_state.selected_index,
                &app_state.preview_contents,
                app_state.input_mode,
                &app_state.input_buffer,
                app_state.show_help,
            );
        })?;

        // Wait for at least one event or timeout for periodic refresh
        if event::poll(Duration::from_millis(50))? {
            let ev = event::read()?;
            match event_handler::handle_key_event(&ev, &mut app_state) {
                Action::Quit => return Ok(()),
                Action::Continue => {}
            }
            // Drain all remaining pending events before next capture/draw cycle
            while event::poll(Duration::ZERO)? {
                let ev = event::read()?;
                match event_handler::handle_key_event(&ev, &mut app_state) {
                    Action::Quit => return Ok(()),
                    Action::Continue => {}
                }
            }
        }
    }
}
