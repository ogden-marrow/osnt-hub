//! Deploys the OSNT portal to a Raspberry Pi over SSH.
//!
//!   cargo deploy            build, copy, install unit, restart
//!   cargo deploy status     show service status
//!   cargo deploy logs       tail service logs
//!
//! Environment: OSNT_DEPLOY_HOST (default osnt@osnt-pi.local), OSNT_SSH_KEY (default ~/.ssh/osnt_pi_ed25519).

use std::{
    env,
    io::Write,
    process::{Command, ExitCode, Stdio},
};

const TARGET: &str = "aarch64-unknown-linux-musl";
const BIN: &str = "osnt-portal";
const UNIT: &str = include_str!("../../../deploy/osnt-portal.service");

struct Remote {
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

fn build() -> Result<String, String> {
    let status = Command::new("cargo")
        .args(["build", "--release", "-p", BIN, "--target", TARGET])
        .status()
        .map_err(|e| format!("cargo: {e}"))?;
    check(status, "cargo build")?;
    Ok(format!("target/{TARGET}/release/{BIN}"))
}

fn deploy(r: &Remote) -> Result<(), String> {
    let bin = build()?;
    eprintln!("==> copying {bin} to {}", r.host);
    r.scp(&bin, &format!("/tmp/{BIN}"))?;
    eprintln!("==> installing and restarting {BIN}");
    r.ssh(
        &format!(
            "sudo install -m 0755 /tmp/{BIN} /usr/local/bin/{BIN} && rm /tmp/{BIN} \
             && sudo tee /etc/systemd/system/{BIN}.service >/dev/null \
             && sudo systemctl daemon-reload && sudo systemctl enable {BIN} \
             && sudo systemctl restart {BIN} && sleep 1 && systemctl is-active {BIN}"
        ),
        Some(UNIT),
    )
}

fn main() -> ExitCode {
    let r = Remote::from_env();
    let result = match env::args().nth(1).as_deref() {
        None | Some("deploy") => deploy(&r),
        Some("status") => r.ssh(&format!("systemctl status {BIN} --no-pager"), None),
        Some("logs") => r.ssh(&format!("journalctl -u {BIN} -n 50 --no-pager"), None),
        Some(other) => Err(format!("unknown command {other:?} (deploy | status | logs)")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
