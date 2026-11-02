use std::{env, net::SocketAddr};

use axum::{
    Json, Router,
    extract::Path,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use rust_embed::RustEmbed;
use serde::Serialize;

#[derive(RustEmbed)]
#[folder = "assets/"]
struct Assets;

#[derive(Serialize)]
struct MapConfig {
    tile_url: String,
    attribution: &'static str,
    center: [f64; 2],
    zoom: u8,
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
        .route("/api/map-config", get(map_config))
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

/// Tile source is configurable so it can later point at a self-hosted tile server.
async fn map_config() -> Json<MapConfig> {
    Json(MapConfig {
        tile_url: env::var("OSNT_TILE_URL")
            .unwrap_or_else(|_| "https://tile.openstreetmap.org/{z}/{x}/{y}.png".into()),
        attribution: "&copy; <a href=\"https://www.openstreetmap.org/copyright\">OpenStreetMap</a> contributors",
        center: [20.0, 0.0],
        zoom: 2,
    })
}
