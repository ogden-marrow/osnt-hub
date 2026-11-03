//! Gateway trust keys as code. The master key is generated once on this machine (~/.config/osnt/gateway.key, mode 0600),
//! pushed to the Pi for the portal (/etc/osnt/gateway.key, root-only, handed over by systemd), and used here to derive the
//! per-plugin key each plugin receives at install time. See the portal's gateway.rs for how it is used.

use std::{env, fs, io::Read, os::unix::fs::PermissionsExt, path::PathBuf};

use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::Remote;

fn path() -> PathBuf {
    PathBuf::from(env::var("HOME").unwrap_or_default()).join(".config/osnt/gateway.key")
}

fn master() -> Result<String, String> {
    let p = path();
    if let Ok(t) = fs::read_to_string(&p) {
        if !t.trim().is_empty() {
            return Ok(t.trim().to_string());
        }
    }
    fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    let mut b = [0u8; 32];
    fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).map_err(|e| e.to_string())?;
    let key: String = b.iter().map(|x| format!("{x:02x}")).collect();
    fs::write(&p, &key).map_err(|e| e.to_string())?;
    fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    Ok(key)
}

/// hex(HMAC-SHA256(master, "osnt-gateway:<id>")): the key a plugin holds as OSNT_GATEWAY_KEY.
pub fn derived_key(id: &str) -> Result<String, String> {
    let mut m = Hmac::<Sha256>::new_from_slice(master()?.as_bytes()).map_err(|e| e.to_string())?;
    m.update(format!("osnt-gateway:{id}").as_bytes());
    Ok(m.finalize().into_bytes().iter().map(|x| format!("{x:02x}")).collect())
}

/// Copy the master key to the Pi for the portal.
pub fn push(r: &Remote) -> Result<(), String> {
    let key = master()?;
    r.ssh("sudo install -d -m 0755 /etc/osnt && sudo sh -c 'umask 077; cat > /etc/osnt/gateway.key' && sudo chmod 0600 /etc/osnt/gateway.key", Some(&key))
}
