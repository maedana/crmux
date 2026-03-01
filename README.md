# crmux

A TUI viewer for Claude Code sessions in tmux.

Inspired by [cmux](https://www.cmux.dev). crmux brings a similar multi-session management experience using tmux and a terminal UI.

Monitor multiple Claude Code sessions from a single sidebar, preview their output, and quickly switch between them.

## Requirements

- tmux
- Rust (cargo)
- [tmux-claude-state](https://github.com/maedana/claudeye/tree/main/crates/tmux-claude-state)

## Installation

```sh
cargo install --path .
```

## Usage

Run inside a tmux session:

```sh
crmux
```

### Layout

```
┌─ Sessions ──────┬─ Preview ───────────────────────┐
│  project-a      │                                  │
│  Running 2m     │  (output of selected session)    │
│                 │                                  │
│  project-b      │                                  │
│  Idle 5m        │                                  │
│                 │                                  │
├─────────────────┴──────────────────────────────────┤
│ crmux | j/k:Nav Space:Mark Enter:Switch Pane ...   │
└────────────────────────────────────────────────────┘
```

### Keybindings

#### Normal mode

| Key | Action |
|-----|--------|
| `j` / `↓` | Move cursor down |
| `k` / `↑` | Move cursor up |
| `Space` | Mark/unmark session for multi-preview |
| `Enter` | Switch tmux focus to the selected session's pane |
| `i` | Enter input mode |
| `q` / `Esc` | Quit |

#### Input mode

| Key | Action |
|-----|--------|
| `Ctrl+Enter` | Send typed text to the session and return to normal mode |
| `Ctrl+d` | Same as `Ctrl+Enter` (universal fallback) |
| `Enter` | Insert a newline |
| `Esc` | Cancel input and return to normal mode |
| `Backspace` | Delete the last character |

## MCP Integration

crmux includes a built-in MCP server that allows Claude Code sessions to report their current plan titles. Plan titles are displayed on the bottom border of each session card in the sidebar.

### Setup

```sh
claude mcp add --transport stdio --scope user crmux -- crmux mcp
```

### Architecture

```
┌──────────────────┐     stdio (JSON-RPC)     ┌─────────────────┐
│  Claude Code #1  │ ──── MCP tools/call ────▶ │  crmux mcp      │ ─┐
│  Claude Code #2  │ ──── MCP tools/call ────▶ │  crmux mcp      │ ─┤ Unix socket
└──────────────────┘                           └─────────────────┘  │
                                                                    ▼
                                               ┌─────────────────┐
                                               │  crmux (TUI)    │ ← /tmp/crmux.sock
                                               └─────────────────┘
```

Each Claude Code session launches `crmux mcp` as a stdio MCP server. When a plan is created or updated, the `notify_plan` tool sends the plan title via Unix socket to the running crmux TUI.

## License

MIT
