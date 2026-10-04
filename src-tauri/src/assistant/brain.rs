//! Where answers come from: the built-in AI (llama.cpp's server, bundled with Murmur and run in
//! the background on a local port) or, if chosen in Settings, an Ollama the user already runs.
//! Both stream replies token by token, and nothing leaves the Mac either way.

use crate::assistant::llm::{Message, Ollama};
use crate::model;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Context window for the built-in model: room for a conversation plus a few views of a web page
/// or window while working through a task. The memory it takes is halved by an 8-bit cache.
const CONTEXT_TOKENS: u32 = 16384;
/// Loading the model from disk takes a few seconds; give up if the server isn't up by then.
const START_TIMEOUT: Duration = Duration::from_secs(90);

struct Running {
    child: Child,
    port: u16,
    /// The model has loaded and the server answers. Until then questions wait in `ensure`.
    ready: bool,
}

/// llama.cpp's server, started on demand and stopped when idle to give its memory back.
pub struct LocalServer {
    running: Mutex<Option<Running>>,
    /// Serialises starts so two questions in a row don't launch two servers.
    starting: Mutex<()>,
    last_used: Mutex<Instant>,
    /// Which built-in model to run (`model::BRAINS` id).
    model: Mutex<String>,
    client: reqwest::blocking::Client,
}

impl LocalServer {
    fn new() -> Self {
        let client = reqwest::blocking::Client::builder().connect_timeout(Duration::from_secs(3)).timeout(None).build().expect("http client");
        Self { running: Mutex::new(None), starting: Mutex::new(()), last_used: Mutex::new(Instant::now()), model: Mutex::new(String::new()), client }
    }

    /// The bundled `llama-server`, next to Murmur's own executable (where Tauri puts sidecars).
    fn executable() -> Result<PathBuf> {
        let exe = std::env::current_exe()?;
        let path = exe.parent().ok_or_else(|| anyhow!("no app folder"))?.join("llama-server");
        if path.exists() {
            Ok(path)
        } else {
            Err(anyhow!("The built-in AI is missing from this copy of Murmur"))
        }
    }

    fn pid_file() -> PathBuf {
        crate::config::data_dir().join("llama-server.pid")
    }

