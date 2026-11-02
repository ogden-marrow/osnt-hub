//! Sign-in setup as code: one-time setup codes and the SD-card reset file.
//!
//!   cargo deploy auth code       arm setup on the Pi with a fresh one-time code (valid 30 minutes) and print it
//!   cargo deploy auth template   put the reset file template on the Pi's boot partition (if it is not there)

use std::{fs, io::Read};

use sha2::{Digest, Sha256};

use crate::Remote;

pub const TEMPLATE: &str = include_str!("../../../deploy/OSNT-RESET-PASSKEYS.txt");

fn random(n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).expect("read /dev/urandom");
    b
}

/// Four groups of four from an alphabet without look-alike characters, e.g. K7QM-2WXD-9PHE-TRFA.
fn new_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    random(16).chunks(4).map(|g| g.iter().map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char).collect::<String>()).collect::<Vec<_>>().join("-")
}

pub fn code(r: &Remote) -> Result<(), String> {
    let code = new_code();
    let hash: String = Sha256::digest(code.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    let id: String = random(16).iter().map(|b| format!("{b:02x}")).collect();
    let request = format!("{{\"id\":\"{id}\",\"code_sha256\":\"{hash}\"}}\n");
    r.ssh("sudo install -d -m 0755 /etc/osnt/recovery && sudo tee /etc/osnt/recovery/request.json >/dev/null", Some(&request))?;
    println!("\nSetup is open for 30 minutes. Open  https://{}/setup  and enter this code:\n\n    {code}\n", crate::https::host());
    println!("Creating the passkey removes any passkeys registered before.");
    Ok(())
}

pub fn template(r: &Remote) -> Result<(), String> {
    r.ssh("test -f /boot/firmware/OSNT-RESET-PASSKEYS.txt || sudo tee /boot/firmware/OSNT-RESET-PASSKEYS.txt >/dev/null", Some(TEMPLATE))
}

/// True when the Pi has never had sign-in configured (no auth state yet).
pub fn is_fresh(r: &Remote) -> bool {
    r.ssh_out("sudo test -s /var/lib/osnt/auth.json && echo yes || echo no").map(|o| o.trim() == "no").unwrap_or(false)
}

pub fn run(r: &Remote, args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("code") => code(r),
        Some("template") => template(r),
        _ => Err("usage: auth code | template".into()),
    }
}
