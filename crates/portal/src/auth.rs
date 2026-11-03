//! Passkey-only sign-in (WebAuthn, ES256) and the session gate in front of everything.
//!
//! There are no passwords. Setup (creating a passkey) is open exactly when no passkey is registered: on a brand-new hub
//! the first visitor creates theirs, and once one exists setup is closed for good. Recovery is a reset: a request from
//! the SD card's boot partition (the tiny root `osnt-recover` service writes /etc/osnt/recovery/request.json at boot) or
//! from `cargo deploy auth reset` removes every passkey and session, which re-opens setup.

use std::{
    collections::HashMap,
    env, fs,
    io::Cursor,
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json,
    extract::Request,
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use ciborium::value::Value as Cbor;
use p256::{
    EncodedPoint,
    ecdsa::{Signature, VerifyingKey, signature::Verifier},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Write;

const SESSION_SECS: u64 = 14 * 24 * 3600;
const CEREMONY_SECS: u64 = 300;

#[derive(Serialize, Deserialize, Default, Clone)]
struct Cred {
    id: String,
    x: String,
    y: String,
    sign_count: u32,
    name: String,
    created: u64,
}

#[derive(Serialize, Deserialize, Clone)]
struct Session {
    hash: String,
    expires: u64,
}

#[derive(Serialize, Deserialize, Default)]
struct Store {
    user_id: String,
    credentials: Vec<Cred>,
    sessions: Vec<Session>,
    processed_request: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Setup,
    Add,
    Login,
}

struct Ceremony {
    challenge: String,
    kind: Kind,
    started: Instant,
}

static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
static CEREMONIES: Mutex<Option<HashMap<String, Ceremony>>> = Mutex::new(None);
static FAILURES: Mutex<Option<(u32, Instant)>> = Mutex::new(None);

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn state_dir() -> PathBuf {
    PathBuf::from(env::var("OSNT_STATE_DIR").unwrap_or_else(|_| "/var/lib/osnt".into()))
}

fn recovery_dir() -> PathBuf {
    PathBuf::from(env::var("OSNT_RECOVERY_DIR").unwrap_or_else(|_| "/etc/osnt/recovery".into()))
}

fn rp_id() -> String {
    env::var("OSNT_PUBLIC_HOST").unwrap_or_else(|_| "osnt-pi.local".into())
}

fn expected_origin() -> String {
    env::var("OSNT_ORIGIN").unwrap_or_else(|_| format!("https://{}", rp_id()))
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    fs::File::open("/dev/urandom").and_then(|mut f| std::io::Read::read_exact(&mut f, &mut b)).expect("read /dev/urandom");
    b
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn sha_hex(s: &str) -> String {
    hex(&Sha256::digest(s.as_bytes()))
}

fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn store() -> &'static Mutex<Store> {
    STORE.get_or_init(|| Mutex::new(fs::read_to_string(state_dir().join("auth.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()))
}

fn save(s: &Store) {
    let write = || -> std::io::Result<()> {
        fs::create_dir_all(state_dir())?;
        let path = state_dir().join("auth.json");
        let tmp = path.with_extension("tmp");
        let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(serde_json::to_string(s)?.as_bytes())?;
        f.sync_all()?;
        fs::rename(tmp, path)
    };
    if let Err(e) = write() {
        tracing::error!("could not save auth state: {e}");
    }
}

/// Pick up a reset request left by `osnt-recover` or `cargo deploy auth reset`: it removes every passkey and session,
/// which re-opens setup. Each request id is honoured once.
fn intake(s: &mut Store) {
    let Ok(text) = fs::read_to_string(recovery_dir().join("request.json")) else { return };
    let Ok(v) = serde_json::from_str::<Value>(&text) else { return };
    let Some(id) = v["id"].as_str() else { return };
    if id == s.processed_request {
        return;
    }
    tracing::warn!("sign-in reset requested: removing all passkeys and sessions; setup is open");
    s.credentials.clear();
    s.sessions.clear();
    s.processed_request = id.into();
    save(s);
}

/// Setup is open exactly when no passkey is registered.
fn setup_open(s: &Store) -> bool {
    s.credentials.is_empty()
}

fn json_err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

fn session_cookie(s: &mut Store) -> String {
    let token = hex(&random_bytes::<32>());
    let t = now();
    s.sessions.retain(|x| x.expires > t);
    s.sessions.push(Session { hash: sha_hex(&token), expires: t + SESSION_SECS });
    format!("osnt_session={token}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age={SESSION_SECS}")
}

fn cookie_value(headers: &HeaderMap) -> Option<String> {
    headers.get(header::COOKIE)?.to_str().ok()?.split(';').find_map(|c| c.trim().strip_prefix("osnt_session=").map(str::to_owned))
}

fn logged_in(headers: &HeaderMap) -> bool {
    let Some(tok) = cookie_value(headers) else { return false };
    let h = sha_hex(&tok);
    let t = now();
    store().lock().unwrap().sessions.iter().any(|s| s.expires > t && ct_eq(&s.hash, &h))
}

fn with_cookie(cookie: String, body: Value) -> Response {
    let mut res = Json(body).into_response();
    if let Ok(v) = HeaderValue::from_str(&cookie) {
        res.headers_mut().append(header::SET_COOKIE, v);
    }
    res
}

// ---- ceremonies ------------------------------------------------------------------------------------------------

fn start_ceremony(kind: Kind) -> (String, String) {
    let challenge = B64.encode(random_bytes::<32>());
    let id = hex(&random_bytes::<16>());
    let mut guard = CEREMONIES.lock().unwrap();
    let map = guard.get_or_insert_with(HashMap::new);
    map.retain(|_, c| c.started.elapsed().as_secs() < CEREMONY_SECS);
    map.insert(id.clone(), Ceremony { challenge: challenge.clone(), kind, started: Instant::now() });
    (id, challenge)
}

fn take_ceremony(id: &str, kind: Kind) -> Result<String, String> {
    let mut guard = CEREMONIES.lock().unwrap();
    let c = guard.get_or_insert_with(HashMap::new).remove(id).ok_or("this attempt expired; start again")?;
    if c.kind != kind || c.started.elapsed().as_secs() >= CEREMONY_SECS {
        return Err("this attempt expired; start again".into());
    }
    Ok(c.challenge)
}

fn too_many_failures() -> bool {
    let mut g = FAILURES.lock().unwrap();
    match g.as_mut() {
        Some((n, at)) if at.elapsed().as_secs() < 60 => *n >= 5,
        _ => {
            *g = None;
            false
        }
    }
}

fn note_failure() {
    let mut g = FAILURES.lock().unwrap();
    match g.as_mut() {
        Some((n, at)) if at.elapsed().as_secs() < 60 => *n += 1,
        _ => *g = Some((1, Instant::now())),
    }
}

// ---- WebAuthn verification (pure Rust, ES256, no attestation) ---------------------------------------------------

struct AuthData<'a> {
    rp_hash: &'a [u8],
    flags: u8,
    sign_count: u32,
    rest: &'a [u8],
}

fn parse_auth_data(d: &[u8]) -> Result<AuthData<'_>, String> {
    if d.len() < 37 {
        return Err("authenticator data too short".into());
    }
    Ok(AuthData { rp_hash: &d[..32], flags: d[32], sign_count: u32::from_be_bytes([d[33], d[34], d[35], d[36]]), rest: &d[37..] })
}

fn check_rp_and_flags(a: &AuthData) -> Result<(), String> {
    if a.rp_hash != Sha256::digest(rp_id().as_bytes()).as_slice() {
        return Err("wrong relying party (hostname mismatch)".into());
    }
    if a.flags & 0x01 == 0 || a.flags & 0x04 == 0 {
        return Err("user presence and verification are required".into());
    }
    Ok(())
}

/// Checks type, challenge and origin; returns the raw clientDataJSON bytes (needed for the assertion hash).
fn check_client_data(b64: &str, typ: &str, challenge: &str) -> Result<Vec<u8>, String> {
    let raw = B64.decode(b64).map_err(|_| "bad clientDataJSON")?;
    let v: Value = serde_json::from_slice(&raw).map_err(|_| "bad clientDataJSON")?;
    if v["type"] != typ || v["challenge"] != challenge || v["origin"] != expected_origin().as_str() {
        return Err("sign-in response did not match this attempt or site".into());
    }
    Ok(raw)
}

fn cbor_int(v: &Cbor) -> Option<i128> {
    if let Cbor::Integer(i) = v { Some(i128::from(*i)) } else { None }
}

struct NewCred {
    id: Vec<u8>,
    x: Vec<u8>,
    y: Vec<u8>,
    sign_count: u32,
}

fn verify_registration(att_b64: &str, client_b64: &str, challenge: &str) -> Result<NewCred, String> {
    check_client_data(client_b64, "webauthn.create", challenge)?;
    let att = B64.decode(att_b64).map_err(|_| "bad attestationObject")?;
    let Cbor::Map(map) = ciborium::from_reader::<Cbor, _>(&att[..]).map_err(|_| "bad attestationObject")? else { return Err("bad attestationObject".into()) };
    let auth_data = map
        .iter()
        .find_map(|(k, v)| (k == &Cbor::Text("authData".into())).then_some(v))
        .and_then(|v| if let Cbor::Bytes(b) = v { Some(b.clone()) } else { None })
        .ok_or("attestationObject has no authData")?;
    let a = parse_auth_data(&auth_data)?;
    check_rp_and_flags(&a)?;
    if a.flags & 0x40 == 0 || a.rest.len() < 18 {
        return Err("no credential in the response".into());
    }
    let id_len = u16::from_be_bytes([a.rest[16], a.rest[17]]) as usize;
    let after = &a.rest[18..];
    if id_len == 0 || id_len > 1023 || after.len() < id_len {
        return Err("bad credential id".into());
    }
    let (id, key_bytes) = after.split_at(id_len);
    let Cbor::Map(key) = ciborium::from_reader::<Cbor, _>(&mut Cursor::new(key_bytes)).map_err(|_| "bad public key")? else { return Err("bad public key".into()) };
    let get = |label: i128| key.iter().find_map(|(k, v)| (cbor_int(k) == Some(label)).then_some(v));
    let bytes = |v: Option<&Cbor>| if let Some(Cbor::Bytes(b)) = v { Some(b.clone()) } else { None };
    if get(1).and_then(cbor_int) != Some(2) || get(3).and_then(cbor_int) != Some(-7) || get(-1).and_then(cbor_int) != Some(1) {
        return Err("only ES256 (P-256) passkeys are supported".into());
    }
    let (x, y) = (bytes(get(-2)).ok_or("bad public key")?, bytes(get(-3)).ok_or("bad public key")?);
    if x.len() != 32 || y.len() != 32 {
        return Err("bad public key".into());
    }
    Ok(NewCred { id: id.to_vec(), x, y, sign_count: a.sign_count })
}

fn verifying_key(c: &Cred) -> Result<VerifyingKey, String> {
    let (x, y) = (B64.decode(&c.x).map_err(|_| "corrupt key")?, B64.decode(&c.y).map_err(|_| "corrupt key")?);
    if x.len() != 32 || y.len() != 32 {
        return Err("corrupt key".into());
    }
    let point = EncodedPoint::from_affine_coordinates(p256::FieldBytes::from_slice(&x), p256::FieldBytes::from_slice(&y), false);
    VerifyingKey::from_encoded_point(&point).map_err(|_| "corrupt key".into())
}

/// Returns the new signature counter.
fn verify_assertion(c: &Cred, auth_b64: &str, client_b64: &str, sig_b64: &str, challenge: &str) -> Result<u32, String> {
    let client = check_client_data(client_b64, "webauthn.get", challenge)?;
    let auth = B64.decode(auth_b64).map_err(|_| "bad authenticatorData")?;
    let a = parse_auth_data(&auth)?;
    check_rp_and_flags(&a)?;
    let sig = Signature::from_der(&B64.decode(sig_b64).map_err(|_| "bad signature")?).map_err(|_| "bad signature")?;
    let mut msg = auth.clone();
    msg.extend_from_slice(&Sha256::digest(&client));
    verifying_key(c)?.verify(&msg, &sig).map_err(|_| "signature check failed")?;
    if a.sign_count != 0 && c.sign_count != 0 && a.sign_count <= c.sign_count {
        return Err("this passkey looks cloned (counter did not increase)".into());
    }
    Ok(a.sign_count)
}

// ---- HTTP ------------------------------------------------------------------------------------------------------

pub async fn status(headers: HeaderMap) -> Response {
    let li = logged_in(&headers);
    let mut s = store().lock().unwrap();
    intake(&mut s);
    Json(json!({ "configured": !s.credentials.is_empty(), "setup_open": setup_open(&s), "logged_in": li, "host": rp_id() })).into_response()
}

fn creation_options(s: &mut Store, name: &str, challenge: &str, ceremony: &str) -> Value {
    if s.user_id.is_empty() {
        s.user_id = B64.encode(random_bytes::<16>());
        save(s);
    }
    json!({ "ceremony": ceremony, "publicKey": {
        "rp": { "id": rp_id(), "name": "OSNT" },
        "user": { "id": s.user_id, "name": name, "displayName": name },
        "challenge": challenge,
        "pubKeyCredParams": [ { "type": "public-key", "alg": -7 } ],
        "authenticatorSelection": { "residentKey": "preferred", "requireResidentKey": false, "userVerification": "required" },
        "attestation": "none", "timeout": 120000,
        "excludeCredentials": s.credentials.iter().map(|c| json!({ "type": "public-key", "id": c.id })).collect::<Vec<_>>(),
    }})
}

#[derive(Deserialize)]
pub struct SetupBegin {
    #[serde(default)]
    name: String,
}

fn clean_name(n: &str) -> String {
    let n: String = n.chars().filter(|c| !c.is_control()).take(40).collect();
    if n.trim().is_empty() { "OSNT user".into() } else { n.trim().to_string() }
}

pub async fn setup_begin(Json(b): Json<SetupBegin>) -> Response {
    let mut s = store().lock().unwrap();
    intake(&mut s);
    if !setup_open(&s) {
        return json_err(StatusCode::FORBIDDEN, "setup is already done; sign in with your passkey");
    }
    let (ceremony, challenge) = start_ceremony(Kind::Setup);
    Json(creation_options(&mut s, &clean_name(&b.name), &challenge, &ceremony)).into_response()
}

#[derive(Deserialize)]
pub struct Finish {
    ceremony: String,
    #[serde(default)]
    name: String,
    response: Value,
}

fn register(s: &mut Store, f: &Finish, kind: Kind) -> Result<(), String> {
    let challenge = take_ceremony(&f.ceremony, kind)?;
    let r = &f.response;
    let (att, client) = (r["attestationObject"].as_str().ok_or("missing attestation")?, r["clientDataJSON"].as_str().ok_or("missing clientData")?);
    let nc = verify_registration(att, client, &challenge)?;
    let id = B64.encode(&nc.id);
    if s.credentials.iter().any(|c| c.id == id) {
        return Err("this passkey is already registered".into());
    }
    s.credentials.push(Cred { id, x: B64.encode(&nc.x), y: B64.encode(&nc.y), sign_count: nc.sign_count, name: clean_name(&f.name), created: now() });
    Ok(())
}

pub async fn setup_finish(Json(f): Json<Finish>) -> Response {
    let mut s = store().lock().unwrap();
    intake(&mut s);
    if !setup_open(&s) {
        return json_err(StatusCode::FORBIDDEN, "setup is already done; sign in with your passkey");
    }
    if let Err(e) = register(&mut s, &f, Kind::Setup) {
        return json_err(StatusCode::BAD_REQUEST, &e);
    }
    let cookie = session_cookie(&mut s);
    save(&s);
    with_cookie(cookie, json!({ "ok": true }))
}

/// Add another passkey while signed in.
pub async fn add_begin(headers: HeaderMap, Json(b): Json<SetupBegin>) -> Response {
    if !logged_in(&headers) {
        return json_err(StatusCode::UNAUTHORIZED, "sign in first");
    }
    let (ceremony, challenge) = start_ceremony(Kind::Add);
    let mut s = store().lock().unwrap();
    Json(creation_options(&mut s, &clean_name(&b.name), &challenge, &ceremony)).into_response()
}

pub async fn add_finish(headers: HeaderMap, Json(f): Json<Finish>) -> Response {
    if !logged_in(&headers) {
        return json_err(StatusCode::UNAUTHORIZED, "sign in first");
    }
    let mut s = store().lock().unwrap();
    match register(&mut s, &f, Kind::Add) {
        Ok(()) => {
            save(&s);
            Json(json!({ "ok": true })).into_response()
        }
        Err(e) => json_err(StatusCode::BAD_REQUEST, &e),
    }
}

pub async fn login_begin() -> Response {
    let mut s = store().lock().unwrap();
    intake(&mut s);
    if s.credentials.is_empty() {
        return json_err(StatusCode::CONFLICT, "no passkey is registered yet");
    }
    let (ceremony, challenge) = start_ceremony(Kind::Login);
    Json(json!({ "ceremony": ceremony, "publicKey": {
        "rpId": rp_id(), "challenge": challenge, "userVerification": "required", "timeout": 120000,
        "allowCredentials": s.credentials.iter().map(|c| json!({ "type": "public-key", "id": c.id })).collect::<Vec<_>>(),
    }}))
    .into_response()
}

#[derive(Deserialize)]
pub struct LoginFinish {
    ceremony: String,
    id: String,
    response: Value,
}

pub async fn login_finish(Json(f): Json<LoginFinish>) -> Response {
    if too_many_failures() {
        return json_err(StatusCode::TOO_MANY_REQUESTS, "too many failed attempts; wait a minute");
    }
    let fail = |msg: &str| {
        note_failure();
        json_err(StatusCode::UNAUTHORIZED, msg)
    };
    let challenge = match take_ceremony(&f.ceremony, Kind::Login) {
        Ok(c) => c,
        Err(e) => return fail(&e),
    };
    let r = &f.response;
    let (Some(a), Some(c), Some(sig)) = (r["authenticatorData"].as_str(), r["clientDataJSON"].as_str(), r["signature"].as_str()) else { return fail("incomplete response") };
    let mut s = store().lock().unwrap();
    let Some(idx) = s.credentials.iter().position(|c| c.id == f.id) else { return fail("unknown passkey") };
    match verify_assertion(&s.credentials[idx], a, c, sig, &challenge) {
        Ok(count) => {
            s.credentials[idx].sign_count = count;
            let cookie = session_cookie(&mut s);
            save(&s);
            with_cookie(cookie, json!({ "ok": true }))
        }
        Err(e) => fail(&e),
    }
}

pub async fn logout(headers: HeaderMap) -> Response {
    if let Some(tok) = cookie_value(&headers) {
        let h = sha_hex(&tok);
        let mut s = store().lock().unwrap();
        s.sessions.retain(|x| x.hash != h);
        save(&s);
    }
    with_cookie("osnt_session=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0".into(), json!({ "ok": true }))
}

/// Everything except the sign-in pages and /auth/* requires a valid session.
pub async fn gate(req: Request, next: Next) -> Response {
    let path = req.uri().path();
    if path == "/login" || path == "/setup" || path.starts_with("/auth/") || logged_in(req.headers()) {
        return next.run(req).await;
    }
    let wants_html = req.method() == Method::GET && req.headers().get(header::ACCEPT).and_then(|v| v.to_str().ok()).is_some_and(|a| a.contains("text/html"));
    if wants_html {
        let configured = !store().lock().unwrap().credentials.is_empty();
        return Redirect::temporary(if configured { "/login" } else { "/setup" }).into_response();
    }
    json_err(StatusCode::UNAUTHORIZED, "sign in required")
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::{SigningKey, signature::Signer};

    const CHALLENGE: &str = "dGVzdC1jaGFsbGVuZ2U";

    fn client_json(typ: &str, challenge: &str, origin: &str) -> (String, Vec<u8>) {
        let raw = serde_json::to_vec(&json!({ "type": typ, "challenge": challenge, "origin": origin })).unwrap();
        (B64.encode(&raw), raw)
    }

    fn cose(sk: &SigningKey) -> Vec<u8> {
        let p = sk.verifying_key().to_encoded_point(false);
        let key = Cbor::Map(vec![
            (Cbor::Integer(1.into()), Cbor::Integer(2.into())),
            (Cbor::Integer(3.into()), Cbor::Integer((-7).into())),
            (Cbor::Integer((-1).into()), Cbor::Integer(1.into())),
            (Cbor::Integer((-2).into()), Cbor::Bytes(p.x().unwrap().to_vec())),
            (Cbor::Integer((-3).into()), Cbor::Bytes(p.y().unwrap().to_vec())),
        ]);
        let mut out = Vec::new();
        ciborium::into_writer(&key, &mut out).unwrap();
        out
    }

    fn auth_data(rp: &str, flags: u8, count: u32, cred: Option<(&[u8], Vec<u8>)>) -> Vec<u8> {
        let mut d = Sha256::digest(rp.as_bytes()).to_vec();
        d.push(flags);
        d.extend_from_slice(&count.to_be_bytes());
        if let Some((id, key)) = cred {
            d.extend_from_slice(&[0u8; 16]);
            d.extend_from_slice(&(id.len() as u16).to_be_bytes());
            d.extend_from_slice(id);
            d.extend_from_slice(&key);
        }
        d
    }

    fn attestation(auth: Vec<u8>) -> String {
        let obj = Cbor::Map(vec![(Cbor::Text("fmt".into()), Cbor::Text("none".into())), (Cbor::Text("attStmt".into()), Cbor::Map(vec![])), (Cbor::Text("authData".into()), Cbor::Bytes(auth))]);
        let mut out = Vec::new();
        ciborium::into_writer(&obj, &mut out).unwrap();
        B64.encode(&out)
    }

    fn registered(sk: &SigningKey) -> Cred {
        let (client, _) = client_json("webauthn.create", CHALLENGE, &expected_origin());
        let att = attestation(auth_data(&rp_id(), 0x45, 0, Some((b"credential-1", cose(sk)))));
        let nc = verify_registration(&att, &client, CHALLENGE).expect("registration verifies");
        assert_eq!(nc.id, b"credential-1");
        Cred { id: B64.encode(&nc.id), x: B64.encode(&nc.x), y: B64.encode(&nc.y), sign_count: nc.sign_count, name: "t".into(), created: 0 }
    }

    fn assertion(sk: &SigningKey, rp: &str, flags: u8, count: u32, origin: &str, challenge: &str) -> (String, String, String) {
        let auth = auth_data(rp, flags, count, None);
        let (client_b64, client_raw) = client_json("webauthn.get", challenge, origin);
        let mut msg = auth.clone();
        msg.extend_from_slice(&Sha256::digest(&client_raw));
        let sig: Signature = sk.sign(&msg);
        (B64.encode(&auth), client_b64, B64.encode(sig.to_der().as_bytes()))
    }

    #[test]
    fn registration_and_login_round_trip() {
        let sk = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let cred = registered(&sk);
        let (a, c, s) = assertion(&sk, &rp_id(), 0x05, 5, &expected_origin(), CHALLENGE);
        assert_eq!(verify_assertion(&cred, &a, &c, &s, CHALLENGE), Ok(5));
    }

    #[test]
    fn rejects_wrong_challenge_origin_rp_and_flags() {
        let sk = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let cred = registered(&sk);
        let (a, c, s) = assertion(&sk, &rp_id(), 0x05, 1, &expected_origin(), "other-challenge");
        assert!(verify_assertion(&cred, &a, &c, &s, CHALLENGE).is_err());
        let (a, c, s) = assertion(&sk, &rp_id(), 0x05, 1, "https://evil.example", CHALLENGE);
        assert!(verify_assertion(&cred, &a, &c, &s, CHALLENGE).is_err());
        let (a, c, s) = assertion(&sk, "evil.example", 0x05, 1, &expected_origin(), CHALLENGE);
        assert!(verify_assertion(&cred, &a, &c, &s, CHALLENGE).is_err());
        let (a, c, s) = assertion(&sk, &rp_id(), 0x01, 1, &expected_origin(), CHALLENGE); // no user verification
        assert!(verify_assertion(&cred, &a, &c, &s, CHALLENGE).is_err());
    }

    #[test]
    fn rejects_other_keys_tampering_and_replayed_counters() {
        let sk = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let attacker = SigningKey::from_slice(&[9u8; 32]).unwrap();
        let mut cred = registered(&sk);
        let (a, c, s) = assertion(&attacker, &rp_id(), 0x05, 1, &expected_origin(), CHALLENGE);
        assert!(verify_assertion(&cred, &a, &c, &s, CHALLENGE).is_err(), "signature from another key");
        let (a, c, mut s) = assertion(&sk, &rp_id(), 0x05, 1, &expected_origin(), CHALLENGE);
        let mut raw = B64.decode(&s).unwrap();
        *raw.last_mut().unwrap() ^= 1;
        s = B64.encode(&raw);
        assert!(verify_assertion(&cred, &a, &c, &s, CHALLENGE).is_err(), "tampered signature");
        cred.sign_count = 10;
        let (a, c, s) = assertion(&sk, &rp_id(), 0x05, 10, &expected_origin(), CHALLENGE);
        assert!(verify_assertion(&cred, &a, &c, &s, CHALLENGE).unwrap_err().contains("cloned"));
        let (a, c, s) = assertion(&sk, &rp_id(), 0x05, 0, &expected_origin(), CHALLENGE);
        assert_eq!(verify_assertion(&cred, &a, &c, &s, CHALLENGE), Ok(0), "synced passkeys report a zero counter");
    }

    #[test]
    fn registration_rejects_bad_input() {
        let sk = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let (client, _) = client_json("webauthn.create", CHALLENGE, &expected_origin());
        let no_at = attestation(auth_data(&rp_id(), 0x05, 0, None));
        assert!(verify_registration(&no_at, &client, CHALLENGE).is_err());
        let wrong_rp = attestation(auth_data("evil.example", 0x45, 0, Some((b"x", cose(&sk)))));
        assert!(verify_registration(&wrong_rp, &client, CHALLENGE).is_err());
        let (client_get, _) = client_json("webauthn.get", CHALLENGE, &expected_origin());
        let ok = attestation(auth_data(&rp_id(), 0x45, 0, Some((b"x", cose(&sk)))));
        assert!(verify_registration(&ok, &client_get, CHALLENGE).is_err(), "wrong ceremony type");
        assert!(verify_registration("!!!", &client, CHALLENGE).is_err());
    }
}
