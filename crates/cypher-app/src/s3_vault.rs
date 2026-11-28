//! S3 접속 정보 금고: 저장소 주소와, 암호화한 접근 키·비밀 키를 `%APPDATA%\cypher\s3\connections.json`에 둔다.
//!
//! 키는 파일에 그대로 적지 않는다. 두 겹으로 잠근다:
//!
//! 1. (고르면) **잠금 암호**: 사용자가 정한 암호에서 PBKDF2-HMAC-SHA256(60만 번)으로 끌어낸 키로 AES-256-GCM 암호화한다.
//!    잠금 암호는 어디에도 저장하지 않고 접속할 때마다 묻는다. 그래서 이 PC에서 같은 Windows 계정으로 도는 다른 프로그램도,
//!    파일과 Windows 계정을 통째로 가져간 사람도 잠금 암호 없이는 풀지 못한다.
//! 2. (늘) **Windows 계정 잠금**(DPAPI, `CryptProtectData`): 지금 로그인한 Windows 계정으로만 풀린다.
//!    파일만 복사해 가서는(다른 PC, 다른 계정, 백업·동기화 폴더에서 샌 사본) 풀 수 없다.
//!
//! 한계(사용자에게도 알린다):
//! - 키는 요청에 서명하는 데 원문이 필요하므로 비밀번호처럼 "되돌릴 수 없게" 저장할 수는 없다.
//! - Windows 계정 잠금은 이 파일 하나만 샜을 때를 막는다. 사용자 폴더를 통째로 가져간 사람(그 안에 DPAPI의 열쇠가
//!   Windows 암호로 잠겨 들어 있다)이 Windows 암호까지 알면 풀 수 있고, 회사 도메인의 계정이면 도메인 관리자도 풀 수 있다.
//!   잠금 암호를 쓰지 않으면 같은 Windows 계정으로 도는 악성 프로그램도 풀 수 있다.
//! - 잠금 암호는 저장해 둔 상태를 지킨다. 지키는 힘은 암호의 길이에 달려 있어(짧은 암호는 대입해 보면 뚫린다) 8자 이상만 받는다.
//!   접속하려고 암호를 치는 순간과 접속해 있는 동안에는 암호와 키가 이 프로그램의 메모리에 있다
//!   (쓰고 난 사본은 지우지만, 화면에서 넘어오는 길의 사본까지 모두 지우지는 못한다).
//!
//! 화면에는 키를 돌려주지 않는다. 화면은 키를 저장할 때 한 번 넘겨줄 뿐이고, 고칠 때는 비워 두면 그대로 둔다.

use base64::Engine;
use cypher_core::s3::{parse_location, Endpoint, Signature};
use ring::rand::SecureRandom;
use serde::{Deserialize, Serialize};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use zeroize::Zeroize;

/// 암호화한 덩어리의 머리: 형식과 판. 그 뒤의 한 바이트가 잠금 방식(0 = Windows 계정만, 1 = 잠금 암호도).
const MAGIC: &[u8; 4] = b"CYS1";
const PBKDF2_ROUNDS: u32 = 600_000;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const MAX_CONNECTIONS: usize = 200;
/// 잠금 암호의 가장 짧은 길이.
const MIN_PASSWORD: usize = 8;

/// 금고 파일을 읽고 쓰는 동안 잡는다(읽고 → 고치고 → 쓰는 사이에 다른 저장이 끼지 않게).
static LOCK: Mutex<()> = Mutex::new(());

pub fn vault_path() -> PathBuf {
    crate::user_dir().join("s3").join("connections.json")
}

/// 접근 키와 비밀 키. 쓰고 나면 메모리에서 지운다.
#[derive(Clone, Serialize, Deserialize)]
pub struct Keys {
    pub access: String,
    pub secret: String,
}

impl Drop for Keys {
    fn drop(&mut self) {
        self.access.zeroize();
        self.secret.zeroize();
    }
}

