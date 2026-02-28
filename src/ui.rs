use ansi_to_tui::IntoText as _;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
};
use std::time::Instant;
use tmux_claude_state::claude_state::ClaudeState;

use crate::state::{InputMode, ManagedSession};

/// Format elapsed time since an `Instant` into a human-readable string.
pub fn format_elapsed(since: Instant) -> String {
    let secs = since.elapsed().as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3600)
    }
}

/// Map a `ClaudeState` to a display color.
pub const fn state_color(state: &ClaudeState) -> Color {
    match state {
        ClaudeState::Working => Color::Blue,
        ClaudeState::WaitingForApproval => Color::LightRed,
        ClaudeState::Idle => Color::White,
    }
}

/// Map a `ClaudeState` to a short label.
pub const fn state_label(state: &ClaudeState) -> &'static str {
    match state {
        ClaudeState::Working => "Running",
        ClaudeState::WaitingForApproval => "Approval",
        ClaudeState::Idle => "Idle",
    }
}

/// Draw the full TUI: session list (left) + preview pane (right).
pub fn draw(
    f: &mut ratatui::Frame,
    sessions: &[ManagedSession],
    selected_index: usize,
    preview_contents: &[(String, String)],
    input_mode: InputMode,
    input_buffer: &str,
) {
    let size = f.area();

    // Top-level horizontal split: left (sidebar) | right (preview)
    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(30), Constraint::Min(0)])
        .split(size);

    // Left panel: title + sessions list + instructions
    draw_left_panel(f, sessions, h_chunks[0], selected_index);

    // Right panel: preview (optionally with input bar at bottom)
    draw_right_panel(f, preview_contents, input_mode, input_buffer, h_chunks[1]);
}

/// Draw the right panel: preview(s) + optional input bar.
fn draw_right_panel(
    f: &mut ratatui::Frame,
    preview_contents: &[(String, String)],
    input_mode: InputMode,
    input_buffer: &str,
    area: ratatui::layout::Rect,
) {
    if input_mode == InputMode::Input {
        // Count lines in input buffer to size the input bar (min 3, max 8)
        let line_count = input_buffer.chars().filter(|&c| c == '\n').count() + 1;
        #[allow(clippy::cast_possible_truncation)]
        let input_height = (line_count as u16 + 2).clamp(3, 8); // +2 for borders

        let v_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(input_height)])
            .split(area);

        // Preview(s)
        draw_preview_panes(f, preview_contents, v_chunks[0]);

        // Input bar
        let input_text = Text::raw(input_buffer);
        let input_bar = Paragraph::new(input_text).block(
            Block::default()
                .title("Input (C-Enter/C-d: send | Esc: cancel)")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow)),
        );
        f.render_widget(input_bar, v_chunks[1]);

        // Place cursor at end of input
        let inner = Block::default().borders(Borders::ALL).inner(v_chunks[1]);
        let last_line = input_buffer.lines().last().unwrap_or("");
        #[allow(clippy::cast_possible_truncation)]
        let cursor_x = inner.x + last_line.len() as u16;
        #[allow(clippy::cast_possible_truncation)]
        let cursor_y = inner.y + input_buffer.chars().filter(|&c| c == '\n').count() as u16;
        f.set_cursor_position((cursor_x, cursor_y));
    } else {
        // Normal mode: just preview(s)
        draw_preview_panes(f, preview_contents, area);
    }
}

/// Draw one or more preview panes, splitting the area vertically.
fn draw_preview_panes(
    f: &mut ratatui::Frame,
    preview_contents: &[(String, String)],
    area: ratatui::layout::Rect,
) {
    if preview_contents.is_empty() {
        let preview = Paragraph::new("No session selected").block(
            Block::default()
                .title("Preview")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        );
        f.render_widget(preview, area);
        return;
    }

    if preview_contents.len() == 1 {
        let (name, content) = &preview_contents[0];
        let preview_text = content
            .as_str()
            .into_text()
            .unwrap_or_else(|_| Text::raw(content.as_str()));
        let preview = Paragraph::new(preview_text).block(
            Block::default()
                .title(format!("Preview: {name}"))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        );
        f.render_widget(preview, area);
        return;
    }

    // Multiple previews: split vertically
    #[allow(clippy::cast_possible_truncation)]
    let count = preview_contents.len() as u32;
    let constraints: Vec<Constraint> = preview_contents
        .iter()
        .map(|_| Constraint::Ratio(1, count))
        .collect();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    for (i, (name, content)) in preview_contents.iter().enumerate() {
        let preview_text = content
            .as_str()
            .into_text()
            .unwrap_or_else(|_| Text::raw(content.as_str()));
        let preview = Paragraph::new(preview_text).block(
            Block::default()
                .title(format!("Preview: {name}"))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        );
        f.render_widget(preview, chunks[i]);
    }
}

