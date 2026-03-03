use ansi_to_tui::IntoText as _;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use std::time::{Instant, SystemTime};
use tmux_claude_state::claude_state::ClaudeState;

use crate::state::{InputMode, ManagedSession, PreviewEntry};

const STALE_MIN_SECS: u64 = 5;
const STALE_MAX_SECS: u64 = 15;

const SELECTED_ICON: &str = "> ";
const TITLE_COLOR: Color = Color::Rgb(180, 180, 180);

/// Determine if a session should pulse based on its state and elapsed time.
pub fn should_pulse(state: &ClaudeState, elapsed_secs: u64) -> bool {
    matches!(state, ClaudeState::WaitingForApproval)
        || (matches!(state, ClaudeState::Idle)
            && (STALE_MIN_SECS..=STALE_MAX_SECS).contains(&elapsed_secs))
}

/// Convert a ratatui `Color` to an RGB tuple.
const fn color_to_rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Blue => (0, 0, 255),
        Color::LightRed => (255, 100, 100),
        Color::White => (255, 255, 255),
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (200, 200, 200),
    }
}

/// Return the current sine-wave factor (0.0–1.0) for pulse animations.
fn pulse_factor() -> f64 {
    let t = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    f64::midpoint((t * 16.0).sin(), 1.0) // 0.0 ~ 1.0
}

/// Calculate a pulsing background color (dimmed version of the base color).
fn pulse_bg_color(base: Color) -> Color {
    let intensity = pulse_factor() * 0.25; // 0.0 ~ 0.25
    let (r, g, b) = color_to_rgb(base);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Color::Rgb(
        (f64::from(r) * intensity) as u8,
        (f64::from(g) * intensity) as u8,
        (f64::from(b) * intensity) as u8,
    )
}

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

/// Truncate a title string to `max_chars` characters, appending `…` if truncated.
fn truncate_title(s: &str, max_chars: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars - 1).collect();
        format!("{truncated}…")
    }
}

/// Format a preview pane title: "{name} - {title}" if title is present, else "{name} - {pane_id}".
fn preview_title(name: &str, pane_id: &str, title: &Option<String>) -> String {
    match title {
        Some(t) if !t.is_empty() => format!("{name} - {t}"),
        _ => format!("{name} - {pane_id}"),
    }
}

/// Draw the full TUI: session list (left) + preview pane (right).
#[allow(clippy::too_many_arguments)]
pub fn draw(
    f: &mut ratatui::Frame,
    sessions: &[ManagedSession],
    selected_index: usize,
    preview_contents: &[PreviewEntry],
    input_mode: InputMode,
    input_buffer: &str,
    show_help: bool,
    preview_scroll: u16,
) {
    let size = f.area();

    // Top-level vertical split: main content | footer
    let v_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(size);

    // Main content: horizontal split left (sidebar) | right (preview)
    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(30), Constraint::Min(0)])
        .split(v_chunks[0]);

    // Left panel: sessions list
    draw_left_panel(f, sessions, h_chunks[0], selected_index, input_mode, input_buffer);

    // Right panel: preview (optionally with input bar at bottom)
    let selected_pane_id = sessions
        .get(selected_index)
        .map(|s| s.pane_id.as_str());
    draw_right_panel(f, preview_contents, input_mode, input_buffer, h_chunks[1], selected_pane_id, preview_scroll);

    // Footer: app name + mode indicator + keybindings (full width)
    let instructions = Paragraph::new(Line::from(footer_spans(input_mode)))
        .block(Block::default().borders(Borders::ALL))
        .style(Style::default().fg(Color::Gray));
    f.render_widget(instructions, v_chunks[1]);

    // Help popup overlay
    if show_help {
        draw_help_popup(f, size);
    }
}