/// 파일에 적는 접속 정보 하나.
#[derive(Clone, Serialize, Deserialize)]
struct Stored {
    id: String,
    name: String,
    endpoint: String,
    #[serde(default)]
    region: String,
    #[serde(default = "yes")]
    path_style: bool,
    #[serde(default)]
    signature: String,
    /// 접속하면 처음 열 곳(`s3://버킷/폴더/`). 비어 있으면 버킷 목록.
    #[serde(default)]
    start: String,
    /// 서버 인증서를 확인하지 않을지(스스로 서명한 인증서를 쓰는 저장소).
    #[serde(default)]
    insecure: bool,
    /// 잠금 암호로도 잠갔는지.
    #[serde(default)]
    locked: bool,
    /// 암호화한 키(Base64).
    #[serde(default)]
    secret: String,
}

fn yes() -> bool {
    true
}

#[derive(Default, Serialize, Deserialize)]
struct Vault {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    connections: Vec<Stored>,
}

/// 화면에 보여 줄 접속 정보(키는 없다).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConnInfo {
    pub id: String,
    pub name: String,
    pub endpoint: String,
    pub region: String,
    pub path_style: bool,
    pub signature: String,
    pub start: String,
    pub insecure: bool,
    pub locked: bool,
}

impl From<&Stored> for ConnInfo {
    fn from(stored: &Stored) -> ConnInfo {
        ConnInfo {
            id: stored.id.clone(),
            name: stored.name.clone(),
            endpoint: stored.endpoint.clone(),
            region: stored.region.clone(),
            path_style: stored.path_style,
            signature: stored.signature.clone(),
            start: stored.start.clone(),
            insecure: stored.insecure,
            locked: stored.locked,
        }
    }
}

/// 화면에서 받은 접속 정보. `id`가 있으면 그것을 고친다.
#[derive(Deserialize)]
pub struct ConnInput {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: String,
    pub endpoint: String,
    #[serde(default)]
    pub region: String,
    #[serde(default = "yes")]
    pub path_style: bool,
    #[serde(default)]
    pub signature: String,
    #[serde(default)]
    pub start: String,
    #[serde(default)]
    pub insecure: bool,
    /// 접근 키와 비밀 키. 고칠 때 둘 다 비워 두면 저장해 둔 것을 그대로 둔다.
    #[serde(default)]
    pub access: String,
    #[serde(default)]
    pub secret: String,
    /// 잠금 암호(선택). 키를 새로 적을 때만 정할 수 있다.
    #[serde(default)]
    pub password: String,
}

impl Drop for ConnInput {
    fn drop(&mut self) {
        self.access.zeroize();
        self.secret.zeroize();
        self.password.zeroize();
    }
}

/// 접속하는 데 필요한 것 모두(금고에서 꺼내 키를 푼 것).
pub struct Opened {
    pub info: ConnInfo,
    pub endpoint: Endpoint,
    pub signature: Signature,
    pub keys: Keys,
}

// ───────────────────────── 암호화 ─────────────────────────

#[repr(C)]
struct Blob {
    len: u32,
    data: *mut u8,
}

#[link(name = "crypt32")]
extern "system" {
    fn CryptProtectData(input: *const Blob, description: *const u16, entropy: *const Blob, reserved: *mut c_void, prompt: *mut c_void, flags: u32, output: *mut Blob) -> i32;
    fn CryptUnprotectData(input: *const Blob, description: *mut *mut u16, entropy: *const Blob, reserved: *mut c_void, prompt: *mut c_void, flags: u32, output: *mut Blob) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
}

/// 창을 띄우지 않는다(묻는 창이 필요한 상황이면 실패로 끝낸다).
const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;

