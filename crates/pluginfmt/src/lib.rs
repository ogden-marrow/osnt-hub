//! The plugin manifest (`plugin.toml`) and the files generated from it, shared by the installer on your computer
//! (`cargo deploy plugin add`) and the root helper on the Pi (one-click installs from the web UI), so both validate
//! the same way and write byte-identical units and manifests.

use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Deserialize)]
pub struct PluginToml {
    pub id: String,
    pub name: String,
    pub description: String,
    pub grid: String,
    pub port: u16,
    pub version: String,
    /// Plugins this one calls over their own APIs; they must be installed first.
    #[serde(default)]
    pub requires: Vec<String>,
    /// API names this plugin vends to others (informational; shown at install time).
    #[serde(default)]
    pub provides: Vec<String>,
    #[serde(default)]
    pub service: Service,
    #[serde(default, rename = "secret")]
    pub secrets: Vec<Secret>,
    /// Software the plugin needs on the machine itself. Installed by the web installer only after the user approves it.
    #[serde(default)]
    pub system: SystemDeps,
}

/// `[system]` in plugin.toml:
///
/// ```toml
/// [system]
/// apt = ["ffmpeg"]                                   # Raspberry Pi OS packages
/// [[system.download]]                                # a program downloaded from a GitHub release
/// name = "yt-dlp"
/// url = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux_aarch64"
/// sha256_url = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/SHA2-256SUMS"   # optional
/// ```
#[derive(Deserialize, Default, Clone)]
pub struct SystemDeps {
    #[serde(default)]
    pub apt: Vec<String>,
    #[serde(default)]
    pub download: Vec<Download>,
}

#[derive(Deserialize, Clone)]
pub struct Download {
    /// File name under /usr/local/bin.
    pub name: String,
    pub url: String,
    /// A checksum file (lines of `<sha256>  <file name>`) from the same project; without it the download is unverified.
    #[serde(default)]
    pub sha256_url: Option<String>,
}

impl SystemDeps {
    pub fn is_empty(&self) -> bool {
        self.apt.is_empty() && self.download.is_empty()
    }

    /// Stable strings naming each thing that would be installed. The page shows them, the user approves them, and the
    /// helper installs only what was approved.
    pub fn approvals(&self) -> Vec<String> {
        self.apt.iter().map(|a| format!("apt:{a}")).chain(self.download.iter().map(|d| format!("download:{}", d.url))).collect()
    }
}

pub fn valid_apt_name(s: &str) -> bool {
    (2..=64).contains(&s.len()) && s.as_bytes()[0].is_ascii_alphanumeric() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'+' | b'.' | b'-'))
}

fn github_url(u: &str) -> bool {
    u.len() <= 240
        && u.strip_prefix("https://github.com/").is_some_and(|rest| rest.split('/').count() >= 4 && rest.split('/').all(|seg| !seg.is_empty() && seg != ".." && seg != "." && seg.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))))
}

#[derive(Deserialize, Default)]
pub struct Service {
    #[serde(default)]
    pub state_dir: bool,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Deserialize)]
pub struct Secret {
    pub name: String,
    pub label: String,
    #[serde(default)]
    pub help_url: Option<String>,
    #[serde(default)]
    pub required: bool,
}

pub fn valid_id(s: &str) -> bool {
    (1..=32).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && !s.starts_with('-')
}

pub fn valid_env_name(k: &str) -> bool {
    !k.is_empty() && k.len() <= 64 && k.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_') && k != "OSNT_PLUGIN_TOKEN" && k != "OSNT_CORE"
}

/// Everything about a manifest that can be checked without the plugin's source.
pub fn validate(p: &PluginToml) -> Result<(), String> {
    if !valid_id(&p.id) || p.id == "portal" {
        return Err("id must be 1-32 chars of a-z, 0-9, '-' and not 'portal'".into());
    }
    if !(8100..=8999).contains(&p.port) || p.port == 8080 {
        return Err("port must be within 8100-8999".into());
    }
    let clean = |s: &str| s.len() <= 300 && !s.chars().any(char::is_control) && !s.contains('"') && !s.contains('\\');
    if ![&p.name, &p.description, &p.grid, &p.version].iter().all(|s| clean(s)) {
        return Err("name/description/grid/version must be short plain text".into());
    }
    if !p.requires.iter().all(|s| valid_id(s)) || !p.provides.iter().all(|s| clean(s)) || p.requires.contains(&p.id) {
        return Err("requires must list other plugin ids; provides must be short plain text".into());
    }
    for (k, v) in &p.service.env {
        if !valid_env_name(k) || v.len() > 300 || v.chars().any(|c| c.is_control() || c == '"' || c == '\\') {
            return Err(format!("service.env entry {k} is not allowed"));
        }
    }
    for s in &p.secrets {
        if !valid_env_name(&s.name) || !clean(&s.label) || s.help_url.as_deref().is_some_and(|u| !u.starts_with("https://") || !clean(u)) {
            return Err(format!("secret {} is not valid", s.name));
        }
    }
    if p.system.apt.len() > 10 || !p.system.apt.iter().all(|a| valid_apt_name(a)) {
        return Err("system.apt must list at most 10 plain package names".into());
    }
    if p.system.download.len() > 3 {
        return Err("system.download may list at most 3 programs".into());
    }
    for d in &p.system.download {
        let name_ok = (1..=40).contains(&d.name.len()) && d.name.as_bytes()[0].is_ascii_alphanumeric() && d.name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')) && !d.name.starts_with("osnt-");
        if !name_ok || !github_url(&d.url) || d.sha256_url.as_deref().is_some_and(|u| !github_url(u)) {
            return Err(format!("system.download entry '{}' is not allowed (a plain name, and https://github.com/ release URLs only)", d.name));
        }
    }
    Ok(())
}

