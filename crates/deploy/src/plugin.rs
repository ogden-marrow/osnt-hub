//! Plugin management for a Pi: install from a GitHub folder link or a plugin repository, list, remove.
//!
//! A plugin folder holds `plugin.toml` (manifest + a small `[service]` section), a standalone Rust crate named
//! `osnt-<id>` and its assets. The installer pins the link to a commit, validates the manifest, asks for
//! confirmation, builds for the Pi, and installs a fixed, sandboxed systemd unit it generates itself: a plugin can
//! only choose its port, a state directory, non-secret environment values and the credentials it asks for.

use std::{
    collections::BTreeMap,
    env, fs,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    process::Command,
};

use serde::Deserialize;

use crate::{Remote, TARGET};

#[derive(Deserialize)]
struct PluginToml {
    id: String,
    name: String,
    description: String,
    grid: String,
    port: u16,
    version: String,
    /// Plugins this one calls over their own APIs; they must be installed first.
    #[serde(default)]
    requires: Vec<String>,
    /// API names this plugin vends to others (informational; shown at install time).
    #[serde(default)]
    provides: Vec<String>,
    #[serde(default)]
    service: Service,
    #[serde(default, rename = "secret")]
    secrets: Vec<Secret>,
}

#[derive(Deserialize, Default)]
struct Service {
    #[serde(default)]
    state_dir: bool,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct Secret {
    name: String,
    label: String,
    #[serde(default)]
    help_url: Option<String>,
    #[serde(default)]
    required: bool,
}

struct Source {
    owner: String,
    repo: String,
    git_ref: String,
    path: String,
}

fn safe_segment(s: &str) -> bool {
    !s.is_empty() && s != ".." && s != "." && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// `https://github.com/<owner>/<repo>[/tree/<ref>[/<path>]]`. A ref containing `/` is not supported; use a tag or commit.
fn parse_source(url: &str) -> Result<Source, String> {
    let rest = url.strip_prefix("https://github.com/").ok_or("only public https://github.com/... links are supported")?;
    let parts: Vec<&str> = rest.trim_end_matches('/').split('/').collect();
    let (git_ref, path): (&str, Vec<&str>) = match parts.as_slice() {
        [_, _] => ("HEAD", vec![]),
        [_, _, "tree", r, rest @ ..] => (r, rest.to_vec()),
        _ => return Err("expected https://github.com/<owner>/<repo>/tree/<ref>/<folder>".into()),
    };
    let repo = parts[1].trim_end_matches(".git");
    if ![parts[0], repo, git_ref].iter().all(|s| safe_segment(s)) || !path.iter().all(|s| safe_segment(s)) {
        return Err("link contains unsupported characters".into());
    }
    Ok(Source { owner: parts[0].into(), repo: repo.into(), git_ref: git_ref.into(), path: path.join("/") })
}

fn run_ok(cmd: &mut Command, what: &str) -> Result<(), String> {
    let status = cmd.status().map_err(|e| format!("{what}: {e}"))?;
    status.success().then_some(()).ok_or_else(|| format!("{what} failed: {status}"))
}

fn curl_text(args: &[&str]) -> Result<String, String> {
    let out = Command::new("curl").args(["-fsSL", "--max-time", "60"]).args(args).output().map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("download failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn reject_symlinks(dir: &Path) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        if ty.is_symlink() {
            return Err(format!("plugin contains a symlink ({}); refusing", entry.path().display()));
        }
        if ty.is_dir() {
            reject_symlinks(&entry.path())?;
        }
    }
    Ok(())
}

struct Fetched {
    /// Temp directory to delete afterwards; None for a local folder (never delete the user's own files).
    base: Option<PathBuf>,
    dir: PathBuf,
    commit: String,
}

/// A plugin folder on this machine, for development and for installing from a local checkout of a plugin repository.
fn local(path: &str) -> Result<Fetched, String> {
    let dir = Path::new(path).canonicalize().map_err(|_| format!("{path}: no such folder"))?;
    let commit = Command::new("git")
        .arg("-C")
        .arg(&dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "local".into());
    Ok(Fetched { base: None, dir, commit })
}

/// Resolve the link to an exact commit, download that commit's tarball into a fresh temp dir and locate the folder.
fn fetch(src: &Source) -> Result<Fetched, String> {
    let commit = curl_text(&["-H", "Accept: application/vnd.github.sha", &format!("https://api.github.com/repos/{}/{}/commits/{}", src.owner, src.repo, src.git_ref)])?;
    if commit.len() != 40 || !commit.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("could not resolve {} to a commit", src.git_ref));
    }
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let base = env::temp_dir().join(format!("osnt-plugin-{}-{nanos}", std::process::id()));
    let extract = base.join("x");
    fs::create_dir_all(&extract).map_err(|e| e.to_string())?;
    let tgz = base.join("src.tar.gz");
    run_ok(Command::new("curl").args(["-fsSL", "--max-time", "300", "-o"]).arg(&tgz).arg(format!("https://codeload.github.com/{}/{}/tar.gz/{commit}", src.owner, src.repo)), "download")?;
    run_ok(Command::new("tar").arg("-xzf").arg(&tgz).arg("-C").arg(&extract), "extract")?;
    let root = fs::read_dir(&extract).map_err(|e| e.to_string())?.flatten().find(|e| e.path().is_dir()).ok_or("empty archive")?.path();
    let dir = if src.path.is_empty() { root.clone() } else { root.join(&src.path) };
    let canon = dir.canonicalize().map_err(|_| format!("folder '{}' not found in the repository", src.path))?;
    if !canon.starts_with(root.canonicalize().map_err(|e| e.to_string())?) {
        return Err("folder escapes the repository".into());
    }
    reject_symlinks(&canon)?;
    Ok(Fetched { base: Some(base), dir: canon, commit })
}