/// Windows 계정 잠금(DPAPI)을 걸거나 푼다. `entropy`는 같은 값으로 풀어야 하는 덧붙임 값이다.
fn dpapi(data: &[u8], entropy: &[u8], protect: bool) -> Result<Vec<u8>, String> {
    let len = |bytes: &[u8]| u32::try_from(bytes.len()).map_err(|_| "너무 큰 값입니다.".to_string());
    let input = Blob { len: len(data)?, data: data.as_ptr() as *mut u8 };
    let extra = Blob { len: len(entropy)?, data: entropy.as_ptr() as *mut u8 };
    let mut output = Blob { len: 0, data: std::ptr::null_mut() };
    // SAFETY: 입력 덩어리는 호출하는 동안 살아 있는 슬라이스를 가리키고(읽기만 한다), 출력은 Windows가 할당해 채운다.
    let ok = unsafe {
        if protect {
            CryptProtectData(&input, std::ptr::null(), &extra, std::ptr::null_mut(), std::ptr::null_mut(), CRYPTPROTECT_UI_FORBIDDEN, &mut output)
        } else {
            CryptUnprotectData(&input, std::ptr::null_mut(), &extra, std::ptr::null_mut(), std::ptr::null_mut(), CRYPTPROTECT_UI_FORBIDDEN, &mut output)
        }
    };
    if ok == 0 || output.data.is_null() {
        return Err(if protect {
            format!("Windows 계정으로 잠그지 못했습니다: {}", std::io::Error::last_os_error())
        } else {
            "저장해 둔 키를 이 Windows 계정으로 풀 수 없습니다(다른 PC나 다른 계정에서 저장했거나, 파일이 바뀌었습니다). Access key와 Secret key를 다시 적어 저장해 주세요.".to_string()
        });
    }
    // SAFETY: 성공했으므로 `output`은 Windows가 할당한 `len` 바이트를 가리킨다. 복사한 뒤 지우고 돌려준다.
    let bytes = unsafe {
        let slice = std::slice::from_raw_parts_mut(output.data, output.len as usize);
        let copy = slice.to_vec();
        slice.zeroize();
        LocalFree(output.data.cast());
        copy
    };
    Ok(bytes)
}

fn random(buffer: &mut [u8]) -> Result<(), String> {
    ring::rand::SystemRandom::new().fill(buffer).map_err(|_| "난수를 만들지 못했습니다.".to_string())
}

/// 잠금 암호에서 AES 키를 끌어낸다.
fn password_key(password: &str, salt: &[u8]) -> ring::aead::LessSafeKey {
    let mut key = [0u8; 32];
    let rounds = std::num::NonZeroU32::new(PBKDF2_ROUNDS).expect("0이 아니다");
    ring::pbkdf2::derive(ring::pbkdf2::PBKDF2_HMAC_SHA256, rounds, salt, password.as_bytes(), &mut key);
    let unbound = ring::aead::UnboundKey::new(&ring::aead::AES_256_GCM, &key).expect("키 길이가 맞다");
    key.zeroize();
    ring::aead::LessSafeKey::new(unbound)
}

/// Windows 계정 잠금에 덧붙이는 값: 접속 정보의 번호에 묶어, 다른 접속 정보의 덩어리를 옮겨 붙여 쓰지 못하게 한다.
fn entropy(id: &str) -> Vec<u8> {
    format!("cypher.s3.v1:{id}").into_bytes()
}

/// 키를 암호화해 파일에 적을 글(Base64)로 만든다. `password`가 있으면 잠금 암호로 먼저 잠근다.
fn seal(id: &str, keys: &Keys, password: Option<&str>) -> Result<String, String> {
    let mut plain = serde_json::to_vec(keys).map_err(|e| e.to_string())?;
    let (mode, mut inner) = match password {
        Some(password) => {
            let mut head = vec![0u8; SALT_LEN + NONCE_LEN];
            random(&mut head)?;
            let key = password_key(password, &head[..SALT_LEN]);
            let nonce = ring::aead::Nonce::try_assume_unique_for_key(&head[SALT_LEN..]).expect("길이가 맞다");
            let mut sealed = std::mem::take(&mut plain);
            key.seal_in_place_append_tag(nonce, ring::aead::Aad::from(entropy(id)), &mut sealed).map_err(|_| "암호화하지 못했습니다.".to_string())?;
            head.extend_from_slice(&sealed);
            (1u8, head)
        }
        None => (0u8, std::mem::take(&mut plain)),
    };
    let wrapped = dpapi(&inner, &entropy(id), true);
    inner.zeroize();
    plain.zeroize();
    let mut blob = MAGIC.to_vec();
    blob.push(mode);
    blob.extend_from_slice(&wrapped?);
    Ok(base64::engine::general_purpose::STANDARD.encode(blob))
}

