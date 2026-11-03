//! Gateway trust: the portal is the only place users sign in, so it vouches for them to plugins.
//!
//! Every proxied request carries `X-OSNT-Gateway: v1.<unix time>.<hmac>`, where the HMAC-SHA256 key is derived per
//! plugin from a master key only the portal (and the installer on your computer) holds:
//! `key(plugin) = HMAC(master, "osnt-gateway:" + id)`. A plugin gets only its own derived key, so it can verify requests
//! meant for it but cannot forge requests to any other plugin. The signature covers the method and the forwarded path.

use std::{env, fs};

use hmac::{Hmac, Mac};
use sha2::Sha256;

fn master() -> Option<Vec<u8>> {
    let from_file = env::var("CREDENTIALS_DIRECTORY").ok().and_then(|d| fs::read_to_string(format!("{d}/gateway.key")).ok());
    let text = from_file.or_else(|| env::var("OSNT_GATEWAY_MASTER").ok())?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.as_bytes().to_vec())
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("hmac accepts any key length");
    m.update(data);
    m.finalize().into_bytes().to_vec()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The key a plugin holds (as OSNT_GATEWAY_KEY): hex of HMAC(master, "osnt-gateway:<id>").
pub fn derived_key_hex(master: &[u8], id: &str) -> String {
    hex(&hmac(master, format!("osnt-gateway:{id}").as_bytes()))
}

pub fn sign(id: &str, method: &str, path_and_query: &str) -> Option<String> {
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let key = derived_key_hex(&master()?, id);
    let mac = hex(&hmac(key.as_bytes(), format!("{ts}.{method}.{path_and_query}").as_bytes()));
    Some(format!("v1.{ts}.{mac}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_is_stable_and_bound_to_plugin_and_request() {
        let master = b"test-master";
        let key_a = derived_key_hex(master, "credentials");
        assert_eq!(key_a, derived_key_hex(master, "credentials"));
        assert_ne!(key_a, derived_key_hex(master, "adsb"), "each plugin gets a different key");
        let m1 = hex(&hmac(key_a.as_bytes(), b"100.GET./api/credentials"));
        let m2 = hex(&hmac(key_a.as_bytes(), b"100.PUT./api/credentials"));
        assert_ne!(m1, m2, "method is covered");
    }
}
