# crmux

A TUI viewer for Claude Code sessions in tmux.

Inspired by [cmux](https://www.cmux.dev). crmux brings a similar multi-session management experience using tmux and a terminal UI.

Monitor multiple Claude Code sessions from a single sidebar, preview their output, and quickly switch between them.

## Demo
![gif][1]

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
| `Space` | Mark for preview multiple tmux panes |
| `s` | Switch tmux focus to the selected session's pane |
| `i` | Enter input mode |
| `e` | Edit session title |
| `?` | Show help |
| `q` | Quit |

#### Input mode

Keystrokes are forwarded directly to the selected session's tmux pane via `send-keys` (passthrough).

| Key | Action |
|-----|--------|
| `Esc` | Return to normal mode |
| Any other key | Forwarded to the tmux pane |

## Limitations

Input mode uses tmux `send-keys` to forward keystrokes, which has inherent limitations:

- **Modifier keys**: Some modifier key combinations (e.g. Shift+Enter, Ctrl+Enter) cannot be accurately reproduced via tmux `send-keys`
- **Terminal dependency**: Terminals without Kitty keyboard protocol support (VTE-based terminals such as XFCE Terminal, GNOME Terminal) cannot distinguish some modified key events from their unmodified counterparts
- **IME**: Input via IME (e.g. CJK input methods) is not supported

## License

MIT

[1]: https://raw.githubusercontent.com/maedana/crmux/main/demos/demo.gif