fn valid_env_name(k: &str) -> bool {
    !k.is_empty() && k.len() <= 64 && k.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_') && k != "OSNT_PLUGIN_TOKEN" && k != "OSNT_CORE"
}

fn validate(dir: &Path) -> Result<PluginToml, String> {
    let text = fs::read_to_string(dir.join("plugin.toml")).map_err(|_| "no plugin.toml in that folder")?;
    let p: PluginToml = toml::from_str(&text).map_err(|e| format!("plugin.toml: {e}"))?;
    let id_ok = (1..=32).contains(&p.id.len()) && p.id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && !p.id.starts_with('-');
    if !id_ok || p.id == "portal" {
        return Err("id must be 1-32 chars of a-z, 0-9, '-' and not 'portal'".into());
    }
    if !(8100..=8999).contains(&p.port) || p.port == 8080 {
        return Err("port must be within 8100-8999".into());
    }
    let clean = |s: &str| s.len() <= 300 && !s.chars().any(char::is_control) && !s.contains('"');
    if ![&p.name, &p.description, &p.grid, &p.version].iter().all(|s| clean(s)) {
        return Err("name/description/grid/version must be short plain text".into());
    }
    let id_like = |s: &String| (1..=32).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !p.requires.iter().all(id_like) || !p.provides.iter().all(|s| clean(s)) || p.requires.contains(&p.id) {
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
    let cargo: toml::Value = toml::from_str(&fs::read_to_string(dir.join("Cargo.toml")).map_err(|_| "no Cargo.toml")?).map_err(|e| format!("Cargo.toml: {e}"))?;
    if cargo.get("package").and_then(|p| p.get("name")).and_then(|n| n.as_str()) != Some(&format!("osnt-{}", p.id)) {
        return Err(format!("Cargo package must be named osnt-{}", p.id));
    }
    if !dir.join("src/main.rs").exists() {
        return Err("src/main.rs missing".into());
    }
    Ok(p)
}

fn unit_for(p: &PluginToml) -> String {
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

fn manifest_for(p: &PluginToml, token_hash: &str) -> String {
    let mut m = format!(
        "token_sha256 = \"{token_hash}\"\nid = \"{}\"\nname = \"{}\"\ndescription = \"{}\"\ngrid = \"{}\"\nport = {}\nversion = \"{}\"\n",
        p.id, p.name, p.description, p.grid, p.port, p.version
    );
    for s in &p.secrets {
        m.push_str(&format!("\n[[secret]]\nname = \"{}\"\nlabel = \"{}\"\nrequired = {}\n", s.name, s.label, s.required));
        if let Some(u) = &s.help_url {
            m.push_str(&format!("help_url = \"{u}\"\n"));
        }
    }
    m
}

fn confirm(yes: bool, question: &str) -> Result<(), String> {
    if yes {
        return Ok(());
    }
    eprint!("{question} Type 'yes' to continue: ");
    io::stderr().flush().ok();
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line).map_err(|e| e.to_string())?;
    (line.trim() == "yes").then_some(()).ok_or_else(|| "cancelled".to_string())
}

// ---- plugin repositories -------------------------------------------------------------------------------------

fn repos_path() -> PathBuf {
    PathBuf::from(env::var("HOME").unwrap_or_default()).join(".config/osnt/repos.toml")
}

fn load_repos() -> Vec<String> {
    #[derive(Deserialize, Default)]
    struct Repos {
        #[serde(default)]
        repos: Vec<String>,
    }
    fs::read_to_string(repos_path()).ok().and_then(|t| toml::from_str::<Repos>(&t).ok()).unwrap_or_default().repos
}

fn save_repos(repos: &[String]) -> Result<(), String> {
    let path = repos_path();
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let list: Vec<String> = repos.iter().map(|r| format!("\"{r}\"")).collect();
    fs::write(path, format!("repos = [{}]\n", list.join(", "))).map_err(|e| e.to_string())
}

#[derive(Deserialize)]
struct Index {
    #[serde(default, rename = "plugin")]
    plugins: Vec<IndexEntry>,
}

#[derive(Deserialize)]
struct IndexEntry {
    id: String,
    name: String,
    description: String,
    path: String,
    #[serde(default)]
    version: String,
}

/// (repo url, entry) for every plugin listed in a registered repository's index.toml.
fn index_entries() -> Vec<(String, IndexEntry)> {
    let mut out = Vec::new();
    for repo in load_repos() {
        let Ok(src) = parse_source(&repo) else { continue };
        let url = format!("https://raw.githubusercontent.com/{}/{}/HEAD/index.toml", src.owner, src.repo);
        match curl_text(&[&url]).and_then(|t| toml::from_str::<Index>(&t).map_err(|e| e.to_string())) {
            Ok(idx) => out.extend(idx.plugins.into_iter().filter(|e| e.path.split('/').all(safe_segment)).map(|e| (repo.clone(), e))),
            Err(e) => eprintln!("warning: could not read index of {repo}: {e}"),
        }
    }
    out
}

fn install_url(repo: &str, e: &IndexEntry) -> String {
    format!("{}/tree/HEAD/{}", repo.trim_end_matches('/'), e.path)
}

// ---- commands ------------------------------------------------------------------------------------------------

pub fn run(r: &Remote, args: &[String]) -> Result<(), String> {
    let yes = args.iter().any(|a| a == "--yes");
    let purge = args.iter().any(|a| a == "--purge");
    let rest: Vec<&String> = args.iter().skip(1).filter(|a| !a.starts_with("--")).collect();
    match (args.first().map(String::as_str), rest.first()) {
        (Some("add"), Some(target)) => add(r, target, yes),
        (Some("remove"), Some(id)) => remove(r, id, purge, yes),
        (Some("list"), _) => list(r),
        (Some("search"), term) => search(term.map(|s| s.as_str())),
        _ => Err("usage: plugin add <github-folder-url | local-folder | plugin-id> [--yes] | plugin remove <id> [--purge] [--yes] | plugin list | plugin search [term]".into()),
    }
}

pub fn repo(args: &[String]) -> Result<(), String> {
    let mut repos = load_repos();
    match (args.first().map(String::as_str), args.get(1)) {
        (Some("add"), Some(url)) => {
            let src = parse_source(url)?;
            let canonical = format!("https://github.com/{}/{}", src.owner, src.repo);
            if !repos.contains(&canonical) {
                repos.push(canonical.clone());
                save_repos(&repos)?;
            }
            println!("added {canonical}");
            Ok(())
        }
        (Some("remove"), Some(url)) => {
            repos.retain(|r| r != url.trim_end_matches('/'));
            save_repos(&repos)
        }
        (Some("list"), _) | (None, _) => {
            repos.iter().for_each(|r| println!("{r}"));
            Ok(())
        }
        _ => Err("usage: repo add <github-repo-url> | repo remove <url> | repo list".into()),
    }
}

fn search(term: Option<&str>) -> Result<(), String> {
    if load_repos().is_empty() {
        return Err("no plugin repositories registered; add one with: repo add https://github.com/<owner>/<repo>".into());
    }
    for (repo, e) in index_entries().into_iter().filter(|(_, e)| term.is_none_or(|t| format!("{} {} {}", e.id, e.name, e.description).to_lowercase().contains(&t.to_lowercase()))) {
        println!("{:<14} {:<8} {}\n{:<14} {}\n{:<14} {}\n", e.id, e.version, e.name, "", e.description, "", install_url(&repo, &e));
    }
    Ok(())
}

fn add(r: &Remote, target: &str, yes: bool) -> Result<(), String> {
    let is_local = target.starts_with('/') || target.starts_with("./") || target.starts_with("../") || target.starts_with('~');
    let url = if is_local {
        format!("local:{target}")
    } else if target.starts_with("https://") {
        target.to_string()
    } else {
        let matches: Vec<_> = index_entries().into_iter().filter(|(_, e)| e.id == target).collect();
        match matches.as_slice() {
            [(repo, e)] => install_url(repo, e),
            [] => return Err(format!("no plugin '{target}' in your registered repositories (see: repo list, plugin search)")),
            _ => return Err(format!("'{target}' exists in several repositories; pass its full GitHub folder URL")),
        }
    };
    let fetched = if is_local {
        let expanded = target.replacen('~', &env::var("HOME").unwrap_or_default(), 1);
        eprintln!("==> using local folder {expanded}");
        local(&expanded)?
    } else {
        let src = parse_source(&url)?;
        eprintln!("==> fetching {}/{} @ {}", src.owner, src.repo, src.git_ref);
        fetch(&src)?
    };
    let result = (|| -> Result<(), String> {
        let p = validate(&fetched.dir)?;

        let existing = r.ssh_out(
            "for f in /etc/osnt/plugins.d/*.toml; do [ -f \"$f\" ] && echo \"$(sed -n 's/^id = \"\\(.*\\)\"/\\1/p' $f) $(sed -n 's/^port = \\(.*\\)/\\1/p' $f)\"; done; true",
        )?;
        for line in existing.lines() {
            let mut it = line.split_whitespace();
            let (eid, eport) = (it.next().unwrap_or(""), it.next().unwrap_or(""));
            if eport == p.port.to_string() && eid != p.id {
                return Err(format!("port {} is already used by plugin '{eid}'", p.port));
            }
        }
        let missing: Vec<&String> = p.requires.iter().filter(|dep| !existing.lines().any(|l| l.split_whitespace().next() == Some(dep.as_str()))).collect();
        if !missing.is_empty() {
            return Err(format!(
                "requires plugin(s) that are not installed: {}. Install them first (plugin add <id>)",
                missing.iter().map(|m| m.as_str()).collect::<Vec<_>>().join(", ")
            ));
        }
        let upgrading = existing.lines().any(|l| l.split_whitespace().next() == Some(p.id.as_str()));

        eprintln!("\n  {} {} (id {}, v{})", if upgrading { "Update" } else { "Install" }, p.name, p.id, p.version);
        eprintln!("  {}", p.description);
        eprintln!("  source: {url}\n  commit: {}", fetched.commit);
        eprintln!("  grid: {} | port: {} | state directory: {}", p.grid, p.port, if p.service.state_dir { "yes" } else { "no" });
        if !p.requires.is_empty() {
            eprintln!("  requires: {}", p.requires.join(", "));
        }
        if !p.provides.is_empty() {
            eprintln!("  provides APIs: {}", p.provides.join(", "));
        }
        if p.secrets.is_empty() {
            eprintln!("  credentials: none");
        } else {
            eprintln!("  credentials it can read: {}", p.secrets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", "));
        }
        eprintln!("\n  This builds and runs third-party code. It is built on THIS machine (build scripts run here) and runs on the Pi");
        eprintln!("  in a sandbox with network access. Read the source first: {}", fetched.dir.display());
        confirm(yes, "\n  Install this plugin?")?;

        eprintln!("==> building {} for {TARGET}", p.id);
        let target_dir = PathBuf::from(env::var("HOME").unwrap_or_default()).join(".cache/osnt/plugin-target");
        run_ok(
            Command::new("cargo")
                .current_dir(env::temp_dir())
                .env("CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER", "rust-lld")
                .args(["build", "--release", "--target", TARGET, "--target-dir"])
                .arg(&target_dir)
                .arg("--manifest-path")
                .arg(fetched.dir.join("Cargo.toml")),
            "cargo build",
        )?;
        let bin = target_dir.join(TARGET).join("release").join(format!("osnt-{}", p.id));
        let sha = {
            let out = Command::new("sha256sum").arg(&bin).output().map_err(|e| e.to_string())?;
            String::from_utf8_lossy(&out.stdout).split_whitespace().next().unwrap_or("").to_string()
        };

        eprintln!("==> installing on {}", r.host);
        r.scp(&bin.to_string_lossy(), &format!("/tmp/osnt-{}", p.id))?;
        let hash = r.ensure_token_hash(&p.id)?;
        r.ssh(&format!("sudo mkdir -p /etc/osnt/plugins.d && sudo tee /etc/osnt/plugins.d/{}.toml >/dev/null", p.id), Some(&manifest_for(&p, &hash)))?;
        let record = format!(
            "{{\"id\":\"{}\",\"name\":\"{}\",\"version\":\"{}\",\"source\":\"{url}\",\"commit\":\"{}\",\"binary_sha256\":\"{sha}\",\"requires\":[{}]}}\n",
            p.id, p.name, p.version, fetched.commit,
            p.requires.iter().map(|d| format!("\"{d}\"")).collect::<Vec<_>>().join(",")
        );
        r.ssh(&format!("sudo mkdir -p /etc/osnt/installed.d && sudo tee /etc/osnt/installed.d/{}.json >/dev/null", p.id), Some(&record))?;
        let id = &p.id;
        r.ssh(
            &format!(
                "sudo install -m 0755 /tmp/osnt-{id} /usr/local/bin/osnt-{id} && rm /tmp/osnt-{id} \
                 && sudo tee /etc/systemd/system/osnt-{id}.service >/dev/null \
                 && sudo systemctl daemon-reload && sudo systemctl enable osnt-{id} && sudo systemctl restart osnt-{id} \
                 && sleep 1 && systemctl is-active osnt-{id}"
            ),
            Some(&unit_for(&p)),
        )?;
        if !p.secrets.is_empty() {
            eprintln!("==> this plugin uses credentials: add them on the portal's Credentials page (/credentials)");
        }
        Ok(())
    })();
    if let Some(base) = &fetched.base {
        let _ = fs::remove_dir_all(base);
    }
    result
}

