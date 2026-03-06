use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::thread;

/// Type alias for a request handler function.
pub type RequestHandler = Arc<dyn Fn(&str, &Value) -> Value + Send + Sync>;

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

/// Encode an RPC request as msgpack-rpc wire format: `[0, msgid, method, params]`
#[cfg(test)]
pub fn encode_request(msgid: u32, method: &str, params: &Value) -> Vec<u8> {
    let mut buf = Vec::new();
    rmp::encode::write_array_len(&mut buf, 4).expect("encode array len");
    rmp::encode::write_uint(&mut buf, 0).expect("encode type");
    rmp::encode::write_uint(&mut buf, u64::from(msgid)).expect("encode msgid");
    rmp::encode::write_str(&mut buf, method).expect("encode method");
    let params_bytes = rmp_serde::to_vec(params).expect("encode params");
    buf.extend_from_slice(&params_bytes);
    buf
}

/// Decode an RPC request from msgpack-rpc wire format: `[0, msgid, method, params]`
pub fn decode_request(data: &[u8]) -> io::Result<(u32, String, Value)> {
    let mut cursor = io::Cursor::new(data);

    let array_len = rmp::decode::read_array_len(&mut cursor)
        .map_err(|e: rmp::decode::ValueReadError| {
            io::Error::new(io::ErrorKind::InvalidData, e.to_string())
        })?;
    if array_len != 4 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("expected array of 4, got {array_len}"),
        ));
    }

    let msg_type = rmp::decode::read_int::<u64, _>(&mut cursor)
        .map_err(|e: rmp::decode::NumValueReadError| {
            io::Error::new(io::ErrorKind::InvalidData, e.to_string())
        })?;
    if msg_type != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("expected type 0 (request), got {msg_type}"),
        ));
    }

    // msgpack-rpc msgid is a small sequential counter; u32 is sufficient.
    #[allow(clippy::cast_possible_truncation)]
    let msgid = rmp::decode::read_int::<u64, _>(&mut cursor)
        .map_err(|e: rmp::decode::NumValueReadError| {
            io::Error::new(io::ErrorKind::InvalidData, e.to_string())
        })? as u32;

    let mut method_buf = vec![0u8; 256];
    let method = rmp::decode::read_str(&mut cursor, &mut method_buf)
        .map_err(|e: rmp::decode::DecodeStringError<'_>| {
            io::Error::new(io::ErrorKind::InvalidData, e.to_string())
        })?
        .to_string();

    // Cursor position within a single RPC message; always fits in usize.
    #[allow(clippy::cast_possible_truncation)]
    let remaining = &data[cursor.position() as usize..];
    let params: Value = rmp_serde::from_slice(remaining)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    Ok((msgid, method, params))
}

/// Encode an RPC response as msgpack-rpc wire format: `[1, msgid, null, result]`
pub fn encode_response(msgid: u32, result: &Value) -> Vec<u8> {
    let mut buf = Vec::new();
    rmp::encode::write_array_len(&mut buf, 4).expect("encode array len");
    rmp::encode::write_uint(&mut buf, 1).expect("encode type");
    rmp::encode::write_uint(&mut buf, u64::from(msgid)).expect("encode msgid");
    rmp::encode::write_nil(&mut buf).expect("encode nil error");
    let result_bytes = rmp_serde::to_vec(result).expect("encode result");
    buf.extend_from_slice(&result_bytes);
    buf
}

/// Decode an RPC response from msgpack-rpc wire format: `[1, msgid, null, result]`
#[cfg(test)]
pub fn decode_response(data: &[u8]) -> io::Result<(u32, Value)> {
    let mut cursor = io::Cursor::new(data);

    let array_len = rmp::decode::read_array_len(&mut cursor)
        .map_err(|e: rmp::decode::ValueReadError| {
            io::Error::new(io::ErrorKind::InvalidData, e.to_string())
        })?;
    if array_len != 4 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("expected array of 4, got {array_len}"),
        ));
    }

    let msg_type = rmp::decode::read_int::<u64, _>(&mut cursor)
        .map_err(|e: rmp::decode::NumValueReadError| {
            io::Error::new(io::ErrorKind::InvalidData, e.to_string())
        })?;
    if msg_type != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("expected type 1 (response), got {msg_type}"),
        ));
    }

    // msgpack-rpc msgid is a small sequential counter; u32 is sufficient.
    #[allow(clippy::cast_possible_truncation)]
    let msgid = rmp::decode::read_int::<u64, _>(&mut cursor)
        .map_err(|e: rmp::decode::NumValueReadError| {
            io::Error::new(io::ErrorKind::InvalidData, e.to_string())
        })? as u32;

    // Skip the error field (null)
    rmp::decode::read_nil(&mut cursor)
        .map_err(|e: rmp::decode::ValueReadError| {
            io::Error::new(io::ErrorKind::InvalidData, e.to_string())
        })?;

    // Cursor position within a single RPC message; always fits in usize.
    #[allow(clippy::cast_possible_truncation)]
    let remaining = &data[cursor.position() as usize..];
    let result: Value = rmp_serde::from_slice(remaining)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    Ok((msgid, result))
}

