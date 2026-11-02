use std::{env, net::SocketAddr};

use axum::{
    Json, Router,
    extract::Path,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};

#[derive(RustEmbed)]
#[folder = "assets/"]
struct Assets;

const DEFAULT_LAYERS: &str = include_str!("../layers.toml");
const DEFAULT_TOOLS: &str = include_str!("../tools.toml");

#[derive(Serialize, Deserialize)]
struct Tools {
    #[serde(rename = "grid")]
    grids: Vec<Grid>,
}

#[derive(Serialize, Deserialize)]
struct Grid {
    name: String,
    #[serde(rename = "tool")]
    tools: Vec<Tool>,
}

#[derive(Serialize, Deserialize)]
struct Tool {
    name: String,
    description: String,
    #[serde(default)]
    href: Option<String>,
    /// live | planned
    #[serde(default = "planned")]
    status: String,
}

fn planned() -> String {
    "planned".into()
}

#[derive(Serialize, Deserialize)]
struct Config {
    map: MapView,
    #[serde(rename = "layer")]
    layers: Vec<Layer>,
}

#[derive(Serialize, Deserialize)]
struct MapView {
    center: [f64; 2],
    zoom: u8,
}

#[derive(Serialize, Deserialize)]
struct Layer {
    id: String,
    name: String,
    kind: String,
    group: String,
    url: String,
    #[serde(default)]
    attribution: String,
    #[serde(default)]
    subdomains: Vec<String>,
    #[serde(default)]
    wms_layers: Option<String>,
    #[serde(default)]
    max_zoom: Option<u8>,
    #[serde(default)]
    opacity: Option<f32>,
    #[serde(default)]
    default: bool,
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "osnt_portal=info".into()),
        )
        .init();

    let app = Router::new()
        .route("/", get(index))
        .route("/api/health", get(health))
        .route("/map", get(map_page))
        .route("/api/config", get(config))
        .route("/api/tools", get(tools))
        .route("/static/{*path}", get(static_file));

    let addr: SocketAddr = env::var("OSNT_BIND")
        .unwrap_or_else(|_| "0.0.0.0:8080".into())
        .parse()
        .expect("OSNT_BIND must be host:port");
    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
    tracing::info!("listening on http://{addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .expect("serve");
}

async fn index() -> Response {
    serve("index.html")
}

async fn map_page() -> Response {
    serve("map.html")
}

async fn static_file(Path(path): Path<String>) -> Response {
    serve(&path)
}

fn serve(path: &str) -> Response {
    match Assets::get(path) {
        Some(file) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            ([(header::CONTENT_TYPE, mime.to_string())], file.data).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn health() -> Json<Health> {
    Json(Health { status: "ok", version: env!("CARGO_PKG_VERSION") })
}

/// Tool launcher catalog: OSNT_TOOLS (re-read per request) or the embedded default.
async fn tools() -> Result<Json<Tools>, (StatusCode, String)> {
    let text = match env::var("OSNT_TOOLS") {
        Ok(path) => std::fs::read_to_string(&path)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{path}: {e}")))?,
        Err(_) => DEFAULT_TOOLS.to_string(),
    };
    toml::from_str(&text)
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("tools.toml: {e}")))
}

/// Layer catalog: OSNT_LAYERS (re-read per request, so edits apply without a restart) or the embedded default.
async fn config() -> Result<Json<Config>, (StatusCode, String)> {
    let text = match env::var("OSNT_LAYERS") {
        Ok(path) => std::fs::read_to_string(&path)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{path}: {e}")))?,
        Err(_) => DEFAULT_LAYERS.to_string(),
    };
    toml::from_str(&text)
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("layers.toml: {e}")))
}