/// Build the footer spans: app name, optional vim-style mode indicator, and keybindings.
fn footer_spans(input_mode: InputMode) -> Vec<Span<'static>> {
    let mut spans = vec![Span::styled(
        concat!("crmux v", env!("CARGO_PKG_VERSION")),
        Style::default().fg(Color::White),
    )];
    match input_mode {
        InputMode::Normal => {
            spans.push(Span::raw(" | j/k:Nav C-u/C-d:Scroll gg:Top G:Bottom Space:Multi-preview s:Switch i:Input(selected) I:Input(marked) e:Title ?:Help q:Quit"));
        }
        InputMode::Input => {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                "-- INSERT --",
                Style::default().add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" | Keys sent to selected pane via send-keys. Esc:Back"));
        }
        InputMode::Title => {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                "-- TITLE --",
                Style::default().add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" | Edit session title. Esc:Save&Exit"));
        }
        InputMode::Broadcast => {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                "-- BROADCAST --",
                Style::default().add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" | Keys sent to marked panes. Esc:Back"));
        }
    }
    spans
}

/// Draw the right panel: preview pane(s).
fn draw_right_panel(
    f: &mut ratatui::Frame,
    preview_contents: &[PreviewEntry],
    _input_mode: InputMode,
    _input_buffer: &str,
    area: ratatui::layout::Rect,
    selected_pane_id: Option<&str>,
    preview_scroll: u16,
) {
    draw_preview_panes(f, preview_contents, area, selected_pane_id, preview_scroll);
}

/// Draw one or more preview panes, splitting the area vertically.
fn draw_preview_panes(
    f: &mut ratatui::Frame,
    preview_contents: &[PreviewEntry],
    area: ratatui::layout::Rect,
    selected_pane_id: Option<&str>,
    preview_scroll: u16,
) {
    if preview_contents.is_empty() {
        let preview = Paragraph::new("No session selected").block(
            Block::default()
                .title("Preview")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Gray)),
        );
        f.render_widget(preview, area);
        return;
    }

    if preview_contents.len() == 1 {
        let entry = &preview_contents[0];
        let preview_text = entry
            .content
            .as_str()
            .into_text()
            .unwrap_or_else(|_| Text::raw(entry.content.as_str()));
        #[allow(clippy::cast_possible_truncation)]
        let text_lines = preview_text.lines.len() as u16;
        let inner_height = area.height.saturating_sub(2);
        let max_scroll = text_lines.saturating_sub(inner_height);
        let effective_scroll = preview_scroll.min(max_scroll);
        let scroll_y = max_scroll.saturating_sub(effective_scroll);
        let mut title = preview_title(&entry.name, &entry.pane_id, &entry.title);
        if preview_scroll > 0 {
            title.push_str(" [SCROLL]");
        }
        let preview = Paragraph::new(preview_text)
            .block(
                Block::default()
                    .title(format!("{SELECTED_ICON}{title}"))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Gray)),
            )
            .scroll((scroll_y, 0));
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

    for (i, entry) in preview_contents.iter().enumerate() {
        let preview_text = entry
            .content
            .as_str()
            .into_text()
            .unwrap_or_else(|_| Text::raw(entry.content.as_str()));
        let text_lines = preview_text.lines.len() as u16;
        let inner_height = chunks[i].height.saturating_sub(2);
        let is_focused = selected_pane_id == Some(entry.pane_id.as_str());
        let scroll_y = if is_focused {
            let max_scroll = text_lines.saturating_sub(inner_height);
            let effective_scroll = preview_scroll.min(max_scroll);
            max_scroll.saturating_sub(effective_scroll)
        } else {
            text_lines.saturating_sub(inner_height)
        };
        let title_prefix = if is_focused { SELECTED_ICON } else { "" };
        let title = preview_title(&entry.name, &entry.pane_id, &entry.title);
        let preview = Paragraph::new(preview_text)
            .block(
                Block::default()
                    .title(format!("{title_prefix}{title}"))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Gray)),
            )
            .scroll((scroll_y, 0));
        f.render_widget(preview, chunks[i]);
    }
}

