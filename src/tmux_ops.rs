use std::io;
use std::process::Command;

/// Build arguments for `tmux join-pane`.
/// `-d` keeps focus on the current pane.
pub fn join_pane_args(source_pane: &str, target_window: &str) -> Vec<String> {
    vec![
        "join-pane".into(),
        "-d".into(),
        "-s".into(),
        source_pane.into(),
        "-t".into(),
        target_window.into(),
    ]
}

/// Build arguments for `tmux resize-pane`.
pub fn resize_pane_args(pane_id: &str, width: Option<u16>, height: Option<u16>) -> Vec<String> {
    let mut args = vec!["resize-pane".to_string(), "-t".to_string(), pane_id.into()];
    if let Some(w) = width {
        args.push("-x".into());
        args.push(w.to_string());
    }
    if let Some(h) = height {
        args.push("-y".into());
        args.push(h.to_string());
    }
    args
}

/// Build arguments for `tmux swap-pane`.
#[allow(dead_code)]
pub fn swap_pane_args(src: &str, dst: &str) -> Vec<String> {
    vec![
        "swap-pane".into(),
        "-d".into(),
        "-s".into(),
        src.into(),
        "-t".into(),
        dst.into(),
    ]
}

/// Build arguments for `tmux break-pane`.
/// `-d` prevents the broken-out pane from becoming active.
#[allow(dead_code)]
pub fn break_pane_args(pane_id: &str) -> Vec<String> {
    vec![
        "break-pane".into(),
        "-d".into(),
        "-s".into(),
        pane_id.into(),
    ]
}

/// Execute a tmux command with the given arguments.
pub fn run_tmux(args: &[String]) -> io::Result<String> {
    let output = Command::new("tmux")
        .args(args)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(io::Error::other(stderr.to_string()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Parse the output of `tmux display-message -p '#{window_width} #{window_height}'`.
pub fn parse_window_size(output: &str) -> Option<(u16, u16)> {
    let trimmed = output.trim();
    let mut parts = trimmed.split_whitespace();
    let width: u16 = parts.next()?.parse().ok()?;
    let height: u16 = parts.next()?.parse().ok()?;
    Some((width, height))
}

/// Pane entry from `tmux list-panes`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct TmuxPane {
    pub pane_id: String,
    pub pid: u32,
    pub width: u16,
    pub height: u16,
}

/// Parse a single line of `tmux list-panes -F '#{pane_id} #{pane_pid} #{pane_width} #{pane_height}'`.
#[allow(dead_code)]
pub fn parse_pane_line(line: &str) -> Option<TmuxPane> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 4 {
        return None;
    }
    Some(TmuxPane {
        pane_id: parts[0].to_string(),
        pid: parts[1].parse().ok()?,
        width: parts[2].parse().ok()?,
        height: parts[3].parse().ok()?,
    })
}

/// Parse multiple lines of `tmux list-panes` output.
#[allow(dead_code)]
pub fn parse_pane_list(output: &str) -> Vec<TmuxPane> {
    output.lines().filter_map(parse_pane_line).collect()
}

/// Get the window size of the current tmux window.
pub fn get_window_size() -> io::Result<(u16, u16)> {
    let output = run_tmux(&[
        "display-message".into(),
        "-p".into(),
        "#{window_width} #{window_height}".into(),
    ])?;
    parse_window_size(&output).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "Failed to parse window size")
    })
}

/// List all panes in the "claude" window.
#[allow(dead_code)]
pub fn list_panes_in_claude_window() -> io::Result<Vec<TmuxPane>> {
    let output = run_tmux(&[
        "list-panes".into(),
        "-t".into(),
        "claude".into(),
        "-F".into(),
        "#{pane_id} #{pane_pid} #{pane_width} #{pane_height}".into(),
    ])?;
    Ok(parse_pane_list(&output))
}

/// Launch the sidebar window: `tmux new-window -S -n claude 'crmux --internal-sidebar'`.
pub fn launch_sidebar_window() -> io::Result<()> {
    let exe = std::env::current_exe()?;
    let cmd = format!("{} --internal-sidebar", exe.display());
    run_tmux(&[
        "new-window".into(),
        "-S".into(),
        "-n".into(),
        "claude".into(),
        cmd,
    ])?;
    Ok(())
}

/// Get the pane ID of the current pane (our sidebar pane).
pub fn get_own_pane_id() -> io::Result<String> {
    let output = run_tmux(&[
        "display-message".into(),
        "-p".into(),
        "#{pane_id}".into(),
    ])?;
    Ok(output.trim().to_string())
}

/// Move a pane into the "claude" window.
pub fn join_pane_to_claude_window(source_pane: &str) -> io::Result<()> {
    let args = join_pane_args(source_pane, "claude");
    run_tmux(&args)?;
    Ok(())
}

/// Swap two panes.
#[allow(dead_code)]
pub fn swap_panes(src: &str, dst: &str) -> io::Result<()> {
    let args = swap_pane_args(src, dst);
    run_tmux(&args)?;
    Ok(())
}

