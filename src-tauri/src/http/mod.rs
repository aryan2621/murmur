//! Localhost REST API (`127.0.0.1` only) for agents and the MCP stdio proxy.

use crate::integration::{self, AskRequest, SpeakRequest};
use crate::Murmur;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;
use tauri::AppHandle;

struct ApiState {
    murmur: Arc<Murmur>,
}

pub fn start(app: &AppHandle, murmur: Arc<Murmur>) {
    let handle = app.clone();
    std::thread::spawn(move || {
        if let Err(e) = run_server(handle, murmur) {
            mlog!("HTTP API stopped: {e}");
        }
    });
}

fn run_server(_app: AppHandle, murmur: Arc<Murmur>) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        loop {
            let cfg = murmur.shared.cfg();
            if !cfg.http_enabled {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }
            let addr = format!("127.0.0.1:{}", cfg.http_port);
            let state = Arc::new(ApiState { murmur: murmur.clone() });
            let app = Router::new()
                .route("/v1/status", get(status))
                .route("/v1/last-dictation", get(last_dictation))
                .route("/v1/ask", post(ask))
                .route("/v1/speak", post(speak))
                .route("/v1/stop", post(stop))
                .with_state(state);
            mlog!("HTTP API listening on http://{addr}");
            let (watched, port) = (murmur.clone(), cfg.http_port);
            // Stop serving when the API is switched off or moved, so the loop can rebind.
            let changed = async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    let c = watched.shared.cfg();
                    if !c.http_enabled || c.http_port != port {
                        break;
                    }
                }
            };
            match tokio::net::TcpListener::bind(&addr).await {
                Ok(listener) => {
                    if let Err(e) = axum::serve(listener, app).with_graceful_shutdown(changed).await {
                        mlog!("HTTP serve error: {e}");
                    }
                }
                Err(e) => mlog!("HTTP bind failed on {addr}: {e}"),
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
    Ok(())
}

/// Bearer token plus a Host check: a web page that rebinds its DNS name to 127.0.0.1 still
/// sends its own Host, so it can't reach the API.
fn authorize(state: &ApiState, headers: &HeaderMap) -> Result<(), StatusCode> {
    let header = |name| headers.get(name).and_then(|v: &axum::http::HeaderValue| v.to_str().ok());
    let port = state.murmur.shared.cfg().http_port;
    let host_ok = matches!(header(axum::http::header::HOST), Some(h)
        if h == format!("127.0.0.1:{port}") || h == format!("localhost:{port}"));
    if !host_ok {
        return Err(StatusCode::FORBIDDEN);
    }
    integration::check_auth(&state.murmur.shared, header(axum::http::header::AUTHORIZATION))
}

async fn status(State(state): State<Arc<ApiState>>, headers: HeaderMap) -> Result<Json<integration::StatusResponse>, StatusCode> {
    authorize(&state, &headers)?;
    Ok(Json(integration::status(&state.murmur.shared)))
}

async fn last_dictation(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
) -> Result<Json<integration::DictateResult>, StatusCode> {
    authorize(&state, &headers)?;
    Ok(Json(integration::last_dictation(&state.murmur)))
}

async fn ask(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(body): Json<AskRequest>,
) -> Result<StatusCode, StatusCode> {
    authorize(&state, &headers)?;
    integration::ask(&state.murmur, &body.question).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::ACCEPTED)
}

async fn speak(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(body): Json<SpeakRequest>,
) -> Result<StatusCode, StatusCode> {
    authorize(&state, &headers)?;
    integration::speak(&state.murmur, &body.text, body.force)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::OK)
}

async fn stop(State(state): State<Arc<ApiState>>, headers: HeaderMap) -> Result<Json<integration::StopResponse>, StatusCode> {
    authorize(&state, &headers)?;
    Ok(Json(integration::stop(&state.murmur)))
}
