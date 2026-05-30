//! Minimal QEMU Monitor Protocol (QMP) client.
//!
//! QMP is a JSON line-based protocol exposed by QEMU over a socket. On Windows
//! we use a TCP socket (`-qmp tcp:127.0.0.1:PORT,server,nowait`). The handshake
//! is: read the server greeting, send `qmp_capabilities`, then issue commands.
//!
//! This is deliberately synchronous and short-lived: open a connection, run one
//! command, close. VM lifecycle actions are infrequent, so this keeps the code
//! simple and avoids holding a long-lived socket we'd have to babysit.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

/// Connect to the QMP port, perform the capabilities handshake, and execute a
/// single command (no arguments). Returns the raw JSON response value.
pub fn execute(port: u16, command: &str) -> Result<serde_json::Value, String> {
    execute_args(port, command, serde_json::Value::Null)
}

/// Like [`execute`], but passes an `arguments` object with the command. Pass
/// `Value::Null` for no arguments.
pub fn execute_args(
    port: u16,
    command: &str,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let stream = TcpStream::connect(("127.0.0.1", port))
        .map_err(|e| format!("QMP connect failed on port {port}: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .ok();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .ok();

    let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);

    // 1. Server greeting (the {"QMP": {...}} banner).
    read_json_line(&mut reader)?;

    // 2. Enter command mode.
    send(&mut writer, &serde_json::json!({ "execute": "qmp_capabilities" }))?;
    read_json_line(&mut reader)?; // capabilities ack

    // 3. The actual command, with arguments if any.
    let mut request = serde_json::json!({ "execute": command });
    if !args.is_null() {
        request["arguments"] = args;
    }
    send(&mut writer, &request)?;

    // QMP may emit asynchronous events before the command's `return`/`error`.
    // Read lines until we see one of those.
    loop {
        let value = read_json_line(&mut reader)?;
        if value.get("return").is_some() {
            return Ok(value);
        }
        if let Some(err) = value.get("error") {
            return Err(format!("QMP error for '{command}': {err}"));
        }
        // Otherwise it's an event (has "event" key) — keep reading.
    }
}

/// Run a Human Monitor (HMP) command over QMP via `human-monitor-command`.
///
/// Used for snapshot operations (`savevm`/`loadvm`/`delvm`) which have no
/// first-class QMP equivalent in the synchronous form we want. The catch: HMP
/// reports failures as *text in the success `return`* rather than a QMP `error`
/// object, so any non-empty output is treated as an error message.
pub fn hmp(port: u16, command_line: &str) -> Result<(), String> {
    let resp = execute_args(
        port,
        "human-monitor-command",
        serde_json::json!({ "command-line": command_line }),
    )?;
    let output = resp
        .get("return")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if output.is_empty() {
        Ok(())
    } else {
        Err(output.to_string())
    }
}

fn send(writer: &mut TcpStream, value: &serde_json::Value) -> Result<(), String> {
    let mut line = serde_json::to_string(value).map_err(|e| e.to_string())?;
    line.push('\n');
    writer
        .write_all(line.as_bytes())
        .map_err(|e| format!("QMP write failed: {e}"))?;
    writer.flush().map_err(|e| e.to_string())
}

fn read_json_line(reader: &mut BufReader<TcpStream>) -> Result<serde_json::Value, String> {
    let mut line = String::new();
    let n = reader
        .read_line(&mut line)
        .map_err(|e| format!("QMP read failed: {e}"))?;
    if n == 0 {
        return Err("QMP connection closed unexpectedly".into());
    }
    serde_json::from_str(line.trim())
        .map_err(|e| format!("QMP returned invalid JSON ({e}): {line}"))
}