/// Determine the msgpack-rpc message type (0=request, 1=response, 2=notification).
pub fn message_type(data: &[u8]) -> io::Result<u8> {
    let mut cursor = io::Cursor::new(data);

    let _array_len = rmp::decode::read_array_len(&mut cursor)
        .map_err(|e: rmp::decode::ValueReadError| {
            io::Error::new(io::ErrorKind::InvalidData, e.to_string())
        })?;

    // msgpack-rpc type field is 0, 1, or 2; fits in u8.
    #[allow(clippy::cast_possible_truncation)]
    let msg_type = rmp::decode::read_int::<u64, _>(&mut cursor)
        .map_err(|e: rmp::decode::NumValueReadError| {
            io::Error::new(io::ErrorKind::InvalidData, e.to_string())
        })? as u8;

    Ok(msg_type)
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

    // Cursor position within a single RPC message; always fits in usize.
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
    pub fn start(handler: Option<RequestHandler>) -> io::Result<Self> {
        let path = socket_path();

        // Remove stale socket file if it exists
        if path.exists() {
            std::fs::remove_file(&path)?;
        }

        let listener = UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;

        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            Self::accept_loop(&listener, &tx, handler.as_ref());
        });

        Ok(Self {
            receiver: rx,
            socket_path: path,
        })
    }

    fn accept_loop(
        listener: &UnixListener,
        tx: &mpsc::Sender<RpcMessage>,
        handler: Option<&RequestHandler>,
    ) {
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    Self::handle_connection(stream, tx, handler);
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

    fn handle_connection(
        mut stream: UnixStream,
        tx: &mpsc::Sender<RpcMessage>,
        handler: Option<&RequestHandler>,
    ) {
        let mut buf = Vec::new();
        if stream.read_to_end(&mut buf).is_err() || buf.is_empty() {
            return;
        }

        let Ok(msg_type) = message_type(&buf) else {
            return;
        };

        match msg_type {
            0 => {
                // Request: decode, call handler, send response
                if let (Some(handler), Ok((msgid, method, params))) =
                    (handler, decode_request(&buf))
                {
                    let result = handler(&method, &params);
                    let response = encode_response(msgid, &result);
                    let _ = stream.write_all(&response);
                    let _ = stream.shutdown(std::net::Shutdown::Write);
                }
            }
            2 => {
                // Notification: decode and forward via channel
                if let Ok(msg) = decode_notification(&buf) {
                    let _ = tx.send(msg);
                }
            }
            _ => {}
        }
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

    // --- request/response encode/decode round-trip ---

    #[test]
    fn test_encode_decode_request_round_trip() {
        let params = serde_json::json!({"key": "value"});
        let encoded = encode_request(42, "get_sessions", &params);
        let (msgid, method, decoded_params) = decode_request(&encoded).unwrap();
        assert_eq!(msgid, 42);
        assert_eq!(method, "get_sessions");
        assert_eq!(decoded_params["key"], "value");
    }

    #[test]
    fn test_encode_decode_response_round_trip() {
        let result = serde_json::json!({"sessions": []});
        let encoded = encode_response(42, &result);
        let (msgid, decoded_result) = decode_response(&encoded).unwrap();
        assert_eq!(msgid, 42);
        assert_eq!(decoded_result["sessions"], serde_json::json!([]));
    }

    // --- message_type ---

    #[test]
    fn test_message_type_request() {
        let encoded = encode_request(1, "test", &serde_json::json!({}));
        assert_eq!(message_type(&encoded).unwrap(), 0);
    }

    #[test]
    fn test_message_type_response() {
        let encoded = encode_response(1, &serde_json::json!({}));
        assert_eq!(message_type(&encoded).unwrap(), 1);
    }

    #[test]
    fn test_message_type_notification() {
        let encoded = encode_notification("test", &serde_json::json!({}));
        assert_eq!(message_type(&encoded).unwrap(), 2);
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
            RpcServer::accept_loop(&listener, &tx, None::<&RequestHandler>);
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

    #[test]
    fn test_server_handles_request_response() {
        use std::sync::Arc;

        let uid = unsafe { libc::getuid() };
        let test_path = PathBuf::from(format!(
            "/tmp/crmux-test-req-{uid}-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&test_path);

        let listener = UnixListener::bind(&test_path).unwrap();
        listener.set_nonblocking(true).unwrap();

        let (tx, _rx) = mpsc::channel();

        let handler: RequestHandler = Arc::new(|method, _params| {
            if method == "get_sessions" {
                serde_json::json!({"sessions": [{"name": "test"}]})
            } else {
                serde_json::json!(null)
            }
        });

        thread::spawn(move || {
            RpcServer::accept_loop(&listener, &tx, Some(&handler));
        });

        // Send a request
        let mut stream = UnixStream::connect(&test_path).unwrap();
        let data = encode_request(1, "get_sessions", &serde_json::json!({}));
        stream.write_all(&data).unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();

        // Read response
        let mut response_buf = Vec::new();
        stream.read_to_end(&mut response_buf).unwrap();

        let (msgid, result) = decode_response(&response_buf).unwrap();
        assert_eq!(msgid, 1);
        assert_eq!(result["sessions"][0]["name"], "test");

        let _ = std::fs::remove_file(&test_path);
    }

    #[test]
    fn test_server_still_handles_notifications_with_handler() {
        use std::sync::Arc;

        let uid = unsafe { libc::getuid() };
        let test_path = PathBuf::from(format!(
            "/tmp/crmux-test-notif-{uid}-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&test_path);

        let listener = UnixListener::bind(&test_path).unwrap();
        listener.set_nonblocking(true).unwrap();

        let (tx, rx) = mpsc::channel();

        let handler: RequestHandler = Arc::new(|_method, _params| serde_json::json!(null));

        thread::spawn(move || {
            RpcServer::accept_loop(&listener, &tx, Some(&handler));
        });

        // Send a notification
        let params = serde_json::json!({"pane_id": "%1"});
        let mut stream = UnixStream::connect(&test_path).unwrap();
        let data = encode_notification("session_start", &params);
        stream.write_all(&data).unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
        drop(stream);

        let msg = rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        assert_eq!(msg.method, "session_start");
        assert_eq!(msg.params["pane_id"], "%1");

        let _ = std::fs::remove_file(&test_path);
    }
}
