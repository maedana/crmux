use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

/// An RPC notification received from a client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcMessage {
    pub method: String,
    pub params: Value,
}

/// Return the socket path: `/tmp/crmux-{uid}.sock`
pub fn socket_path() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/tmp/crmux-{uid}.sock"))
}

/// Encode an RPC notification as msgpack-rpc wire format: `[2, method, params]`
pub fn encode_notification(method: &str, params: &Value) -> Vec<u8> {
    let mut buf = Vec::new();
    rmp::encode::write_array_len(&mut buf, 3).expect("encode array len");
    rmp::encode::write_uint(&mut buf, 2).expect("encode type");
    rmp::encode::write_str(&mut buf, method).expect("encode method");
    let params_bytes = rmp_serde::to_vec(params).expect("encode params");
    buf.extend_from_slice(&params_bytes);
    buf
}

/// Decode an RPC notification from msgpack-rpc wire format: `[2, method, params]`
pub fn decode_notification(data: &[u8]) -> io::Result<RpcMessage> {
    let mut cursor = io::Cursor::new(data);

    let array_len = rmp::decode::read_array_len(&mut cursor)
        .map_err(|e: rmp::decode::ValueReadError| {
            io::Error::new(io::ErrorKind::InvalidData, e.to_string())
        })?;
    if array_len != 3 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("expected array of 3, got {array_len}"),
        ));
    }

    let msg_type = rmp::decode::read_int::<u64, _>(&mut cursor)
        .map_err(|e: rmp::decode::NumValueReadError| {
            io::Error::new(io::ErrorKind::InvalidData, e.to_string())
        })?;
    if msg_type != 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("expected type 2 (notification), got {msg_type}"),
        ));
    }

    let mut method_buf = vec![0u8; 256];
    let method = rmp::decode::read_str(&mut cursor, &mut method_buf)
        .map_err(|e: rmp::decode::DecodeStringError<'_>| {
            io::Error::new(io::ErrorKind::InvalidData, e.to_string())
        })?
        .to_string();

    #[allow(clippy::cast_possible_truncation)]
    let remaining = &data[cursor.position() as usize..];
    let params: Value = rmp_serde::from_slice(remaining)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    Ok(RpcMessage { method, params })
}

/// RPC server that listens on a Unix domain socket in a background thread.
pub struct RpcServer {
    receiver: mpsc::Receiver<RpcMessage>,
    socket_path: PathBuf,
}

impl RpcServer {
    /// Start the RPC server, binding to the socket and spawning a listener thread.
    pub fn start() -> io::Result<Self> {
        let path = socket_path();

        // Remove stale socket file if it exists
        if path.exists() {
            std::fs::remove_file(&path)?;
        }

        let listener = UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;

        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            Self::accept_loop(&listener, &tx);
        });

        Ok(Self {
            receiver: rx,
            socket_path: path,
        })
    }

    fn accept_loop(listener: &UnixListener, tx: &mpsc::Sender<RpcMessage>) {
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    if let Some(msg) = Self::read_message(stream)
                        && tx.send(msg).is_err()
                    {
                        // Main thread dropped the receiver, exit
                        break;
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    // No pending connections, sleep briefly
                    thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(_) => {
                    break;
                }
            }
        }
    }

    fn read_message(mut stream: UnixStream) -> Option<RpcMessage> {
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).ok()?;
        decode_notification(&buf).ok()
    }

    /// Non-blocking receive of the next RPC message.
    pub fn try_recv(&self) -> Option<RpcMessage> {
        self.receiver.try_recv().ok()
    }
}

impl Drop for RpcServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

// --- Client side ---