fn remove(r: &Remote, id: &str, purge: bool, yes: bool) -> Result<(), String> {
    if !(1..=32).contains(&id.len()) || !id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') || id == "portal" {
        return Err("invalid plugin id".into());
    }
    let dependents = r.ssh_out(&format!("grep -l '\"requires\":\\[[^]]*\"{id}\"' /etc/osnt/installed.d/*.json 2>/dev/null | xargs -r -n1 basename | sed 's/.json$//'; true"))?;
    if !dependents.is_empty() {
        eprintln!("Warning: these plugins require '{id}' and will stop working: {}", dependents.split_whitespace().collect::<Vec<_>>().join(", "));
    }
    confirm(yes, &format!("Remove plugin '{id}'{}?", if purge { " and its stored data" } else { "" }))?;
    r.ssh(
        &format!(
            "sudo systemctl disable --now osnt-{id} 2>/dev/null; sudo rm -f /usr/local/bin/osnt-{id} /etc/systemd/system/osnt-{id}.service \
             /etc/osnt/plugins.d/{id}.toml /etc/osnt/tokens/{id}.env /etc/osnt/installed.d/{id}.json; sudo systemctl daemon-reload; \
             {}echo removed {id}",
            if purge { format!("sudo rm -rf /var/lib/private/osnt-{id}; ") } else { String::new() }
        ),
        None,
    )?;
    eprintln!("(credentials stay in the portal store; remove them on the Credentials page if you no longer need them)");
    Ok(())
}

