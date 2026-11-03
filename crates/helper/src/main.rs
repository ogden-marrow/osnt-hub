//! osnt-helper: the one root-privileged part of the web installer.
//!
//! The Packages plugin runs sandboxed as an unprivileged dynamic user, so it cannot install anything itself. It drops a
//! small request file into its own state directory; a systemd path unit starts this oneshot service, which does the
//! privileged work and writes progress to a job file the plugin (and so the page) can read.
//!
//! Trust model: a request only names a plugin id and a repository the user registered. The helper fetches the build
//! itself from that repository's `latest` release (using the GitHub token the user connected, if any), verifies its
//! SHA-256 against the release's manifest.json, accepts only `plugin.toml` + `plugin` from the archive, validates the
//! manifest with the same rules as `cargo deploy plugin add`, and writes the same fixed, sandboxed systemd unit.
//! Request files are untrusted input.

use std::{
    env, fs,
    io::Read,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use hmac::{Hmac, Mac};
use osnt_pluginfmt::{PluginToml, manifest_for, unit_for, valid_id};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const MAX_MANIFEST: u64 = 512 * 1024;
const MAX_ARCHIVE: u64 = 64 * 1024 * 1024;
const MAX_BINARY: u64 = 120 * 1024 * 1024;
/// The tools that manage everything else; removing them from the page would strand the user.
const CORE: [&str; 2] = ["packages", "credentials"];

struct Paths {
    state: PathBuf,
    etc: PathBuf,
    bin: PathBuf,
    units: PathBuf,
    lib: PathBuf,
}

impl Paths {
    fn real() -> Self {
        let root = env::var("OSNT_HELPER_ROOT").unwrap_or_default();
        let p = |s: &str| PathBuf::from(format!("{root}{s}"));
        Self { state: p("/var/lib/private/osnt-packages"), etc: p("/etc/osnt"), bin: p("/usr/local/bin"), units: p("/etc/systemd/system"), lib: p("/var/lib/private") }
    }
}

#[derive(Deserialize)]
struct Request {
    job: String,
    action: String,
    id: String,
    #[serde(default)]
    repo: String,
    #[serde(default)]
    purge: bool,
}

/// Progress record the page polls: queued -> running -> done | failed.
struct Job {
    file: PathBuf,
    id: String,
    action: String,
    plugin: String,
    log: Vec<String>,
}

impl Job {
    fn write(&self, state: &str, message: &str) {
        let body = json!({ "job": self.id, "action": self.action, "id": self.plugin, "state": state, "message": message, "log": self.log });
        let tmp = self.file.with_extension("tmp");
        if fs::write(&tmp, body.to_string()).is_ok() {
            let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644));
            let _ = fs::rename(tmp, &self.file);
        }
    }
    fn step(&mut self, line: &str) {
        eprintln!("[{}] {line}", self.plugin);
        self.log.push(line.to_string());
        self.write("running", line);
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

fn safe_segment(s: &str) -> bool {
    (1..=100).contains(&s.len()) && s != "." && s != ".." && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn parse_repo(url: &str) -> Option<(String, String)> {
    let rest = url.trim().strip_prefix("https://github.com/")?;
    let mut it = rest.split('/');
    let (o, r) = (it.next()?, it.next()?);
    let r = r.strip_suffix(".git").unwrap_or(r);
    (safe_segment(o) && safe_segment(r)).then(|| (o.to_string(), r.to_string()))
}

fn agent() -> ureq::Agent {
    let provider = Arc::new(rustls_rustcrypto::provider());
    let tls = ureq::tls::TlsConfig::builder().provider(ureq::tls::TlsProvider::Rustls).unversioned_rustls_crypto_provider(provider).build();
    ureq::Agent::config_builder().tls_config(tls).timeout_global(Some(Duration::from_secs(180))).user_agent("osnt-helper").build().into()
}

fn github_token(p: &Paths) -> Option<String> {
    let v: Value = serde_json::from_str(&fs::read_to_string(p.state.join("github.json")).ok()?).ok()?;
    if v["expires_at"].as_u64().is_some_and(|t| t <= now()) {
        return None;
    }
    v["access_token"].as_str().map(str::to_string)
}

fn get(agent: &ureq::Agent, url: &str, accept: &str, token: Option<&str>, limit: u64) -> Result<Vec<u8>, String> {
    let mut req = agent.get(url).header("Accept", accept).header("X-GitHub-Api-Version", "2022-11-28");
    if let Some(t) = token {
        req = req.header("Authorization", &format!("Bearer {t}"));
    }
    let mut resp = req.call().map_err(|e| match e {
        ureq::Error::StatusCode(401) => "GitHub sign-in expired. Reconnect GitHub on the Packages page".to_string(),
        ureq::Error::StatusCode(404) => "not found: the repository has no published build (its release workflow has not run), or the GitHub App cannot see it".to_string(),
        ureq::Error::StatusCode(c) => format!("GitHub answered {c}"),
        other => format!("download failed: {other}"),
    })?;
    resp.body_mut().with_config().limit(limit).read_to_vec().map_err(|e| format!("download failed: {e}"))
}

/// The two files a build archive may contain.
fn unpack(archive: &[u8]) -> Result<(String, Vec<u8>), String> {
    let gz = flate2::read::GzDecoder::new(archive).take(MAX_BINARY + MAX_MANIFEST);
    let mut tar = tar::Archive::new(gz);
    let (mut toml_text, mut binary) = (None, None);
    for entry in tar.entries().map_err(|e| format!("archive unreadable: {e}"))? {
        let mut entry = entry.map_err(|e| format!("archive unreadable: {e}"))?;
        if !entry.header().entry_type().is_file() {
            return Err("archive contains something other than plain files; refusing".into());
        }
        let path = entry.path().map_err(|e| e.to_string())?.to_string_lossy().trim_start_matches("./").to_string();
        match path.as_str() {
            "plugin.toml" => {
                let mut s = String::new();
                (&mut entry).take(MAX_MANIFEST).read_to_string(&mut s).map_err(|e| format!("plugin.toml unreadable: {e}"))?;
                toml_text = Some(s);
            }
            "plugin" => {
                let mut b = Vec::new();
                (&mut entry).take(MAX_BINARY).read_to_end(&mut b).map_err(|e| format!("binary unreadable: {e}"))?;
                binary = Some(b);
            }
            other => return Err(format!("unexpected file in archive: {other}")),
        }
    }
    Ok((toml_text.ok_or("archive has no plugin.toml")?, binary.ok_or("archive has no plugin binary")?))
}

fn write_atomic(path: &Path, data: &[u8], mode: u32) -> Result<(), String> {
    let tmp = path.with_extension("new");
    fs::write(&tmp, data).map_err(|e| format!("write {}: {e}", path.display()))?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(mode)).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| format!("install {}: {e}", path.display()))
}

