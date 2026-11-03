//! Deploys OSNT components to a Raspberry Pi over SSH.
//!
//!   cargo deploy                     build and deploy the portal (the hub core)
//!   cargo deploy status [name]       show service status
//!   cargo deploy logs [name]         recent service logs
//!   cargo deploy plugin add <github-folder-url | id> | remove <id> | list | search [term]
//!   cargo deploy repo add <github-repo-url> | list | remove <url>   (plugin repositories)
//!   cargo deploy https init | renew | ca   your own CA + the Pi's HTTPS certificate
//!   cargo deploy auth reset | template     remove all passkeys (re-opens setup) / place the SD-card reset file
//!
//! Environment: OSNT_DEPLOY_HOST (default osnt@osnt-pi.local), OSNT_SSH_KEY (default ~/.ssh/osnt_pi_ed25519).

mod auth;
mod gateway;
mod https;
mod plugin;

use std::{
    env,
    io::Write,
    process::{Command, ExitCode, Stdio},
};

pub(crate) const TARGET: &str = "aarch64-unknown-linux-musl";

/// A deployable part of the hub itself. Plugins are installed with `plugin add`, not built here.
struct Component {
    name: &'static str,
    bin: &'static str,
    unit: &'static str,
}

const COMPONENTS: &[Component] = &[
    Component { name: "recover", bin: "osnt-recover", unit: include_str!("../../../deploy/osnt-recover.service") },
    Component { name: "portal", bin: "osnt-portal", unit: include_str!("../../../deploy/osnt-portal.service") },
];

pub(crate) struct Remote {
    host: String,
    key: String,
}

impl Remote {
    fn from_env() -> Self {
        let home = env::var("HOME").expect("HOME");
        Self {
            host: env::var("OSNT_DEPLOY_HOST").unwrap_or_else(|_| "osnt@osnt-pi.local".into()),
            key: env::var("OSNT_SSH_KEY").unwrap_or_else(|_| format!("{home}/.ssh/osnt_pi_ed25519")),
        }
    }

    fn ssh_opts(&self) -> Vec<String> {
        ["-i", &self.key, "-o", "IdentitiesOnly=yes", "-o", "StrictHostKeyChecking=accept-new"]
            .map(String::from)
            .to_vec()
    }

