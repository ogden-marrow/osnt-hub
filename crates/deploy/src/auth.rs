//! Sign-in setup as code: one-time setup codes and the SD-card reset file.
//!
//!   cargo deploy auth reset      remove all passkeys on the Pi (re-opens setup); same as the SD-card reset file
//!   cargo deploy auth template   put the reset file template on the Pi's boot partition (if it is not there)

use std::{fs, io::Read};

use crate::Remote;

pub const TEMPLATE: &str = include_str!("../../../deploy/OSNT-RESET-PASSKEYS.txt");

fn random_hex(n: usize) -> String {
    let mut b = vec![0u8; n];
    fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).expect("read /dev/urandom");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Remove every passkey and session on the Pi, which re-opens setup (same effect as the SD-card reset file).
pub fn reset(r: &Remote) -> Result<(), String> {
    let request = format!("{{\"id\":\"{}\"}}\n", random_hex(16));
    r.ssh("sudo install -d -m 0755 /etc/osnt/recovery && sudo tee /etc/osnt/recovery/request.json >/dev/null", Some(&request))?;
    println!("\nAll passkeys will be removed the next time the portal checks (a few seconds). Open https://{}/setup now to create a new one.", crate::https::host());
    Ok(())
}

pub fn template(r: &Remote) -> Result<(), String> {
    // Written when missing or when it is an older template; a file that has an active RESET line is left alone.
    r.ssh("f=/boot/firmware/OSNT-RESET-PASSKEYS.txt; if ! test -f $f || { ! grep -q '^RESET' $f && ! grep -q 'Type the word RESET' $f; }; then sudo tee $f >/dev/null; fi", Some(TEMPLATE))
}

/// True when the Pi has never had sign-in configured (no auth state yet).
pub fn is_fresh(r: &Remote) -> bool {
    r.ssh_out("sudo test -s /var/lib/osnt/auth.json && echo yes || echo no").map(|o| o.trim() == "no").unwrap_or(false)
}

pub fn run(r: &Remote, args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("reset") => reset(r),
        Some("template") => template(r),
        _ => Err("usage: auth reset | template".into()),
    }
}