/// Send an RPC notification to the running crmux instance.
pub fn send_notification(method: &str, params: &Value) -> io::Result<()> {
    let path = socket_path();
    let mut stream = UnixStream::connect(&path)?;
    let data = encode_notification(method, params);
    stream.write_all(&data)?;
    stream.shutdown(std::net::Shutdown::Write)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- encode/decode round-trip ---

    #[test]
    fn test_encode_decode_round_trip() {
        let params = serde_json::json!({
            "pane_id": "%5",
            "session_id": "abc-123",
            "cwd": "/home/user/project",
            "model": "claude-sonnet-4-6",
        });

        let encoded = encode_notification("session_start", &params);
        let decoded = decode_notification(&encoded).unwrap();

        assert_eq!(decoded.method, "session_start");
        assert_eq!(decoded.params["pane_id"], "%5");
        assert_eq!(decoded.params["session_id"], "abc-123");
        assert_eq!(decoded.params["cwd"], "/home/user/project");
        assert_eq!(decoded.params["model"], "claude-sonnet-4-6");
    }

    #[test]
    fn test_encode_decode_nested_params() {
        let params = serde_json::json!({
            "pane_id": "%1",
            "model": { "display_name": "Opus" },
        });

        let encoded = encode_notification("status_update", &params);
        let decoded = decode_notification(&encoded).unwrap();

        assert_eq!(decoded.method, "status_update");
        assert_eq!(decoded.params["model"]["display_name"], "Opus");
    }

    #[test]
    fn test_encode_decode_empty_params() {
        let params = serde_json::json!({});
        let encoded = encode_notification("ping", &params);
        let decoded = decode_notification(&encoded).unwrap();

        assert_eq!(decoded.method, "ping");
        assert_eq!(decoded.params, serde_json::json!({}));
    }

    #[test]
    fn test_decode_invalid_array_len() {
        // Encode a 2-element array instead of 3
        let mut buf = Vec::new();
        rmp::encode::write_array_len(&mut buf, 2).unwrap();
        rmp::encode::write_uint(&mut buf, 2).unwrap();
        rmp::encode::write_str(&mut buf, "test").unwrap();

        let result = decode_notification(&buf);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("expected array of 3"));
    }

    #[test]
    fn test_decode_invalid_type() {
        // Type 0 (request) instead of 2 (notification)
        let mut buf = Vec::new();
        rmp::encode::write_array_len(&mut buf, 3).unwrap();
        rmp::encode::write_uint(&mut buf, 0).unwrap();
        rmp::encode::write_str(&mut buf, "test").unwrap();
        let params = serde_json::json!({});
        buf.extend_from_slice(&rmp_serde::to_vec(&params).unwrap());

        let result = decode_notification(&buf);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("expected type 2"));
    }

    #[test]
    fn test_decode_empty_data() {
        let result = decode_notification(&[]);
        assert!(result.is_err());
    }

    // --- socket_path ---

    #[test]
    fn test_socket_path_contains_uid() {
        let path = socket_path();
        let uid = unsafe { libc::getuid() };
        assert_eq!(
            path,
            PathBuf::from(format!("/tmp/crmux-{uid}.sock"))
        );
    }

    // --- RpcServer integration test ---

    #[test]
    fn test_server_client_round_trip() {
        // Use a unique socket path for this test
        let uid = unsafe { libc::getuid() };
        let test_path = PathBuf::from(format!("/tmp/crmux-test-{uid}-{}.sock", std::process::id()));

        // Clean up any stale socket
        let _ = std::fs::remove_file(&test_path);

        let listener = UnixListener::bind(&test_path).unwrap();
        listener.set_nonblocking(true).unwrap();

        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            RpcServer::accept_loop(&listener, &tx);
        });

        // Send a notification
        let params = serde_json::json!({
            "pane_id": "%5",
            "session_id": "test-session",
        });

        let mut stream = UnixStream::connect(&test_path).unwrap();
        let data = encode_notification("session_start", &params);
        stream.write_all(&data).unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
        drop(stream);

        // Wait for message
        let msg = rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        assert_eq!(msg.method, "session_start");
        assert_eq!(msg.params["pane_id"], "%5");
        assert_eq!(msg.params["session_id"], "test-session");

        // Cleanup
        let _ = std::fs::remove_file(&test_path);
    }
}
