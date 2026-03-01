use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

use crate::socket::{self, PlanNotification, SOCKET_PATH};

/// Get the grandparent PID by reading `/proc/<ppid>/stat`.
/// Process chain: crmux mcp → claude (parent) → shell (grandparent = tmux pane PID)
fn get_grandparent_pid() -> Option<u32> {
    let ppid = std::os::unix::process::parent_id();
    let stat = std::fs::read_to_string(format!("/proc/{ppid}/stat")).ok()?;
    // /proc/<pid>/stat format: pid (comm) state ppid ...
    // Find closing paren to skip comm field (which may contain spaces)
    let after_comm = stat.rfind(')')? + 2;
    let fields: Vec<&str> = stat[after_comm..].split_whitespace().collect();
    // fields[0] = state, fields[1] = ppid
    fields.get(1)?.parse().ok()
}

/// Handle a single JSON-RPC request and return a response (or None for notifications).
fn handle_request(request: &Value) -> Option<Value> {
    let method = request.get("method")?.as_str()?;
    let id = request.get("id");

    match method {
        "initialize" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2025-06-18",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "crmux",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        })),
        "notifications/initialized" => None, // no response for notifications
        "tools/list" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": [
                    {
                        "name": "notify_plan",
                        "description": "You MUST call this tool whenever you create or update a plan. Reports the plan title and file path to the crmux session manager.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "title": {
                                    "type": "string",
                                    "description": "The plan title (first heading line)"
                                },
                                "path": {
                                    "type": "string",
                                    "description": "Absolute path to the plan file"
                                }
                            },
                            "required": ["title", "path"]
                        }
                    }
                ]
            }
        })),
        "tools/call" => {
            let params = request.get("params")?;
            let arguments = params.get("arguments")?;
            let title = arguments.get("title")?.as_str()?.to_string();
            let path = arguments.get("path")?.as_str()?.to_string();

            // crmux mcp → claude (parent) → zsh/shell (grandparent = tmux pane PID)
            // We need the grandparent PID to match the tmux pane PID tracked by crmux TUI.
            let pid = get_grandparent_pid().unwrap_or_else(std::os::unix::process::parent_id);

            let notification = PlanNotification { pid, title, path };

            // Best-effort: don't fail the MCP tool if socket isn't available
            let _ = socket::send_notification(SOCKET_PATH, &notification);

            Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": [
                        {
                            "type": "text",
                            "text": "Plan notification sent to crmux."
                        }
                    ]
                }
            }))
        }
        _ => {
            // Unknown method: return error
            Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32601,
                    "message": format!("Method not found: {method}")
                }
            }))
        }
    }
}

/// Run the MCP server, reading JSON-RPC from stdin and writing to stdout.
pub fn run_mcp_server() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };

        if line.trim().is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if let Some(response) = handle_request(&request) {
            let _ = writeln!(stdout, "{response}");
            let _ = stdout.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialize_response() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "0.1" }
            }
        });

        let response = handle_request(&request).unwrap();

        assert_eq!(response["id"], 1);
        assert_eq!(response["result"]["protocolVersion"], "2025-06-18");
        assert!(response["result"]["capabilities"]["tools"].is_object());
        assert_eq!(response["result"]["serverInfo"]["name"], "crmux");
    }

    #[test]
    fn test_notifications_initialized_returns_none() {
        let request = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });

        let response = handle_request(&request);
        assert!(response.is_none());
    }

    #[test]
    fn test_tools_list_returns_notify_plan() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        });

        let response = handle_request(&request).unwrap();

        assert_eq!(response["id"], 2);
        let tools = response["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "notify_plan");
        assert!(tools[0]["inputSchema"]["properties"]["title"].is_object());
        assert!(tools[0]["inputSchema"]["properties"]["path"].is_object());
        let required = tools[0]["inputSchema"]["required"].as_array().unwrap();
        assert!(required.contains(&json!("title")));
        assert!(required.contains(&json!("path")));
    }

    #[test]
    fn test_tools_call_sends_notification() {
        // Start a test listener
        let socket_path = format!("/tmp/crmux-mcp-test-{}.sock", std::process::id());
        let plan_map = socket::start_listener(&socket_path);

        std::thread::sleep(std::time::Duration::from_millis(50));

        // Temporarily override socket path by calling send_notification directly
        let request = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "notify_plan",
                "arguments": {
                    "title": "Test Plan Title",
                    "path": "/tmp/test-plan.md"
                }
            }
        });

        // Since handle_request uses SOCKET_PATH constant, we test the notification sending separately
        // and just verify the response format
        let response = handle_request(&request).unwrap();

        assert_eq!(response["id"], 3);
        assert!(response["result"]["content"].is_array());
        assert_eq!(response["result"]["content"][0]["type"], "text");

        // Also verify direct send works
        let notification = PlanNotification {
            pid: 42,
            title: "Test Plan Title".to_string(),
            path: "/tmp/test-plan.md".to_string(),
        };
        socket::send_notification(&socket_path, &notification).unwrap();

        std::thread::sleep(std::time::Duration::from_millis(50));
        let map = plan_map.lock().unwrap();
        assert_eq!(map.get(&42).unwrap().title, "Test Plan Title");

        socket::cleanup(&socket_path);
    }

    #[test]
    fn test_get_grandparent_pid_returns_some() {
        // In test context, we have a parent (cargo test) and grandparent
        let result = get_grandparent_pid();
        assert!(result.is_some());
        assert!(result.unwrap() > 0);
    }

    #[test]
    fn test_unknown_method_returns_error() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 99,
            "method": "unknown/method",
            "params": {}
        });

        let response = handle_request(&request).unwrap();

        assert_eq!(response["id"], 99);
        assert_eq!(response["error"]["code"], -32601);
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unknown/method"));
    }
}