    fn ssh(&self, script: &str, stdin: Option<&str>) -> Result<(), String> {
        let mut child = Command::new("ssh")
            .args(self.ssh_opts())
            .arg(&self.host)
            .arg(script)
            .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::inherit() })
            .spawn()
            .map_err(|e| format!("ssh: {e}"))?;
        if let Some(data) = stdin {
            child.stdin.take().unwrap().write_all(data.as_bytes()).map_err(|e| e.to_string())?;
        }
        check(child.wait().map_err(|e| e.to_string())?, "ssh")
    }

    fn ssh_out(&self, script: &str) -> Result<String, String> {
        let out = Command::new("ssh").args(self.ssh_opts()).arg(&self.host).arg(script).output().map_err(|e| format!("ssh: {e}"))?;
        if !out.status.success() {
            return Err(format!("ssh failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Each plugin gets a private token in a root-only env file; only its SHA-256 goes into the (world-readable)
    /// manifest, so the portal can tell plugins apart and hand each one only the secrets it declares.
    fn ensure_token_hash(&self, id: &str) -> Result<String, String> {
        let gateway_key = gateway::derived_key(id)?;
        self.ssh_out(&format!(
            "set -e; sudo mkdir -p /etc/osnt/tokens && sudo chmod 700 /etc/osnt/tokens; f=/etc/osnt/tokens/{id}.env; \
             t=$(sudo sed -n 's/^OSNT_PLUGIN_TOKEN=//p' $f 2>/dev/null | tr -d '\\n'); \
             if [ -z \"$t\" ]; then t=$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \\n'); fi; \
             printf 'OSNT_PLUGIN_TOKEN=%s\\nOSNT_GATEWAY_KEY=%s\\n' \"$t\" '{gateway_key}' | sudo tee $f >/dev/null; sudo chmod 600 $f; \
             printf '%s' \"$t\" | sha256sum | cut -d' ' -f1"
        ))
    }

    fn scp(&self, local: &str, remote: &str) -> Result<(), String> {
        let status = Command::new("scp")
            .args(self.ssh_opts())
            .arg(local)
            .arg(format!("{}:{remote}", self.host))
            .status()
            .map_err(|e| format!("scp: {e}"))?;
        check(status, "scp")
    }
}

fn check(status: std::process::ExitStatus, what: &str) -> Result<(), String> {
    status.success().then_some(()).ok_or_else(|| format!("{what} failed: {status}"))
}

fn find(name: &str) -> Result<&'static Component, String> {
    COMPONENTS.iter().find(|c| c.name == name).ok_or_else(|| {
        let known: Vec<_> = COMPONENTS.iter().map(|c| c.name).collect();
        format!("unknown component {name:?} (known: {})", known.join(", "))
    })
}

fn build(c: &Component) -> Result<String, String> {
    let status = Command::new("cargo")
        .args(["build", "--release", "-p", c.bin, "--target", TARGET])
        .status()
        .map_err(|e| format!("cargo: {e}"))?;
    check(status, "cargo build")?;
    Ok(format!("target/{TARGET}/release/{}", c.bin))
}

fn deploy(r: &Remote, c: &Component) -> Result<(), String> {
    if c.name == "portal" {
        // HTTPS as code: the CA and the Pi's certificate are created on first deploy and pushed every time.
        let host = https::host();
        let created = https::ensure(&host, false)?;
        https::push(r, &host)?;
        gateway::push(r)?;
        if created {
            https::trust_help();
        }
    }
    if c.name == "recover" {
        // The service's sandbox needs its directories to exist before it can start.
        r.ssh("sudo install -d -m 0755 /etc/osnt/recovery", None)?;
    }
    let bin = build(c)?;
    eprintln!("==> {}: copying {bin} to {}", c.name, r.host);
    r.scp(&bin, &format!("/tmp/{}", c.bin))?;

    eprintln!("==> {}: installing and restarting service", c.name);
    let bin_name = c.bin;
    r.ssh(
        &format!(
            "sudo install -m 0755 /tmp/{bin_name} /usr/local/bin/{bin_name} && rm /tmp/{bin_name} \
             && sudo tee /etc/systemd/system/{bin_name}.service >/dev/null \
             && sudo systemctl daemon-reload && sudo systemctl enable {bin_name} \
             && sudo systemctl restart {bin_name} && sleep 1 && systemctl is-active {bin_name}"
        ),
        Some(c.unit),
    )
}

fn run(r: &Remote, args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("plugin") => plugin::run(r, &args[1..]),
        Some("https") => https::run(r, &args[1..]),
        Some("auth") => auth::run(r, &args[1..]),
        Some("repo") => plugin::repo(&args[1..]),
        Some(verb @ ("status" | "logs")) => {
            let units: Vec<&str> = match args.get(1) {
                Some(name) => vec![find(name)?.bin],
                None => COMPONENTS.iter().map(|c| c.bin).collect(),
            };
            for unit in units {
                let cmd = if verb == "status" { format!("systemctl status {unit} --no-pager") } else { format!("journalctl -u {unit} -n 40 --no-pager") };
                eprintln!("==> {unit}");
                // status exits non-zero for inactive units; keep going so every component is shown.
                let _ = r.ssh(&cmd, None);
            }
            Ok(())
        }
        _ => {
            let chosen: Vec<&Component> = if args.is_empty() {
                COMPONENTS.iter().collect()
            } else {
                args.iter().map(|a| find(a)).collect::<Result<_, _>>()?
            };
            for c in chosen {
                deploy(r, c)?;
                match c.name {
                    "recover" => auth::template(r)?,
                    // A brand-new Pi has no passkey yet: setup is open, so just say where to go.
                    "portal" if auth::is_fresh(r) => eprintln!("\nNo passkey yet. Open https://{}/setup and create yours.", https::host()),
                    _ => {}
                }
            }
            Ok(())
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match run(&Remote::from_env(), &args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