/// 파일에 적힌 글에서 키를 꺼낸다.
fn open(id: &str, sealed: &str, password: Option<&str>) -> Result<Keys, String> {
    let broken = || "저장해 둔 키를 읽을 수 없습니다(파일이 손상됐습니다). 접근 키와 비밀 키를 다시 적어 저장해 주세요.".to_string();
    let blob = base64::engine::general_purpose::STANDARD.decode(sealed.trim()).map_err(|_| broken())?;
    if blob.len() < MAGIC.len() + 1 || &blob[..MAGIC.len()] != MAGIC {
        return Err(broken());
    }
    let mode = blob[MAGIC.len()];
    let mut inner = dpapi(&blob[MAGIC.len() + 1..], &entropy(id), false)?;
    let mut plain = match mode {
        0 => std::mem::take(&mut inner),
        1 => {
            let password = password.filter(|p| !p.is_empty()).ok_or("잠금 암호가 필요합니다.")?;
            if inner.len() < SALT_LEN + NONCE_LEN + ring::aead::AES_256_GCM.tag_len() {
                return Err(broken());
            }
            let key = password_key(password, &inner[..SALT_LEN]);
            let nonce = ring::aead::Nonce::try_assume_unique_for_key(&inner[SALT_LEN..SALT_LEN + NONCE_LEN]).expect("길이가 맞다");
            let mut sealed = inner.split_off(SALT_LEN + NONCE_LEN);
            let length = key
                .open_in_place(nonce, ring::aead::Aad::from(entropy(id)), &mut sealed)
                .map_err(|_| "잠금 암호가 맞지 않습니다.".to_string())?
                .len();
            sealed.truncate(length);
            sealed
        }
        _ => return Err(broken()),
    };
    let keys = serde_json::from_slice::<Keys>(&plain).map_err(|_| broken());
    plain.zeroize();
    inner.zeroize();
    keys
}

// ───────────────────────── 파일 ─────────────────────────

fn load(path: &Path) -> Result<Vault, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("S3 접속 정보 파일을 읽을 수 없습니다({}): {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vault::default()),
        Err(e) => Err(format!("S3 접속 정보 파일을 읽을 수 없습니다({}): {e}", path.display())),
    }
}

/// 옆에 새로 쓴 뒤 이름을 바꿔 넣는다(쓰다가 꺼져도 앞의 파일이 남는다).
fn store(path: &Path, vault: &Vault) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("S3 접속 정보를 저장하지 못했습니다({}): {e}", path.display());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(fail)?;
    }
    let text = serde_json::to_vec_pretty(vault).map_err(|e| e.to_string())?;
    let temp = path.with_extension("json.new");
    {
        use std::io::Write;
        let mut file = std::fs::File::create(&temp).map_err(fail)?;
        file.write_all(&text).map_err(fail)?;
        // 이름을 바꾸기 전에 디스크까지 쓴다(바꾼 직후 전원이 나가도 빈 파일이 남지 않게).
        file.sync_all().map_err(fail)?;
    }
    std::fs::rename(&temp, path).map_err(fail)
}