fn installed_ports(p: &Paths) -> Vec<(String, u16)> {
    #[derive(Deserialize)]
    struct M {
        id: String,
        port: u16,
    }
    let Ok(rd) = fs::read_dir(p.etc.join("plugins.d")) else { return vec![] };
    rd.flatten().filter_map(|e| toml::from_str::<M>(&fs::read_to_string(e.path()).ok()?).ok()).map(|m| (m.id, m.port)).collect()
}

fn dependents(p: &Paths, id: &str) -> Vec<String> {
    #[derive(Deserialize)]
    struct R {
        id: String,
        #[serde(default)]
        requires: Vec<String>,
    }
    let Ok(rd) = fs::read_dir(p.etc.join("installed.d")) else { return vec![] };
    rd.flatten().filter_map(|e| serde_json::from_str::<R>(&fs::read_to_string(e.path()).ok()?).ok()).filter(|r| r.requires.iter().any(|d| d == id)).map(|r| r.id).collect()
}

fn token_env(p: &Paths, id: &str) -> Result<String, String> {
    let dir = p.etc.join("tokens");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
    let file = dir.join(format!("{id}.env"));
    let existing = fs::read_to_string(&file).ok().and_then(|t| t.lines().find_map(|l| l.strip_prefix("OSNT_PLUGIN_TOKEN=").map(str::to_string))).filter(|t| !t.is_empty());
    let token = match existing {
        Some(t) => t,
        None => {
            let mut b = [0u8; 32];
            fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).map_err(|e| format!("no randomness: {e}"))?;
            hex(&b)
        }
    };
    let master = fs::read_to_string(p.etc.join("gateway.key")).map_err(|_| "the hub's gateway key is missing; run `cargo deploy portal` once".to_string())?;
    let mut m = Hmac::<Sha256>::new_from_slice(master.trim().as_bytes()).map_err(|e| e.to_string())?;
    m.update(format!("osnt-gateway:{id}").as_bytes());
    let gateway_key = hex(&m.finalize().into_bytes());
    write_atomic(&file, format!("OSNT_PLUGIN_TOKEN={token}\nOSNT_GATEWAY_KEY={gateway_key}\n").as_bytes(), 0o600)?;
    Ok(sha256_hex(token.as_bytes()))
}

