use crate::engine::DatabaseEngine;
use nova_core::document::Document;
use nova_core::error::Result;
use nova_core::value::Value;
use nova_protocol::ResponsePayload;
use nova_query::parse_nql;
use serde_json::json;
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tracing::{error, info, warn};

/// Start the embedded NOVA Studio HTTP and Server-Sent Events server.
pub async fn run_web_studio(
    host: String,
    port: u16,
    engine: Arc<DatabaseEngine>,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> Result<()> {
    let bind_addr = format!("{host}:{port}");
    let listener = TcpListener::bind(&bind_addr).await.map_err(|e| {
        nova_core::error::NovaError::storage(format!(
            "Failed to bind Web Studio to {bind_addr}: {e}"
        ))
    })?;

    info!(
        address = %bind_addr,
        "NOVA Studio Web Console listening"
    );

    loop {
        tokio::select! {
            accept_res = listener.accept() => {
                match accept_res {
                    Ok((stream, _)) => {
                        let engine_clone = Arc::clone(&engine);
                        tokio::spawn(async move {
                            if let Err(e) = handle_http_connection(stream, engine_clone).await {
                                warn!("Web Studio connection ended: {e}");
                            }
                        });
                    }
                    Err(e) => {
                        error!("Web Studio TCP accept failed: {e}");
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                info!("Web Studio received shutdown signal");
                break;
            }
        }
    }

    Ok(())
}

async fn handle_http_connection(mut stream: TcpStream, engine: Arc<DatabaseEngine>) -> Result<()> {
    let mut buffer = vec![0u8; 8192];
    let bytes_read = stream.read(&mut buffer).await.map_err(|e| {
        nova_core::error::NovaError::storage(format!("Failed to read HTTP request: {e}"))
    })?;

    if bytes_read == 0 {
        return Ok(());
    }

    let request_str = String::from_utf8_lossy(&buffer[..bytes_read]);
    let mut lines = request_str.lines();
    let first_line = lines.next().unwrap_or("");
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let path = parts.next().unwrap_or("/");

    // Handle CORS preflight
    if method == "OPTIONS" {
        let response = "HTTP/1.1 204 No Content\r\n\
            Access-Control-Allow-Origin: *\r\n\
            Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
            Access-Control-Allow-Headers: Content-Type\r\n\
            Access-Control-Max-Age: 86400\r\n\r\n";
        stream.write_all(response.as_bytes()).await.ok();
        return Ok(());
    }

    match (method, path) {
        ("GET", "/") => {
            let html = STUDIO_HTML;
            let response = format!(
                "HTTP/1.1 200 OK\r\n\
                Content-Type: text/html; charset=utf-8\r\n\
                Content-Length: {}\r\n\
                Access-Control-Allow-Origin: *\r\n\
                Connection: close\r\n\r\n{}",
                html.len(),
                html
            );
            stream.write_all(response.as_bytes()).await.ok();
        }
        ("GET", "/api/status") => {
            let status = engine.get_system_status().await;
            let body = serde_json::to_string(&status).unwrap_or_default();
            let response = format!(
                "HTTP/1.1 200 OK\r\n\
                Content-Type: application/json; charset=utf-8\r\n\
                Content-Length: {}\r\n\
                Access-Control-Allow-Origin: *\r\n\
                Connection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.ok();
        }
        ("POST", "/api/query") => {
            let body_json = extract_json_body(&request_str);
            let query = body_json
                .get("query")
                .and_then(|q| q.as_str())
                .unwrap_or("");
            let namespace = body_json
                .get("namespace")
                .and_then(|n| n.as_str())
                .unwrap_or("default");

            let res = execute_query_api(&engine, query, namespace).await;
            let body = serde_json::to_string(&res).unwrap_or_default();
            let response = format!(
                "HTTP/1.1 200 OK\r\n\
                Content-Type: application/json; charset=utf-8\r\n\
                Content-Length: {}\r\n\
                Access-Control-Allow-Origin: *\r\n\
                Connection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.ok();
        }
        ("POST", "/api/seed") => {
            let res = seed_sample_data(&engine).await;
            let body = serde_json::to_string(&res).unwrap_or_default();
            let response = format!(
                "HTTP/1.1 200 OK\r\n\
                Content-Type: application/json; charset=utf-8\r\n\
                Content-Length: {}\r\n\
                Access-Control-Allow-Origin: *\r\n\
                Connection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.ok();
        }
        ("GET", "/api/events") => {
            let sse_header = "HTTP/1.1 200 OK\r\n\
                Content-Type: text/event-stream\r\n\
                Cache-Control: no-cache\r\n\
                Connection: keep-alive\r\n\
                Access-Control-Allow-Origin: *\r\n\r\n";
            if stream.write_all(sse_header.as_bytes()).await.is_err() {
                return Ok(());
            }

            // Initial connection ack
            let _ = stream.write_all(b": connected\n\n").await;

            let mut event_rx = engine.subscribe_events();
            loop {
                tokio::select! {
                    recv_res = event_rx.recv() => {
                        match recv_res {
                            Ok(event) => {
                                let event_type_str = event.event_type.as_str();
                                let doc_json = event
                                    .after
                                    .as_ref()
                                    .or(event.before.as_ref())
                                    .map(document_to_json);
                                let payload = json!({
                                    "sequence": event.sequence,
                                    "event_type": event_type_str,
                                    "namespace": event.namespace,
                                    "collection": event.collection,
                                    "document_id": event.document_id.as_str(),
                                    "document": doc_json,
                                    "timestamp": event.timestamp,
                                });
                                let msg = format!("data: {}\n\n", serde_json::to_string(&payload).unwrap_or_default());
                                if stream.write_all(msg.as_bytes()).await.is_err() {
                                    break;
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                }
            }
        }
        _ => {
            let response = "HTTP/1.1 404 Not Found\r\n\
                Content-Type: text/plain\r\n\
                Content-Length: 9\r\n\
                Connection: close\r\n\r\nNot Found";
            stream.write_all(response.as_bytes()).await.ok();
        }
    }

    Ok(())
}

fn extract_json_body(request_str: &str) -> serde_json::Value {
    if let Some(pos) = request_str.find("\r\n\r\n") {
        let body = &request_str[pos + 4..];
        serde_json::from_str(body).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    }
}

async fn execute_query_api(
    engine: &DatabaseEngine,
    query: &str,
    namespace: &str,
) -> serde_json::Value {
    if query.trim().is_empty() {
        return json!({ "success": false, "error": "Query string cannot be empty" });
    }

    let parsed = match parse_nql(query) {
        Ok(stmt) => stmt,
        Err(e) => return json!({ "success": false, "error": format!("Syntax Error: {e}") }),
    };

    let start = Instant::now();
    let exec_res = engine.execute(parsed, namespace).await;
    let elapsed_us = start.elapsed().as_micros();

    match exec_res {
        Ok(payload) => match payload {
            ResponsePayload::Records { documents, count } => {
                let docs: Vec<serde_json::Value> = documents.iter().map(document_to_json).collect();
                json!({
                    "success": true,
                    "execution_time_us": elapsed_us,
                    "type": "records",
                    "count": count,
                    "data": docs,
                })
            }
            ResponsePayload::Success { message, affected } => json!({
                "success": true,
                "execution_time_us": elapsed_us,
                "type": "success",
                "message": message,
                "affected": affected,
            }),
            ResponsePayload::Count(c) => json!({
                "success": true,
                "execution_time_us": elapsed_us,
                "type": "count",
                "count": c,
            }),
            ResponsePayload::Exists(e) => json!({
                "success": true,
                "execution_time_us": elapsed_us,
                "type": "exists",
                "exists": e,
            }),
            ResponsePayload::Error(err) => json!({
                "success": false,
                "execution_time_us": elapsed_us,
                "error": err.to_string(),
            }),
            ResponsePayload::WatchAck { subscription_id } => json!({
                "success": true,
                "execution_time_us": elapsed_us,
                "type": "watch",
                "subscription_id": subscription_id,
            }),
            ResponsePayload::Pong => json!({
                "success": true,
                "execution_time_us": elapsed_us,
                "type": "pong",
            }),
        },
        Err(e) => json!({
            "success": false,
            "execution_time_us": elapsed_us,
            "error": e.to_string(),
        }),
    }
}

async fn seed_sample_data(engine: &DatabaseEngine) -> serde_json::Value {
    let queries = [
        // Users collection
        r#"INSERT INTO users VALUES ({ "id": "usr_01", "name": "Elena Rostova", "role": "Lead Architect", "team": "Core Systems", "points": 98, "active": true })"#,
        r#"INSERT INTO users VALUES ({ "id": "usr_02", "name": "Marcus Vance", "role": "Site Reliability Engineer", "team": "Infrastructure", "points": 88, "active": true })"#,
        r#"INSERT INTO users VALUES ({ "id": "usr_03", "name": "Aria Chen", "role": "Data Systems Engineer", "team": "Analytics", "points": 92, "active": true })"#,
        r#"INSERT INTO users VALUES ({ "id": "usr_04", "name": "Devon Miller", "role": "Security Specialist", "team": "Security", "points": 74, "active": false })"#,
        // Sensors collection
        r#"INSERT INTO sensors VALUES ({ "id": "sns_north_01", "name": "Cryo Tank Alpha", "location": "Zone 1", "temp": 64.2, "pressure": 101.3, "status": "nominal" })"#,
        r#"INSERT INTO sensors VALUES ({ "id": "sns_north_02", "name": "Core Turbine 4", "location": "Zone 2", "temp": 82.5, "pressure": 114.7, "status": "warning" })"#,
        r#"INSERT INTO sensors VALUES ({ "id": "sns_south_01", "name": "Reactor Shield B", "location": "Zone 3", "temp": 91.0, "pressure": 128.2, "status": "critical" })"#,
        r#"INSERT INTO sensors VALUES ({ "id": "sns_south_02", "name": "Cooling Valve 9", "location": "Zone 1", "temp": 52.8, "pressure": 98.4, "status": "nominal" })"#,
        // Indices
        "CREATE INDEX ON users (points)",
        "CREATE INDEX ON sensors (temp)",
    ];

    let mut seeded = 0;
    for q in queries {
        if let Ok(stmt) = parse_nql(q) {
            let _ = engine.execute(stmt, "default").await;
            seeded += 1;
        }
    }

    json!({
        "success": true,
        "message": format!("Seeded {seeded} sample documents and indexes into 'default' namespace"),
        "count": seeded,
    })
}

fn document_to_json(doc: &Document) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert("id".to_string(), json!(doc.id.as_str()));
    for (k, v) in &doc.fields {
        map.insert(k.clone(), value_to_json(v));
    }
    serde_json::Value::Object(map)
}

fn value_to_json(v: &Value) -> serde_json::Value {
    match v {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => json!(b),
        Value::Int(i) => json!(i),
        Value::Float(f) => json!(f),
        Value::String(s) => json!(s),
        Value::Bytes(b) => json!(format!("<{} bytes>", b.len())),
        Value::Array(arr) => serde_json::Value::Array(arr.iter().map(value_to_json).collect()),
        Value::Object(obj) => {
            let mut m = serde_json::Map::new();
            for (k, val) in obj {
                m.insert(k.clone(), value_to_json(val));
            }
            serde_json::Value::Object(m)
        }
        Value::Timestamp(t) => json!(t),
        Value::Uuid(u) => json!(uuid::Uuid::from_bytes(*u).to_string()),
    }
}

/// Embedded Single-Page Application for NOVA Studio.
pub const STUDIO_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>NOVA Studio — Real-time Data Flow Database</title>
  <link rel="preconnect" href="https://fonts.googleapis.com">
  <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
  <link href="https://fonts.googleapis.com/css2?family=Inter:wght@300;400;500;600;700&family=JetBrains+Mono:wght@400;500;600&display=swap" rel="stylesheet">
  <style>
    :root {
      --bg: #07090e;
      --card-bg: rgba(14, 20, 32, 0.75);
      --card-border: rgba(255, 255, 255, 0.08);
      --accent: #00f2fe;
      --accent-glow: rgba(0, 242, 254, 0.35);
      --violet: #8a2be2;
      --emerald: #00f5a0;
      --emerald-glow: rgba(0, 245, 160, 0.3);
      --crimson: #ff4757;
      --text: #f1f5f9;
      --text-muted: #94a3b8;
      --code-bg: #03060a;
    }

    * { box-sizing: border-box; margin: 0; padding: 0; }
    body {
      background-color: var(--bg);
      background-image: 
        radial-gradient(circle at 15% 10%, rgba(138, 43, 226, 0.15), transparent 45%),
        radial-gradient(circle at 85% 15%, rgba(0, 242, 254, 0.15), transparent 45%);
      color: var(--text);
      font-family: 'Inter', -apple-system, sans-serif;
      min-height: 100vh;
      display: flex;
      flex-direction: column;
    }

    /* Header Bar */
    header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 16px 32px;
      border-bottom: 1px solid var(--card-border);
      background: rgba(7, 9, 14, 0.85);
      backdrop-filter: blur(16px);
      position: sticky;
      top: 0;
      z-index: 100;
    }
    .brand {
      display: flex;
      align-items: center;
      gap: 14px;
    }
    .brand-logo {
      width: 38px;
      height: 38px;
      border-radius: 10px;
      background: linear-gradient(135deg, #00f2fe, #7f00ff);
      display: flex;
      align-items: center;
      justify-content: center;
      font-weight: 700;
      font-size: 20px;
      color: #fff;
      box-shadow: 0 0 20px var(--accent-glow);
    }
    .brand-text h1 {
      font-size: 19px;
      font-weight: 700;
      letter-spacing: 0.5px;
      background: linear-gradient(90deg, #fff, #b4c6ef);
      -webkit-background-clip: text;
      -webkit-text-fill-color: transparent;
    }
    .brand-text p {
      font-size: 11px;
      color: var(--text-muted);
      letter-spacing: 0.3px;
    }
    .header-actions {
      display: flex;
      align-items: center;
      gap: 16px;
    }
    .status-pill {
      display: flex;
      align-items: center;
      gap: 8px;
      background: rgba(0, 245, 160, 0.1);
      border: 1px solid rgba(0, 245, 160, 0.25);
      padding: 6px 14px;
      border-radius: 20px;
      font-size: 12px;
      font-weight: 600;
      color: var(--emerald);
    }
    .pulse-dot {
      width: 8px;
      height: 8px;
      border-radius: 50%;
      background: var(--emerald);
      box-shadow: 0 0 10px var(--emerald);
      animation: pulse 1.8s infinite;
    }
    @keyframes pulse {
      0% { transform: scale(0.9); opacity: 0.7; }
      50% { transform: scale(1.3); opacity: 1; }
      100% { transform: scale(0.9); opacity: 0.7; }
    }
    .btn {
      background: rgba(255, 255, 255, 0.06);
      border: 1px solid var(--card-border);
      color: var(--text);
      padding: 8px 16px;
      border-radius: 8px;
      cursor: pointer;
      font-size: 13px;
      font-weight: 500;
      transition: all 0.2s ease;
      display: inline-flex;
      align-items: center;
      gap: 6px;
    }
    .btn:hover {
      background: rgba(255, 255, 255, 0.12);
      border-color: rgba(255, 255, 255, 0.2);
    }
    .btn-primary {
      background: linear-gradient(135deg, #00f2fe, #4facfe);
      border: none;
      color: #030814;
      font-weight: 600;
      box-shadow: 0 0 16px var(--accent-glow);
    }
    .btn-primary:hover {
      transform: translateY(-1px);
      box-shadow: 0 0 24px rgba(0, 242, 254, 0.6);
    }

    /* Metrics Strip */
    .metrics-bar {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
      gap: 16px;
      padding: 24px 32px 12px 32px;
    }
    .metric-card {
      background: var(--card-bg);
      border: 1px solid var(--card-border);
      backdrop-filter: blur(12px);
      border-radius: 12px;
      padding: 16px 20px;
      transition: transform 0.2s ease;
    }
    .metric-card:hover {
      transform: translateY(-2px);
      border-color: rgba(0, 242, 254, 0.2);
    }
    .metric-label {
      font-size: 11px;
      text-transform: uppercase;
      letter-spacing: 0.8px;
      color: var(--text-muted);
      margin-bottom: 6px;
    }
    .metric-value {
      font-size: 24px;
      font-weight: 700;
      color: #fff;
      display: flex;
      align-items: baseline;
      gap: 6px;
    }
    .metric-sub {
      font-size: 11px;
      color: var(--accent);
      font-weight: 500;
    }

    /* Main Container */
    .container {
      display: flex;
      flex: 1;
      padding: 12px 32px 32px 32px;
      gap: 24px;
    }

    /* Tabs Layout */
    .tabs-header {
      display: flex;
      gap: 8px;
      border-bottom: 1px solid var(--card-border);
      margin-bottom: 20px;
    }
    .tab-btn {
      background: none;
      border: none;
      color: var(--text-muted);
      font-size: 14px;
      font-weight: 600;
      padding: 10px 18px;
      cursor: pointer;
      position: relative;
      transition: color 0.2s ease;
    }
    .tab-btn.active {
      color: var(--accent);
    }
    .tab-btn.active::after {
      content: '';
      position: absolute;
      bottom: -1px;
      left: 0;
      right: 0;
      height: 2px;
      background: var(--accent);
      box-shadow: 0 0 10px var(--accent);
    }

    /* Main Panel */
    .main-workspace {
      flex: 1;
      background: var(--card-bg);
      border: 1px solid var(--card-border);
      border-radius: 16px;
      padding: 24px;
      display: flex;
      flex-direction: column;
      backdrop-filter: blur(16px);
    }

    /* Query Studio */
    .query-templates {
      display: flex;
      gap: 8px;
      margin-bottom: 12px;
      flex-wrap: wrap;
    }
    .template-tag {
      background: rgba(255, 255, 255, 0.04);
      border: 1px solid var(--card-border);
      padding: 4px 10px;
      border-radius: 6px;
      font-size: 11px;
      font-family: 'JetBrains Mono', monospace;
      color: #b4c6ef;
      cursor: pointer;
      transition: all 0.2s;
    }
    .template-tag:hover {
      background: rgba(0, 242, 254, 0.1);
      border-color: var(--accent);
      color: #fff;
    }
    .editor-wrapper {
      position: relative;
      margin-bottom: 16px;
    }
    .query-editor {
      width: 100%;
      height: 120px;
      background: var(--code-bg);
      border: 1px solid var(--card-border);
      border-radius: 10px;
      padding: 14px 18px;
      font-family: 'JetBrains Mono', monospace;
      font-size: 14px;
      color: #38bdf8;
      resize: vertical;
      outline: none;
      line-height: 1.5;
    }
    .query-editor:focus {
      border-color: var(--accent);
      box-shadow: 0 0 12px var(--accent-glow);
    }
    .query-toolbar {
      display: flex;
      justify-content: space-between;
      align-items: center;
      margin-bottom: 16px;
    }
    .query-stats {
      font-size: 12px;
      color: var(--text-muted);
      display: flex;
      gap: 16px;
    }
    .query-stats span {
      color: var(--emerald);
      font-weight: 600;
    }

    /* Results Table & JSON */
    .results-panel {
      flex: 1;
      background: var(--code-bg);
      border: 1px solid var(--card-border);
      border-radius: 10px;
      padding: 16px;
      overflow: auto;
      max-height: 480px;
      font-family: 'JetBrains Mono', monospace;
      font-size: 13px;
    }
    table {
      width: 100%;
      border-collapse: collapse;
      text-align: left;
    }
    th {
      border-bottom: 1px solid var(--card-border);
      padding: 10px 14px;
      color: var(--text-muted);
      font-size: 11px;
      text-transform: uppercase;
      letter-spacing: 0.6px;
    }
    td {
      padding: 10px 14px;
      border-bottom: 1px solid rgba(255, 255, 255, 0.03);
      color: #e2e8f0;
    }
    tr:hover td {
      background: rgba(255, 255, 255, 0.02);
    }

    /* Live Data Flow View */
    .flow-pipeline {
      display: flex;
      align-items: center;
      justify-content: space-around;
      padding: 20px;
      background: rgba(255, 255, 255, 0.02);
      border-radius: 12px;
      border: 1px solid var(--card-border);
      margin-bottom: 24px;
    }
    .flow-node {
      text-align: center;
      padding: 12px 20px;
      background: var(--card-bg);
      border: 1px solid var(--card-border);
      border-radius: 10px;
      min-width: 140px;
    }
    .flow-node.active {
      border-color: var(--accent);
      box-shadow: 0 0 16px var(--accent-glow);
    }
    .flow-node h4 {
      font-size: 13px;
      font-weight: 600;
      color: #fff;
    }
    .flow-node p {
      font-size: 11px;
      color: var(--text-muted);
      margin-top: 4px;
    }
    .flow-arrow {
      color: var(--accent);
      font-size: 20px;
      animation: floatArrow 2s infinite ease-in-out;
    }
    @keyframes floatArrow {
      0%, 100% { transform: translateX(0); }
      50% { transform: translateX(5px); }
    }
    .events-stream {
      display: flex;
      flex-direction: column;
      gap: 10px;
      max-height: 440px;
      overflow-y: auto;
    }
    .event-card {
      background: rgba(255, 255, 255, 0.03);
      border: 1px solid var(--card-border);
      border-radius: 8px;
      padding: 12px 16px;
      display: flex;
      align-items: center;
      justify-content: space-between;
      animation: slideIn 0.3s ease-out;
    }
    @keyframes slideIn {
      from { opacity: 0; transform: translateY(-10px); }
      to { opacity: 1; transform: translateY(0); }
    }
    .event-badge {
      padding: 3px 8px;
      border-radius: 4px;
      font-size: 11px;
      font-weight: 700;
      font-family: 'JetBrains Mono', monospace;
    }
    .badge-insert { background: rgba(0, 245, 160, 0.15); color: var(--emerald); border: 1px solid var(--emerald); }
    .badge-update { background: rgba(0, 242, 254, 0.15); color: var(--accent); border: 1px solid var(--accent); }
    .badge-delete { background: rgba(255, 71, 87, 0.15); color: var(--crimson); border: 1px solid var(--crimson); }
    .event-info {
      font-family: 'JetBrains Mono', monospace;
      font-size: 12px;
      color: #cbd5e1;
    }
    .event-time {
      font-size: 11px;
      color: var(--text-muted);
    }

    /* Collections Sidebar */
    .sidebar {
      width: 280px;
      background: var(--card-bg);
      border: 1px solid var(--card-border);
      border-radius: 16px;
      padding: 20px;
      backdrop-filter: blur(16px);
      display: flex;
      flex-direction: column;
    }
    .sidebar h3 {
      font-size: 13px;
      text-transform: uppercase;
      letter-spacing: 0.8px;
      color: var(--text-muted);
      margin-bottom: 14px;
    }
    .coll-item {
      display: flex;
      justify-content: space-between;
      align-items: center;
      padding: 10px 14px;
      border-radius: 8px;
      background: rgba(255, 255, 255, 0.02);
      border: 1px solid transparent;
      cursor: pointer;
      margin-bottom: 8px;
      transition: all 0.2s;
    }
    .coll-item:hover, .coll-item.active {
      background: rgba(0, 242, 254, 0.08);
      border-color: rgba(0, 242, 254, 0.3);
    }
    .coll-name {
      font-size: 13px;
      font-weight: 500;
      color: #f8fafc;
    }
    .coll-count {
      font-size: 11px;
      background: rgba(255, 255, 255, 0.08);
      padding: 2px 8px;
      border-radius: 12px;
      color: var(--accent);
      font-weight: 600;
    }
  </style>
</head>
<body>

  <!-- Top Header -->
  <header>
    <div class="brand">
      <div class="brand-logo">✦</div>
      <div class="brand-text">
        <h1>NOVA DB STUDIO</h1>
        <p>A database that understands the flow of your data</p>
      </div>
    </div>
    <div class="header-actions">
      <div class="status-pill">
        <div class="pulse-dot"></div>
        <span id="header-status">ONLINE — 127.0.0.1:7400</span>
      </div>
      <button class="btn" id="btn-seed" onclick="seedDemoData()">🌱 Seed Demo Fleet</button>
      <button class="btn btn-primary" onclick="simulateTrafficPulse()">⚡ Traffic Pulse</button>
    </div>
  </header>

  <!-- Metrics Strip -->
  <section class="metrics-bar">
    <div class="metric-card">
      <div class="metric-label">Total Queries</div>
      <div class="metric-value"><span id="metric-queries">0</span> <span class="metric-sub">ops</span></div>
    </div>
    <div class="metric-card">
      <div class="metric-label">Data Flow Events</div>
      <div class="metric-value"><span id="metric-events">0</span> <span class="metric-sub">emitted</span></div>
    </div>
    <div class="metric-card">
      <div class="metric-label">Writes / Mutations</div>
      <div class="metric-value"><span id="metric-writes">0</span> <span class="metric-sub">WAL synced</span></div>
    </div>
    <div class="metric-card">
      <div class="metric-label">Storage Durability</div>
      <div class="metric-value" style="font-size: 16px; margin-top: 4px;">CRC32 • ALWAYS</div>
    </div>
  </section>

  <!-- Main Container -->
  <div class="container">
    
    <!-- Sidebar: Collections -->
    <aside class="sidebar">
      <h3>Namespaces & Collections</h3>
      <div id="collections-list">
        <div class="coll-item active" onclick="loadCollection('users')">
          <span class="coll-name">📁 users</span>
          <span class="coll-count" id="count-users">4</span>
        </div>
        <div class="coll-item" onclick="loadCollection('sensors')">
          <span class="coll-name">📁 sensors</span>
          <span class="coll-count" id="count-sensors">4</span>
        </div>
      </div>
      <div style="margin-top: auto; padding-top: 16px; border-top: 1px solid var(--card-border);">
        <p style="font-size: 11px; color: var(--text-muted); line-height: 1.6;">
          Binary Port: <strong style="color:#fff">7400</strong><br>
          Web Studio: <strong style="color:var(--accent)">7401</strong><br>
          Protocol: <strong style="color:var(--emerald)">NVP1 Framed</strong>
        </p>
      </div>
    </aside>

    <!-- Workspace -->
    <main class="main-workspace">
      <!-- Tabs -->
      <div class="tabs-header">
        <button class="tab-btn active" onclick="switchTab('query')">NQL Query Studio</button>
        <button class="tab-btn" onclick="switchTab('flow')">Reactive Data Flow</button>
        <button class="tab-btn" onclick="switchTab('json')">Document Inspector</button>
      </div>

      <!-- Tab 1: Query Studio -->
      <div id="tab-query" class="tab-content">
        <div class="query-templates">
          <span class="template-tag" onclick="setQuery('FIND users WHERE points >= 80')">⚡ High Point Users</span>
          <span class="template-tag" onclick="setQuery('FIND sensors WHERE temp > 70.0')">🔥 Temperature Overheating</span>
          <span class="template-tag" onclick="setQuery('INSERT INTO sensors VALUES ({ id: \'sns_live_\' + Math.floor(Math.random()*900+100), name: \'Aux Cooler\', temp: 88.4, status: \'critical\' })')">+ Insert Live Sensor</span>
          <span class="template-tag" onclick="setQuery('UPDATE users SET role = \'Principal Architect\' WHERE points > 90')">👑 Promote Architect</span>
          <span class="template-tag" onclick="setQuery('COUNT users')">📊 Total Users</span>
        </div>
        <div class="editor-wrapper">
          <textarea id="query-input" class="query-editor" spellcheck="false">FIND users WHERE points >= 80</textarea>
        </div>
        <div class="query-toolbar">
          <button class="btn btn-primary" id="btn-run" onclick="runQuery()">▶ Execute Query (⌘ + Enter)</button>
          <div class="query-stats">
            <div>Execution Time: <span id="stat-latency">--</span></div>
            <div>Records: <span id="stat-count">--</span></div>
          </div>
        </div>
        <div class="results-panel" id="query-results">
          <p style="color: var(--text-muted);">Press "Execute Query" to inspect records...</p>
        </div>
      </div>

      <!-- Tab 2: Reactive Data Flow -->
      <div id="tab-flow" class="tab-content" style="display: none;">
        <div class="flow-pipeline">
          <div class="flow-node active">
            <h4>Client Request</h4>
            <p>NVP Binary / Web</p>
          </div>
          <div class="flow-arrow">➔</div>
          <div class="flow-node active">
            <h4>Write-Ahead Log</h4>
            <p>CRC32 • Append-Only</p>
          </div>
          <div class="flow-arrow">➔</div>
          <div class="flow-node active">
            <h4>Index Update</h4>
            <p>Hash & B-Tree</p>
          </div>
          <div class="flow-arrow">➔</div>
          <div class="flow-node active" style="border-color: var(--emerald);">
            <h4>Reactive Stream</h4>
            <p>Live WATCH Bus</p>
          </div>
        </div>

        <h4 style="font-size: 13px; color: var(--text-muted); margin-bottom: 12px;">Real-Time Mutation Stream (SSE)</h4>
        <div class="events-stream" id="events-container">
          <div class="event-card">
            <span class="event-badge badge-insert">INIT</span>
            <span class="event-info">Listening to database mutation stream on /api/events...</span>
            <span class="event-time">Ready</span>
          </div>
        </div>
      </div>

      <!-- Tab 3: JSON Inspector -->
      <div id="tab-json" class="tab-content" style="display: none;">
        <div class="results-panel" style="max-height: 520px;">
          <pre id="json-viewer" style="color: #38bdf8; line-height: 1.5;">Select a collection or execute a query to view formatted JSON records.</pre>
        </div>
      </div>

    </main>
  </div>

  <script>
    let currentTab = 'query';
    let lastQueryResults = [];

    function switchTab(tab) {
      currentTab = tab;
      document.querySelectorAll('.tab-btn').forEach(b => b.classList.remove('active'));
      document.querySelectorAll('.tab-content').forEach(c => c.style.display = 'none');
      
      const idx = tab === 'query' ? 0 : (tab === 'flow' ? 1 : 2);
      document.querySelectorAll('.tab-btn')[idx].classList.add('active');
      document.getElementById('tab-' + tab).style.display = 'block';

      if (tab === 'json') {
        document.getElementById('json-viewer').textContent = JSON.stringify(lastQueryResults, null, 2);
      }
    }

    function setQuery(q) {
      document.getElementById('query-input').value = q;
      runQuery();
    }

    async function fetchStatus() {
      try {
        const res = await fetch('/api/status');
        const data = await res.json();
        if (data.metrics) {
          document.getElementById('metric-queries').textContent = data.metrics.total_queries;
          document.getElementById('metric-events').textContent = data.metrics.total_events;
          document.getElementById('metric-writes').textContent = data.metrics.total_writes;
        }
        if (data.namespaces && data.namespaces[0]) {
          const list = document.getElementById('collections-list');
          list.innerHTML = '';
          data.namespaces[0].collections.forEach(c => {
            const el = document.createElement('div');
            el.className = 'coll-item';
            el.innerHTML = `<span class="coll-name">📁 ${c.name}</span><span class="coll-count">${c.document_count}</span>`;
            el.onclick = () => {
              setQuery(`FIND ${c.name}`);
            };
            list.appendChild(el);
          });
        }
      } catch (err) {
        console.error('Failed to fetch status:', err);
      }
    }

    async function runQuery() {
      let q = document.getElementById('query-input').value.trim();
      if (!q) return;

      // Handle JS template evaluation if present
      if (q.includes('Math.random()')) {
        try {
          q = q.replace(/Math\.floor\(Math\.random\(\)\*900\+100\)/g, Math.floor(Math.random() * 900 + 100));
        } catch (e) {}
      }

      const resultsDiv = document.getElementById('query-results');
      resultsDiv.innerHTML = '<p style="color:var(--accent);">Executing on database engine...</p>';

      try {
        const res = await fetch('/api/query', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ query: q, namespace: 'default' })
        });
        const result = await res.json();
        
        document.getElementById('stat-latency').textContent = (result.execution_time_us || 0) + ' µs';

        if (!result.success) {
          resultsDiv.innerHTML = `<p style="color:var(--crimson); font-weight:600;">Error: ${result.error}</p>`;
          return;
        }

        if (result.type === 'records') {
          lastQueryResults = result.data;
          document.getElementById('stat-count').textContent = result.count + ' document(s)';
          if (!result.data || result.data.length === 0) {
            resultsDiv.innerHTML = '<p style="color:var(--text-muted);">Query succeeded. 0 records matched.</p>';
            return;
          }

          // Build dynamic table
          const keys = Object.keys(result.data[0]);
          let html = '<table><thead><tr>';
          keys.forEach(k => html += `<th>${k}</th>`);
          html += '</tr></thead><tbody>';
          result.data.forEach(row => {
            html += '<tr>';
            keys.forEach(k => {
              const val = typeof row[k] === 'object' ? JSON.stringify(row[k]) : row[k];
              html += `<td>${val !== undefined ? val : ''}</td>`;
            });
            html += '</tr>';
          });
          html += '</tbody></table>';
          resultsDiv.innerHTML = html;
        } else if (result.type === 'success') {
          document.getElementById('stat-count').textContent = (result.affected || 0) + ' affected';
          resultsDiv.innerHTML = `<p style="color:var(--emerald);">✓ ${result.message} (${result.affected || 0} affected)</p>`;
        } else if (result.type === 'count') {
          document.getElementById('stat-count').textContent = result.count;
          resultsDiv.innerHTML = `<p style="color:var(--accent); font-size:20px; font-weight:700;">Count: ${result.count}</p>`;
        }

        fetchStatus();
      } catch (err) {
        resultsDiv.innerHTML = `<p style="color:var(--crimson);">Network error: ${err.message}</p>`;
      }
    }

    async function seedDemoData() {
      try {
        const btn = document.getElementById('btn-seed');
        btn.textContent = '🌱 Seeding...';
        const res = await fetch('/api/seed', { method: 'POST' });
        const data = await res.json();
        btn.textContent = '✓ Seeded!';
        setTimeout(() => btn.textContent = '🌱 Seed Demo Fleet', 2000);
        fetchStatus();
        setQuery('FIND users');
      } catch (e) {
        console.error(e);
      }
    }

    async function simulateTrafficPulse() {
      for (let i = 0; i < 3; i++) {
        const id = 'sns_pulse_' + Math.floor(Math.random() * 899 + 100);
        const temp = (60.0 + Math.random() * 35.0).toFixed(1);
        const q = `INSERT INTO sensors VALUES ({ id: "${id}", name: "Core Node", temp: ${temp}, status: "${temp > 80 ? 'critical' : 'nominal'}" })`;
        await fetch('/api/query', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ query: q, namespace: 'default' })
        });
      }
      fetchStatus();
    }

    // Connect Server-Sent Events stream for live Data Flow
    function initEventStream() {
      const evtSource = new EventSource('/api/events');
      const container = document.getElementById('events-container');

      evtSource.onmessage = function(e) {
        try {
          const ev = JSON.parse(e.data);
          const card = document.createElement('div');
          card.className = 'event-card';
          
          let badgeClass = 'badge-insert';
          if (ev.event_type === 'UPDATE') badgeClass = 'badge-update';
          if (ev.event_type === 'DELETE') badgeClass = 'badge-delete';

          const docSummary = ev.document ? JSON.stringify(ev.document).substring(0, 75) + '...' : '(removed)';

          card.innerHTML = `
            <span class="event-badge ${badgeClass}">${ev.event_type}</span>
            <span class="event-info">#${ev.sequence} • ${ev.namespace}.${ev.collection} [id: ${ev.document_id}] ➔ ${docSummary}</span>
            <span class="event-time">Just now</span>
          `;

          container.insertBefore(card, container.firstChild);
          if (container.children.length > 25) {
            container.removeChild(container.lastChild);
          }
        } catch (err) {
          console.error(err);
        }
      };
    }

    // Keyboard shortcut for query execution
    document.getElementById('query-input').addEventListener('keydown', function(e) {
      if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
        runQuery();
      }
    });

    // Auto-init on load
    window.addEventListener('DOMContentLoaded', () => {
      fetchStatus();
      initEventStream();
      runQuery();
      setInterval(fetchStatus, 3000);
    });
  </script>
</body>
</html>
"#;