/// 지역을 적지 않았을 때 서명에 넣을 지역 이름. Ncloud Storage(`kr.ncloudstorage.com`, 사설망은 `kr-private.…`)는
/// 주소의 지역 코드(`kr`)를 쓰고, 그 밖의 저장소는 S3 도구들의 기본값(`us-east-1`)을 쓴다.
fn default_region(host: &str) -> &str {
    let name = host.split(':').next().unwrap_or(host);
    match name.strip_suffix(".ncloudstorage.com") {
        // 예전 Object Storage(`kr.object.ncloudstorage.com`)는 지역 이름이 `kr-standard`다.
        Some(front) if front == "object" || front.ends_with(".object") => "kr-standard",
        // 가상 호스트 방식으로 버킷까지 적은 주소(`버킷.kr.ncloudstorage.com`)여도 마지막 마디가 지역 코드다.
        Some(front) => {
            let code = front.rsplit('.').next().unwrap_or(front);
            let code = code.strip_suffix("-private").unwrap_or(code);
            if code.is_empty() { "us-east-1" } else { code }
        }
        None => "us-east-1",
    }
}

fn signature_of(text: &str) -> Signature {
    if text.eq_ignore_ascii_case("v2") { Signature::V2 } else { Signature::V4 }
}

/// 처음 열 곳을 `s3://버킷/폴더/` 모양으로 다듬는다(비어 있으면 빈 글).
fn clean_start(text: &str) -> Result<String, String> {
    let at = parse_location(text)?;
    if at.bucket.is_empty() {
        return Ok(String::new());
    }
    let mut key = at.key;
    if !key.is_empty() && !key.ends_with('/') {
        key.push('/');
    }
    Ok(format!("s3://{}/{}", at.bucket, key))
}

pub fn list_at(path: &Path) -> Result<Vec<ConnInfo>, String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    Ok(load(path)?.connections.iter().map(ConnInfo::from).collect())
}

/// 접속 정보를 더하거나 고친다.
pub fn save_at(path: &Path, input: &ConnInput) -> Result<ConnInfo, String> {
    let endpoint = Endpoint::parse(&input.endpoint, input.path_style)?;
    let start = clean_start(&input.start)?;
    let access = input.access.trim();
    let secret = input.secret.trim();
    if access.is_empty() != secret.is_empty() {
        return Err("접근 키와 비밀 키는 둘 다 적어 주세요.".into());
    }
    let name = match input.name.trim() {
        "" => endpoint.host.clone(),
        name => name.chars().take(80).collect(),
    };
    let region = match input.region.trim() {
        "" => default_region(&endpoint.host).to_string(),
        region => region.to_string(),
    };
    let signature = if signature_of(&input.signature) == Signature::V2 { "v2" } else { "v4" }.to_string();

    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut vault = load(path)?;
    let at = match input.id.as_deref().filter(|id| !id.is_empty()) {
        Some(id) => Some(vault.connections.iter().position(|c| c.id == id).ok_or("고치려는 접속 정보가 없습니다(지워졌습니다).")?),
        None => None,
    };
    let id = match at {
        Some(at) => vault.connections[at].id.clone(),
        None => {
            let mut bytes = [0u8; 8];
            random(&mut bytes)?;
            format!("c{}", cypher_core::s3::hex(&bytes))
        }
    };
    let (sealed, locked) = if access.is_empty() {
        let old = at.map(|at| &vault.connections[at]).filter(|old| !old.secret.is_empty()).ok_or("접근 키와 비밀 키를 적어 주세요.")?;
        if !input.password.is_empty() {
            return Err("잠금 암호를 정하거나 바꾸려면 접근 키와 비밀 키를 다시 적어 주세요.".into());
        }
        (old.secret.clone(), old.locked)
    } else {
        let keys = Keys { access: access.to_string(), secret: secret.to_string() };
        let password = Some(input.password.as_str()).filter(|p| !p.is_empty());
        if password.is_some_and(|p| p.chars().count() < MIN_PASSWORD) {
            return Err(format!("잠금 암호는 {MIN_PASSWORD}자 이상으로 정해 주세요(짧은 암호는 대입해 보면 뚫립니다)."));
        }
        (seal(&id, &keys, password)?, password.is_some())
    };
    let stored = Stored { id, name, endpoint: endpoint.display(), region, path_style: input.path_style, signature, start, insecure: input.insecure, locked, secret: sealed };
    let info = ConnInfo::from(&stored);
    match at {
        Some(at) => vault.connections[at] = stored,
        None => {
            if vault.connections.len() >= MAX_CONNECTIONS {
                return Err(format!("접속 정보는 {MAX_CONNECTIONS}개까지 저장합니다."));
            }
            vault.connections.push(stored);
        }
    }
    vault.version = 1;
    store(path, &vault)?;
    Ok(info)
}