const HELP_TEXT: &str = "\
Keybindings (Normal mode):
  j / ↓          Move cursor down in session list
  k / ↑          Move cursor up in session list
  Ctrl+u         Scroll preview up (half page)
  Ctrl+d         Scroll preview down (half page)
  gg             Scroll preview to top
  G              Scroll preview to bottom
  Space          Mark for preview multiple tmux panes
  s              Switch to tmux pane
  i              Enter input mode (send keys to the selected session)
  I              Enter input mode (send keys to all marked sessions)
  e              Enter title mode (set a title for the session)
  ?              Show this help
  q              Quit crmux

Keybindings (Input mode):
  Esc            Return to normal mode
  Any other key  Forwarded to the tmux pane via send-keys

Keybindings (Broadcast mode):
  Esc            Return to normal mode
  Any other key  Forwarded to all marked panes via send-keys

Keybindings (Title mode):
  Esc            Save and return to normal mode
  Backspace      Delete the last character";

/// Draw a centered help popup overlay.
fn draw_help_popup(f: &mut ratatui::Frame, area: Rect) {
    let popup_width = area.width.min(60);
    let popup_height = area.height.min(20);
    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" Help (? to close) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let paragraph = Paragraph::new(HELP_TEXT)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, popup_area);
}

/// Draw the left panel (session list).
fn draw_left_panel(
    f: &mut ratatui::Frame,
    sessions: &[ManagedSession],
    area: ratatui::layout::Rect,
    selected_index: usize,
    input_mode: InputMode,
    input_buffer: &str,
) {
    draw_sessions_list(f, sessions, area, selected_index, input_mode, input_buffer);
}

