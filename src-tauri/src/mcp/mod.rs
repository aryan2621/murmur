//! Minimal MCP server over stdio; forwards tool calls to the in-app localhost REST API.

use crate::config;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

struct Client {
    base: String,
    http: reqwest::blocking::Client,
    token: String,
}

impl Client {
    fn new(cfg: config::Config) -> Self {
        Self {
            base: format!("http://127.0.0.1:{}", cfg.http_port),
            http: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("http client"),
            token: cfg.http_token,
        }
    }

    fn auth(&self, req: reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder {
        if self.token.is_empty() {
            req
        } else {
            req.header("Authorization", format!("Bearer {}", self.token))
        }
    }

    fn get(&self, path: &str) -> Result<String, String> {
        self.auth(self.http.get(format!("{}{path}", self.base)))
            .send()
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .text()
            .map_err(|e| e.to_string())
    }

    fn post_json(&self, path: &str, body: Value) -> Result<(), String> {
        self.auth(self.http.post(format!("{}{path}", self.base)).json(&body))
            .send()
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

fn tools_list() -> Value {
    json!({
        "tools": [
            {"name": "murmur_get_status", "description": "Model loaded, assistant, version", "inputSchema": {"type": "object", "properties": {}}},
            {"name": "murmur_ask", "description": "Ask the assistant", "inputSchema": {"type": "object", "properties": {"question": {"type": "string"}}, "required": ["question"]}},
            {"name": "murmur_speak", "description": "Speak text", "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}},
            {"name": "murmur_stop", "description": "Stop speech and cancel assistant work", "inputSchema": {"type": "object", "properties": {}}},
            {"name": "murmur_dictate", "description": "Last dictated text", "inputSchema": {"type": "object", "properties": {}}},
        ]
    })
}

fn http_unavailable() -> Value {
    json!({
        "content": [{"type": "text", "text": "Could not reach Murmur. Launch the app and enable Settings → Advanced → Local HTTP API."}],
        "isError": true
    })
}

fn call_tool(client: &Client, name: &str, args: &Value) -> Value {
    let ok_text = |t: String| json!({"content": [{"type": "text", "text": t}]});
    match name {
        "murmur_get_status" => match client.get("/v1/status") {
            Ok(t) => ok_text(t),
            Err(_) => http_unavailable(),
        },
        "murmur_dictate" => match client.get("/v1/last-dictation") {
            Ok(t) => ok_text(t),
            Err(_) => http_unavailable(),
        },
        "murmur_ask" => {
            let question = args.get("question").and_then(|v| v.as_str()).unwrap_or("").trim();
            if question.is_empty() {
                return json!({"content": [{"type": "text", "text": "question is required"}], "isError": true});
            }
            match client.post_json("/v1/ask", json!({ "question": question })) {
                Ok(()) => ok_text("Question sent; answer streams in the Murmur app.".into()),
                Err(_) => http_unavailable(),
            }
        }
        "murmur_speak" => {
            let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("").trim();
            if text.is_empty() {
                return json!({"content": [{"type": "text", "text": "text is required"}], "isError": true});
            }
            match client.post_json("/v1/speak", json!({ "text": text, "force": true })) {
                Ok(()) => ok_text("Speaking.".into()),
                Err(_) => http_unavailable(),
            }
        }
        "murmur_stop" => match client.post_json("/v1/stop", json!({})) {
            Ok(()) => ok_text("Stopped.".into()),
            Err(_) => http_unavailable(),
        },
        _ => json!({"content": [{"type": "text", "text": "unknown tool"}], "isError": true}),
    }
}

#[derive(Deserialize)]
struct RpcRequest {
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

fn respond(out: &mut impl Write, id: Value, result: Value) -> io::Result<()> {
    let resp = JsonRpcResponse { jsonrpc: "2.0", id, result: Some(result), error: None };
    writeln!(out, "{}", serde_json::to_string(&resp).unwrap())?;
    out.flush()
}

fn respond_err(out: &mut impl Write, id: Value, message: &str) -> io::Result<()> {
    let resp = JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(JsonRpcError { code: -32601, message: message.into() }),
    };
    writeln!(out, "{}", serde_json::to_string(&resp).unwrap())?;
    out.flush()
}

pub fn run_stdio() -> anyhow::Result<()> {
    let cfg = config::load();
    if !cfg.mcp_enabled {
        anyhow::bail!("MCP is off. Turn it on in Murmur → Settings → Advanced (agents).");
    }
    let client = Client::new(cfg);
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let req: RpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let id = req.id.clone().unwrap_or(Value::Null);
        let result = match req.method.as_str() {
            "initialize" => json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "murmur", "version": env!("CARGO_PKG_VERSION") }
            }),
            "notifications/initialized" | "initialized" => continue,
            "tools/list" => tools_list(),
            "tools/call" => {
                let name = req.params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let args = req.params.get("arguments").cloned().unwrap_or(json!({}));
                call_tool(&client, name, &args)
            }
            "ping" => json!({}),
            // Notifications have no id and never get a reply.
            _ if req.id.is_none() => continue,
            _ => {
                respond_err(&mut stdout, id, &format!("method not found: {}", req.method))?;
                continue;
            }
        };
        respond(&mut stdout, id, result)?;
    }
    Ok(())
}