pub fn remove_at(path: &Path, id: &str) -> Result<(), String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut vault = load(path)?;
    let before = vault.connections.len();
    vault.connections.retain(|c| c.id != id);
    if vault.connections.len() == before {
        return Ok(());
    }
    store(path, &vault)
}

/// 접속 정보를 꺼내 키를 푼다. 잠금 암호로 잠근 것이면 `password`가 맞아야 한다.
pub fn open_at(path: &Path, id: &str, password: Option<&str>) -> Result<Opened, String> {
    let stored = {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        load(path)?.connections.into_iter().find(|c| c.id == id).ok_or("저장해 둔 접속 정보가 없습니다(지워졌습니다). 다른 접속 정보를 골라 주세요.")?
    };
    let endpoint = Endpoint::parse(&stored.endpoint, stored.path_style)?;
    let keys = open(&stored.id, &stored.secret, password)?;
    Ok(Opened { info: ConnInfo::from(&stored), endpoint, signature: signature_of(&stored.signature), keys })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_file(name: &str) -> PathBuf {
        let dir = std::env::current_exe().unwrap().parent().unwrap().join("test-files").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("connections.json")
    }

    fn input(access: &str, secret: &str, password: &str) -> ConnInput {
        ConnInput {
            id: None, name: " 사내 저장소 ".into(), endpoint: "http://10.0.0.5:9000/".into(), region: String::new(), path_style: true,
            signature: "V4".into(), start: "logs/2024".into(), insecure: false,
            access: access.into(), secret: secret.into(), password: password.into(),
        }
    }

    #[test]
    fn 키는_파일에_그대로_적히지_않고_다시_풀린다() {
        let path = test_file("s3-vault-plain");
        let saved = save_at(&path, &input(" AKIDEXAMPLE-TOKEN \n", "s3cr3t/토큰+값==", "")).unwrap();
        assert_eq!(saved.name, "사내 저장소");
        assert_eq!(saved.endpoint, "http://10.0.0.5:9000");
        assert_eq!(saved.region, "us-east-1");
        assert_eq!(saved.start, "s3://logs/2024/");
        assert!(!saved.locked);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("AKIDEXAMPLE") && !text.contains("s3cr3t"), "키가 파일에 그대로 적혔다");
        let opened = open_at(&path, &saved.id, None).unwrap();
        assert_eq!((opened.keys.access.as_str(), opened.keys.secret.as_str()), ("AKIDEXAMPLE-TOKEN", "s3cr3t/토큰+값=="));
        assert_eq!(list_at(&path).unwrap(), vec![saved]);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn 잠금_암호로_잠근_키는_암호가_맞아야_풀린다() {
        let path = test_file("s3-vault-locked");
        let saved = save_at(&path, &input("access", "secret", "여는 암호 12")).unwrap();
        // 짧은 잠금 암호는 받지 않는다.
        assert!(save_at(&path, &input("access", "secret", "1234567")).unwrap_err().contains("8자"));
        assert!(saved.locked);
        assert_eq!(open_at(&path, &saved.id, Some("여는 암호 12")).unwrap().keys.secret, "secret");
        assert!(open_at(&path, &saved.id, Some("틀린 암호")).err().unwrap().contains("맞지 않습니다"));
        assert!(open_at(&path, &saved.id, None).err().unwrap().contains("필요합니다"));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn 다른_접속_정보의_덩어리를_옮겨_붙이면_풀리지_않는다() {
        let path = test_file("s3-vault-swap");
        let a = save_at(&path, &input("a-access", "a-secret", "")).unwrap();
        let b = save_at(&path, &input("b-access", "b-secret", "")).unwrap();
        let mut vault = load(&path).unwrap();
        let from_a = vault.connections[0].secret.clone();
        vault.connections[1].secret = from_a;
        store(&path, &vault).unwrap();
        assert_eq!(open_at(&path, &a.id, None).unwrap().keys.access, "a-access");
        assert!(open_at(&path, &b.id, None).is_err());
        // 손상된 덩어리도 까닭을 알리고 끝난다.
        vault.connections[1].secret = "AAAA".into();
        store(&path, &vault).unwrap();
        assert!(open_at(&path, &b.id, None).err().unwrap().contains("손상"));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn 고칠_때_키를_비워_두면_저장해_둔_키를_그대로_둔다() {
        let path = test_file("s3-vault-edit");
        let saved = save_at(&path, &input("access", "secret", "pw-12345")).unwrap();
        let mut edit = input("", "", "");
        edit.id = Some(saved.id.clone());
        edit.name = "새 이름".into();
        edit.endpoint = "https://s3.example.com".into();
        edit.signature = "v2".into();
        edit.start = String::new();
        let edited = save_at(&path, &edit).unwrap();
        assert_eq!((edited.id.as_str(), edited.name.as_str(), edited.endpoint.as_str(), edited.signature.as_str(), edited.start.as_str()), (saved.id.as_str(), "새 이름", "https://s3.example.com", "v2", ""));
        assert!(edited.locked);
        assert_eq!(open_at(&path, &saved.id, Some("pw-12345")).unwrap().keys.access, "access");
        assert_eq!(list_at(&path).unwrap().len(), 1);

        // 키 없이 잠금 암호만 바꿀 수는 없다. 키를 한쪽만 적어도 안 된다. 새 접속 정보는 키가 있어야 한다.
        let mut only_password = input("", "", "new");
        only_password.id = Some(saved.id.clone());
        assert!(save_at(&path, &only_password).is_err());
        assert!(save_at(&path, &input("access", "", "")).is_err());
        assert!(save_at(&path, &input("", "", "")).is_err());
        // 키를 다시 적으면 잠금을 풀 수 있다.
        let mut unlock = input("access2", "secret2", "");
        unlock.id = Some(saved.id.clone());
        assert!(!save_at(&path, &unlock).unwrap().locked);
        assert_eq!(open_at(&path, &saved.id, None).unwrap().keys.access, "access2");

        remove_at(&path, &saved.id).unwrap();
        assert!(list_at(&path).unwrap().is_empty());
        assert!(open_at(&path, &saved.id, None).is_err());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn 지역을_비우면_주소에_맞춰_정한다() {
        assert_eq!(default_region("kr.ncloudstorage.com"), "kr");
        assert_eq!(default_region("kr-private.ncloudstorage.com"), "kr");
        assert_eq!(default_region("kr.ncloudstorage.com:443"), "kr");
        assert_eq!(default_region("kr.object.ncloudstorage.com"), "kr-standard");
        assert_eq!(default_region("10.0.0.5:9000"), "us-east-1");
        assert_eq!(default_region("s3.example.com"), "us-east-1");
        let path = test_file("s3-vault-region");
        let mut ncloud = input("a", "b", "");
        ncloud.endpoint = "https://kr.ncloudstorage.com".into();
        assert_eq!(save_at(&path, &ncloud).unwrap().region, "kr");
        ncloud.region = "kr-standard".into();
        assert_eq!(save_at(&path, &ncloud).unwrap().region, "kr-standard");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn 깨진_금고_파일은_덮어쓰지_않는다() {
        let path = test_file("s3-vault-broken");
        std::fs::write(&path, "{ 깨진 파일").unwrap();
        assert!(list_at(&path).is_err());
        assert!(save_at(&path, &input("a", "b", "")).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ 깨진 파일");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
