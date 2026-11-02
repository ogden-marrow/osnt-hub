//! Boot-time sign-in recovery. Runs as root before the portal starts.
//!
//! If the boot partition holds OSNT-RESET-PASSKEYS.txt with a one-time code on its first non-comment line, this writes a
//! reset request (id + SHA-256 of the code, never the code) to /etc/osnt/recovery/request.json and puts the file back to
//! its template so the code does not stay on the card. The portal then removes all passkeys and opens setup for 30 minutes.
//! Physical access to the SD card is the only way to trigger it, which is the point.

use std::{env, fs, io::Read, path::Path};

use sha2::{Digest, Sha256};

const FILE_NAME: &str = "OSNT-RESET-PASSKEYS.txt";
const MIN_CODE_LEN: usize = 12;

const TEMPLATE: &str = include_str!("../../../deploy/OSNT-RESET-PASSKEYS.txt");

fn random_hex() -> String {
    let mut b = [0u8; 16];
    fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).expect("read /dev/urandom");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn main() {
    let boot = env::var("OSNT_BOOT_DIR").unwrap_or_else(|_| "/boot/firmware".into());
    let out_dir = env::var("OSNT_RECOVERY_DIR").unwrap_or_else(|_| "/etc/osnt/recovery".into());
    let file = Path::new(&boot).join(FILE_NAME);

    let Ok(text) = fs::read_to_string(&file) else {
        println!("osnt-recover: no reset file, nothing to do");
        return;
    };
    let Some(code) = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#')) else {
        println!("osnt-recover: reset file has no code, nothing to do");
        return;
    };
    if code.len() < MIN_CODE_LEN {
        eprintln!("osnt-recover: the code in {FILE_NAME} is shorter than {MIN_CODE_LEN} characters; ignoring it");
        return;
    }

    let hash: String = Sha256::digest(code.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    let request = format!("{{\"id\":\"{}\",\"code_sha256\":\"{hash}\"}}\n", random_hex());
    fs::create_dir_all(&out_dir).expect("create recovery dir");
    let (tmp, dst) = (Path::new(&out_dir).join("request.json.tmp"), Path::new(&out_dir).join("request.json"));
    fs::write(&tmp, request).expect("write request");
    fs::rename(&tmp, &dst).expect("publish request");

    // Blank the code from the card. If this fails the request still exists, and the same code would re-arm only once
    // because the portal remembers processed request ids.
    if let Err(e) = fs::write(&file, TEMPLATE) {
        eprintln!("osnt-recover: could not rewrite {FILE_NAME}: {e}");
    }
    println!("osnt-recover: reset requested; setup opens on the portal for 30 minutes");
}