fn systemctl(args: &[&str]) -> Result<(), String> {
    let out = Command::new("systemctl").args(args).output().map_err(|e| format!("systemctl: {e}"))?;
    out.status.success().then_some(()).ok_or_else(|| format!("systemctl {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()))
}

fn is_active(unit: &str) -> bool {
    Command::new("systemctl").args(["is-active", "--quiet", unit]).status().is_ok_and(|s| s.success())
}

fn install(p: &Paths, req: &Request, job: &mut Job) -> Result<String, String> {
    if !valid_id(&req.id) {
        return Err("invalid plugin id".into());
    }
    let (owner, repo) = parse_repo(&req.repo).ok_or("not a GitHub repository link")?;
    let canonical = format!("https://github.com/{owner}/{repo}");
    let registered: Vec<String> = fs::read_to_string(p.state.join("repos.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    if !registered.contains(&canonical) {
        return Err("that repository is not in your list".into());
    }
    let token = github_token(p);
    let agent = agent();
    job.step(&format!("Looking up the latest build in {owner}/{repo}"));
    let release: Value = serde_json::from_slice(&get(&agent, &format!("https://api.github.com/repos/{owner}/{repo}/releases/tags/latest"), "application/vnd.github+json", token.as_deref(), MAX_MANIFEST)?).map_err(|_| "unreadable release data")?;
    let asset_id = |name: &str| release["assets"].as_array().into_iter().flatten().find(|a| a["name"] == name).and_then(|a| a["id"].as_u64());
    let download = |name: &str, limit: u64| -> Result<Vec<u8>, String> {
        let id = asset_id(name).ok_or_else(|| format!("the release has no {name}"))?;
        get(&agent, &format!("https://api.github.com/repos/{owner}/{repo}/releases/assets/{id}"), "application/octet-stream", token.as_deref(), limit)
    };
    let index: Value = serde_json::from_slice(&download("manifest.json", MAX_MANIFEST)?).map_err(|_| "manifest.json is not valid")?;
    let entry = index["plugins"].as_array().into_iter().flatten().find(|e| e["id"] == req.id.as_str()).ok_or_else(|| format!("'{}' has no published build in this repository", req.id))?;
    let want = entry["sha256"].as_str().unwrap_or("").to_ascii_lowercase();
    let version = entry["version"].as_str().unwrap_or("").to_string();
    job.step(&format!("Downloading {} {version}", req.id));
    let archive = download(&format!("{}.tar.gz", req.id), MAX_ARCHIVE)?;
    if sha256_hex(&archive) != want {
        return Err("the download does not match the checksum in the release; refusing to install".into());
    }
    job.step("Checking the plugin");
    let (toml_text, binary) = unpack(&archive)?;
    let plugin: PluginToml = toml::from_str(&toml_text).map_err(|e| format!("plugin.toml: {e}"))?;
    osnt_pluginfmt::validate(&plugin)?;
    if plugin.id != req.id {
        return Err("the archive's plugin id does not match the request".into());
    }
    if let Some((other, _)) = installed_ports(p).into_iter().find(|(i, port)| *port == plugin.port && *i != plugin.id) {
        return Err(format!("port {} is already used by '{other}'", plugin.port));
    }
    if let Some(missing) = plugin.requires.iter().find(|d| !p.etc.join("plugins.d").join(format!("{d}.toml")).exists()) {
        return Err(format!("it needs '{missing}' installed first"));
    }
    job.step("Installing");
    let id = &plugin.id;
    let hash = token_env(p, id)?;
    fs::create_dir_all(p.etc.join("plugins.d")).and_then(|_| fs::create_dir_all(p.etc.join("installed.d"))).map_err(|e| e.to_string())?;
    write_atomic(&p.bin.join(format!("osnt-{id}")), &binary, 0o755)?;
    write_atomic(&p.etc.join("plugins.d").join(format!("{id}.toml")), manifest_for(&plugin, &hash).as_bytes(), 0o644)?;
    let record = json!({ "id": id, "name": plugin.name, "version": plugin.version, "source": format!("release:{canonical}"), "commit": "", "binary_sha256": sha256_hex(&binary), "requires": plugin.requires });
    write_atomic(&p.etc.join("installed.d").join(format!("{id}.json")), record.to_string().as_bytes(), 0o644)?;
    write_atomic(&p.units.join(format!("osnt-{id}.service")), unit_for(&plugin).as_bytes(), 0o644)?;
    let unit = format!("osnt-{id}.service");
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", &unit])?;
    systemctl(&["restart", &unit])?;
    for _ in 0..8 {
        std::thread::sleep(Duration::from_millis(500));
        if is_active(&unit) {
            let hint = if plugin.secrets.is_empty() { String::new() } else { format!(" It asks for: {}.", plugin.secrets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", ")) };
            return Ok(format!("Installed {} {}.{hint}", plugin.name, plugin.version));
        }
    }
    let log = Command::new("journalctl").args(["-u", &unit, "-n", "6", "--no-pager", "-o", "cat"]).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    Err(format!("installed, but the service did not start. {log}"))
}

fn remove(p: &Paths, req: &Request, job: &mut Job) -> Result<String, String> {
    if !valid_id(&req.id) {
        return Err("invalid plugin id".into());
    }
    if CORE.contains(&req.id.as_str()) {
        return Err("this one runs the Settings pages themselves, so it cannot be removed from here".into());
    }
    let deps = dependents(p, &req.id);
    if !deps.is_empty() {
        return Err(format!("{} still need{} it. Remove {} first", deps.join(", "), if deps.len() == 1 { "s" } else { "" }, if deps.len() == 1 { "that one" } else { "those" }));
    }
    let id = &req.id;
    job.step("Stopping the service");
    let unit = format!("osnt-{id}.service");
    let _ = systemctl(&["disable", "--now", &unit]);
    job.step("Removing files");
    let files = [p.bin.join(format!("osnt-{id}")), p.units.join(&unit), p.etc.join("plugins.d").join(format!("{id}.toml")), p.etc.join("tokens").join(format!("{id}.env")), p.etc.join("installed.d").join(format!("{id}.json"))];
    for f in &files {
        let _ = fs::remove_file(f);
    }
    if req.purge {
        let _ = fs::remove_dir_all(p.lib.join(format!("osnt-{id}")));
    }
    let _ = systemctl(&["daemon-reload"]);
    Ok(format!("Removed {id}.{}", if req.purge { " Its stored data was deleted too." } else { " Its data was kept." }))
}

fn process(p: &Paths, req: Request) {
    let jobs = p.state.join("jobs");
    let _ = fs::create_dir_all(&jobs);
    let mut job = Job { file: jobs.join(format!("{}.json", req.job)), id: req.job.clone(), action: req.action.clone(), plugin: req.id.clone(), log: vec![] };
    job.step("Starting");
    let result = match req.action.as_str() {
        "install" => install(p, &req, &mut job),
        "remove" => remove(p, &req, &mut job),
        _ => Err("unknown action".into()),
    };
    match result {
        Ok(msg) => job.write("done", &msg),
        Err(e) => {
            job.log.push(e.clone());
            job.write("failed", &e)
        }
    }
}

fn main() {
    let p = Paths::real();
    let dir = p.state.join("requests");
    let mut files: Vec<PathBuf> = fs::read_dir(&dir).map(|rd| rd.flatten().map(|e| e.path()).filter(|f| f.extension().is_some_and(|x| x == "json")).collect()).unwrap_or_default();
    files.sort_by_key(|f| fs::metadata(f).and_then(|m| m.modified()).ok());
    for f in files {
        // Take the file out of the queue first so a crash cannot make it loop.
        let text = fs::File::open(&f).ok().and_then(|h| {
            let mut s = String::new();
            h.take(4096).read_to_string(&mut s).ok().map(|_| s)
        });
        let _ = fs::remove_file(&f);
        let Some(req) = text.and_then(|t| serde_json::from_str::<Request>(&t).ok()) else { continue };
        let job_ok = (8..=64).contains(&req.job.len()) && req.job.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-');
        if job_ok {
            process(&p, req);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive(files: &[(&str, &[u8])], dir_entry: bool) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        for (name, data) in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            // Written raw: the builder itself refuses `..` paths, but a hostile archive would not.
            h.as_gnu_mut().unwrap().name[..name.len()].copy_from_slice(name.as_bytes());
            h.set_cksum();
            b.append(&h, *data).unwrap();
        }
        if dir_entry {
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Directory);
            h.set_size(0);
            h.set_mode(0o755);
            h.set_cksum();
            b.append_data(&mut h, "evil/", &[][..]).unwrap();
        }
        let tar = b.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &tar).unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn unpacks_only_the_two_expected_files() {
        let (t, b) = unpack(&archive(&[("plugin.toml", b"id = \"x\""), ("plugin", b"\x7fELF")], false)).unwrap();
        assert_eq!((t.as_str(), b.as_slice()), ("id = \"x\"", &b"\x7fELF"[..]));
        assert!(unpack(&archive(&[("plugin.toml", b"a"), ("plugin", b"b"), ("../../etc/passwd", b"x")], false)).is_err());
        assert!(unpack(&archive(&[("plugin.toml", b"a"), ("other", b"b")], false)).is_err());
        assert!(unpack(&archive(&[("plugin.toml", b"a")], false)).is_err(), "binary is required");
        assert!(unpack(&archive(&[("plugin.toml", b"a"), ("plugin", b"b")], true)).is_err(), "directories are refused");
    }

    #[test]
    fn repo_links_are_strict() {
        assert!(parse_repo("https://github.com/a/b").is_some());
        assert!(parse_repo("https://github.com/a/..").is_none());
        assert!(parse_repo("https://github.com.evil/a/b").is_none());
    }

    #[test]
    fn checksum_is_lowercase_hex_sha256() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn remove_refuses_core_and_dependents() {
        let dir = env::temp_dir().join(format!("osnt-helper-test-{}", std::process::id()));
        let p = Paths { state: dir.join("state"), etc: dir.join("etc"), bin: dir.join("bin"), units: dir.join("units"), lib: dir.join("lib") };
        fs::create_dir_all(p.etc.join("installed.d")).unwrap();
        fs::write(p.etc.join("installed.d/child.json"), r#"{"id":"child","requires":["base"]}"#).unwrap();
        let mut job = Job { file: dir.join("job.json"), id: "j".into(), action: "remove".into(), plugin: "x".into(), log: vec![] };
        fs::create_dir_all(&dir).unwrap();
        let req = |id: &str| Request { job: "abcdef012".into(), action: "remove".into(), id: id.into(), repo: String::new(), purge: false };
        assert!(remove(&p, &req("packages"), &mut job).unwrap_err().contains("cannot be removed"));
        assert!(remove(&p, &req("base"), &mut job).unwrap_err().contains("child"));
        let _ = fs::remove_dir_all(&dir);
    }
}