/// Draw the left panel (session list + instructions).
fn draw_left_panel(
    f: &mut ratatui::Frame,
    sessions: &[ManagedSession],
    area: ratatui::layout::Rect,
    selected_index: usize,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(area);

    // Sessions list
    draw_sessions_list(f, sessions, chunks[0], selected_index);

    // Instructions
    let instructions = Paragraph::new("j/k:Nav Space:Mark Enter:Focus i:Input q:Quit")
        .block(Block::default().borders(Borders::ALL))
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(instructions, chunks[1]);
}

/// Draw the list of Claude sessions.
fn draw_sessions_list(
    f: &mut ratatui::Frame,
    sessions: &[ManagedSession],
    area: ratatui::layout::Rect,
    selected_index: usize,
) {
    let block = Block::default()
        .title(format!("Sessions ({})", sessions.len()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::White));

    let inner_area = block.inner(area);
    f.render_widget(block, area);

    if sessions.is_empty() {
        let empty_msg = Paragraph::new("No Claude sessions detected")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(empty_msg, inner_area);
        return;
    }

    let constraints: Vec<Constraint> = sessions.iter().map(|_| Constraint::Length(3)).collect();

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner_area);

    for (idx, session) in sessions.iter().enumerate() {
        if idx >= layout.len() {
            break;
        }
        let is_selected = idx == selected_index;
        let color = state_color(&session.state);
        let elapsed = format_elapsed(session.state_changed_at);
        let label = state_label(&session.state);

        let text_color = if is_selected { Color::Yellow } else { color };
        let mark_indicator = if session.marked { "* " } else { "  " };
        let spans = vec![
            Span::styled(mark_indicator, Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled(
                &session.project_name,
                Style::default()
                    .fg(text_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(label, Style::default().fg(text_color)),
            Span::raw(" "),
            Span::styled(elapsed, Style::default().fg(text_color)),
        ];

        let border_style = if is_selected {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(color)
        };

        let bg_style = if is_selected {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };

        let paragraph = Paragraph::new(Line::from(spans))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(border_style),
            )
            .style(bg_style);

        f.render_widget(paragraph, layout[idx]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_elapsed_seconds() {
        let now = Instant::now();
        let result = format_elapsed(now);
        assert_eq!(result, "0s");
    }

    #[test]
    fn test_format_elapsed_minutes() {
        let since = Instant::now() - std::time::Duration::from_secs(120);
        assert_eq!(format_elapsed(since), "2m");
    }

    #[test]
    fn test_format_elapsed_hours() {
        let since = Instant::now() - std::time::Duration::from_secs(7200);
        assert_eq!(format_elapsed(since), "2h");
    }

    #[test]
    fn test_format_elapsed_boundary_59s() {
        let since = Instant::now() - std::time::Duration::from_secs(59);
        assert_eq!(format_elapsed(since), "59s");
    }

    #[test]
    fn test_format_elapsed_boundary_60s() {
        let since = Instant::now() - std::time::Duration::from_secs(60);
        assert_eq!(format_elapsed(since), "1m");
    }

    #[test]
    fn test_format_elapsed_boundary_3599s() {
        let since = Instant::now() - std::time::Duration::from_secs(3599);
        assert_eq!(format_elapsed(since), "59m");
    }

    #[test]
    fn test_format_elapsed_boundary_3600s() {
        let since = Instant::now() - std::time::Duration::from_secs(3600);
        assert_eq!(format_elapsed(since), "1h");
    }

    #[test]
    fn test_state_color() {
        assert_eq!(state_color(&ClaudeState::Working), Color::Blue);
        assert_eq!(
            state_color(&ClaudeState::WaitingForApproval),
            Color::LightRed
        );
        assert_eq!(state_color(&ClaudeState::Idle), Color::White);
    }

    #[test]
    fn test_state_label() {
        assert_eq!(state_label(&ClaudeState::Working), "Running");
        assert_eq!(state_label(&ClaudeState::WaitingForApproval), "Approval");
        assert_eq!(state_label(&ClaudeState::Idle), "Idle");
    }

    #[test]
    fn test_ansi_to_text_plain() {
        let text = "hello world".into_text().unwrap();
        assert_eq!(text.lines.len(), 1);
    }

    #[test]
    fn test_ansi_to_text_with_colors() {
        let ansi = "\x1b[31mred\x1b[0m normal";
        let text = ansi.into_text().unwrap();
        assert!(!text.lines.is_empty());
    }
}
