use std::{env, net::SocketAddr, path::PathBuf, time::Duration};

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Request},
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Redirect, Response},
    routing::{any, get},
};
use hyper_util::{client::legacy::Client, rt::TokioExecutor};
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
    /// live | offline | planned
    #[serde(default = "planned")]
    status: String,
}

fn planned() -> String {
    "planned".into()
}

/// A plugin is a separate local service; its manifest tells the core where it listens and how to show it.
#[derive(Deserialize)]
struct Manifest {
    id: String,
    name: String,
    description: String,
    grid: String,
    port: u16,
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
        .route("/map", get(map_page))
        .route("/api/health", get(health))
        .route("/api/config", get(config))
        .route("/api/tools", get(tools))
        .route("/p/{id}", get(plugin_root_redirect))
        .route("/p/{id}/", any(proxy))
        .route("/p/{id}/{*rest}", any(proxy))
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

fn read_override(var: &str, default: &str) -> Result<String, (StatusCode, String)> {
    match env::var(var) {
        Ok(path) => std::fs::read_to_string(&path)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{path}: {e}"))),
        Err(_) => Ok(default.to_string()),
    }
}

/// Plugin manifests live in OSNT_PLUGINS_DIR (default /etc/osnt/plugins.d), one TOML file each.
/// Re-read on every request, so installing a manifest makes a plugin appear without restarting the core.
fn manifests() -> Vec<Manifest> {
    let dir = PathBuf::from(env::var("OSNT_PLUGINS_DIR").unwrap_or_else(|_| "/etc/osnt/plugins.d".into()));
    let Ok(entries) = std::fs::read_dir(&dir) else { return vec![] };
    let mut out: Vec<Manifest> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
        .filter_map(|e| {
            let text = std::fs::read_to_string(e.path()).ok()?;
            toml::from_str(&text)
                .inspect_err(|err| tracing::warn!("bad manifest {}: {err}", e.path().display()))
                .ok()
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

async fn is_up(port: u16) -> bool {
    tokio::time::timeout(Duration::from_millis(300), tokio::net::TcpStream::connect(("127.0.0.1", port)))
        .await
        .is_ok_and(|r| r.is_ok())
}

/// Launcher grids from tools.toml, with each installed plugin's card merged in
/// (replacing a "planned" card of the same name, or added to its grid).
async fn tools() -> Result<Json<Tools>, (StatusCode, String)> {
    let text = read_override("OSNT_TOOLS", DEFAULT_TOOLS)?;
    let mut tools: Tools = toml::from_str(&text)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("tools.toml: {e}")))?;

    for m in manifests() {
        let card = Tool {
            name: m.name.clone(),
            description: m.description,
            href: Some(format!("/p/{}/", m.id)),
            status: if is_up(m.port).await { "live" } else { "offline" }.into(),
        };
        let grid = match tools.grids.iter().position(|g| g.name == m.grid) {
            Some(i) => &mut tools.grids[i],
            None => {
                tools.grids.push(Grid { name: m.grid, tools: vec![] });
                tools.grids.last_mut().unwrap()
            }
        };
        match grid.tools.iter().position(|t| t.name == m.name) {
            Some(i) => grid.tools[i] = card,
            None => grid.tools.push(card),
        }
    }
    Ok(Json(tools))
}

/// Layer catalog: OSNT_LAYERS (re-read per request, so edits apply without a restart) or the embedded default.
async fn config() -> Result<Json<Config>, (StatusCode, String)> {
    let text = read_override("OSNT_LAYERS", DEFAULT_LAYERS)?;
    toml::from_str(&text)
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("layers.toml: {e}")))
}

async fn plugin_root_redirect(Path(id): Path<String>) -> Redirect {
    Redirect::permanent(&format!("/p/{id}/"))
}

/// Reverse proxy: /p/<id>/<rest> -> 127.0.0.1:<port>/<rest>. Plugins only ever listen on loopback.
async fn proxy(req: Request) -> Response {
    let path = req.uri().path().to_owned();
    let mut parts = path.trim_start_matches("/p/").splitn(2, '/');
    let id = parts.next().unwrap_or_default();
    let rest = parts.next().unwrap_or_default();

    let Some(m) = manifests().into_iter().find(|m| m.id == id) else {
        return (StatusCode::NOT_FOUND, "no such plugin").into_response();
    };
    let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
    let Ok(uri) = format!("http://127.0.0.1:{}/{rest}{query}", m.port).parse::<Uri>() else {
        return StatusCode::BAD_REQUEST.into_response();
    };

    let (mut head, body) = req.into_parts();
    head.uri = uri;
    head.headers.remove(header::HOST);
    let req = Request::from_parts(head, body);

    let client: Client<_, Body> = Client::builder(TokioExecutor::new()).build_http();
    match client.request(req).await {
        Ok(res) => res.map(Body::new),
        Err(e) => {
            tracing::warn!("plugin {id} unreachable: {e}");
            (StatusCode::BAD_GATEWAY, format!("plugin {id} is not running")).into_response()
        }
    }
}