fn list(r: &Remote) -> Result<(), String> {
    let out = r.ssh_out(
        "for f in /etc/osnt/plugins.d/*.toml; do [ -f \"$f\" ] || continue; id=$(sed -n 's/^id = \"\\(.*\\)\"/\\1/p' $f); \
         name=$(sed -n 's/^name = \"\\(.*\\)\"/\\1/p' $f); st=$(systemctl is-active osnt-$id 2>/dev/null); \
         src=$(sed -n 's/.*\"source\":\"\\([^\"]*\\)\".*/\\1/p' /etc/osnt/installed.d/$id.json 2>/dev/null); \
         ver=$(sed -n 's/.*\"version\":\"\\([^\"]*\\)\".*/\\1/p' /etc/osnt/installed.d/$id.json 2>/dev/null); \
         echo \"$id|$name|$st|${ver:--}|${src:-deployed from a checkout}\"; done; true",
    )?;
    println!("{:<14} {:<26} {:<10} {:<8} SOURCE", "ID", "NAME", "STATE", "VERSION");
    for line in out.lines() {
        let f: Vec<&str> = line.splitn(5, '|').collect();
        if f.len() == 5 {
            println!("{:<14} {:<26} {:<10} {:<8} {}", f[0], f[1], f[2], f[3], f[4]);
        }
    }
    Ok(())
}
