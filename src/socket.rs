use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex};
use std::thread;

/// Plan information stored per session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanInfo {
    pub title: String,
    pub path: String,
}

/// Message sent from MCP server to TUI via Unix socket.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanNotification {
    pub pid: u32,
    pub title: String,
    pub path: String,
}

pub const SOCKET_PATH: &str = "/tmp/crmux.sock";

/// Read a single length-prefixed `MessagePack` message from a stream.
fn read_notification(stream: &mut UnixStream) -> std::io::Result<PlanNotification> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;

    let mut msg_buf = vec![0u8; len];
    stream.read_exact(&mut msg_buf)?;

    rmp_serde::from_slice(&msg_buf)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Send a length-prefixed `MessagePack` message to a Unix socket.
pub fn send_notification(socket_path: &str, notification: &PlanNotification) -> std::io::Result<()> {
    let mut stream = UnixStream::connect(socket_path)?;
    let data = rmp_serde::to_vec(notification)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let len = u32::try_from(data.len())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(&data)?;
    Ok(())
}

/// Start a background listener thread that accepts connections on the Unix socket
/// and updates the shared plan map.
///
/// Returns the shared plan map that the TUI can read from.
pub fn start_listener(socket_path: &str) -> Arc<Mutex<HashMap<u32, PlanInfo>>> {
    let plan_map: Arc<Mutex<HashMap<u32, PlanInfo>>> = Arc::new(Mutex::new(HashMap::new()));
    let map_clone = Arc::clone(&plan_map);
    let path = socket_path.to_string();

    // Remove existing socket file
    let _ = std::fs::remove_file(&path);

    let listener = UnixListener::bind(&path).expect("Failed to bind Unix socket");

    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(mut stream) => {
                    if let Ok(notification) = read_notification(&mut stream)
                        && let Ok(mut map) = map_clone.lock()
                    {
                        map.insert(
                            notification.pid,
                            PlanInfo {
                                title: notification.title,
                                path: notification.path,
                            },
                        );
                    }
                }
                Err(_) => break,
            }
        }
    });

    plan_map
}

/// Clean up the socket file.
pub fn cleanup(socket_path: &str) {
    let _ = std::fs::remove_file(socket_path);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn temp_socket_path() -> String {
        format!("/tmp/crmux-test-{}.sock", std::process::id())
    }

    #[test]
    fn test_plan_notification_roundtrip_msgpack() {
        let notification = PlanNotification {
            pid: 12345,
            title: "Implement feature X".to_string(),
            path: "/tmp/plan.md".to_string(),
        };

        let data = rmp_serde::to_vec(&notification).unwrap();
        let decoded: PlanNotification = rmp_serde::from_slice(&data).unwrap();

        assert_eq!(notification, decoded);
    }

    #[test]
    fn test_listener_receives_notification() {
        let socket_path = temp_socket_path();
        let plan_map = start_listener(&socket_path);

        // Give listener thread time to start
        thread::sleep(Duration::from_millis(50));

        let notification = PlanNotification {
            pid: 100,
            title: "Test plan".to_string(),
            path: "/tmp/plan.md".to_string(),
        };

        send_notification(&socket_path, &notification).unwrap();

        // Give listener time to process
        thread::sleep(Duration::from_millis(50));

        let map = plan_map.lock().unwrap();
        assert_eq!(map.len(), 1);
        assert_eq!(
            map.get(&100),
            Some(&PlanInfo {
                title: "Test plan".to_string(),
                path: "/tmp/plan.md".to_string(),
            })
        );

        cleanup(&socket_path);
    }

    #[test]
    fn test_listener_receives_multiple_notifications() {
        let socket_path = temp_socket_path() + "-multi";
        let plan_map = start_listener(&socket_path);

        thread::sleep(Duration::from_millis(50));

        send_notification(
            &socket_path,
            &PlanNotification {
                pid: 100,
                title: "Plan A".to_string(),
                path: "/tmp/a.md".to_string(),
            },
        )
        .unwrap();

        send_notification(
            &socket_path,
            &PlanNotification {
                pid: 200,
                title: "Plan B".to_string(),
                path: "/tmp/b.md".to_string(),
            },
        )
        .unwrap();

        thread::sleep(Duration::from_millis(50));

        let map = plan_map.lock().unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(
            map.get(&100),
            Some(&PlanInfo {
                title: "Plan A".to_string(),
                path: "/tmp/a.md".to_string(),
            })
        );
        assert_eq!(
            map.get(&200),
            Some(&PlanInfo {
                title: "Plan B".to_string(),
                path: "/tmp/b.md".to_string(),
            })
        );

        cleanup(&socket_path);
    }

    #[test]
    fn test_notification_updates_existing_pid() {
        let socket_path = temp_socket_path() + "-update";
        let plan_map = start_listener(&socket_path);

        thread::sleep(Duration::from_millis(50));

        send_notification(
            &socket_path,
            &PlanNotification {
                pid: 100,
                title: "Old plan".to_string(),
                path: "/tmp/old.md".to_string(),
            },
        )
        .unwrap();

        thread::sleep(Duration::from_millis(50));

        send_notification(
            &socket_path,
            &PlanNotification {
                pid: 100,
                title: "New plan".to_string(),
                path: "/tmp/new.md".to_string(),
            },
        )
        .unwrap();

        thread::sleep(Duration::from_millis(50));

        let map = plan_map.lock().unwrap();
        assert_eq!(map.len(), 1);
        assert_eq!(
            map.get(&100),
            Some(&PlanInfo {
                title: "New plan".to_string(),
                path: "/tmp/new.md".to_string(),
            })
        );

        cleanup(&socket_path);
    }

    #[test]
    fn test_send_notification_fails_when_no_listener() {
        let result = send_notification("/tmp/crmux-nonexistent.sock", &PlanNotification {
            pid: 1,
            title: "test".to_string(),
            path: "/tmp/test.md".to_string(),
        });

        assert!(result.is_err());
    }
}
