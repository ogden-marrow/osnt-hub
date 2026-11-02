//! Your own CA and the Pi's HTTPS certificate, as code.
//!
//!   cargo deploy https init     create the CA (once) and a certificate for the Pi, push it, print how to trust the CA
//!   cargo deploy https renew    issue a fresh certificate from the same CA and push it
//!   cargo deploy https ca       print the path of the CA file to install on your devices
//!
//! Everything lives under ~/.config/osnt/ca/. The CA private key never leaves this machine; only the Pi's
//! certificate and key are copied to the Pi (/etc/osnt/tls, handed to the portal through systemd credentials).
//! The CA is name-constrained to ".local", so even if its key leaked it could not impersonate real websites.

use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    time::{Duration, SystemTime},
};

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, GeneralSubtree, IsCa, KeyPair,
    KeyUsagePurpose, NameConstraints, date_time_ymd,
};

use crate::Remote;

/// Apple rejects server certificates valid for longer than 825 days, so stay under it.
const SERVER_VALID_DAYS: i64 = 800;
const RENEW_AFTER: Duration = Duration::from_secs(700 * 24 * 3600);

pub fn host() -> String {
    env::var("OSNT_HOST").unwrap_or_else(|_| "osnt-pi.local".into())
}

fn dir() -> PathBuf {
    PathBuf::from(env::var("HOME").unwrap_or_default()).join(".config/osnt/ca")
}

pub fn ca_cert_path() -> PathBuf {
    dir().join("osnt-ca.crt")
}

