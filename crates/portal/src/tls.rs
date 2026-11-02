//! HTTPS for the portal, using certificate files handed over by systemd (LoadCredential) or OSNT_TLS_DIR.
//! The certificate is issued by your own CA (`cargo deploy https init`); the private key never leaves the Pi
//! except through that command. Pure-Rust TLS: no C toolchain needed to build for the Pi.

use std::{env, fs, io::BufReader, net::SocketAddr, path::PathBuf, sync::Arc};

use axum::{Router, extract::Request, response::Redirect};
use hyper_util::{rt::{TokioExecutor, TokioIo}, server::conn::auto::Builder, service::TowerToHyperService};
use tokio_rustls::TlsAcceptor;

fn cred_dir() -> Option<PathBuf> {
    env::var("OSNT_TLS_DIR").ok().or_else(|| env::var("CREDENTIALS_DIRECTORY").ok()).map(PathBuf::from)
}

fn load_config() -> Result<Arc<rustls::ServerConfig>, String> {
    let dir = cred_dir().ok_or("no credentials directory")?;
    let cert_pem = fs::read(dir.join("tls.crt")).map_err(|e| format!("tls.crt: {e}"))?;
    let key_pem = fs::read(dir.join("tls.key")).map_err(|e| format!("tls.key: {e}"))?;
    let certs = rustls_pemfile::certs(&mut BufReader::new(&cert_pem[..])).collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
    let key = rustls_pemfile::private_key(&mut BufReader::new(&key_pem[..])).map_err(|e| e.to_string())?.ok_or("no private key in tls.key")?;
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(rustls_rustcrypto::provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("certificate/key rejected: {e}"))?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

/// Serve `app` over HTTPS on OSNT_TLS_BIND, if a certificate is available. Returns false when TLS is not configured.
pub async fn spawn(app: Router) -> bool {
    let Ok(bind) = env::var("OSNT_TLS_BIND") else { return false };
    let config = match load_config() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("HTTPS disabled: {e}");
            return false;
        }
    };
    let addr: SocketAddr = bind.parse().expect("OSNT_TLS_BIND must be host:port");
    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind HTTPS");
    tracing::info!("HTTPS on https://{addr}");
    let acceptor = TlsAcceptor::from(config);
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else { continue };
            let (acceptor, app) = (acceptor.clone(), app.clone());
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(tcp).await else { return };
                let _ = Builder::new(TokioExecutor::new()).serve_connection_with_upgrades(TokioIo::new(tls), TowerToHyperService::new(app)).await;
            });
        }
    });
    true
}

/// Plain-HTTP listener that only redirects to HTTPS (OSNT_HTTP_BIND, host from OSNT_PUBLIC_HOST).
pub async fn spawn_redirect() {
    let (Ok(bind), Ok(host)) = (env::var("OSNT_HTTP_BIND"), env::var("OSNT_PUBLIC_HOST")) else { return };
    let addr: SocketAddr = bind.parse().expect("OSNT_HTTP_BIND must be host:port");
    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind HTTP redirect");
    let app = Router::new().fallback(move |req: Request| {
        let host = host.clone();
        async move {
            let target = req.uri().path_and_query().map(|pq| pq.as_str().to_string()).unwrap_or_else(|| "/".into());
            Redirect::permanent(&format!("https://{host}{target}"))
        }
    });
    tracing::info!("HTTP on {addr} redirects to https://");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
}