/// Draw the list of Claude sessions.
fn draw_sessions_list(
    f: &mut ratatui::Frame,
    sessions: &[ManagedSession],
    area: ratatui::layout::Rect,
    selected_index: usize,
    input_mode: InputMode,
    input_buffer: &str,
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
        let elapsed_secs = session.state_changed_at.elapsed().as_secs();
        let is_pulsing = should_pulse(&session.state, elapsed_secs);
        let color = state_color(&session.state);
        let elapsed = format_elapsed(session.state_changed_at);
        let label = state_label(&session.state);

        let text_color = color;
        let mark_indicator = if session.marked { "* " } else { "  " };

        let border_style = Style::default().fg(color);

        let bg_style = if is_pulsing {
            Style::default().bg(pulse_bg_color(state_color(&session.state)))
        } else {
            Style::default()
        };

        let title_prefix = if is_selected { SELECTED_ICON } else { "" };

        let mut title_spans = vec![
            Span::styled(
                format!("{title_prefix}{}", &session.project_name),
                Style::default().fg(text_color).add_modifier(Modifier::BOLD),
            ),
        ];
        if let Some(ref branch) = session.git_branch {
            title_spans.push(Span::styled(
                format!(" ({branch})"),
                Style::default().fg(Color::DarkGray),
            ));
        }
        let project_title = Line::from(title_spans);

        let mark_span = Span::styled(mark_indicator, Style::default().fg(Color::Green).add_modifier(Modifier::BOLD));

        let mut status_spans = Vec::new();
        if let Some(ref model) = session.model {
            status_spans.push(Span::styled(
                model.as_str(),
                Style::default().fg(Color::DarkGray),
            ));
            status_spans.push(Span::raw(" "));
        }
        status_spans.push(Span::styled(label, Style::default().fg(text_color)));
        status_spans.push(Span::raw(" "));
        status_spans.push(Span::styled(elapsed, Style::default().fg(text_color)));
        let status_line = Line::from(status_spans);
        let is_editing_title = is_selected && input_mode == InputMode::Title;
        let combined_line = if is_editing_title {
            let max_width = layout[idx].width.saturating_sub(4) as usize; // borders + mark
            let (display_text, text_color) = if input_buffer.is_empty() {
                ("Type a title".to_string(), Color::DarkGray)
            } else {
                (truncate_title(input_buffer, max_width), Color::Yellow)
            };
            Line::from(vec![
                mark_span,
                Span::styled(display_text, Style::default().fg(text_color)),
            ])
        } else if let Some(display) = session.display_title() {
            let max_width = layout[idx].width.saturating_sub(4) as usize; // borders + mark
            let truncated = truncate_title(display, max_width);
            let color = if session.title.is_some() {
                TITLE_COLOR
            } else {
                Color::DarkGray
            };
            Line::from(vec![
                mark_span,
                Span::styled(truncated, Style::default().fg(color)),
            ])
        } else {
            Line::from(vec![
                mark_span,
                Span::styled("Press e to edit title", Style::default().fg(TITLE_COLOR)),
            ])
        };
        let paragraph = Paragraph::new(vec![combined_line]);

        let card_border_style = if is_editing_title {
            Style::default().fg(Color::Yellow)
        } else {
            border_style
        };

        let block = Block::default()
            .title(project_title)
            .title_bottom(status_line.right_aligned())
            .borders(Borders::ALL)
            .border_style(card_border_style);

        let paragraph = paragraph.block(block).style(bg_style);

        f.render_widget(paragraph, layout[idx]);

        // Set cursor position for inline title editing
        if is_editing_title {
            let inner = Block::default().borders(Borders::ALL).inner(layout[idx]);
            // Cursor after mark indicator (2 chars) + buffer text
            #[allow(clippy::cast_possible_truncation)]
            let cursor_x = inner.x + 2 + input_buffer.chars().count().min((inner.width.saturating_sub(2)) as usize) as u16;
            let cursor_y = inner.y;
            f.set_cursor_position((cursor_x, cursor_y));
        }
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

    // --- should_pulse tests ---

    #[test]
    fn test_should_pulse_approval() {
        // WaitingForApproval should always pulse regardless of elapsed time
        assert!(should_pulse(&ClaudeState::WaitingForApproval, 0));
        assert!(should_pulse(&ClaudeState::WaitingForApproval, 100));
    }

    #[test]
    fn test_should_pulse_idle_stale() {
        // Idle within STALE_MIN_SECS..=STALE_MAX_SECS should pulse
        assert!(should_pulse(&ClaudeState::Idle, 5));
        assert!(should_pulse(&ClaudeState::Idle, 10));
        assert!(should_pulse(&ClaudeState::Idle, 15));
    }

    #[test]
    fn test_should_pulse_idle_not_stale() {
        // Idle outside the stale range should NOT pulse
        assert!(!should_pulse(&ClaudeState::Idle, 4));
        assert!(!should_pulse(&ClaudeState::Idle, 16));
    }

    #[test]
    fn test_should_pulse_working() {
        // Working should never pulse
        assert!(!should_pulse(&ClaudeState::Working, 0));
        assert!(!should_pulse(&ClaudeState::Working, 10));
    }

    // --- color_to_rgb tests ---

    #[test]
    fn test_color_to_rgb() {
        assert_eq!(color_to_rgb(Color::Blue), (0, 0, 255));
        assert_eq!(color_to_rgb(Color::LightRed), (255, 100, 100));
        assert_eq!(color_to_rgb(Color::White), (255, 255, 255));
    }

    // --- pulse_bg_color tests ---

    #[test]
    fn test_pulse_bg_color_returns_rgb() {
        let result = pulse_bg_color(Color::LightRed);
        assert!(matches!(result, Color::Rgb(_, _, _)));
    }

    #[test]
    fn test_selected_icon_is_not_empty() {
        assert!(!SELECTED_ICON.is_empty());
    }

    // --- preview_title tests ---

    #[test]
    fn test_preview_title_with_title() {
        assert_eq!(preview_title("crmux", "%1", &Some("development".to_string())), "crmux - development");
    }

    #[test]
    fn test_preview_title_without_title() {
        assert_eq!(preview_title("crmux", "%1", &None), "crmux - %1");
    }

    #[test]
    fn test_preview_title_with_empty_title() {
        assert_eq!(preview_title("crmux", "%1", &Some("".to_string())), "crmux - %1");
    }

    // --- footer_spans tests ---

    #[test]
    fn test_footer_normal_mode_starts_with_app_name() {
        let spans = footer_spans(InputMode::Normal);
        assert!(spans[0].content.starts_with("crmux v"));
    }

    #[test]
    fn test_footer_normal_mode_has_no_mode_indicator() {
        let spans = footer_spans(InputMode::Normal);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(!text.contains("--"));
    }

    #[test]
    fn test_footer_input_mode_starts_with_app_name() {
        let spans = footer_spans(InputMode::Input);
        assert!(spans[0].content.starts_with("crmux v"));
    }

    #[test]
    fn test_footer_input_mode_has_insert_indicator() {
        let spans = footer_spans(InputMode::Input);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("-- INSERT --"));
    }

    #[test]
    fn test_footer_title_mode_starts_with_app_name() {
        let spans = footer_spans(InputMode::Title);
        assert!(spans[0].content.starts_with("crmux v"));
    }

    #[test]
    fn test_footer_title_mode_has_title_edit_indicator() {
        let spans = footer_spans(InputMode::Title);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("-- TITLE --"));
    }

    // --- truncate_title tests ---

    #[test]
    fn test_truncate_short_title() {
        assert_eq!(truncate_title("short", 20), "short");
    }

    #[test]
    fn test_truncate_exact_length() {
        assert_eq!(truncate_title("abcde", 5), "abcde");
    }

    #[test]
    fn test_truncate_long_title() {
        assert_eq!(truncate_title("abcdef", 5), "abcd…");
    }

    #[test]
    fn test_truncate_multibyte() {
        // UTF-8 safe: "あいう" is 3 chars
        assert_eq!(truncate_title("あいうえお", 4), "あいう…");
    }

    #[test]
    fn test_truncate_empty() {
        assert_eq!(truncate_title("", 10), "");
    }

    #[test]
    fn test_footer_normal_mode_contains_help_key() {
        let spans = footer_spans(InputMode::Normal);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("?:Help"), "Normal mode footer should contain '?:Help', got: {text}");
    }

    #[test]
    fn test_footer_broadcast_mode_has_broadcast_indicator() {
        let spans = footer_spans(InputMode::Broadcast);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("-- BROADCAST --"), "Broadcast mode footer should contain '-- BROADCAST --', got: {text}");
    }

    #[test]
    fn test_footer_normal_mode_contains_broadcast_key() {
        let spans = footer_spans(InputMode::Normal);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("I:Input(marked)"), "Normal mode footer should contain 'I:Input(marked)', got: {text}");
    }

    #[test]
    fn test_footer_normal_mode_contains_scroll_keys() {
        let spans = footer_spans(InputMode::Normal);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("C-u/C-d:Scroll"), "Normal mode footer should contain 'C-u/C-d:Scroll', got: {text}");
    }

    #[test]
    fn test_footer_normal_mode_contains_g_bottom() {
        let spans = footer_spans(InputMode::Normal);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("G:Bottom"), "Normal mode footer should contain 'G:Bottom', got: {text}");
    }

    #[test]
    fn test_pulse_bg_color_within_intensity_range() {
        // bg intensity ranges from 0.0 to 0.25
        let base = Color::White; // (255, 255, 255)
        let result = pulse_bg_color(base);
        if let Color::Rgb(r, g, b) = result {
            assert!(r <= 63, "r={r} exceeds max intensity");
            assert!(g <= 63, "g={g} exceeds max intensity");
            assert!(b <= 63, "b={b} exceeds max intensity");
        } else {
            panic!("Expected Color::Rgb");
        }
    }
}