fn write_secret(path: &PathBuf, data: &str) -> Result<(), String> {
    fs::write(path, data).map_err(|e| format!("{}: {e}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())
}

fn today_plus(days: i64) -> time::OffsetDateTime {
    time::OffsetDateTime::now_utc() + time::Duration::days(days)
}

fn ca_params() -> Result<CertificateParams, String> {
    let mut p = CertificateParams::new(Vec::<String>::new()).map_err(|e| e.to_string())?;
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "OSNT local CA");
    dn.push(DnType::OrganizationName, "OSNT");
    p.distinguished_name = dn;
    p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    p.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    p.name_constraints = Some(NameConstraints { permitted_subtrees: vec![GeneralSubtree::DnsName("local".into())], excluded_subtrees: vec![] });
    p.not_before = date_time_ymd(2026, 1, 1);
    p.not_after = today_plus(3650);
    Ok(p)
}

/// Create the CA if it does not exist yet. Returns true when it was just created.
fn ensure_ca() -> Result<bool, String> {
    let d = dir();
    if d.join("ca.key").exists() && d.join("osnt-ca.crt").exists() {
        return Ok(false);
    }
    fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    fs::set_permissions(&d, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    let key = KeyPair::generate().map_err(|e| e.to_string())?;
    let cert = ca_params()?.self_signed(&key).map_err(|e| e.to_string())?;
    write_secret(&d.join("ca.key"), &key.serialize_pem())?;
    fs::write(d.join("osnt-ca.crt"), cert.pem()).map_err(|e| e.to_string())?;
    Ok(true)
}

fn server_paths(host: &str) -> (PathBuf, PathBuf) {
    (dir().join(format!("{host}.crt")), dir().join(format!("{host}.key")))
}

fn issue_server(host: &str) -> Result<(), String> {
    let ca_key = KeyPair::from_pem(&fs::read_to_string(dir().join("ca.key")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let ca_pem = fs::read_to_string(dir().join("osnt-ca.crt")).map_err(|e| e.to_string())?;
    // Rebuild the issuer from the stored CA certificate; the CA file you installed on devices is never regenerated.
    let issuer = CertificateParams::from_ca_cert_pem(&ca_pem).map_err(|e| e.to_string())?.self_signed(&ca_key).map_err(|e| e.to_string())?;

    let key = KeyPair::generate().map_err(|e| e.to_string())?;
    let mut p = CertificateParams::new(vec![host.to_string()]).map_err(|e| e.to_string())?;
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, host);
    p.distinguished_name = dn;
    p.is_ca = IsCa::NoCa;
    p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    p.not_before = today_plus(-1);
    p.not_after = today_plus(SERVER_VALID_DAYS);
    let cert = p.signed_by(&key, &issuer, &ca_key).map_err(|e| e.to_string())?;
    let (crt, k) = server_paths(host);
    fs::write(crt, cert.pem()).map_err(|e| e.to_string())?;
    write_secret(&k, &key.serialize_pem())
}

fn stale(host: &str) -> bool {
    let (crt, key) = server_paths(host);
    match fs::metadata(&crt).and_then(|m| m.modified()) {
        Ok(t) if key.exists() => SystemTime::now().duration_since(t).map(|age| age > RENEW_AFTER).unwrap_or(false),
        _ => true,
    }
}

/// Make sure the CA and a fresh-enough certificate exist locally. Returns true if the CA was just created.
pub fn ensure(host: &str, force_renew: bool) -> Result<bool, String> {
    let created = ensure_ca()?;
    if force_renew || stale(host) {
        eprintln!("==> issuing a certificate for {host}");
        issue_server(host)?;
    }
    Ok(created)
}

/// Copy the Pi's certificate and key to /etc/osnt/tls (root-only key); systemd hands them to the portal.
pub fn push(r: &Remote, host: &str) -> Result<(), String> {
    let (crt, key) = server_paths(host);
    let (crt, key) = (fs::read_to_string(crt).map_err(|e| e.to_string())?, fs::read_to_string(key).map_err(|e| e.to_string())?);
    r.ssh("sudo install -d -m 0755 /etc/osnt/tls && sudo tee /etc/osnt/tls/tls.crt >/dev/null && sudo chmod 0644 /etc/osnt/tls/tls.crt", Some(&crt))?;
    r.ssh("sudo install -d -m 0755 /etc/osnt/tls && sudo sh -c 'umask 077; cat > /etc/osnt/tls/tls.key' && sudo chmod 0600 /etc/osnt/tls/tls.key", Some(&key))
}

pub fn trust_help() {
    let ca = ca_cert_path();
    eprintln!("\nTrust this CA once on each device (it is the only file you install; keep ca.key private):\n  {}\n", ca.display());
    eprintln!("  iPhone/iPad : AirDrop or email the file, open it, Settings > Profile Downloaded > Install, then");
    eprintln!("                Settings > General > About > Certificate Trust Settings > enable \"OSNT local CA\"");
    eprintln!("  Android     : Settings > Security > Encryption & credentials > Install a certificate > CA certificate");
    eprintln!("  macOS       : open the file in Keychain Access, set \"Always Trust\" for SSL");
    eprintln!("  Windows     : double-click > Install > Local Machine > Trusted Root Certification Authorities");
    eprintln!("  Linux       : sudo cp {0} /usr/local/share/ca-certificates/osnt-ca.crt && sudo update-ca-certificates", ca.display());
    eprintln!("  Chrome/Firefox on Linux use their own store: Settings > Privacy > Certificates (Authorities) > Import");
}

pub fn run(r: &Remote, args: &[String]) -> Result<(), String> {
    let h = host();
    match args.first().map(String::as_str) {
        Some("init") | None => {
            let created = ensure(&h, false)?;
            push(r, &h)?;
            if created {
                eprintln!("==> created a new CA");
            }
            r.ssh("sudo systemctl restart osnt-portal; sleep 1; systemctl is-active osnt-portal", None).ok();
            trust_help();
            Ok(())
        }
        Some("renew") => {
            ensure(&h, true)?;
            push(r, &h)?;
            r.ssh("sudo systemctl restart osnt-portal", None)
        }
        Some("ca") => {
            ensure_ca()?;
            println!("{}", ca_cert_path().display());
            Ok(())
        }
        _ => Err("usage: https init | renew | ca".into()),
    }
}
