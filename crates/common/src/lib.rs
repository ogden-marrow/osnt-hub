//! Helpers shared by OSNT plugins.

use std::{sync::Arc, time::Duration};

/// HTTP(S) client with pure-Rust TLS. `user_agent` should identify the plugin; upstreams are free community services.
pub fn http_agent(user_agent: &str) -> ureq::Agent {
    let provider = Arc::new(rustls_rustcrypto::provider());
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::Rustls)
        .unversioned_rustls_crypto_provider(provider)
        .build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_global(Some(Duration::from_secs(15)))
        .user_agent(user_agent)
        .build()
        .into()
}