pub fn unit_for(p: &PluginToml) -> String {
    let state_dir = format!("/var/lib/osnt-{}", p.id);
    let mut env_lines = String::new();
    for (k, v) in &p.service.env {
        let v = v.replace("{port}", &p.port.to_string()).replace("{state_dir}", &state_dir);
        env_lines.push_str(&format!("Environment=\"{k}={v}\"\n"));
    }
    format!(
        "[Unit]\nDescription=OSNT plugin {id}\nAfter=network-online.target\nWants=network-online.target\n\n[Service]\n\
         ExecStart=/usr/local/bin/osnt-{id}\nEnvironmentFile=/etc/osnt/tokens/{id}.env\nEnvironment=OSNT_CORE=127.0.0.1:8080\n{env_lines}\
         Restart=on-failure\nRestartSec=3\nDynamicUser=yes\n{state}NoNewPrivileges=yes\nProtectSystem=strict\nProtectHome=yes\nPrivateTmp=yes\n\
         PrivateDevices=yes\nProtectKernelTunables=yes\nProtectControlGroups=yes\nLockPersonality=yes\n\
         RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX\n\n[Install]\nWantedBy=multi-user.target\n",
        id = p.id,
        state = if p.service.state_dir { format!("StateDirectory=osnt-{}\n", p.id) } else { String::new() },
    )
}

pub fn manifest_for(p: &PluginToml, token_hash: &str) -> String {
    let mut m = format!(
        "token_sha256 = \"{token_hash}\"\nid = \"{}\"\nname = \"{}\"\ndescription = \"{}\"\ngrid = \"{}\"\nport = {}\nversion = \"{}\"\n",
        p.id, p.name, p.description, p.grid, p.port, p.version
    );
    if !p.requires.is_empty() {
        m.push_str(&format!("requires = [{}]\n", p.requires.iter().map(|r| format!("\"{r}\"")).collect::<Vec<_>>().join(", ")));
    }
    for s in &p.secrets {
        m.push_str(&format!("\n[[secret]]\nname = \"{}\"\nlabel = \"{}\"\nrequired = {}\n", s.name, s.label, s.required));
        if let Some(u) = &s.help_url {
            m.push_str(&format!("help_url = \"{u}\"\n"));
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PluginToml {
        toml::from_str("id = \"demo\"\nname = \"Demo\"\ndescription = \"d\"\ngrid = \"G\"\nport = 8150\nversion = \"1.0.0\"\nrequires = [\"adsb\"]\n[service]\nstate_dir = true\n[service.env]\nBIND = \"127.0.0.1:{port}\"\n[[secret]]\nname = \"K\"\nlabel = \"Key\"\n").unwrap()
    }

    #[test]
    fn valid_manifest_generates_unit_and_manifest() {
        let p = sample();
        validate(&p).unwrap();
        let u = unit_for(&p);
        assert!(u.contains("Environment=\"BIND=127.0.0.1:8150\"") && u.contains("StateDirectory=osnt-demo") && u.contains("DynamicUser=yes"));
        let m = manifest_for(&p, "abc");
        assert!(m.contains("requires = [\"adsb\"]") && m.contains("[[secret]]") && m.contains("version = \"1.0.0\""));
        toml::from_str::<toml::Value>(&m).expect("manifest is valid toml");
    }

    #[test]
    fn system_dependencies_are_parsed_validated_and_listed() {
        let mut p = sample();
        p.system = toml::from_str("apt = [\"ffmpeg\"]\n[[download]]\nname = \"yt-dlp\"\nurl = \"https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux_aarch64\"\nsha256_url = \"https://github.com/yt-dlp/yt-dlp/releases/latest/download/SHA2-256SUMS\"\n").unwrap();
        validate(&p).unwrap();
        assert_eq!(p.system.approvals(), vec!["apt:ffmpeg", "download:https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux_aarch64"]);
        for bad in ["Ffmpeg", "a b", "-x", "x;rm", "a", "../x"] {
            let mut q = sample();
            q.system.apt = vec![bad.into()];
            assert!(validate(&q).is_err(), "{bad}");
        }
        for url in ["http://github.com/a/b/c/d", "https://evil.com/a/b/c/d", "https://github.com/a/b", "https://github.com/a/../c/d", "https://github.com.evil.com/a/b/c/d"] {
            let mut q = sample();
            q.system.download = vec![Download { name: "tool".into(), url: url.into(), sha256_url: None }];
            assert!(validate(&q).is_err(), "{url}");
        }
        let mut q = sample();
        q.system.download = vec![Download { name: "osnt-evil".into(), url: "https://github.com/a/b/releases/x".into(), sha256_url: None }];
        assert!(validate(&q).is_err());
    }

    #[test]
    fn rejects_bad_manifests() {
        let mut p = sample();
        p.port = 8080;
        assert!(validate(&p).is_err());
        let mut p = sample();
        p.id = "portal".into();
        assert!(validate(&p).is_err());
        let mut p = sample();
        p.name = "a\"b".into();
        assert!(validate(&p).is_err());
        let mut p = sample();
        p.service.env.insert("OSNT_PLUGIN_TOKEN".into(), "x".into());
        assert!(validate(&p).is_err());
    }
}