    /// Port of a running, healthy server, starting one if needed.
    fn ensure(&self) -> Result<u16> {
        *self.last_used.lock().unwrap() = Instant::now();
        if let Some(port) = self.alive_port() {
            return Ok(port);
        }
        let _guard = self.starting.lock().unwrap();
        if let Some(port) = self.alive_port() {
            return Ok(port);
        }
        let id = self.model.lock().unwrap().clone();
        let model = model::brain_path(&crate::config::data_dir(), &id);
        if !model.exists() {
            return Err(anyhow!("The AI model isn't downloaded yet. Download it in Murmur → Assistant."));
        }
        kill_leftover();
        let port = free_port()?;
        let log = std::fs::File::create(crate::config::data_dir().join("llama-server.log"))?;
        let started = Instant::now();
        let child = Command::new(Self::executable()?)
            .args(["--model", &model.to_string_lossy()])
            .args(["--host", "127.0.0.1", "--port", &port.to_string()])
            .args(["--ctx-size", &CONTEXT_TOKENS.to_string(), "--n-gpu-layers", "999", "--parallel", "1"])
            .args(["--flash-attn", "on", "--cache-type-k", "q8_0", "--cache-type-v", "q8_0"])
            // Qwen3 8B thinks out loud before answering unless told not to; spoken answers can't wait.
            .args(["--reasoning-budget", "0"])
            // Use the chat template stored in the model file.
            .arg("--jinja")
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .spawn()?;
        let _ = std::fs::write(Self::pid_file(), child.id().to_string());
        *self.running.lock().unwrap() = Some(Running { child, port, ready: false });

        // Wait until the model is loaded (/health answers 200).
        while started.elapsed() < START_TIMEOUT {
            if self.exited() {
                self.stop();
                return Err(anyhow!("The built-in AI stopped while starting (see llama-server.log)"));
            }
            if self.client.get(format!("http://127.0.0.1:{port}/health")).send().is_ok_and(|r| r.status().is_success()) {
                mlog!("built-in AI ready in {} ms (port {port})", started.elapsed().as_millis());
                if let Some(r) = self.running.lock().unwrap().as_mut() {
                    r.ready = true;
                }
                return Ok(port);
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        self.stop();
        Err(anyhow!("The built-in AI took too long to start"))
    }

    /// Port of a server that is up and has its model loaded.
    fn alive_port(&self) -> Option<u16> {
        let mut running = self.running.lock().unwrap();
        let r = running.as_mut()?;
        match r.child.try_wait() {
            Ok(None) => r.ready.then_some(r.port),
            _ => {
                *running = None;
                None
            }
        }
    }

    fn exited(&self) -> bool {
        self.running.lock().unwrap().as_mut().is_none_or(|r| !matches!(r.child.try_wait(), Ok(None)))
    }

    pub fn stop(&self) {
        if let Some(mut r) = self.running.lock().unwrap().take() {
            let _ = r.child.kill();
            let _ = r.child.wait();
            let _ = std::fs::remove_file(Self::pid_file());
            mlog!("built-in AI stopped");
        }
    }

    /// Stops the server after `minutes` without a question (0 = never).
    pub fn stop_if_idle(&self, minutes: u64) {
        if minutes > 0 && self.last_used.lock().unwrap().elapsed() >= Duration::from_secs(minutes * 60) && self.alive_port().is_some() {
            mlog!("built-in AI idle for {minutes} minutes");
            self.stop();
        }
    }

    /// Streams a spoken reply. Spoken answers are short, so the length cap also bounds how long
    /// a reply that goes wrong can ramble.
    fn chat(&self, messages: &[Message], temperature: f32, mut on_text: impl FnMut(&str) -> bool) -> Result<()> {
        let port = self.ensure()?;
        // Sampling Qwen recommends for its instruct models, with the presence penalty it suggests
        // against endless repetition.
        let body = json!({
            "messages": messages.iter().map(Message::to_json).collect::<Vec<_>>(),
            "stream": true,
            "temperature": temperature,
            "top_p": 0.8,
            "top_k": 20,
            "presence_penalty": 1.0,
            "max_tokens": 350,
        });
        let resp = self.post(port, &body)?;
        // Server-sent events: "data: {...}" lines, ending with "data: [DONE]".
        for line in BufReader::new(resp).lines() {
            let line = line?;
            let Some(data) = line.strip_prefix("data: ") else { continue };
            if data.trim() == "[DONE]" {
                break;
            }
            let chunk: Value = serde_json::from_str(data)?;
            if let Some(text) = chunk["choices"][0]["delta"]["content"].as_str() {
                if !text.is_empty() && !on_text(text) {
                    return Ok(());
                }
            }
        }
        *self.last_used.lock().unwrap() = Instant::now();
        Ok(())
    }

    /// A reply forced, token by token, to match the JSON `schema`.
    fn decide(&self, messages: &[Message], schema: &Value) -> Result<Value> {
        let port = self.ensure()?;
        let body = json!({
            "messages": messages.iter().map(Message::to_json).collect::<Vec<_>>(),
            "temperature": 0,
            "max_tokens": 200,
            "response_format": { "type": "json_schema", "json_schema": { "name": "decision", "schema": schema } },
        });
        let resp: Value = self.post(port, &body)?.json()?;
        *self.last_used.lock().unwrap() = Instant::now();
        let text = resp["choices"][0]["message"]["content"].as_str().unwrap_or_default();
        Ok(serde_json::from_str(text)?)
    }

    fn post(&self, port: u16, body: &Value) -> Result<reqwest::blocking::Response> {
        let resp = self.client.post(format!("http://127.0.0.1:{port}/v1/chat/completions")).json(body).send()?;
        if !resp.status().is_success() {
            let status = resp.status();
            return Err(anyhow!("The built-in AI failed ({status}): {}", resp.text().unwrap_or_default()));
        }
        Ok(resp)
    }

    /// Reads the instructions into the server's cache without answering, so the first real
    /// question only has to read itself.
    fn prime(&self, messages: &[Message]) -> Result<()> {
        let port = self.ensure()?;
        let body = json!({ "messages": messages.iter().map(Message::to_json).collect::<Vec<_>>(), "max_tokens": 1 });
        self.post(port, &body)?;
        Ok(())
    }
}

/// A server left behind by a crash of an earlier Murmur would hold a few GB of memory: stop it.
fn kill_leftover() {
    let Ok(pid) = std::fs::read_to_string(LocalServer::pid_file()) else { return };
    let pid = pid.trim();
    let is_ours = Command::new("ps").args(["-p", pid, "-o", "comm="]).output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("llama-server"));
    if is_ours {
        let _ = Command::new("kill").arg(pid).status();
        mlog!("stopped a leftover built-in AI (pid {pid})");
    }
    let _ = std::fs::remove_file(LocalServer::pid_file());
}

fn free_port() -> Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

/// The AI Murmur answers with, as chosen in Settings.
pub struct Brain {
    pub local: LocalServer,
    pub ollama: Ollama,
    use_ollama: AtomicBool,
}

impl Brain {
    pub fn new(cfg: &crate::config::Config) -> Self {
        let brain = Self { local: LocalServer::new(), ollama: Ollama::new(&cfg.ollama_url, &cfg.llm_model, &cfg.keep_alive), use_ollama: AtomicBool::new(false) };
        brain.configure(cfg);
        brain
    }