/// Resize a pane.
pub fn resize_pane(pane_id: &str, width: Option<u16>, height: Option<u16>) -> io::Result<()> {
    let args = resize_pane_args(pane_id, width, height);
    run_tmux(&args)?;
    Ok(())
}

/// Break a pane out of the current window into a new window.
#[allow(dead_code)]
pub fn break_pane(pane_id: &str) -> io::Result<()> {
    let args = break_pane_args(pane_id);
    run_tmux(&args)?;
    Ok(())
}

/// Select (focus) a specific pane.
pub fn select_pane(pane_id: &str) -> io::Result<()> {
    run_tmux(&[
        "select-pane".into(),
        "-t".into(),
        pane_id.into(),
    ])?;
    Ok(())
}

/// Apply a custom layout to the "claude" window.
#[allow(dead_code)]
pub fn select_layout(layout_str: &str) -> io::Result<()> {
    run_tmux(&[
        "select-layout".into(),
        "-t".into(),
        "claude".into(),
        layout_str.into(),
    ])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- join_pane_args ---

    #[test]
    fn test_join_pane_args() {
        let args = join_pane_args("%5", "claude");
        assert_eq!(
            args,
            vec!["join-pane", "-d", "-s", "%5", "-t", "claude"]
        );
    }

    // --- resize_pane_args ---

    #[test]
    fn test_resize_pane_args_width_only() {
        let args = resize_pane_args("%1", Some(80), None);
        assert_eq!(args, vec!["resize-pane", "-t", "%1", "-x", "80"]);
    }

    #[test]
    fn test_resize_pane_args_height_only() {
        let args = resize_pane_args("%1", None, Some(24));
        assert_eq!(args, vec!["resize-pane", "-t", "%1", "-y", "24"]);
    }

    #[test]
    fn test_resize_pane_args_both() {
        let args = resize_pane_args("%2", Some(120), Some(40));
        assert_eq!(
            args,
            vec!["resize-pane", "-t", "%2", "-x", "120", "-y", "40"]
        );
    }

    #[test]
    fn test_resize_pane_args_none() {
        let args = resize_pane_args("%3", None, None);
        assert_eq!(args, vec!["resize-pane", "-t", "%3"]);
    }

    // --- swap_pane_args ---

    #[test]
    fn test_swap_pane_args() {
        let args = swap_pane_args("%1", "%2");
        assert_eq!(args, vec!["swap-pane", "-d", "-s", "%1", "-t", "%2"]);
    }

    // --- break_pane_args ---

    #[test]
    fn test_break_pane_args() {
        let args = break_pane_args("%5");
        assert_eq!(args, vec!["break-pane", "-d", "-s", "%5"]);
    }

    // --- parse_window_size ---

    #[test]
    fn test_parse_window_size_valid() {
        assert_eq!(parse_window_size("200 50\n"), Some((200, 50)));
    }

    #[test]
    fn test_parse_window_size_with_whitespace() {
        assert_eq!(parse_window_size("  120  40  \n"), Some((120, 40)));
    }

    #[test]
    fn test_parse_window_size_invalid() {
        assert_eq!(parse_window_size("abc def"), None);
    }

    #[test]
    fn test_parse_window_size_empty() {
        assert_eq!(parse_window_size(""), None);
    }

    #[test]
    fn test_parse_window_size_single_value() {
        assert_eq!(parse_window_size("200"), None);
    }

    // --- parse_pane_line ---

    #[test]
    fn test_parse_pane_line_valid() {
        let pane = parse_pane_line("%1 12345 80 24").unwrap();
        assert_eq!(pane.pane_id, "%1");
        assert_eq!(pane.pid, 12345);
        assert_eq!(pane.width, 80);
        assert_eq!(pane.height, 24);
    }

    #[test]
    fn test_parse_pane_line_invalid_pid() {
        assert!(parse_pane_line("%1 abc 80 24").is_none());
    }

    #[test]
    fn test_parse_pane_line_too_few_fields() {
        assert!(parse_pane_line("%1 123").is_none());
    }

    #[test]
    fn test_parse_pane_line_empty() {
        assert!(parse_pane_line("").is_none());
    }

    // --- parse_pane_list ---

    #[test]
    fn test_parse_pane_list_multiple() {
        let output = "%1 100 80 24\n%2 200 120 40\n%3 300 60 20\n";
        let panes = parse_pane_list(output);
        assert_eq!(panes.len(), 3);
        assert_eq!(panes[0].pane_id, "%1");
        assert_eq!(panes[1].pane_id, "%2");
        assert_eq!(panes[2].pane_id, "%3");
    }

    #[test]
    fn test_parse_pane_list_with_invalid_lines() {
        let output = "%1 100 80 24\nbad line\n%2 200 120 40\n";
        let panes = parse_pane_list(output);
        assert_eq!(panes.len(), 2);
    }

    #[test]
    fn test_parse_pane_list_empty() {
        let panes = parse_pane_list("");
        assert!(panes.is_empty());
    }
}
