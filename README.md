# crmux

A TUI viewer for Claude Code sessions in tmux.

Inspired by [cmux](https://www.cmux.dev). crmux brings a similar multi-session management experience using tmux and a terminal UI.

- Monitor multiple Claude Code sessions from a single sidebar and preview their output
- Mark sessions to preview multiple panes simultaneously
- Switch to any session's tmux pane instantly
- Send prompts directly via tmux `send-keys` in input mode without leaving crmux
- Pulse animation to highlight sessions that need attention (approval idle, stale idle)

## Demo
![gif][1]

## Prerequisites

- tmux
- [tmux-claude-state](https://github.com/maedana/claudeye/tree/main/crates/tmux-claude-state)
- Rust (latest stable version) -- only required when building from source
- (Optional) [claudeye](https://github.com/maedana/claudeye) for overlay integration (`o` key to toggle)

## Installation

### Quick install

```sh
curl -sSL https://raw.githubusercontent.com/maedana/crmux/main/install.sh | sh
```

### From crates.io

```sh
cargo install crmux --locked
```

After installation, make sure `~/.cargo/bin` is in your PATH, then you can run:

```sh
crmux
```

### Build from source

```sh
git clone https://github.com/maedana/crmux.git
cd crmux
cargo build --release
```

## Usage

Run inside a tmux session:

```sh
crmux
```

### Keybindings

Keybindings are shown in the app footer. Run `crmux -h` or press `?` in the app to see all available keybindings.

## Claude Code Hook Setup (Optional)

By configuring Claude Code hooks, crmux can display additional session metadata such as the model name and context window usage.

Add the following to `~/.claude/settings.json`:

```json
{
  "hooks": {
    "SessionStart": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "crmux notify session-start"
          }
        ]
      }
    ]
  }
}
```

To also display model display name (e.g. "Opus") and context window usage percentage, configure a `statusLine` command. The simplest setup uses `crmux notify status-update` directly:

```json
{
  "statusLine": {
    "type": "command",
    "command": "crmux notify status-update",
    "padding": 0
  }
}
```

> **Note:** The `statusLine` command's stdout is used as Claude Code's own status line display. Since `crmux notify` produces no output, Claude Code's status line will be blank with this setup. If you want both crmux sidebar info **and** Claude Code's status line, use a wrapper script instead.

<details>
<summary>Wrapper script example (ccstatus)</summary>

Create `~/.local/bin/ccstatus`:

```bash
#!/bin/bash
input=$(cat)

# Notify crmux of status update (non-blocking)
echo "$input" | crmux notify status-update &

MODEL=$(echo "$input" | jq -r '.model.display_name')
CONTEXT_SIZE=$(echo "$input" | jq -r '.context_window.context_window_size')
USAGE=$(echo "$input" | jq '.context_window.current_usage')

if [ "$USAGE" != "null" ]; then
    CURRENT_TOKENS=$(echo "$USAGE" | jq '.input_tokens + .cache_creation_input_tokens + .cache_read_input_tokens')
    PERCENT_USED=$((CURRENT_TOKENS * 100 / CONTEXT_SIZE))
    echo "[$MODEL] Context: ${PERCENT_USED}%"
else
    echo "[$MODEL] Context: 0%"
fi
```

```json
{
  "statusLine": {
    "type": "command",
    "command": "ccstatus",
    "padding": 0
  }
}
```

</details>

## Roadmap

- Display worktree info per session
- Programmatic control from external tools (e.g. tmux-style subcommands, MCP server)

## Limitations

Input mode uses tmux `send-keys` to forward keystrokes, which has inherent limitations:

- **Modifier keys**: Some modifier key combinations (e.g. Shift+Enter, Ctrl+Enter) cannot be accurately reproduced via tmux `send-keys`
- **Terminal dependency**: Terminals without Kitty keyboard protocol support (VTE-based terminals such as XFCE Terminal, GNOME Terminal) cannot distinguish some modified key events from their unmodified counterparts
- **Paste on macOS**: `Cmd+v` is intercepted by the terminal emulator and never reaches crmux as a key event. Text paste works via bracketed paste, but image paste (used by Claude Code) cannot be forwarded. On Linux, `Ctrl+v` is forwarded as a key event, so image paste works through the target Claude Code session.

## License

MIT

[1]: https://raw.githubusercontent.com/maedana/crmux/main/demos/demo.gif