    pub fn configure(&self, cfg: &crate::config::Config) {
        self.ollama.configure(&cfg.ollama_url, &cfg.llm_model, &cfg.keep_alive);
        let id = model::brain(&cfg.builtin_model).id.to_string();
        let old = std::mem::replace(&mut *self.local.model.lock().unwrap(), id.clone());
        if !old.is_empty() && old != id {
            // Another built-in model was chosen: the next question starts it.
            mlog!("built-in AI changed from {old} to {id}");
            self.local.stop();
        }
        let use_ollama = cfg.brain == "ollama";
        if !self.use_ollama.swap(use_ollama, Ordering::SeqCst) && use_ollama {
            // Switched to Ollama: free the built-in model's memory.
            self.local.stop();
        }
    }

    pub fn uses_ollama(&self) -> bool {
        self.use_ollama.load(Ordering::SeqCst)
    }

    /// Streams a reply; `on_text` returns false to stop early.
    pub fn chat(&self, messages: &[Message], temperature: f32, on_text: impl FnMut(&str) -> bool) -> Result<()> {
        if self.uses_ollama() {
            self.ollama.chat(messages, temperature, on_text)
        } else {
            self.local.chat(messages, temperature, on_text)
        }
    }

    /// A reply that is JSON matching `schema`: the model can't write anything else.
    pub fn decide(&self, messages: &[Message], schema: &Value) -> Result<Value> {
        if self.uses_ollama() {
            self.ollama.decide(messages, schema)
        } else {
            self.local.decide(messages, schema)
        }
    }

    /// Gets the model into memory ahead of the first question, with the instructions (`prefix`,
    /// the same at the start of every question) already read.
    pub fn warm_up(&self, prefix: &[Message]) -> Result<()> {
        if self.uses_ollama() {
            self.ollama.warm_up()
        } else {
            self.local.prime(prefix)
        }
    }

    /// Name shown in History.
    pub fn model_name(&self) -> String {
        if self.uses_ollama() {
            self.ollama.model()
        } else {
            model::brain(&self.local.model.lock().unwrap()).label.to_string()
        }
    }
}
