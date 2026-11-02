//! Boot-time sign-in recovery. Runs as root before the portal starts.
//!
//! If the boot partition holds OSNT-RESET-PASSKEYS.txt with a line that says RESET (no leading #), this writes a reset
//! request to /etc/osnt/recovery/request.json and puts the file back to its template so it only fires once. The portal then
//! removes all passkeys and sessions, which re-opens setup. Physical access to the SD card is the only way to trigger it.

use std::{env, fs, io::Read, path::Path};

const FILE_NAME: &str = "OSNT-RESET-PASSKEYS.txt";

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
    if !text.lines().any(|l| l.trim().eq_ignore_ascii_case("reset")) {
        println!("osnt-recover: no RESET line in the reset file, nothing to do");
        return;
    }
    let request = format!("{{\"id\":\"{}\"}}\n", random_hex());
    fs::create_dir_all(&out_dir).expect("create recovery dir");
    let (tmp, dst) = (Path::new(&out_dir).join("request.json.tmp"), Path::new(&out_dir).join("request.json"));
    fs::write(&tmp, request).expect("write request");
    fs::rename(&tmp, &dst).expect("publish request");

    // Put the template back so the reset fires only once. If this fails the request still exists, and the portal only
    // honours each request id once.
    if let Err(e) = fs::write(&file, TEMPLATE) {
        eprintln!("osnt-recover: could not rewrite {FILE_NAME}: {e}");
    }
    println!("osnt-recover: reset requested; the portal will remove all passkeys and re-open setup");
}
