//! S3 호환 저장소 탐색기(화면의 `s3.js`)가 쓰는 명령: 접속, 목록, 미리보기, 올리기·내려받기·지우기·복사·옮기기.
//!
//! 저장소에 닿는 일은 모두 여기서 한다(화면은 무엇을 할지만 정한다). 요청의 주소와 서명은 `cypher_core::s3`가 만든다.
//!
//! 저장소에 부담을 주지 않게 지키는 것:
//! - 접속은 사용자가 [접속]을 눌렀을 때만 한다. 접속 그 자체는 요청을 보내지 않고(키를 풀어 메모리에 둘 뿐),
//!   다시 켠 뒤에도 스스로 접속하지 않는다.
//! - 목록은 `/`로 묶어 그 층만, 한 번에 `limit`개씩 읽는다. 다음 쪽은 화면이 달라고 할 때만 읽는다.
//!   주기적으로 다시 읽거나 폴더 크기를 재려고 아래를 훑는 일은 없다.
//! - 폴더째 지우기·복사·내려받기처럼 아래를 다 알아야 하는 일만 1,000개씩 이어 읽는다(사용자가 시킨 뒤에).
//! - 요청은 한 작업 안에서 하나씩 차례로 보낸다(동시에 여러 개를 보내지 않는다). 실패한 요청을 스스로 되풀이하지 않는다
//!   (다만 전송 라이브러리가, 다시 쓰려던 연결이 그 사이 끊겨 있었을 때 본문 없는 요청을 한 번 다시 보내는 일은 있다).
//! - 스스로 이어 읽는 목록은 저장소가 같은 쪽을 되풀이해 주면 멈추고, 받은 키가 물은 앞머리로 시작하지 않아도 멈춘다
//!   (앞머리를 못 알아듣는 저장소에서 폴더째 지우기가 버킷 전체를 지우는 일이 없게).
//!
//! 오래 걸리는 일(올리기, 내려받기, 지우기, 복사·옮기기)은 "작업"으로 돌린다: 명령은 작업 번호만 돌려주고,
//! 진행은 `s3-job` 알림으로 화면에 알린다. 이미 있는 것을 덮어쓰게 되면 멈추고 화면에 물은 뒤(`conflict`) 답을 기다린다.

use crate::s3_vault as vault;
use base64::Engine;
use cypher_core::s3::{self as api, Endpoint, Request, Signature};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{mpsc, Arc, LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle, Emitter};
use zeroize::Zeroize;

const USER_AGENT: &str = concat!("Cypher/", env!("CARGO_PKG_VERSION"));
/// 한 번에 읽는 목록의 가장 큰 크기(S3의 한도).
const PAGE_MAX: u32 = 1000;
/// 이 크기까지는 한 번에 올리고, 넘으면 나눠 올린다.
const SINGLE_PUT_MAX: u64 = 100 << 20;
/// 나눠 올릴 때 조각 하나의 크기.
const PART_SIZE: u64 = 16 << 20;
/// 나눠 올릴 수 있는 조각 수(S3의 한도).
const MAX_PARTS: u64 = 10_000;
/// 이미 있는지 하나씩 물어보는(HEAD) 가장 많은 수. 넘으면 그 폴더의 목록을 읽어 견준다.
const HEAD_MAX: usize = 50;
/// 목록 한 쪽(1,000개)이 이만큼의 HEAD를 덜어 줄 때만 목록으로 견준다. 놓일 자리에 객체가 훨씬 많아 그 값어치가
/// 없으면 읽기를 그만두고 남은 것은 하나씩 묻는다(요청 수가 내가 다루는 객체 수를 넘지 않게).
const LIST_WORTH: usize = 20;
/// 글로 보여 줄 때 읽는 가장 큰 크기와, 그림으로 보여 줄 때의 가장 큰 크기.
const TEXT_MAX: u64 = 1 << 20;
const IMAGE_MAX: u64 = 8 << 20;
/// 진행 알림 사이의 가장 짧은 간격.
const TICK: Duration = Duration::from_millis(150);
/// 연달아 이만큼 실패하면 작업을 멈춘다(권한이 없거나 연결이 끊긴 채로 수천 번 되풀이하지 않게).
const FAIL_STREAK: u32 = 3;
/// 작업의 알림에 싣는 실패·겹침 목록의 길이.
const SHOWN: usize = 20;
/// 이 크기까지는 한 번의 복사 요청으로 복사하고(S3와 Ncloud Storage의 한도 5GB), 넘으면 조각으로 나눠 복사한다.
const COPY_SINGLE_MAX: u64 = 5 << 30;
/// 나눠 복사할 때 조각 하나의 크기.
const COPY_PART: u64 = 512 << 20;
const CANCELED: &str = "취소했습니다.";
/// 이력에 남겨 두는 요청의 수(넘으면 오래된 것부터 버린다).
const HISTORY_MAX: usize = 100;
/// 이력에 남기는 본문(보낸 XML, 실패한 답)의 가장 큰 길이.
const HISTORY_BODY: usize = 4 << 10;

// ───────────────────────── 저장소에 보내는 요청 ─────────────────────────

/// 요청이 실패한 까닭.
#[derive(Debug, Clone)]
struct Fail {
    /// HTTP 상태(닿지 못했으면 0).
    status: u16,
    /// 서버가 적어 준 오류 이름(`NoSuchKey` 등. 없으면 빈 글).
    code: String,
    text: String,
}

impl Fail {
    fn local(text: impl Into<String>) -> Fail {
        Fail { status: 0, code: String::new(), text: text.into() }
    }

    fn missing(&self) -> bool {
        self.status == 404 || self.code == "NoSuchKey" || self.code == "NotFound"
    }
}

enum Body<'a> {
    None,
    Bytes(&'a [u8]),
    /// 읽어 가며 보낸다(길이를 미리 안다).
    Reader(&'a mut dyn Read, u64),
}

/// 저장소와 주고받은 요청 하나(이력의 한 줄). 화면의 [요청 이력]이 보여 준다.
///
/// 키·서명·토큰 같은 값은 가려서 적는다(`api::mask_header`). 객체의 내용과 성공한 답의 본문(목록 등)은 적지 않는다:
/// 보낸 본문은 저장소에 시키는 XML(여러 개 지우기, 나눠 올리기 맺기)만, 받은 본문은 실패한 답만 앞부분을 적는다.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Exchange {
    /// 이 접속에서 몇 번째 요청인지(1부터).
    seq: u64,
    /// 보낸 때(1970년부터의 밀리초).
    at: u64,
    method: String,
    url: String,
    request_headers: Vec<(String, String)>,
    /// 보낸 본문의 크기(바이트)와, 저장소에 시키는 XML이면 그 글(앞부분).
    sent: u64,
    request_body: String,
    /// 받은 답의 상태(닿지 못했으면 0)와 헤더.
    status: u16,
    reason: String,
    response_headers: Vec<(String, String)>,
    /// 실패한 답의 본문(앞부분).
    response_body: String,
    /// 닿지 못한 까닭(연결 실패, 인증서 등).
    problem: String,
    /// 답의 헤더를 받을 때까지 걸린 시간(밀리초). 본문을 받는 시간은 들지 않는다.
    millis: u64,
}

/// 이력에 남길 본문: 길면 앞부분만.
fn clip(text: &str) -> String {
    if text.len() <= HISTORY_BODY {
        return text.to_string();
    }
    let mut end = HISTORY_BODY;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… (뒤는 잘랐습니다. 전체 {}바이트)", &text[..end], text.len())
}

/// 접속 하나: 저장소 주소와 풀어 둔 키. 화면의 S3 항목 하나가 접속해 있는 동안 든다.
struct Client {
    /// 이 접속으로 보낸 요청의 이력(지금까지 보낸 수, 최근 `HISTORY_MAX`건). 메모리에만 있고 접속을 끊으면 사라진다.
    history: Mutex<(u64, std::collections::VecDeque<Exchange>)>,
    endpoint: Endpoint,
    region: String,
    signature: Signature,
    keys: vault::Keys,
    agent: ureq::Agent,
    /// 저장소 안에서 오래 걸릴 수 있는 요청(큰 객체 복사, 1,000개 지우기, 나눠 올리기 맺기)에 쓰는, 답을 오래 기다리는 것.
    slow: ureq::Agent,
    /// 여러 객체를 한 번에 지우는 요청을 모르는 저장소로 드러났다(하나씩 지운다).
    single_delete: AtomicBool,
    /// 목록 요청의 `encoding-type`을 받지 않는 저장소로 드러났다(붙이지 않는다).
    plain_keys: AtomicBool,
    /// 목록 요청의 `delimiter`를 받지 않는 저장소로 드러났다(붙이지 않고, 받은 키를 여기서 폴더로 묶는다).
    no_delimiter: AtomicBool,
}

#[repr(C)]
struct CertContext {
    encoding: u32,
    encoded: *const u8,
    len: u32,
    info: *mut c_void,
    store: *mut c_void,
}

#[link(name = "crypt32")]
extern "system" {
    fn CertOpenSystemStoreW(provider: usize, name: *const u16) -> *mut c_void;
    fn CertEnumCertificatesInStore(store: *mut c_void, previous: *const CertContext) -> *const CertContext;
    fn CertCloseStore(store: *mut c_void, flags: u32) -> i32;
}

/// Windows가 믿는 루트 인증서들(사내 인증 기관이 여기에 들어 있다).
fn windows_roots() -> Vec<Vec<u8>> {
    let name: Vec<u16> = "ROOT".encode_utf16().chain(std::iter::once(0)).collect();
    let mut found = Vec::new();
    // SAFETY: 이름은 0으로 끝나는 글이고, 받은 저장소는 끝에서 닫는다. 인증서 문맥은 다음 것을 달라고 할 때 앞의 것이
    // 풀리므로, 가리키는 바이트를 그때그때 복사해 둔다.
    unsafe {
        let store = CertOpenSystemStoreW(0, name.as_ptr());
        if store.is_null() {
            return found;
        }
        let mut context = CertEnumCertificatesInStore(store, std::ptr::null());
        while !context.is_null() {
            let cert = &*context;
            if !cert.encoded.is_null() && cert.len > 0 {
                found.push(std::slice::from_raw_parts(cert.encoded, cert.len as usize).to_vec());
            }
            context = CertEnumCertificatesInStore(store, context);
        }
        CertCloseStore(store, 0);
    }
    found
}

/// 인증서를 확인하지 않는 검증기(접속 정보에서 "인증서 확인 안 함"을 켠 저장소에만 쓴다). 서명은 그대로 확인한다.
#[derive(Debug)]
struct NoVerify(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self, _end: &rustls::pki_types::CertificateDer<'_>, _chain: &[rustls::pki_types::CertificateDer<'_>],
        _name: &rustls::pki_types::ServerName<'_>, _ocsp: &[u8], _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self, message: &[u8], cert: &rustls::pki_types::CertificateDer<'_>, dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self, message: &[u8], cert: &rustls::pki_types::CertificateDer<'_>, dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// TLS 설정. 믿는 인증 기관은 널리 쓰이는 공개 기관들에 Windows가 믿는 것(사내 인증 기관)을 더한 것이다.
fn tls_config(insecure: bool) -> Result<Arc<rustls::ClientConfig>, String> {
    static VERIFIED: LazyLock<Result<Arc<rustls::ClientConfig>, String>> = LazyLock::new(|| build_tls(false));
    if insecure { build_tls(true) } else { VERIFIED.clone() }
}

fn build_tls(insecure: bool) -> Result<Arc<rustls::ClientConfig>, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("TLS를 준비하지 못했습니다: {e}"))?;
    let config = if insecure {
        builder.dangerous().with_custom_certificate_verifier(Arc::new(NoVerify(provider))).with_no_client_auth()
    } else {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        for der in windows_roots() {
            // 읽을 수 없는 인증서(오래된 형식 등)는 건너뛴다.
            let _ = roots.add(rustls::pki_types::CertificateDer::from(der));
        }
        builder.with_root_certificates(roots).with_no_client_auth()
    };
    Ok(Arc::new(config))
}

fn status_text(status: u16, reason: &str) -> String {
    match status {
        301 | 302 | 307 | 308 => format!("저장소가 다른 주소로 가라고 답했습니다(HTTP {status}). 저장소 주소와 주소 방식을 확인하세요."),
        // 본문 없이 온 거절(HEAD 등)은 까닭을 알 수 없다: 권한이 없을 수도, 키·서명·시계가 틀렸을 수도 있다.
        401 | 403 => format!("저장소가 거절했습니다 (HTTP {status}: 권한이 없거나 키·서명·시계가 맞지 않습니다)"),
        404 => "찾을 수 없습니다 (HTTP 404)".to_string(),
        _ => format!("HTTP {status} {reason}").trim().to_string(),
    }
}

/// 실패한 답을 읽는다: 실패한 까닭과, 저장소가 보낸 본문(이력에 남긴다).
fn fail_of(response: ureq::Response) -> (Fail, String) {
    let status = response.status();
    let reason = response.status_text().to_string();
    let mut bytes = Vec::new();
    let _ = response.into_reader().take(64 << 10).read_to_end(&mut bytes);
    let body = String::from_utf8_lossy(&bytes).into_owned();
    let fail = match api::parse_error(&body) {
        Some((code, message)) => Fail { status, text: api::error_text(&code, &message), code },
        None => Fail { status, code: String::new(), text: status_text(status, &reason) },
    };
    (fail, body)
}

fn transport_text(error: &ureq::Transport) -> String {
    let text = error.to_string();
    let lower = text.to_ascii_lowercase();
    if lower.contains("certificate") || lower.contains("unknownissuer") {
        format!("서버의 인증서를 확인할 수 없습니다: {text} — 사내 인증 기관의 인증서가 Windows에 설치돼 있는지 확인하거나, 접속 정보에서 \"인증서 확인 안 함\"을 켜세요.")
    } else {
        format!("저장소에 닿지 못했습니다: {text}")
    }
}

/// 응답 본문을 글로 읽는다(목록·오류 등 작은 응답용).
fn text_of(response: ureq::Response) -> Result<String, Fail> {
    let mut bytes = Vec::new();
    response.into_reader().take(64 << 20).read_to_end(&mut bytes).map_err(|e| Fail::local(format!("응답을 읽다가 끊겼습니다: {e}")))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// 헤더 값으로 보낼 수 있는 글인지(보이는 ASCII와 빈칸).
fn header_safe(value: &str) -> bool {
    value.bytes().all(|b| (0x20..0x7f).contains(&b))
}

impl Client {
    fn new(opened: vault::Opened) -> Result<Client, String> {
        let tls = if opened.endpoint.scheme == "https" { Some(tls_config(opened.info.insecure)?) } else { None };
        let agent = |read: u64| {
            let mut builder = ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(15))
                .timeout_read(Duration::from_secs(read))
                .timeout_write(Duration::from_secs(60))
                .redirects(0)
                .user_agent(USER_AGENT);
            if let Some(tls) = &tls {
                builder = builder.tls_config(tls.clone());
            }
            builder.build()
        };
        Ok(Client {
            history: Mutex::default(),
            endpoint: opened.endpoint,
            region: opened.info.region.clone(),
            signature: opened.signature,
            keys: opened.keys,
            agent: agent(60),
            slow: agent(900),
            single_delete: AtomicBool::new(false),
            plain_keys: AtomicBool::new(false),
            no_delimiter: AtomicBool::new(false),
        })
    }

    fn send(&self, request: &Request, body: Body<'_>) -> Result<ureq::Response, Fail> {
        self.send_on(&self.agent, request, body)
    }

    /// 저장소 안에서 오래 걸릴 수 있는 요청을 보낸다(답을 15분까지 기다린다).
    fn send_slow(&self, request: &Request, body: Body<'_>) -> Result<ureq::Response, Fail> {
        self.send_on(&self.slow, request, body)
    }

    fn send_on(&self, agent: &ureq::Agent, request: &Request, body: Body<'_>) -> Result<ureq::Response, Fail> {
        let now = SystemTime::now();
        let started = Instant::now();
        let signed = api::sign(&self.endpoint, &self.keys.access, &self.keys.secret, &self.region, self.signature, request, now).map_err(Fail::local)?;
        // 내려받는 내용을 서버가 줄여 보내지 않게 한다(범위 요청과 크기 계산이 어긋난다). 객체가 줄인 채(`Content-Encoding: gzip`)
        // 저장돼 있으면 저장소는 그대로 보내고, 여기서도 풀지 않는다: 저장소에 든 바이트를 그대로 다룬다.
        let mut call = agent.request(signed.method, &signed.url).set("accept-encoding", "identity");
        for (name, value) in &signed.headers {
            call = call.set(name, value);
        }
        // 이력에 남길 것: 헤더의 키·서명은 가리고, 본문은 저장소에 시키는 XML일 때만(객체의 내용은 남기지 않는다).
        // (XML 파일을 고쳐 저장하는 PUT도 종류가 XML이지만 그것은 객체의 내용이다: POST로 보내는 것만 남긴다.)
        let orders = signed.method == "POST" && signed.headers.iter().any(|(name, value)| name == "content-type" && value == "application/xml");
        let mut entry = Exchange {
            at: now.duration_since(SystemTime::UNIX_EPOCH).map(|since| since.as_millis() as u64).unwrap_or(0),
            method: signed.method.to_string(),
            url: api::mask_url(&signed.url),
            request_headers: signed.headers.iter().map(|(name, value)| (name.clone(), api::mask_header(name, value))).collect(),
            sent: match &body {
                Body::None => 0,
                Body::Bytes(bytes) => bytes.len() as u64,
                Body::Reader(_, length) => *length,
            },
            request_body: match &body {
                Body::Bytes(bytes) if orders => clip(&String::from_utf8_lossy(bytes)),
                _ => String::new(),
            },
            ..Exchange::default()
        };
        let result = match body {
            Body::None if matches!(signed.method, "PUT" | "POST") => call.send_bytes(&[]),
            Body::None => call.call(),
            Body::Bytes(bytes) => call.send_bytes(bytes),
            Body::Reader(reader, length) => call.set("content-length", &length.to_string()).send(reader),
        };
        let answered = |entry: &mut Exchange, response: &ureq::Response| {
            entry.status = response.status();
            entry.reason = response.status_text().to_string();
            let mut names = response.headers_names();
            names.sort();
            names.dedup();
            entry.response_headers = names.into_iter().map(|name| { let value = api::mask_header(&name, response.header(&name).unwrap_or("")); (name, value) }).collect();
        };
        let outcome = match result {
            Ok(response) if response.status() < 300 => {
                answered(&mut entry, &response);
                Ok(response)
            }
            Ok(response) | Err(ureq::Error::Status(_, response)) => {
                answered(&mut entry, &response);
                let (fail, raw) = fail_of(response);
                entry.response_body = clip(&raw);
                Err(fail)
            }
            Err(ureq::Error::Transport(error)) => {
                let text = transport_text(&error);
                entry.problem = text.clone();
                Err(Fail::local(text))
            }
        };
        entry.millis = started.elapsed().as_millis() as u64;
        let mut history = self.history.lock().unwrap_or_else(|e| e.into_inner());
        history.0 += 1;
        entry.seq = history.0;
        if history.1.len() >= HISTORY_MAX {
            history.1.pop_front();
        }
        history.1.push_back(entry);
        drop(history);
        outcome
    }

    /// 본문의 해시: SigV4에서만 쓴다.
    fn hash(&self, bytes: &[u8]) -> String {
        if self.signature == Signature::V4 { api::sha256_hex(bytes) } else { String::new() }
    }

    fn buckets(&self) -> Result<Vec<api::Bucket>, Fail> {
        let response = self.send(&Request { method: "GET", ..Request::default() }, Body::None)?;
        api::parse_buckets(&text_of(response)?).map_err(Fail::local)
    }

    /// 목록을 한 쪽 읽는다. `delimiter`면 그 층만(아래 폴더는 한 줄로 묶어서) 읽는다.
    ///
    /// 저장소마다 받는 선택 사항이 다르다(문서에 `delimiter`·`encoding-type`이 없는 저장소도 있다). 그래서 요청이
    /// "모르는 인자"로 거절되면 선택 사항을 하나씩 빼고 다시 물어, 통한 모양을 그 접속 동안 기억한다(다시 묻는 것은
    /// 접속마다 많아야 두 번이다). 묶어 주지 않는 저장소의 답은 여기서 폴더 모양으로 묶는다.
    fn list(&self, bucket: &str, prefix: &str, delimiter: bool, limit: u32, next: Option<&str>) -> Result<api::ListPage, Fail> {
        let plain = self.plain_keys.load(Ordering::Relaxed);
        let flat = self.no_delimiter.load(Ordering::Relaxed);
        // (키를 %XX로 받을지, 구분자로 묶어 달라고 할지)를 점점 덜 바라는 순서로.
        let mut shapes = vec![(!plain, delimiter && !flat)];
        if !plain {
            shapes.push((false, delimiter && !flat));
        }
        if delimiter && !flat {
            shapes.push((false, false));
        }
        let mut first: Option<Fail> = None;
        for (encode, grouped) in shapes {
            let request = Request { method: "GET", bucket: Some(bucket.to_string()), query: api::list_query(prefix, grouped, limit, next, encode), ..Request::default() };
            match self.send(&request, Body::None) {
                Ok(response) => {
                    // 통한 모양을 기억한다(실패만 하고 끝났으면 기억하지 않는다: 다른 까닭의 실패일 수 있다).
                    if !encode {
                        self.plain_keys.store(true, Ordering::Relaxed);
                    }
                    if delimiter && !grouped {
                        self.no_delimiter.store(true, Ordering::Relaxed);
                    }
                    let page = api::parse_list(&text_of(response)?).map_err(Fail::local)?;
                    return Ok(if delimiter { api::group_by_slash(page, prefix) } else { page });
                }
                Err(fail) => {
                    let unknown = matches!(fail.status, 400 | 501) && matches!(fail.code.as_str(), "InvalidArgument" | "NotImplemented" | "InvalidRequest" | "");
                    let give_up = !unknown;
                    first.get_or_insert(fail);
                    if give_up {
                        break;
                    }
                }
            }
        }
        Err(first.expect("적어도 한 번은 물었다"))
    }

    fn object(&self, method: &'static str, bucket: &str, key: &str) -> Request {
        Request { method, bucket: Some(bucket.to_string()), key: key.to_string(), object: true, ..Request::default() }
    }

    fn head(&self, bucket: &str, key: &str) -> Result<ureq::Response, Fail> {
        self.send(&self.object("HEAD", bucket, key), Body::None)
    }

    fn exists(&self, bucket: &str, key: &str) -> Result<bool, Fail> {
        match self.head(bucket, key) {
            Ok(_) => Ok(true),
            Err(fail) if fail.missing() => Ok(false),
            Err(fail) => Err(fail),
        }
    }

    fn put_bytes(&self, bucket: &str, key: &str, bytes: &[u8], content_type: &str, meta: &[(String, String)]) -> Result<(), Fail> {
        let mut request = self.object("PUT", bucket, key);
        request.payload_sha256 = self.hash(bytes);
        if !content_type.is_empty() && header_safe(content_type) {
            request.headers.push(("content-type".into(), content_type.to_string()));
        }
        // 사용자 메타데이터와, 객체에 붙어 있던 표준 헤더·저장 등급만 붙인다(그 밖의 이름은 받지 않는다).
        const KEPT: [&str; 5] = ["cache-control", "content-disposition", "content-language", "content-encoding", "x-amz-storage-class"];
        for (name, value) in meta {
            let name = name.to_ascii_lowercase();
            let known = name.starts_with("x-amz-meta-") || KEPT.contains(&name.as_str());
            if known && header_safe(&name) && !name.contains(' ') && !value.is_empty() && header_safe(value) {
                request.headers.push((name, value.clone()));
            }
        }
        self.send(&request, Body::Bytes(bytes)).map(drop)
    }

    fn delete_one(&self, bucket: &str, key: &str) -> Result<(), Fail> {
        self.send(&self.object("DELETE", bucket, key), Body::None).map(drop)
    }

    /// 여러 객체를 지운다. 지우지 못한 것들을 (키, 까닭)으로 돌려준다.
    fn delete_many(&self, bucket: &str, keys: &[String]) -> Result<Vec<(String, String)>, Fail> {
        // 요청 본문(XML)에 적을 수 없는 글자가 든 키는 묶음에 넣지 않고 하나씩 지운다(하나 때문에 묶음 전체가 거절되지 않게).
        let (batch, mut single): (Vec<String>, Vec<String>) = keys.iter().cloned().partition(|key| api::xml_safe(key));
        let mut failed = Vec::new();
        if batch.len() > 1 && !self.single_delete.load(Ordering::Relaxed) {
            let body = api::delete_body(&batch);
            let request = Request {
                method: "POST",
                bucket: Some(bucket.to_string()),
                query: vec![("delete".into(), String::new())],
                headers: vec![("content-md5".into(), api::md5_base64(body.as_bytes())), ("content-type".into(), "application/xml".into())],
                payload_sha256: self.hash(body.as_bytes()),
                ..Request::default()
            };
            match self.send_slow(&request, Body::Bytes(body.as_bytes())) {
                Ok(response) => {
                    // 보낸 키에 대한 실패만 받아들인다.
                    let asked: HashSet<&String> = batch.iter().collect();
                    let reported = api::parse_delete_result(&text_of(response)?).map_err(Fail::local)?;
                    failed.extend(reported.into_iter().filter(|(key, _)| asked.contains(key)));
                }
                // 한 번에 지우기를 모르는 저장소: 하나씩 지운다.
                Err(fail) if matches!(fail.status, 405 | 501) || matches!(fail.code.as_str(), "NotImplemented" | "MethodNotAllowed") => {
                    self.single_delete.store(true, Ordering::Relaxed);
                    single.extend(batch);
                }
                Err(fail) => return Err(fail),
            }
        } else {
            single.extend(batch);
        }
        for key in single {
            if let Err(fail) = self.delete_one(bucket, &key) {
                // 닿지도 못했으면(연결 끊김) 남은 것을 하나씩 더 해 보지 않는다.
                if fail.status == 0 {
                    return Err(fail);
                }
                failed.push((key, fail.text));
            }
        }
        Ok(failed)
    }

    fn copy(&self, bucket: &str, key: &str, to_bucket: &str, to_key: &str) -> Result<(), Fail> {
        let mut request = self.object("PUT", to_bucket, to_key);
        request.headers.push(("x-amz-copy-source".into(), copy_source(bucket, key)?));
        let response = self.send_slow(&request, Body::None)?;
        // 복사는 200으로 답한 뒤 본문에 오류를 적기도 하고, 복사를 모르는 저장소는 빈 답으로 빈 객체를 만들기도 한다.
        // 복사했다는 답(`CopyObjectResult`)일 때만 성공으로 친다: 옮기기는 이 답을 믿고 원본을 지운다.
        api::expect_root(&text_of(response)?, "CopyObjectResult").map(drop).map_err(Fail::local)
    }

    /// 나눠 올리기(나눠 복사하기)를 시작하고 그 번호를 받는다.
    fn begin_multipart(&self, bucket: &str, key: &str, headers: Vec<(String, String)>) -> Result<String, String> {
        let mut request = self.object("POST", bucket, key);
        request.query.push(("uploads".into(), String::new()));
        request.headers = headers;
        let response = self.send(&request, Body::None).map_err(|fail| fail.text)?;
        api::parse_upload_id(&text_of(response).map_err(|fail| fail.text)?)
    }

    fn part_query(upload: &str, number: Option<u32>) -> Vec<(String, String)> {
        let mut query = vec![("uploadId".to_string(), upload.to_string())];
        if let Some(number) = number {
            query.push(("partNumber".into(), number.to_string()));
        }
        query
    }

    /// 조각을 다 올린(복사한) 뒤 하나의 객체로 맺는다.
    fn complete_multipart(&self, bucket: &str, key: &str, upload: &str, parts: &[(u32, String)]) -> Result<(), String> {
        let body = api::complete_body(parts);
        let mut request = self.object("POST", bucket, key);
        request.query = Client::part_query(upload, None);
        request.headers.push(("content-type".into(), "application/xml".into()));
        request.payload_sha256 = self.hash(body.as_bytes());
        let response = self.send_slow(&request, Body::Bytes(body.as_bytes())).map_err(|fail| fail.text)?;
        api::expect_root(&text_of(response).map_err(|fail| fail.text)?, "CompleteMultipartUploadResult").map(drop)
    }

    /// 올리다 만 조각이 저장소에 남지 않게 치우고, 원래의 실패 글을 돌려준다. 치우지 못했으면 그 사실을 덧붙인다
    /// (남은 조각은 저장소에 자리를 차지한다).
    fn abort_multipart(&self, bucket: &str, key: &str, upload: &str, why: String) -> String {
        let mut request = self.object("DELETE", bucket, key);
        request.query = Client::part_query(upload, None);
        match self.send(&request, Body::None) {
            Ok(_) => why,
            Err(_) => format!("{why} — 올리다 만 조각을 치우지 못했습니다. 저장소에 남아 자리를 차지할 수 있습니다(저장소의 관리 화면에서 끝나지 않은 올리기를 지워 주세요)."),
        }
    }

    /// 한 번에 복사할 수 없는 큰 객체를 조각으로 나눠 복사한다(UploadPartCopy). 내용은 저장소 안에서만 오간다.
    /// 원래의 종류와 사용자 메타데이터는 먼저 읽어 새 객체에 붙인다.
    fn copy_in_parts(&self, bucket: &str, key: &str, size: u64, to_bucket: &str, to_key: &str, part: u64, canceled: &dyn Fn() -> bool) -> Result<(), String> {
        let head = self.head(bucket, key).map_err(|fail| fail.text)?;
        let size = head.header("content-length").and_then(|length| length.trim().parse::<u64>().ok()).unwrap_or(size);
        let mut headers: Vec<(String, String)> = head.header("content-type").map(|kind| vec![("content-type".to_string(), kind.to_string())]).unwrap_or_default();
        for name in head.headers_names().into_iter().filter(|name| name.starts_with("x-amz-meta-")) {
            if let Some(value) = head.header(&name).filter(|value| header_safe(value)) {
                headers.push((name.clone(), value.to_string()));
            }
        }
        let part = part.max(size.div_ceil(MAX_PARTS)).max(1);
        let upload = self.begin_multipart(to_bucket, to_key, headers)?;
        let copied = (|| -> Result<Vec<(u32, String)>, String> {
            let mut parts = Vec::new();
            let mut from = 0u64;
            while from < size {
                if canceled() {
                    return Err(CANCELED.into());
                }
                let to = (from + part).min(size) - 1;
                let number = parts.len() as u32 + 1;
                let mut request = self.object("PUT", to_bucket, to_key);
                request.query = Client::part_query(&upload, Some(number));
                request.headers.push(("x-amz-copy-source".into(), copy_source(bucket, key).map_err(|fail| fail.text)?));
                request.headers.push(("x-amz-copy-source-range".into(), format!("bytes={from}-{to}")));
                let response = self.send_slow(&request, Body::None).map_err(|fail| fail.text)?;
                parts.push((number, api::parse_copy_part(&text_of(response).map_err(|fail| fail.text)?)?));
                from = to + 1;
            }
            Ok(parts)
        })();
        copied
            .and_then(|parts| self.complete_multipart(to_bucket, to_key, &upload, &parts))
            .map_err(|why| self.abort_multipart(to_bucket, to_key, &upload, why))
    }
}

/// 복사 원본을 가리키는 헤더 값: `버킷/키`(%XX로 적은 것). 저장소가 이 값을 경로처럼 풀 수 있으므로,
/// `.`·`..`뿐인 마디가 든 키는(다른 객체를 가리키게 될 수 있다) 원본으로도 받지 않는다.
fn copy_source(bucket: &str, key: &str) -> Result<String, Fail> {
    if key.is_empty() || api::has_dot_segment(key) {
        return Err(Fail::local(format!("복사 원본으로 쓸 수 없는 키입니다: {key}")));
    }
    Ok(format!("{}/{}", api::uri_encode(bucket, false), api::uri_encode(key, true)))
}

#[cfg(test)]
impl Client {
    fn for_test(endpoint: Endpoint, signature: Signature, access: &str, secret: &str) -> Client {
        Client {
            history: Mutex::default(),
            endpoint,
            region: "us-east-1".into(),
            signature,
            keys: vault::Keys { access: access.into(), secret: secret.into() },
            agent: ureq::AgentBuilder::new().timeout(Duration::from_secs(20)).redirects(0).build(),
            slow: ureq::AgentBuilder::new().timeout(Duration::from_secs(20)).redirects(0).build(),
            single_delete: AtomicBool::new(false),
            plain_keys: AtomicBool::new(false),
            no_delimiter: AtomicBool::new(false),
        }
    }
}

// ───────────────────────── 접속 ─────────────────────────

static CLIENTS: LazyLock<Mutex<HashMap<u32, Arc<Client>>>> = LazyLock::new(Default::default);
static JOBS: LazyLock<Mutex<HashMap<u32, Arc<Job>>>> = LazyLock::new(Default::default);
static NEXT: AtomicU32 = AtomicU32::new(1);

fn client(handle: u32) -> Result<Arc<Client>, String> {
    CLIENTS.lock().unwrap_or_else(|e| e.into_inner()).get(&handle).cloned().ok_or_else(|| "접속이 끊겼습니다. 다시 접속해 주세요.".to_string())
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work).await.map_err(|e| e.to_string())?
}

/// 저장해 둔 접속 정보(키는 없다).
#[tauri::command]
pub async fn s3_conns() -> Result<Vec<vault::ConnInfo>, String> {
    blocking(|| vault::list_at(&vault::vault_path())).await
}

/// 접속 정보를 더하거나 고친다. 키는 여기서 암호화해 파일에 적는다.
#[tauri::command]
pub async fn s3_conn_save(conn: vault::ConnInput) -> Result<vault::ConnInfo, String> {
    blocking(move || vault::save_at(&vault::vault_path(), &conn)).await
}

#[tauri::command]
pub async fn s3_conn_remove(id: String) -> Result<(), String> {
    blocking(move || vault::remove_at(&vault::vault_path(), &id)).await
}

#[derive(Serialize)]
pub struct Connected {
    handle: u32,
    info: vault::ConnInfo,
}

/// 접속한다: 저장해 둔 키를 풀어 메모리에 둔다. 저장소에는 아직 아무 요청도 보내지 않는다(목록은 화면이 달라고 할 때 읽는다).
#[tauri::command]
pub async fn s3_connect(id: String, password: Option<String>) -> Result<Connected, String> {
    blocking(move || {
        let mut password = password;
        let opened = vault::open_at(&vault::vault_path(), &id, password.as_deref());
        // 받은 잠금 암호는 쓰자마자 지운다.
        if let Some(password) = password.as_mut() {
            password.zeroize();
        }
        let opened = opened?;
        let info = opened.info.clone();
        let client = Client::new(opened)?;
        let handle = NEXT.fetch_add(1, Ordering::Relaxed);
        CLIENTS.lock().unwrap_or_else(|e| e.into_inner()).insert(handle, Arc::new(client));
        Ok(Connected { handle, info })
    })
    .await
}

/// 접속을 끊는다: 풀어 둔 키를 버리고, 그 접속으로 돌던 작업을 멈춘다.
#[tauri::command]
pub fn s3_disconnect(handle: u32) {
    CLIENTS.lock().unwrap_or_else(|e| e.into_inner()).remove(&handle);
    for job in JOBS.lock().unwrap_or_else(|e| e.into_inner()).values().filter(|job| job.handle == handle) {
        job.stop();
    }
}

/// 이 접속으로 저장소와 주고받은 요청의 이력(최근 100건, 새것부터). 앱이 메모리에 적어 둔 것을 돌려줄 뿐
/// 저장소에는 아무것도 보내지 않는다. 키와 서명은 가려져 있고, 파일로 남기지 않으며, 접속을 끊으면 사라진다.
#[tauri::command]
pub fn s3_history(handle: u32) -> Result<Vec<Exchange>, String> {
    let client = client(handle)?;
    let history = client.history.lock().unwrap_or_else(|e| e.into_inner());
    Ok(history.1.iter().rev().cloned().collect())
}

/// 이 접속의 요청 이력을 비운다(화면의 [지우기]). 차례 번호는 이어 센다. 저장소에는 아무것도 보내지 않는다.
#[tauri::command]
pub fn s3_history_clear(handle: u32) -> Result<(), String> {
    let client = client(handle)?;
    client.history.lock().unwrap_or_else(|e| e.into_inner()).1.clear();
    Ok(())
}

// ───────────────────────── 목록과 객체 하나 ─────────────────────────

#[derive(Serialize)]
pub struct BucketRow {
    name: String,
    created: String,
}

#[tauri::command]
pub async fn s3_buckets(handle: u32) -> Result<Vec<BucketRow>, String> {
    let client = client(handle)?;
    blocking(move || {
        let buckets = client.buckets().map_err(|fail| fail.text)?;
        Ok(buckets.into_iter().map(|bucket| BucketRow { name: bucket.name, created: bucket.created }).collect())
    })
    .await
}

/// 목록의 한 줄: 폴더(키가 `/`로 끝난다)이거나 객체.
#[derive(Debug, Serialize, PartialEq)]
pub struct Entry {
    key: String,
    /// 보고 있는 폴더 아래에서의 이름(폴더는 끝의 `/`를 뗀 것).
    name: String,
    folder: bool,
    size: u64,
    modified: String,
    etag: String,
    class: String,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Page {
    entries: Vec<Entry>,
    /// 다음 쪽을 달라고 할 때 넘길 값. 끝까지 읽었으면 없다.
    next: Option<String>,
}

fn page_of(listed: api::ListPage, base: &str) -> Page {
    let name = |key: &str| key.strip_prefix(base).unwrap_or(key).trim_end_matches('/').to_string();
    let mut entries: Vec<Entry> = listed
        .prefixes
        .iter()
        .map(|prefix| Entry { key: prefix.clone(), name: name(prefix), folder: true, size: 0, modified: String::new(), etag: String::new(), class: String::new() })
        .collect();
    for object in listed.objects {
        // 보고 있는 폴더 자신을 나타내는 빈 객체(`폴더/`)는 줄로 보이지 않는다.
        if object.key == base {
            continue;
        }
        entries.push(Entry { name: name(&object.key), folder: false, key: object.key, size: object.size, modified: object.modified, etag: object.etag, class: object.storage_class });
    }
    Page { entries, next: listed.next }
}

/// `prefix`로 시작하는 것을 `/`로 묶어 한 쪽 읽는다. `base`는 지금 보는 폴더(이름을 그 아래에서의 이름으로 적는다).
#[tauri::command]
pub async fn s3_list(handle: u32, bucket: String, prefix: String, base: Option<String>, next: Option<String>, limit: u32) -> Result<Page, String> {
    let client = client(handle)?;
    blocking(move || {
        let listed = client.list(&bucket, &prefix, true, limit.clamp(1, PAGE_MAX), next.as_deref()).map_err(|fail| fail.text)?;
        Ok(page_of(listed, base.as_deref().unwrap_or(&prefix)))
    })
    .await
}

/// 객체 하나의 정보(HEAD).
#[derive(Debug, Clone, Serialize)]
pub struct ObjectInfo {
    key: String,
    size: u64,
    modified: String,
    etag: String,
    content_type: String,
    class: String,
    /// 그 밖의 헤더(사용자가 붙인 `x-amz-meta-…` 등).
    meta: Vec<(String, String)>,
}

fn info_of(key: &str, response: &ureq::Response) -> ObjectInfo {
    let header = |name: &str| response.header(name).unwrap_or("").to_string();
    let mut meta: Vec<(String, String)> = response
        .headers_names()
        .into_iter()
        .filter(|name| name.starts_with("x-amz-meta-") || matches!(name.as_str(), "cache-control" | "content-disposition" | "content-encoding" | "content-language" | "x-amz-version-id" | "x-amz-server-side-encryption"))
        .map(|name| { let value = header(&name); (name, value) })
        .collect();
    meta.sort();
    // 범위로 읽은 응답이면 전체 크기는 `Content-Range: bytes 0-99/1234`의 맨 뒤에 있다.
    let size = response
        .header("content-range")
        .and_then(|range| range.rsplit('/').next())
        .and_then(|total| total.trim().parse::<u64>().ok())
        .or_else(|| response.header("content-length").and_then(|length| length.trim().parse().ok()))
        .unwrap_or(0);
    ObjectInfo { key: key.to_string(), size, modified: header("last-modified"), etag: header("etag").trim_matches('"').to_string(), content_type: header("content-type"), class: header("x-amz-storage-class"), meta }
}

#[tauri::command]
pub async fn s3_head(handle: u32, bucket: String, key: String) -> Result<ObjectInfo, String> {
    let client = client(handle)?;
    blocking(move || client.head(&bucket, &key).map(|response| info_of(&key, &response)).map_err(|fail| fail.text)).await
}

/// 그런 객체나 폴더(그 이름으로 시작하는 것)가 있는지. 주소 줄에 적은 곳이 파일인지 폴더인지 가릴 때 쓴다.
#[tauri::command]
pub async fn s3_exists(handle: u32, bucket: String, key: String) -> Result<bool, String> {
    let client = client(handle)?;
    blocking(move || client.exists(&bucket, &key).map_err(|fail| fail.text)).await
}

const IMAGE_TYPES: [(&str, &str); 9] = [
    ("png", "image/png"), ("jpg", "image/jpeg"), ("jpeg", "image/jpeg"), ("gif", "image/gif"), ("webp", "image/webp"),
    ("bmp", "image/bmp"), ("svg", "image/svg+xml"), ("ico", "image/x-icon"), ("avif", "image/avif"),
];

fn extension(key: &str) -> String {
    let name = key.rsplit('/').next().unwrap_or(key);
    name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default()
}

/// 올릴 파일의 종류(확장자로 본다).
fn content_type_of(key: &str) -> &'static str {
    let ext = extension(key);
    if let Some((_, kind)) = IMAGE_TYPES.iter().find(|(name, _)| *name == ext) {
        return kind;
    }
    match ext.as_str() {
        "txt" | "log" | "ini" | "conf" | "cfg" | "properties" | "env" => "text/plain",
        "md" | "markdown" => "text/markdown",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "js" | "mjs" => "text/javascript",
        "json" | "jsonl" | "ndjson" => "application/json",
        "xml" => "application/xml",
        "yaml" | "yml" => "application/yaml",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        "7z" => "application/x-7z-compressed",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "parquet" => "application/vnd.apache.parquet",
        _ => "application/octet-stream",
    }
}

/// 객체 하나를 화면에 보여 줄 내용.
#[derive(Debug, Serialize)]
pub struct Preview {
    /// `text`(글로 보임), `image`(그림), `binary`(보여 줄 수 없음)
    kind: &'static str,
    text: String,
    encoding: String,
    eol: String,
    /// 읽지 못한 글자가 있는지(있으면 고쳐 저장하지 못하게 한다).
    lossy: bool,
    /// 앞부분만 읽었는지.
    truncated: bool,
    /// 그림이면 `data:` 주소.
    image: String,
    info: ObjectInfo,
}

/// 끝이 글자 중간에서 잘린 UTF-8이면 그 글자를 뗀다.
fn trim_partial_utf8(bytes: &[u8]) -> &[u8] {
    match std::str::from_utf8(bytes) {
        Err(error) if error.error_len().is_none() => &bytes[..error.valid_up_to()],
        _ => bytes,
    }
}

fn preview(client: &Client, bucket: &str, key: &str) -> Result<Preview, Fail> {
    let ext = extension(key);
    let image_type = IMAGE_TYPES.iter().find(|(name, _)| *name == ext).map(|(_, kind)| *kind);
    let limit = if image_type.is_some() { IMAGE_MAX } else { TEXT_MAX };
    let mut request = client.object("GET", bucket, key);
    request.headers.push(("range".into(), format!("bytes=0-{}", limit - 1)));
    let response = match client.send(&request, Body::None) {
        Ok(response) => response,
        // 빈 객체는 범위로 읽을 수 없다(416): 정보만 따로 읽는다.
        Err(fail) if fail.status == 416 => {
            let info = info_of(key, &client.head(bucket, key)?);
            return Ok(Preview { kind: "text", text: String::new(), encoding: cypher_core::textfile::UTF8.into(), eol: cypher_core::textfile::LF.into(), lossy: false, truncated: false, image: String::new(), info });
        }
        Err(fail) => return Err(fail),
    };
    let info = info_of(key, &response);
    let mut bytes = Vec::new();
    // 범위를 모르는 저장소가 통째로 보내도 한도까지만 읽는다.
    response.into_reader().take(limit).read_to_end(&mut bytes).map_err(|e| Fail::local(format!("내용을 읽다가 끊겼습니다: {e}")))?;
    let truncated = info.size > bytes.len() as u64;
    let empty = |kind| Preview { kind, text: String::new(), encoding: String::new(), eol: String::new(), lossy: false, truncated, image: String::new(), info: info.clone() };
    if let Some(kind) = image_type {
        if truncated {
            return Ok(empty("binary"));
        }
        let data = format!("data:{kind};base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes));
        return Ok(Preview { image: data, ..empty("image") });
    }
    let shown = if truncated { trim_partial_utf8(&bytes) } else { &bytes[..] };
    match cypher_core::textfile::decode(shown) {
        Ok(decoded) => Ok(Preview { text: decoded.text, encoding: decoded.encoding.into(), eol: decoded.eol.into(), lossy: decoded.lossy, ..empty("text") }),
        Err(_) => Ok(empty("binary")),
    }
}

/// 객체 하나의 내용을 읽어 온다: 글이면 앞 1MB까지, 그림이면 8MB까지. 그 밖의 것은 정보만.
#[tauri::command]
pub async fn s3_preview(handle: u32, bucket: String, key: String) -> Result<Preview, String> {
    let client = client(handle)?;
    blocking(move || preview(&client, &bucket, &key).map_err(|fail| fail.text)).await
}

/// 고친 글을 그 객체에 쓴다(덮어쓴다. 화면이 먼저 확인을 받는다). 원래의 종류와 사용자 메타데이터, 객체에 붙어 있던
/// 표준 헤더(Cache-Control 등)와 저장 등급은 그대로 붙인다. `etag`(읽었을 때의 확인 값)를 주면 쓰기 직전에 한 번 물어,
/// 그 사이에 다른 사람이 바꿨으면 쓰지 않는다(남의 고침을 모르고 덮어쓰지 않게).
#[tauri::command]
pub async fn s3_put_text(
    handle: u32, bucket: String, key: String, text: String, encoding: String, eol: String, content_type: String, meta: Vec<(String, String)>,
    etag: Option<String>,
) -> Result<(), String> {
    let client = client(handle)?;
    blocking(move || {
        let bytes = cypher_core::textfile::encode(&text, &encoding, &eol)?;
        if let Some(seen) = etag.as_deref().filter(|seen| !seen.is_empty()) {
            match client.head(&bucket, &key) {
                Ok(response) => {
                    if response.header("etag").unwrap_or("").trim_matches('"') != seen.trim_matches('"') {
                        return Err("읽은 뒤에 이 객체가 바뀌었습니다(다른 사람이나 다른 프로그램이 고쳤습니다). 저장하지 않았습니다. 고친 글을 복사해 둔 뒤 다시 열어 고쳐 주세요.".into());
                    }
                }
                Err(fail) if fail.missing() => return Err("읽은 뒤에 이 객체가 지워졌습니다. 저장하지 않았습니다.".into()),
                Err(fail) => return Err(format!("저장하기 전에 객체를 확인하지 못했습니다: {}", fail.text)),
            }
        }
        let kind = if content_type.is_empty() { content_type_of(&key).to_string() } else { content_type };
        client.put_bytes(&bucket, &key, &bytes, &kind, &meta).map_err(|fail| fail.text)
    })
    .await
}

/// 빈 폴더를 만든다(`이름/`이라는 빈 객체). 같은 이름의 것이 이미 있으면 만들지 않는다(있던 것을 덮어쓰지 않게 먼저 묻는다).
#[tauri::command]
pub async fn s3_mkdir(handle: u32, bucket: String, key: String) -> Result<(), String> {
    let client = client(handle)?;
    blocking(move || {
        if !key.ends_with('/') || key.len() < 2 {
            return Err("폴더의 키는 `/`로 끝나야 합니다.".into());
        }
        if client.exists(&bucket, &key).map_err(|fail| fail.text)? {
            return Err("같은 이름의 폴더가 이미 있습니다.".into());
        }
        client.put_bytes(&bucket, &key, &[], "application/x-directory", &[]).map_err(|fail| fail.text)
    })
    .await
}

/// 버킷을 만든다.
#[tauri::command]
pub async fn s3_bucket_create(handle: u32, bucket: String) -> Result<(), String> {
    let client = client(handle)?;
    blocking(move || client.send(&Request { method: "PUT", bucket: Some(bucket), ..Request::default() }, Body::None).map(drop).map_err(|fail| fail.text)).await
}

/// 빈 버킷을 지운다(화면이 먼저 확인을 받는다). 비어 있지 않으면 저장소가 거절한다.
#[tauri::command]
pub async fn s3_bucket_remove(handle: u32, bucket: String) -> Result<(), String> {
    let client = client(handle)?;
    blocking(move || client.send(&Request { method: "DELETE", bucket: Some(bucket), ..Request::default() }, Body::None).map(drop).map_err(|fail| fail.text)).await
}

// ───────────────────────── 작업 ─────────────────────────

/// 돌고 있는 작업 하나.
struct Job {
    handle: u32,
    cancel: AtomicBool,
    /// 겹침을 물어 놓고 답을 기다리는 동안의 답 받는 곳.
    answer: Mutex<Option<mpsc::Sender<String>>>,
}

impl Job {
    fn stop(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        if let Some(sender) = self.answer.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = sender.send("cancel".into());
        }
    }
}

/// 화면에 알리는 작업의 상태.
#[derive(Debug, Clone, Default, Serialize)]
pub struct JobEvent {
    job: u32,
    handle: u32,
    /// `running` | `conflict`(겹침을 묻고 기다리는 중) | `done` | `failed` | `canceled`
    state: &'static str,
    /// 무엇을 하는지(`올리기` 등)
    title: String,
    /// 지금 무엇을 하는 중인지(파일 이름, "목록 읽는 중" 등)
    current: String,
    done: u64,
    total: u64,
    bytes: u64,
    bytes_total: u64,
    /// 작업 전체가 멈춘 까닭(`failed`일 때)
    error: String,
    /// 하나씩 실패한 것들(앞의 몇 개)과 그 수
    failed: Vec<String>,
    failed_count: u64,
    /// 이미 있어서 덮어쓰게 되는 것들(앞의 몇 개)과 그 수
    conflicts: Vec<String>,
    conflict_count: u64,
    /// 이미 있어서 건너뛴 수
    skipped: u64,
    /// 저장소(내려받기에서는 이 PC)에 바뀐 것이 있는지
    changed: bool,
}

type Sink = Box<dyn Fn(&JobEvent) + Send>;

/// 작업을 돌리는 동안의 상태: 진행을 세고, 알리고, 취소를 살핀다.
struct Run {
    job: Arc<Job>,
    sink: Sink,
    event: JobEvent,
    last: Instant,
    streak: u32,
}

impl Run {
    fn canceled(&self) -> bool {
        self.job.cancel.load(Ordering::SeqCst)
    }

    fn check(&self) -> Result<(), String> {
        if self.canceled() { Err(CANCELED.into()) } else { Ok(()) }
    }

    fn emit(&mut self) {
        self.last = Instant::now();
        (self.sink)(&self.event);
    }

    /// 진행을 알린다(너무 잦지 않게).
    fn tick(&mut self) {
        if self.last.elapsed() >= TICK {
            self.emit();
        }
    }

    fn step(&mut self, current: &str) -> Result<(), String> {
        self.check()?;
        self.event.current = current.to_string();
        self.tick();
        Ok(())
    }

    fn ok(&mut self) {
        self.streak = 0;
        self.event.done += 1;
        self.event.changed = true;
    }

    /// 실패 하나를 적어 둔다.
    fn note(&mut self, what: &str, why: &str) {
        self.event.failed_count += 1;
        if self.event.failed.len() < SHOWN {
            self.event.failed.push(format!("{what}: {why}"));
        }
    }

    /// 하나가 실패했다. 연달아 실패하면 작업을 멈춘다.
    fn fail(&mut self, what: &str, why: &str) -> Result<(), String> {
        self.note(what, why);
        self.event.done += 1;
        self.streak += 1;
        if self.streak >= FAIL_STREAK {
            return Err(format!("연달아 실패해 멈췄습니다: {why}"));
        }
        Ok(())
    }

    /// 이미 있는 것을 덮어쓰게 된다: 화면에 묻고 답을 기다린다. 덮어쓰면 참, 건너뛰면 거짓.
    fn ask(&mut self, conflicts: &[String]) -> Result<bool, String> {
        if conflicts.is_empty() {
            return Ok(true);
        }
        let (sender, receiver) = mpsc::channel();
        *self.job.answer.lock().unwrap_or_else(|e| e.into_inner()) = Some(sender);
        self.check()?;
        self.event.state = "conflict";
        self.event.conflicts = conflicts.iter().take(SHOWN).cloned().collect();
        self.event.conflict_count = conflicts.len() as u64;
        self.emit();
        let answer = receiver.recv().unwrap_or_else(|_| "cancel".into());
        self.job.answer.lock().unwrap_or_else(|e| e.into_inner()).take();
        self.event.state = "running";
        self.event.conflicts.clear();
        self.event.conflict_count = 0;
        match answer.as_str() {
            "overwrite" => Ok(true),
            "skip" => Ok(false),
            _ => {
                self.job.cancel.store(true, Ordering::SeqCst);
                Err(CANCELED.into())
            }
        }
    }
}

/// 작업을 따로 도는 줄(스레드)에서 시작하고 번호를 돌려준다.
fn start_job(handle: u32, title: &str, sink: Sink, work: impl FnOnce(&mut Run) -> Result<(), String> + Send + 'static) -> u32 {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let job = Arc::new(Job { handle, cancel: AtomicBool::new(false), answer: Mutex::new(None) });
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).insert(id, job.clone());
    let event = JobEvent { job: id, handle, state: "running", title: title.to_string(), ..JobEvent::default() };
    std::thread::spawn(move || {
        let mut run = Run { job, sink, event, last: Instant::now(), streak: 0 };
        run.emit();
        // 작업이 뜻밖에 무너져도(패닉) 화면에는 끝났다고 알린다(그러지 않으면 "도는 중"으로 영영 남는다).
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&mut run)))
            .unwrap_or_else(|_| Err("작업이 뜻밖의 오류로 멈췄습니다.".to_string()));
        match result {
            Ok(()) => run.event.state = "done",
            Err(_) if run.canceled() => run.event.state = "canceled",
            Err(error) => {
                run.event.state = "failed";
                run.event.error = error;
            }
        }
        run.event.current.clear();
        JOBS.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        run.emit();
    });
    id
}

fn sink_of(app: AppHandle) -> Sink {
    Box::new(move |event| {
        let _ = app.emit_to("main", "s3-job", event.clone());
    })
}

/// 작업을 멈춘다(하던 요청 하나가 끝나면 멈춘다).
#[tauri::command]
pub fn s3_job_cancel(job: u32) {
    if let Some(job) = JOBS.lock().unwrap_or_else(|e| e.into_inner()).get(&job) {
        job.stop();
    }
}

/// 겹침을 물은 작업에 답한다: `overwrite`(덮어쓰기), `skip`(있는 것은 건너뛰기), `cancel`.
#[tauri::command]
pub fn s3_job_answer(job: u32, answer: String) {
    if let Some(job) = JOBS.lock().unwrap_or_else(|e| e.into_inner()).get(&job) {
        if let Some(sender) = job.answer.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = sender.send(answer);
        }
    }
}

/// 화면이 고른 것 하나: 키(폴더는 `/`로 끝난다)와 크기.
#[derive(Debug, Clone, Deserialize)]
pub struct Item {
    key: String,
    #[serde(default)]
    size: u64,
}

/// 저장소의 객체 하나(작업이 다룰 것).
#[derive(Debug, Clone, PartialEq)]
struct Remote {
    key: String,
    size: u64,
}

/// 스스로 이어 읽는 목록에서, 저장소가 같은 쪽을 되풀이해 줄 때(다음 쪽의 표가 방금 쓴 표와 같다) 내는 오류.
fn stuck(prefix: &str) -> String {
    format!("저장소가 같은 목록을 되풀이해 돌려줘 멈췄습니다({prefix}).")
}

/// 그 앞머리 아래의 모든 객체를 1,000개씩 이어 읽어 `each`에 넘긴다.
fn walk(client: &Client, bucket: &str, prefix: &str, run: &mut Run, mut each: impl FnMut(&mut Run, Vec<api::Object>) -> Result<(), String>) -> Result<(), String> {
    let mut next: Option<String> = None;
    loop {
        run.step(&format!("목록 읽는 중: {prefix}"))?;
        let page = client.list(bucket, prefix, false, PAGE_MAX, next.as_deref()).map_err(|fail| format!("목록을 읽지 못했습니다({prefix}): {}", fail.text))?;
        // 물은 앞머리로 시작하지 않는 키가 오면 그 저장소는 앞머리를 알아듣지 못한 것이다. 그대로 두면 폴더 하나를
        // 지우려던 것이 버킷 전체를 지우게 되므로, 아무것도 하지 않고 멈춘다.
        if let Some(stray) = page.objects.iter().find(|object| !object.key.starts_with(prefix)) {
            return Err(format!("저장소가 {prefix} 아래가 아닌 객체({})를 돌려줘 멈췄습니다.", stray.key));
        }
        each(run, page.objects)?;
        match page.next {
            Some(token) if next.as_deref() == Some(token.as_str()) => return Err(stuck(prefix)),
            Some(token) => next = Some(token),
            None => return Ok(()),
        }
    }
}

/// 고른 것들을 객체의 목록으로 편다: 폴더는 그 아래를 모두 읽는다.
fn expand(client: &Client, bucket: &str, items: &[Item], run: &mut Run) -> Result<Vec<Remote>, String> {
    let mut found = Vec::new();
    for item in items {
        if item.key.ends_with('/') {
            walk(client, bucket, &item.key, run, |run, objects| {
                found.extend(objects.into_iter().map(|object| Remote { key: object.key, size: object.size }));
                run.event.total = found.len() as u64;
                Ok(())
            })?;
        } else {
            found.push(Remote { key: item.key.clone(), size: item.size });
        }
    }
    run.event.total = found.len() as u64;
    Ok(found)
}

/// 놓일 자리 한 곳: 그 앞머리 아래에 놓일 키들. `deep`이면 그 아래 전부(폴더째 놓는 것), 아니면 바로 아래(낱개)다.
struct Scope {
    prefix: String,
    deep: bool,
    keys: Vec<String>,
}

/// 놓일 키들 가운데 저장소에 이미 있는 것. 적으면 하나씩 묻고(HEAD), 많으면 그 자리의 목록을 읽어 견준다.
/// 목록으로 견주는 것은 값어치가 있을 때까지만 한다: 그 자리에 객체가 훨씬 많아 읽을 쪽이 많으면 그만두고
/// 남은 것을 하나씩 묻는다(51개를 올리려고 200만 개짜리 폴더를 다 읽지 않는다).
fn existing(client: &Client, bucket: &str, scopes: &[Scope], run: &mut Run) -> Result<HashSet<String>, String> {
    let mut found: HashSet<String> = HashSet::new();
    for scope in scopes {
        let mut ask: Vec<&String> = scope.keys.iter().collect();
        if scope.keys.len() > HEAD_MAX {
            let wanted: HashSet<&String> = scope.keys.iter().collect();
            let budget = (scope.keys.len() / LIST_WORTH).max(1);
            let mut next: Option<String> = None;
            let mut finished = false;
            for _ in 0..budget {
                run.step(&format!("목록 읽는 중: {}", scope.prefix))?;
                let page = client.list(bucket, &scope.prefix, !scope.deep, PAGE_MAX, next.as_deref()).map_err(|fail| format!("목록을 읽지 못했습니다({}): {}", scope.prefix, fail.text))?;
                found.extend(page.objects.into_iter().map(|object| object.key).filter(|key| wanted.contains(key)));
                match page.next {
                    Some(token) if next.as_deref() == Some(token.as_str()) => return Err(stuck(&scope.prefix)),
                    Some(token) => next = Some(token),
                    None => {
                        finished = true;
                        break;
                    }
                }
            }
            if finished {
                ask.clear();
            } else {
                // 다 읽지 못했으니 목록에 없던 것이 "없다"고 할 수 없다: 아직 모르는 것만 하나씩 묻는다.
                ask.retain(|key| !found.contains(*key));
            }
        }
        for key in ask {
            run.step(&format!("이미 있는지 확인하는 중: {key}"))?;
            if client.exists(bucket, key).map_err(|fail| format!("확인하지 못했습니다({key}): {}", fail.text))? {
                found.insert(key.clone());
            }
        }
    }
    Ok(found)
}

/// 맨 위에 놓일 폴더들과 낱개들을 "놓일 자리"로 나눈다. `keys`는 놓일 키 전부다.
fn scopes_of(parent: &str, folders: &[String], files: &[String], keys: &[&String]) -> Vec<Scope> {
    let mut scopes: Vec<Scope> = folders
        .iter()
        .map(|folder| Scope { prefix: folder.clone(), deep: true, keys: keys.iter().filter(|key| key.starts_with(folder.as_str())).map(|key| (*key).clone()).collect() })
        .collect();
    if !files.is_empty() {
        scopes.push(Scope { prefix: parent.to_string(), deep: false, keys: files.to_vec() });
    }
    scopes
}

// ── 올리기 ──

#[derive(Debug, PartialEq)]
struct Local {
    path: PathBuf,
    key: String,
    size: u64,
    /// 빈 폴더(자리만 만든다).
    folder: bool,
}

/// 올릴 것들을 펼친다: 파일은 그대로, 폴더는 그 아래를 모두. (올릴 것, 맨 위 폴더들의 키, 맨 위 파일들의 키)
fn scan_local(paths: &[String], prefix: &str, run: &mut Run) -> Result<(Vec<Local>, Vec<String>, Vec<String>), String> {
    fn name_of(path: &Path) -> Option<String> {
        path.file_name().map(|name| name.to_string_lossy().into_owned()).filter(|name| !name.is_empty())
    }
    fn visit(dir: &Path, key: &str, found: &mut Vec<Local>, run: &mut Run) -> Result<(), String> {
        run.step(&format!("올릴 파일 찾는 중: {}", dir.display()))?;
        let mut children: Vec<std::fs::DirEntry> = std::fs::read_dir(dir).map_err(|e| format!("폴더를 읽지 못했습니다({}): {e}", dir.display()))?.filter_map(Result::ok).collect();
        children.sort_by_key(|child| child.file_name());
        let before = found.len();
        for child in children {
            let Ok(kind) = child.file_type() else { continue };
            let Some(name) = name_of(&child.path()) else { continue };
            // 바로 가기(심볼릭 링크·연결 지점)는 따라가지 않는다(끝없이 돌거나 뜻하지 않은 폴더가 올라간다).
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                visit(&child.path(), &format!("{key}{name}/"), found, run)?;
            } else if kind.is_file() {
                let size = child.metadata().map(|meta| meta.len()).unwrap_or(0);
                found.push(Local { path: child.path(), key: format!("{key}{name}"), size, folder: false });
                run.event.total = found.len() as u64;
            }
        }
        if found.len() == before {
            found.push(Local { path: dir.to_path_buf(), key: key.to_string(), size: 0, folder: true });
        }
        Ok(())
    }
    let (mut found, mut folders, mut files) = (Vec::new(), Vec::new(), Vec::new());
    for given in paths {
        let path = Path::new(given);
        let name = name_of(path).ok_or_else(|| format!("올릴 수 없는 경로입니다: {given}"))?;
        let meta = std::fs::metadata(path).map_err(|e| format!("읽을 수 없습니다({given}): {e}"))?;
        if meta.is_dir() {
            let key = format!("{prefix}{name}/");
            visit(path, &key, &mut found, run)?;
            folders.push(key);
        } else {
            let key = format!("{prefix}{name}");
            found.push(Local { path: path.to_path_buf(), key: key.clone(), size: meta.len(), folder: false });
            files.push(key);
        }
    }
    run.event.total = found.len() as u64;
    run.event.bytes_total = found.iter().map(|local| local.size).sum();
    Ok((found, folders, files))
}

/// 읽어 가는 만큼 진행을 세고, 취소되면 읽기를 멈춘다.
struct Feed<'a, R: Read> {
    inner: R,
    run: &'a mut Run,
}

impl<R: Read> Read for Feed<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.run.canceled() {
            return Err(std::io::Error::other(CANCELED));
        }
        let read = self.inner.read(buffer)?;
        self.run.event.bytes += read as u64;
        self.run.tick();
        Ok(read)
    }
}

fn file_sha256(file: &mut std::fs::File, run: &Run) -> Result<String, String> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        run.check()?;
        let read = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    Ok(api::hex(&hasher.finalize()))
}

/// 파일 하나를 올린다. 크면 조각으로 나눠 올린다.
fn upload_file(client: &Client, bucket: &str, local: &Local, run: &mut Run) -> Result<(), String> {
    let mut file = std::fs::File::open(&local.path).map_err(|e| format!("파일을 열지 못했습니다: {e}"))?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    let kind = content_type_of(&local.key);
    if size <= SINGLE_PUT_MAX {
        let mut request = client.object("PUT", bucket, &local.key);
        request.headers.push(("content-type".into(), kind.into()));
        // 서명에 본문의 해시가 들어가므로 먼저 한 번 읽어 해시를 낸다.
        if client.signature == Signature::V4 {
            request.payload_sha256 = file_sha256(&mut file, run)?;
        }
        // 연 때의 크기만큼만 보낸다(올리는 사이 파일이 커져도 적어 보낸 길이와 어긋나지 않게).
        let mut feed = Feed { inner: file.take(size), run: &mut *run };
        return client.send(&request, Body::Reader(&mut feed, size)).map(drop).map_err(|fail| fail.text);
    }
    let part_size = PART_SIZE.max(size.div_ceil(MAX_PARTS).div_ceil(1 << 20) << 20);
    let upload = client.begin_multipart(bucket, &local.key, vec![("content-type".into(), kind.into())])?;
    let mut send_parts = |run: &mut Run| -> Result<Vec<(u32, String)>, String> {
        let mut parts = Vec::new();
        let mut buffer = vec![0u8; part_size as usize];
        let mut left = size;
        let mut number = 0u32;
        while left > 0 {
            run.check()?;
            let length = left.min(part_size) as usize;
            file.read_exact(&mut buffer[..length]).map_err(|e| format!("파일을 읽지 못했습니다: {e}"))?;
            number += 1;
            let mut request = client.object("PUT", bucket, &local.key);
            request.query = Client::part_query(&upload, Some(number));
            request.payload_sha256 = client.hash(&buffer[..length]);
            let mut feed = Feed { inner: &buffer[..length], run: &mut *run };
            let response = client.send(&request, Body::Reader(&mut feed, length as u64)).map_err(|fail| fail.text)?;
            let etag = response.header("etag").unwrap_or("").to_string();
            if etag.is_empty() {
                return Err("저장소가 조각의 확인 값(ETag)을 돌려주지 않았습니다.".into());
            }
            parts.push((number, etag));
            left -= length as u64;
        }
        Ok(parts)
    };
    send_parts(run)
        .and_then(|parts| client.complete_multipart(bucket, &local.key, &upload, &parts))
        .map_err(|why| client.abort_multipart(bucket, &local.key, &upload, why))
}

fn upload(client: &Client, bucket: &str, prefix: &str, paths: &[String], run: &mut Run) -> Result<(), String> {
    let (locals, folders, files) = scan_local(paths, prefix, run)?;
    let keys: Vec<&String> = locals.iter().map(|local| &local.key).collect();
    let present = existing(client, bucket, &scopes_of(prefix, &folders, &files, &keys), run)?;
    let mut conflicts: Vec<String> = locals.iter().filter(|local| !local.folder && present.contains(&local.key)).map(|local| local.key.clone()).collect();
    conflicts.sort();
    let overwrite = run.ask(&conflicts)?;
    for local in &locals {
        let taken = present.contains(&local.key);
        if taken && (local.folder || !overwrite) {
            if !local.folder {
                run.event.skipped += 1;
            }
            run.event.done += 1;
            run.event.bytes += local.size;
            continue;
        }
        run.step(&local.key)?;
        let before = run.event.bytes;
        let result = if local.folder {
            client.put_bytes(bucket, &local.key, &[], "application/x-directory", &[]).map_err(|fail| fail.text)
        } else {
            upload_file(client, bucket, local, run)
        };
        run.event.bytes = before + local.size;
        match result {
            Ok(()) => run.ok(),
            Err(error) => {
                run.check()?;
                run.fail(&local.key, &error)?;
            }
        }
    }
    Ok(())
}

/// 이 PC의 파일·폴더를 `prefix` 아래에 올린다. 작업 번호를 돌려준다.
#[tauri::command]
pub fn s3_upload(app: AppHandle, handle: u32, bucket: String, prefix: String, paths: Vec<String>) -> Result<u32, String> {
    let client = client(handle)?;
    Ok(start_job(handle, "올리기", sink_of(app), move |run| upload(&client, &bucket, &prefix, &paths, run)))
}

/// 올릴 파일을 고른다(여러 개). 취소하면 빈 목록.
#[tauri::command]
pub async fn s3_pick_files(start: Option<String>) -> Vec<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = rfd::FileDialog::new().set_title("S3 저장소에 올릴 파일");
        if let Some(dir) = start.filter(|dir| Path::new(dir).is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        dialog.pick_files().unwrap_or_default().into_iter().map(|path| path.to_string_lossy().into_owned()).collect()
    })
    .await
    .unwrap_or_default()
}

// ── 내려받기 ──

/// 키의 한 마디를 Windows의 파일 이름으로 쓸 수 있게 다듬는다.
fn safe_name(part: &str) -> String {
    let mut name: String = part.chars().map(|ch| if matches!(ch, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\\' | '/') || ch.is_control() { '_' } else { ch }).collect();
    while name.ends_with('.') || name.ends_with(' ') {
        name.pop();
    }
    if name.is_empty() {
        return "_".into();
    }
    // 장치 이름(CON, COM1 …)은 확장자를 붙여도, 이름 뒤에 빈칸이 있어도 장치로 열린다.
    let stem = name.split('.').next().unwrap_or("").trim_end().to_uppercase();
    let numbered = |front: &str| stem.strip_prefix(front).is_some_and(|rest| matches!(rest, "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"));
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$") || numbered("COM") || numbered("LPT");
    if reserved { format!("_{name}") } else { name }
}

/// 내려받을 것 하나가 놓일 자리.
#[derive(Debug, PartialEq)]
struct Target {
    remote: Remote,
    path: PathBuf,
    /// 폴더 자리만 나타내는 빈 객체(`이름/`)라 폴더만 만든다.
    folder: bool,
}

/// 내려받을 객체마다 이 PC의 경로를 정한다: `base`(보고 있던 폴더) 아래의 경로를 `dest` 아래에 그대로 편다.
fn plan_download(remotes: Vec<Remote>, base: &str, dest: &Path) -> Vec<Target> {
    let mut taken: HashSet<String> = HashSet::new();
    let mut targets = Vec::new();
    for remote in remotes {
        let relative = remote.key.strip_prefix(base).unwrap_or(&remote.key);
        let folder = relative.ends_with('/');
        let parts: Vec<String> = relative.split('/').filter(|part| !part.is_empty()).map(safe_name).collect();
        if parts.is_empty() {
            continue;
        }
        let mut path = dest.to_path_buf();
        path.extend(&parts);
        if !folder {
            // 다듬은 이름이 겹치면(Windows는 대/소문자도 가리지 않는다) 뒤에 번호를 붙인다.
            let mut count = 1;
            let original = path.clone();
            while !taken.insert(path.to_string_lossy().to_lowercase()) {
                count += 1;
                let stem = original.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let ext = original.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
                path = original.with_file_name(format!("{stem} ({count}){ext}"));
            }
        }
        targets.push(Target { remote, path, folder });
    }
    targets
}

fn download_one(client: &Client, bucket: &str, target: &Target, run: &mut Run) -> Result<(), String> {
    if let Some(dir) = target.path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("폴더를 만들지 못했습니다({}): {e}", dir.display()))?;
    }
    let response = client.send(&client.object("GET", bucket, &target.remote.key), Body::None).map_err(|fail| fail.text)?;
    // 다 받은 뒤에 이름을 바꿔 넣는다(받다 만 파일이 제 이름으로 남지 않게).
    // 받는 동안의 이름은 이 작업만의 것으로 하고, 같은 이름의 파일이 이미 있으면 건드리지 않고 그만둔다
    // (받을 폴더에 있던 다른 파일을 비우거나 바꿔치기하지 않게).
    let mut name = target.path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".cypher-part-{}", run.event.job));
    let temp = target.path.with_file_name(name);
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp).map_err(|e| format!("받는 동안 쓸 파일을 만들지 못했습니다({}): {e}", temp.display()))?;
    let result = (|| -> Result<(), String> {
        let mut reader = response.into_reader();
        let mut buffer = vec![0u8; 256 << 10];
        loop {
            run.check()?;
            let read = reader.read(&mut buffer).map_err(|e| format!("받다가 끊겼습니다: {e}"))?;
            if read == 0 {
                break;
            }
            file.write_all(&buffer[..read]).map_err(|e| format!("파일에 쓰지 못했습니다: {e}"))?;
            run.event.bytes += read as u64;
            run.tick();
        }
        file.flush().map_err(|e| e.to_string())?;
        std::fs::rename(&temp, &target.path).map_err(|e| format!("파일을 제자리에 놓지 못했습니다({}): {e}", target.path.display()))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

fn download(client: &Client, bucket: &str, base: &str, items: &[Item], dest: &Path, run: &mut Run) -> Result<(), String> {
    let remotes = expand(client, bucket, items, run)?;
    let targets = plan_download(remotes, base, dest);
    run.event.total = targets.len() as u64;
    run.event.bytes_total = targets.iter().map(|target| target.remote.size).sum();
    let mut conflicts: Vec<String> = targets.iter().filter(|target| !target.folder && target.path.exists()).map(|target| target.path.to_string_lossy().into_owned()).collect();
    conflicts.sort();
    let overwrite = run.ask(&conflicts)?;
    for target in &targets {
        if target.folder {
            let _ = std::fs::create_dir_all(&target.path);
            run.event.done += 1;
            continue;
        }
        if !overwrite && target.path.exists() {
            run.event.skipped += 1;
            run.event.done += 1;
            run.event.bytes += target.remote.size;
            continue;
        }
        run.step(&target.remote.key)?;
        let before = run.event.bytes;
        let result = download_one(client, bucket, target, run);
        run.event.bytes = before + target.remote.size;
        match result {
            Ok(()) => run.ok(),
            Err(error) => {
                run.check()?;
                run.fail(&target.remote.key, &error)?;
            }
        }
    }
    Ok(())
}

/// 고른 객체·폴더를 이 PC의 `dest` 폴더에 내려받는다. `base`는 보고 있던 폴더(그 아래의 경로를 그대로 편다).
#[tauri::command]
pub fn s3_download(app: AppHandle, handle: u32, bucket: String, base: String, items: Vec<Item>, dest: String) -> Result<u32, String> {
    let client = client(handle)?;
    let dest = PathBuf::from(dest);
    if !dest.is_dir() {
        return Err(format!("내려받을 폴더가 없습니다: {}", dest.display()));
    }
    Ok(start_job(handle, "내려받기", sink_of(app), move |run| download(&client, &bucket, &base, &items, &dest, run)))
}

// ── 지우기 ──

fn delete_batch(client: &Client, bucket: &str, keys: &[String], run: &mut Run) -> Result<(), String> {
    for chunk in keys.chunks(PAGE_MAX as usize) {
        run.step(&format!("지우는 중: {}", chunk.first().map(String::as_str).unwrap_or("")))?;
        let failed = client.delete_many(bucket, chunk).map_err(|fail| fail.text)?;
        let done = chunk.len().saturating_sub(failed.len());
        run.event.done += chunk.len() as u64;
        for (key, why) in &failed {
            run.note(key, why);
        }
        if done > 0 {
            run.event.changed = true;
            run.streak = 0;
        } else {
            // 묶음에서 하나도 지우지 못했을 때만 "연달아 실패"로 센다(1,000개 가운데 잠긴 객체 몇 개 때문에 멈추지 않는다).
            run.streak += 1;
            if run.streak >= FAIL_STREAK {
                return Err(format!("연달아 실패해 멈췄습니다: {}", failed.first().map(|(_, why)| why.as_str()).unwrap_or("")));
            }
        }
        run.tick();
    }
    Ok(())
}

fn delete(client: &Client, bucket: &str, items: &[Item], run: &mut Run) -> Result<(), String> {
    let files: Vec<String> = items.iter().filter(|item| !item.key.ends_with('/')).map(|item| item.key.clone()).collect();
    run.event.total = files.len() as u64;
    delete_batch(client, bucket, &files, run)?;
    for folder in items.iter().filter(|item| item.key.ends_with('/')) {
        // 한 쪽(1,000개)을 읽을 때마다 그만큼 지운다: 큰 폴더도 목록을 한 번씩만 읽고, 메모리에 다 올리지 않는다.
        walk(client, bucket, &folder.key, run, |run, objects| {
            let keys: Vec<String> = objects.into_iter().map(|object| object.key).collect();
            run.event.total += keys.len() as u64;
            delete_batch(client, bucket, &keys, run)
        })?;
    }
    Ok(())
}

/// 고른 객체·폴더(그 아래 전부)를 지운다. 화면이 먼저 확인을 받는다.
#[tauri::command]
pub fn s3_delete(app: AppHandle, handle: u32, bucket: String, items: Vec<Item>) -> Result<u32, String> {
    let client = client(handle)?;
    Ok(start_job(handle, "삭제", sink_of(app), move |run| delete(&client, &bucket, &items, run)))
}

// ── 복사·옮기기·이름 바꾸기 ──

/// 복사할 것들과 놓일 곳.
#[derive(Debug, Clone, Deserialize)]
pub struct CopyPlan {
    /// 원본이 있는 버킷과, 보고 있던 폴더(그 아래의 경로를 그대로 옮긴다).
    bucket: String,
    base: String,
    items: Vec<Item>,
    to_bucket: String,
    to_prefix: String,
    /// 원본을 지울지(옮기기·이름 바꾸기).
    #[serde(default)]
    remove: bool,
    /// 하나만 옮기면서 이름을 바꿀 때의 새 이름(폴더도 끝의 `/` 없이).
    #[serde(default)]
    rename: Option<String>,
}

/// 원본 키가 놓일 키. `rename`이 있으면 고른 것(하나)의 이름을 그것으로 바꾼다.
fn destination(plan: &CopyPlan, item: &Item, key: &str) -> String {
    match &plan.rename {
        Some(name) if item.key.ends_with('/') => format!("{}{}/{}", plan.to_prefix, name, key.strip_prefix(&item.key).unwrap_or("")),
        Some(name) => format!("{}{}", plan.to_prefix, name),
        None => format!("{}{}", plan.to_prefix, key.strip_prefix(&plan.base).unwrap_or(key)),
    }
}

fn copy(client: &Client, plan: &CopyPlan, run: &mut Run) -> Result<(), String> {
    copy_with(client, plan, run, COPY_SINGLE_MAX, COPY_PART)
}

/// `single_max`보다 큰 객체는 `part` 크기의 조각으로 나눠 복사한다(시험에서는 작은 값으로 돌려 본다).
fn copy_with(client: &Client, plan: &CopyPlan, run: &mut Run, single_max: u64, part: u64) -> Result<(), String> {
    if plan.rename.is_some() && plan.items.len() != 1 {
        return Err("이름은 하나씩만 바꿀 수 있습니다.".into());
    }
    if let Some(name) = &plan.rename {
        if name.is_empty() || name.contains('/') {
            return Err("이름에 `/`를 쓸 수 없습니다.".into());
        }
    }
    let same_bucket = plan.bucket == plan.to_bucket;
    // (원본 키, 크기, 놓일 키)
    let mut pairs: Vec<(String, u64, String)> = Vec::new();
    let (mut folders, mut files) = (Vec::new(), Vec::new());
    for item in &plan.items {
        let top = destination(plan, item, &item.key);
        if same_bucket && top == item.key {
            return Err(format!("같은 자리에는 놓을 수 없습니다: {}", item.key));
        }
        if item.key.ends_with('/') {
            if same_bucket && top.starts_with(&item.key) {
                return Err(format!("폴더를 자기 안으로는 넣을 수 없습니다: {}", item.key));
            }
            let found = expand(client, &plan.bucket, std::slice::from_ref(item), run)?;
            pairs.extend(found.into_iter().map(|remote| { let to = destination(plan, item, &remote.key); (remote.key, remote.size, to) }));
            folders.push(top);
        } else {
            pairs.push((item.key.clone(), item.size, top.clone()));
            files.push(top);
        }
        run.event.total = pairs.len() as u64;
    }
    let keys: Vec<&String> = pairs.iter().map(|(_, _, to)| to).collect();
    let present = existing(client, &plan.to_bucket, &scopes_of(&plan.to_prefix, &folders, &files, &keys), run)?;
    let mut conflicts: Vec<String> = pairs.iter().filter(|(_, _, to)| present.contains(to) && !to.ends_with('/')).map(|(_, _, to)| to.clone()).collect();
    conflicts.sort();
    let overwrite = run.ask(&conflicts)?;
    run.event.bytes_total = pairs.iter().map(|(_, size, _)| size).sum();
    let mut moved: Vec<String> = Vec::new();
    let flush = |moved: &mut Vec<String>, run: &mut Run| -> Result<(), String> {
        // 옮기기: 복사를 마친 원본만 지운다.
        if moved.is_empty() {
            return Ok(());
        }
        let failed = client.delete_many(&plan.bucket, moved).map_err(|fail| format!("복사한 뒤 원본을 지우지 못했습니다: {}", fail.text))?;
        moved.clear();
        for (key, why) in &failed {
            run.event.failed_count += 1;
            if run.event.failed.len() < SHOWN {
                run.event.failed.push(format!("{key}: 복사는 했지만 원본을 지우지 못했습니다 ({why})"));
            }
        }
        Ok(())
    };
    for (from, size, to) in &pairs {
        if present.contains(to) && (!overwrite || to.ends_with('/')) {
            // 폴더 자리만 나타내는 빈 객체는 이미 있으면 그대로 둔다(옮기기면 원본은 지운다).
            if to.ends_with('/') {
                if plan.remove {
                    moved.push(from.clone());
                }
            } else {
                run.event.skipped += 1;
            }
            run.event.done += 1;
            run.event.bytes += size;
            continue;
        }
        if let Err(error) = run.step(from) {
            let _ = flush(&mut moved, run);
            return Err(error);
        }
        let copied = if *size > single_max {
            let job = run.job.clone();
            client.copy_in_parts(&plan.bucket, from, *size, &plan.to_bucket, to, part, &move || job.cancel.load(Ordering::SeqCst)).map_err(Fail::local)
        } else {
            client.copy(&plan.bucket, from, &plan.to_bucket, to)
        };
        if run.canceled() {
            let _ = flush(&mut moved, run);
            return Err(CANCELED.into());
        }
        match copied {
            Ok(()) => {
                run.ok();
                if plan.remove {
                    moved.push(from.clone());
                }
            }
            Err(fail) => {
                if let Err(stop) = run.fail(from, &fail.text) {
                    let _ = flush(&mut moved, run);
                    return Err(stop);
                }
            }
        }
        run.event.bytes += size;
        if moved.len() >= PAGE_MAX as usize {
            flush(&mut moved, run)?;
        }
    }
    flush(&mut moved, run)
}

/// 저장소 안에서 복사하거나 옮긴다(이름 바꾸기도 옮기기다: S3에는 이름을 바꾸는 요청이 없어 복사한 뒤 원본을 지운다).
#[tauri::command]
pub fn s3_copy(app: AppHandle, handle: u32, plan: CopyPlan) -> Result<u32, String> {
    let client = client(handle)?;
    let title = if plan.rename.is_some() { "이름 바꾸기" } else if plan.remove { "옮기기" } else { "복사" };
    Ok(start_job(handle, title, sink_of(app), move |run| copy(&client, &plan, run)))
}

#[cfg(test)]
#[path = "s3_fake.rs"]
mod fake;

#[cfg(test)]
mod tests {
    use super::fake::{Fake, ACCESS, SECRET};
    use super::*;


    fn client_for(fake: &Fake, signature: Signature) -> Arc<Client> {
        let endpoint = Endpoint::parse(&format!("http://127.0.0.1:{}", fake.port), true).unwrap();
        Arc::new(Client::for_test(endpoint, signature, ACCESS, SECRET))
    }

    /// 작업을 돌리고 알림을 모은다. 겹침을 물으면 `answer`로 답한다.
    fn run_job(answer: &'static str, work: impl FnOnce(&mut Run) -> Result<(), String> + Send + 'static) -> Vec<JobEvent> {
        let (sender, receiver) = mpsc::channel::<JobEvent>();
        let sink: Sink = Box::new(move |event| { let _ = sender.send(event.clone()); });
        let id = start_job(0, "시험", sink, work);
        let mut events = Vec::new();
        loop {
            let event = receiver.recv_timeout(Duration::from_secs(60)).expect("작업이 끝나지 않았다");
            if event.state == "conflict" {
                s3_job_answer(id, answer.to_string());
            }
            let last = matches!(event.state, "done" | "failed" | "canceled");
            events.push(event);
            if last {
                return events;
            }
        }
    }

    fn item(key: &str) -> Item {
        Item { key: key.into(), size: 0 }
    }

    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::current_exe().unwrap().parent().unwrap().join("test-files").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn 목록은_그_층만_한_쪽씩_읽는다() {
        let fake = Fake::start();
        for index in 0..7 {
            fake.put("logs", &format!("2024/파일 {index}.txt"), b"x");
        }
        fake.put("logs", "2024/", b"");
        fake.put("logs", "2024/sub/deep/a.bin", b"12345");
        fake.put("logs", "2024/sub/b.bin", b"1");
        fake.put("logs", "other.txt", b"1");
        let client = client_for(&fake, Signature::V4);
        assert_eq!(client.buckets().unwrap().iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), vec!["logs"]);

        fake.clear_log();
        let first = page_of(client.list("logs", "2024/", true, 5, None).unwrap(), "2024/");
        // 폴더 자신을 나타내는 빈 객체는 줄이 아니고, 아래 폴더는 한 줄로 묶인다.
        assert_eq!(first.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["sub", "파일 0.txt", "파일 1.txt", "파일 2.txt"]);
        assert!(first.entries[0].folder && first.entries[0].key == "2024/sub/");
        let second = page_of(client.list("logs", "2024/", true, 5, first.next.as_deref()).unwrap(), "2024/");
        assert_eq!(second.entries.len(), 4);
        assert_eq!(second.next, None);
        assert_eq!(fake.count("GET /logs/?"), 2, "목록 요청은 달라고 한 쪽만큼만 보낸다: {:?}", fake.log());

        let root = page_of(client.list("logs", "", true, 100, None).unwrap(), "");
        assert_eq!(root.entries.iter().map(|e| (e.name.as_str(), e.folder)).collect::<Vec<_>>(), vec![("2024", true), ("other.txt", false)]);
        assert!(client.list("nope", "", true, 10, None).unwrap_err().text.contains("그런 버킷이 없습니다"));
    }

    #[test]
    fn 서명이_틀리면_저장소가_거절한다() {
        let fake = Fake::start();
        fake.put("b", "a.txt", b"1");
        let mut wrong = Arc::try_unwrap(client_for(&fake, Signature::V4)).ok().unwrap();
        wrong.keys = vault::Keys { access: ACCESS.into(), secret: "틀린 비밀 키".into() };
        let fail = wrong.list("b", "", true, 10, None).unwrap_err();
        assert_eq!(fail.code, "SignatureDoesNotMatch");
        assert!(fail.text.contains("서명이 맞지 않습니다"));
    }

    #[test]
    fn 예전_모양으로_답하는_저장소의_목록도_끝까지_읽는다() {
        let fake = Fake::start();
        fake.set_v1_only(true);
        for index in 0..5 {
            fake.put("b", &format!("d/{index}"), b"x");
        }
        let client = client_for(&fake, Signature::V4);
        let mut keys = Vec::new();
        let mut next = None;
        loop {
            let page = client.list("b", "d/", true, 2, next.as_deref()).unwrap();
            keys.extend(page.objects.into_iter().map(|o| o.key));
            match page.next { Some(token) => next = Some(token), None => break }
        }
        assert_eq!(keys, vec!["d/0", "d/1", "d/2", "d/3", "d/4"]);
    }

    #[test]
    fn 글은_앞부분을_읽어_보여_주고_고쳐_쓸_때_종류와_메타데이터를_지킨다() {
        let fake = Fake::start();
        let client = client_for(&fake, Signature::V4);
        fake.put("b", "seed", b"");
        client.put_bytes("b", "메모/안녕.txt", "첫 줄\r\n둘째 줄\r\n".as_bytes(), "text/plain; charset=utf-8", &[("x-amz-meta-owner".into(), "kim".into())]).unwrap();
        let shown = preview(&client, "b", "메모/안녕.txt").unwrap();
        assert_eq!((shown.kind, shown.text.as_str(), shown.eol.as_str(), shown.truncated), ("text", "첫 줄\n둘째 줄\n", "CRLF", false));
        assert_eq!(shown.info.content_type, "text/plain; charset=utf-8");
        assert_eq!(shown.info.meta, vec![("x-amz-meta-owner".to_string(), "kim".to_string())]);
        assert_eq!(shown.info.size, 21);

        // 큰 글은 앞 1MB만 읽고, 글자 중간에서 끊지 않는다.
        let big = "가".repeat((TEXT_MAX as usize / 3) + 10);
        fake.put("b", "big.log", big.as_bytes());
        let shown = preview(&client, "b", "big.log").unwrap();
        assert!(shown.truncated && !shown.lossy && shown.kind == "text");
        assert_eq!(shown.text.chars().count(), TEXT_MAX as usize / 3);
        assert_eq!(shown.info.size, big.len() as u64);

        fake.put("b", "empty.txt", b"");
        let shown = preview(&client, "b", "empty.txt").unwrap();
        assert_eq!((shown.kind, shown.text.as_str(), shown.info.size), ("text", "", 0));
        fake.put("b", "data.bin", &[0, 1, 2, 0, 255]);
        assert_eq!(preview(&client, "b", "data.bin").unwrap().kind, "binary");
        fake.put("b", "dot.png", &[0x89, b'P', b'N', b'G']);
        let shown = preview(&client, "b", "dot.png").unwrap();
        assert_eq!((shown.kind, shown.image.as_str()), ("image", "data:image/png;base64,iVBORw=="));
        assert!(preview(&client, "b", "없는 것").unwrap_err().missing());
        assert!(client.exists("b", "data.bin").unwrap());
        assert!(!client.exists("b", "없는 것").unwrap());
    }

    #[test]
    fn 올리기는_폴더째_올리고_이미_있는_것은_묻는다() {
        let fake = Fake::start();
        let dir = test_dir("s3-upload");
        std::fs::create_dir_all(dir.join("proj/src/빈 폴더")).unwrap();
        std::fs::write(dir.join("proj/읽어 보기.md"), "# 안녕").unwrap();
        std::fs::write(dir.join("proj/src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.join("낱개.txt"), "새 내용").unwrap();
        fake.put("b", "up/낱개.txt", b"old");
        fake.put("b", "up/proj/src/main.rs", b"old");
        let paths = vec![dir.join("proj").to_string_lossy().into_owned(), dir.join("낱개.txt").to_string_lossy().into_owned()];

        // 건너뛰기: 이미 있는 둘은 그대로 두고 나머지만 올린다.
        let client = client_for(&fake, Signature::V4);
        let (c, p) = (client.clone(), paths.clone());
        let events = run_job("skip", move |run| upload(&c, "b", "up/", &p, run));
        let asked = events.iter().find(|e| e.state == "conflict").expect("겹침을 물어야 한다");
        assert_eq!(asked.conflicts, vec!["up/proj/src/main.rs", "up/낱개.txt"]);
        let last = events.last().unwrap();
        assert_eq!((last.state, last.skipped, last.failed_count, last.changed), ("done", 2, 0, true));
        assert_eq!(fake.get("b", "up/낱개.txt").unwrap(), b"old");
        assert_eq!(fake.get("b", "up/proj/읽어 보기.md").unwrap(), "# 안녕".as_bytes());
        assert_eq!(fake.get("b", "up/proj/src/빈 폴더/").unwrap(), b"");
        assert_eq!(fake.content_type("b", "up/proj/읽어 보기.md"), "text/markdown");

        // 덮어쓰기
        let (c, p) = (client.clone(), paths.clone());
        let events = run_job("overwrite", move |run| upload(&c, "b", "up/", &p, run));
        assert_eq!(events.last().unwrap().state, "done");
        assert_eq!(fake.get("b", "up/낱개.txt").unwrap(), "새 내용".as_bytes());
        assert_eq!(fake.get("b", "up/proj/src/main.rs").unwrap(), b"fn main() {}");

        // 취소: 아무것도 바꾸지 않는다.
        fake.put("b", "up/낱개.txt", b"old");
        let (c, p) = (client.clone(), paths.clone());
        let events = run_job("cancel", move |run| upload(&c, "b", "up/", &p, run));
        assert_eq!((events.last().unwrap().state, events.last().unwrap().changed), ("canceled", false));
        assert_eq!(fake.get("b", "up/낱개.txt").unwrap(), b"old");

        // 겹치는 것이 없으면 묻지 않는다. 예전 서명 방식으로도 올라간다.
        let v2 = client_for(&fake, Signature::V2);
        let p = paths.clone();
        let events = run_job("cancel", move |run| upload(&v2, "b", "새 자리/", &p, run));
        assert!(events.iter().all(|e| e.state != "conflict"));
        assert_eq!(events.last().unwrap().state, "done");
        assert_eq!(fake.get("b", "새 자리/낱개.txt").unwrap(), "새 내용".as_bytes());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 큰_파일은_조각으로_나눠_올린다() {
        let fake = Fake::start();
        let dir = test_dir("s3-multipart");
        let size = SINGLE_PUT_MAX as usize + (3 << 20) + 17;
        let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        std::fs::write(dir.join("big.dat"), &data).unwrap();
        let client = client_for(&fake, Signature::V4);
        let paths = vec![dir.join("big.dat").to_string_lossy().into_owned()];
        let events = run_job("cancel", move |run| upload(&client, "b", "", &paths, run));
        let last = events.last().unwrap();
        assert_eq!((last.state, last.failed_count, last.bytes), ("done", 0, size as u64), "{last:?}");
        assert!(fake.get("b", "big.dat").unwrap() == data, "받은 내용이 올린 것과 다르다");
        assert_eq!(fake.count("PUT /b/big.dat?partNumber="), size.div_ceil(PART_SIZE as usize));
        assert_eq!(fake.uploads(), 0, "끝난 나눠 올리기가 남아 있다");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 내려받기는_폴더_모양대로_펴고_있는_파일은_묻는다() {
        let fake = Fake::start();
        fake.put("b", "data/a.txt", b"A");
        fake.put("b", "data/sub/b:1.txt", b"B1");
        fake.put("b", "data/sub/b_1.txt", b"B2");
        fake.put("b", "data/sub/빈/", b"");
        fake.put("b", "data/CON.log", b"C");
        fake.put("b", "data/혼자.txt", b"solo");
        let dir = test_dir("s3-download");
        std::fs::write(dir.join("혼자.txt"), "있던 것").unwrap();
        let client = client_for(&fake, Signature::V4);
        let (c, d) = (client.clone(), dir.clone());
        let events = run_job("skip", move |run| download(&c, "b", "data/", &[item("data/sub/"), item("data/a.txt"), item("data/CON.log"), item("data/혼자.txt")], &d, run));
        let last = events.last().unwrap();
        assert_eq!((last.state, last.skipped, last.failed_count), ("done", 1, 0), "{last:?}");
        assert_eq!(std::fs::read(dir.join("a.txt")).unwrap(), b"A");
        assert_eq!(std::fs::read(dir.join("sub/b_1.txt")).unwrap(), b"B1");
        assert_eq!(std::fs::read(dir.join("sub/b_1 (2).txt")).unwrap(), b"B2");
        assert_eq!(std::fs::read(dir.join("_CON.log")).unwrap(), b"C");
        assert!(dir.join("sub/빈").is_dir());
        assert_eq!(std::fs::read_to_string(dir.join("혼자.txt")).unwrap(), "있던 것");
        // 받다 만 파일이 남지 않는다.
        assert!(!std::fs::read_dir(&dir).unwrap().flatten().any(|e| e.file_name().to_string_lossy().ends_with(".cypher-part")));

        let (c, d) = (client.clone(), dir.clone());
        let events = run_job("overwrite", move |run| download(&c, "b", "data/", &[item("data/혼자.txt")], &d, run));
        assert_eq!(events.last().unwrap().state, "done");
        assert_eq!(std::fs::read_to_string(dir.join("혼자.txt")).unwrap(), "solo");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 지우기는_폴더_아래까지_지우고_한_번에_지우기를_모르면_하나씩_지운다() {
        for single in [false, true] {
            let fake = Fake::start();
            fake.set_no_multi_delete(single);
            for index in 0..2500 {
                fake.put("b", &format!("big/{index:04}"), b"x");
            }
            fake.put("b", "big/", b"");
            fake.put("b", "bigger/keep", b"x");
            fake.put("b", "one.txt", b"x");
            fake.put("b", "two & <three>.txt", b"x");
            let client = client_for(&fake, Signature::V4);
            fake.clear_log();
            let c = client.clone();
            let events = run_job("cancel", move |run| delete(&c, "b", &[item("big/"), item("one.txt"), item("two & <three>.txt")], run));
            let last = events.last().unwrap();
            assert_eq!((last.state, last.done, last.failed_count), ("done", 2503, 0), "{last:?}");
            assert_eq!(fake.keys("b"), vec!["bigger/keep"]);
            if !single {
                // 목록 3쪽 + 한 번에 지우기 4번(낱개 묶음 1 + 쪽마다 1)
                assert_eq!(fake.count("GET /b/?"), 3, "{:?}", fake.log().iter().take(12).collect::<Vec<_>>());
                assert_eq!(fake.count("POST /b/?delete"), 4);
                assert_eq!(fake.count("DELETE "), 0);
            } else {
                assert_eq!(fake.count("DELETE /b/"), 2503);
            }
        }
    }

    #[test]
    fn 복사와_옮기기와_이름_바꾸기() {
        let fake = Fake::start();
        fake.put("b", "src/a.txt", b"A");
        fake.put("b", "src/dir/", b"");
        fake.put("b", "src/dir/b 한글.txt", b"B");
        fake.put("b", "src/dir/deep/c.txt", b"C");
        fake.put("b", "dst/a.txt", b"old");
        let client = client_for(&fake, Signature::V4);
        let plan = |items: &[&str], to: &str, remove: bool, rename: Option<&str>| CopyPlan {
            bucket: "b".into(), base: "src/".into(), items: items.iter().map(|key| item(key)).collect(),
            to_bucket: "b".into(), to_prefix: to.into(), remove, rename: rename.map(str::to_string),
        };

        // 복사: 이미 있는 것은 묻고, 건너뛰면 그대로 둔다.
        let (c, p) = (client.clone(), plan(&["src/a.txt", "src/dir/"], "dst/", false, None));
        let events = run_job("skip", move |run| copy(&c, &p, run));
        assert_eq!(events.iter().find(|e| e.state == "conflict").unwrap().conflicts, vec!["dst/a.txt"]);
        assert_eq!((events.last().unwrap().state, events.last().unwrap().skipped), ("done", 1));
        assert_eq!(fake.get("b", "dst/a.txt").unwrap(), b"old");
        assert_eq!(fake.get("b", "dst/dir/b 한글.txt").unwrap(), b"B");
        assert_eq!(fake.get("b", "dst/dir/deep/c.txt").unwrap(), b"C");
        assert_eq!(fake.get("b", "src/dir/deep/c.txt").unwrap(), b"C");

        // 이름 바꾸기(파일): 복사한 뒤 원본을 지운다.
        let (c, p) = (client.clone(), plan(&["src/a.txt"], "src/", true, Some("새 이름.txt")));
        assert_eq!(run_job("cancel", move |run| copy(&c, &p, run)).last().unwrap().state, "done");
        assert_eq!(fake.get("b", "src/새 이름.txt").unwrap(), b"A");
        assert!(fake.get("b", "src/a.txt").is_none());

        // 이름 바꾸기(폴더): 아래의 것이 모두 새 이름 아래로 간다.
        let (c, p) = (client.clone(), plan(&["src/dir/"], "src/", true, Some("renamed")));
        assert_eq!(run_job("cancel", move |run| copy(&c, &p, run)).last().unwrap().state, "done");
        assert_eq!(fake.keys("b").into_iter().filter(|k| k.starts_with("src/")).collect::<Vec<_>>(), vec!["src/renamed/", "src/renamed/b 한글.txt", "src/renamed/deep/c.txt", "src/새 이름.txt"]);

        // 옮기기에서 건너뛴 것은 원본도 남는다.
        fake.put("b", "dst/새 이름.txt", b"taken");
        let (c, p) = (client.clone(), plan(&["src/새 이름.txt", "src/renamed/"], "dst/", true, None));
        let events = run_job("skip", move |run| copy(&c, &p, run));
        assert_eq!((events.last().unwrap().state, events.last().unwrap().skipped), ("done", 1));
        assert_eq!(fake.get("b", "src/새 이름.txt").unwrap(), b"A");
        assert_eq!(fake.get("b", "dst/새 이름.txt").unwrap(), b"taken");
        assert!(fake.get("b", "src/renamed/deep/c.txt").is_none());
        assert_eq!(fake.get("b", "dst/renamed/deep/c.txt").unwrap(), b"C");

        // 하면 안 되는 것: 같은 자리, 자기 안으로, 이름에 `/`.
        for (bad, why) in [
            (plan(&["src/새 이름.txt"], "src/", false, None), "같은 자리"),
            (plan(&["dst/renamed/"], "dst/renamed/deep/", true, None), "자기 안"),
            (plan(&["src/새 이름.txt"], "src/", true, Some("a/b")), "`/`"),
        ] {
            let bad = CopyPlan { base: if bad.items[0].key.starts_with("dst/") { "dst/".into() } else { "src/".into() }, ..bad };
            let c = client.clone();
            let events = run_job("cancel", move |run| copy(&c, &bad, run));
            let last = events.last().unwrap();
            assert_eq!(last.state, "failed");
            assert!(last.error.contains(why), "{}", last.error);
            assert!(!last.changed);
        }
    }

    #[test]
    fn 구분자와_인코딩을_모르는_저장소에서도_폴더_모양으로_읽는다() {
        // (거절하는 저장소, 조용히 무시하는 저장소) 둘 다.
        for reject in [true, false] {
            let fake = Fake::start();
            fake.set_plain_list(if reject { "reject" } else { "ignore" });
            for index in 0..30 {
                fake.put("b", &format!("d/big/{index:02}"), b"x");
            }
            fake.put("b", "d/a b.txt", b"x");
            fake.put("b", "d/z.txt", b"x");
            fake.put("b", "d/zz/1", b"x");
            let client = client_for(&fake, Signature::V4);
            fake.clear_log();
            let mut names = Vec::new();
            let mut next = None;
            let mut pages = 0;
            loop {
                let page = page_of(client.list("b", "d/", true, 10, next.as_deref()).unwrap(), "d/");
                names.extend(page.entries.into_iter().map(|e| (e.name, e.folder)));
                pages += 1;
                match page.next { Some(token) => next = Some(token), None => break }
            }
            names.sort();
            assert_eq!(names, vec![("a b.txt".to_string(), false), ("big".to_string(), true), ("z.txt".to_string(), false), ("zz".to_string(), true)], "reject={reject}");
            // 폴더 안(30개)을 다 훑지 않고 건너뛴다: 두 쪽이면 끝난다.
            assert_eq!(pages, 2, "reject={reject}: {:?}", fake.log());
            // 거절당한 선택 사항은 한 번씩만 다시 묻는다(그 뒤로는 기억한다).
            assert_eq!(fake.count("GET /b/?"), if reject { 4 } else { 2 }, "{:?}", fake.log());
            // 아래를 다 읽는 일(폴더째 지우기)도 된다.
            let c = client.clone();
            let events = run_job("cancel", move |run| delete(&c, "b", &[item("d/big/")], run));
            assert_eq!((events.last().unwrap().state, events.last().unwrap().done), ("done", 30));
            assert_eq!(fake.keys("b"), vec!["d/a b.txt", "d/z.txt", "d/zz/1"]);
        }
    }

    #[test]
    fn 한_번에_복사할_수_없는_큰_객체는_조각으로_나눠_복사한다() {
        let fake = Fake::start();
        let data: Vec<u8> = (0..2500u32).map(|i| (i % 251) as u8).collect();
        fake.put_with("b", "src/큰 것.bin", &data, "application/x-thing", &[("x-amz-meta-owner", "kim")]);
        fake.put("b", "src/작은 것.txt", b"small");
        let client = client_for(&fake, Signature::V4);
        let plan = CopyPlan {
            bucket: "b".into(), base: "src/".into(), items: vec![Item { key: "src/큰 것.bin".into(), size: data.len() as u64 }, Item { key: "src/작은 것.txt".into(), size: 5 }],
            to_bucket: "b".into(), to_prefix: "dst/".into(), remove: true, rename: None,
        };
        fake.clear_log();
        let events = run_job("cancel", move |run| copy_with(&client, &plan, run, 1000, 1000));
        let last = events.last().unwrap();
        assert_eq!((last.state, last.failed_count), ("done", 0), "{last:?}");
        assert!(fake.get("b", "dst/큰 것.bin").unwrap() == data);
        assert_eq!(fake.content_type("b", "dst/큰 것.bin"), "application/x-thing");
        assert_eq!(fake.meta("b", "dst/큰 것.bin"), vec![("x-amz-meta-owner".to_string(), "kim".to_string())]);
        assert_eq!(fake.get("b", "dst/작은 것.txt").unwrap(), b"small");
        // 2,500바이트를 1,000바이트씩: 조각 셋. 작은 것은 한 번에.
        assert_eq!(fake.log().iter().filter(|line| line.contains("partNumber=")).count(), 3, "{:?}", fake.log());
        assert_eq!(fake.uploads(), 0);
        assert_eq!(fake.keys("b"), vec!["dst/작은 것.txt", "dst/큰 것.bin"]);
    }

    #[test]
    fn 줄여서_저장한_객체는_풀지_않고_저장소에_든_그대로_다룬다() {
        // 객체를 gzip으로 줄여 올려 둔 경우(Content-Encoding: gzip): 미리보기가 풀린 글의 앞부분을 "다 읽은 글"로 보여 주면,
        // 그것을 고쳐 저장할 때 뒷부분이 사라진다. 저장소에 든 바이트를 그대로 다루므로 글로 보이지 않고(고칠 수 없다),
        // 내려받은 파일도 저장소의 것과 같다.
        use std::io::Write;
        let fake = Fake::start();
        let plain = "한 줄의 기록입니다. 0123456789\n".repeat(60000);
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(plain.as_bytes()).unwrap();
        let packed = encoder.finish().unwrap();
        fake.put_with("b", "app.log", &packed, "text/plain", &[("content-encoding", "gzip")]);
        let client = client_for(&fake, Signature::V4);
        let shown = preview(&client, "b", "app.log").unwrap();
        assert_eq!(shown.kind, "binary");
        assert_eq!(shown.info.size, packed.len() as u64);
        assert!(shown.info.meta.contains(&("content-encoding".to_string(), "gzip".to_string())));
        let dir = test_dir("s3-gzip");
        let (c, d) = (client.clone(), dir.clone());
        let events = run_job("cancel", move |run| download(&c, "b", "", &[item("app.log")], &d, run));
        assert_eq!(events.last().unwrap().state, "done");
        assert!(std::fs::read(dir.join("app.log")).unwrap() == packed, "내려받은 파일이 저장소의 내용과 다르다");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 고쳐_쓰기는_그_사이_바뀐_객체를_덮어쓰지_않고_붙어_있던_헤더를_지킨다() {
        let fake = Fake::start();
        fake.put_with("b", "page.html", b"<p>old</p>", "text/html", &[("cache-control", "max-age=60"), ("x-amz-meta-note", "a  b")]);
        let client = client_for(&fake, Signature::V4);
        let shown = preview(&client, "b", "page.html").unwrap();
        let etag = shown.info.etag.clone();
        // 이어진 빈칸이 든 메타데이터도 서명이 맞는다(저장소는 빈칸을 하나로 줄여 받는다).
        let put = |text: &str, etag: &str| {
            let bytes = text.as_bytes().to_vec();
            if client.head("b", "page.html").unwrap().header("etag").unwrap_or("").trim_matches('"') != etag {
                return Err("바뀜".to_string());
            }
            client.put_bytes("b", "page.html", &bytes, &shown.info.content_type, &shown.info.meta).map_err(|fail| fail.text)
        };
        put("<p>new</p>", &etag).unwrap();
        assert_eq!(fake.get("b", "page.html").unwrap(), b"<p>new</p>");
        assert_eq!(fake.content_type("b", "page.html"), "text/html");
        assert_eq!(fake.meta("b", "page.html"), vec![("cache-control".to_string(), "max-age=60".to_string()), ("x-amz-meta-note".to_string(), "a b".to_string())]);
        // 그 사이에 바뀌었으면(확인 값이 다르다) 쓰지 않는다.
        assert!(put("<p>mine</p>", &etag).is_err());
        assert_eq!(fake.get("b", "page.html").unwrap(), b"<p>new</p>");
        // 명령으로도: 예전 확인 값으로는 거절된다.
        CLIENTS.lock().unwrap().insert(987_654, client.clone());
        let stale = tauri::async_runtime::block_on(s3_put_text(987_654, "b".into(), "page.html".into(), "x".into(), "UTF-8".into(), "LF".into(), "text/html".into(), vec![], Some(etag)));
        assert!(stale.unwrap_err().contains("바뀌었습니다"));
        // 폴더 만들기는 같은 이름이 있으면 덮어쓰지 않는다.
        fake.put_with("b", "dir/", b"marker", "application/x-directory", &[("x-amz-meta-owner", "kim")]);
        assert!(tauri::async_runtime::block_on(s3_mkdir(987_654, "b".into(), "dir/".into())).unwrap_err().contains("이미 있습니다"));
        assert_eq!(fake.get("b", "dir/").unwrap(), b"marker");
        assert!(tauri::async_runtime::block_on(s3_mkdir(987_654, "b".into(), "new/".into())).is_ok());
        CLIENTS.lock().unwrap().remove(&987_654);
    }

    #[test]
    fn 복사를_모르는_저장소의_빈_답을_믿고_원본을_지우지_않는다() {
        let fake = Fake::start();
        fake.put("b", "src/a.txt", b"A");
        fake.set_blind_copy(true);
        let client = client_for(&fake, Signature::V4);
        let plan = CopyPlan { bucket: "b".into(), base: "src/".into(), items: vec![item("src/a.txt")], to_bucket: "b".into(), to_prefix: "dst/".into(), remove: true, rename: None };
        let events = run_job("cancel", move |run| copy(&client, &plan, run));
        let last = events.last().unwrap();
        assert_eq!(last.failed_count, 1, "{last:?}");
        assert_eq!(fake.get("b", "src/a.txt").unwrap(), b"A", "복사가 되지 않았는데 원본을 지웠다");
    }

    #[test]
    fn 스스로_이어_읽는_목록은_되풀이와_엉뚱한_키에서_멈춘다() {
        // 같은 쪽을 되풀이해 주는 저장소
        let fake = Fake::start();
        for index in 0..5 {
            fake.put("b", &format!("d/{index}"), b"x");
        }
        fake.put("b", "other", b"x");
        fake.set_broken_list("repeat");
        let client = client_for(&fake, Signature::V4);
        fake.clear_log();
        let c = client.clone();
        // 지우면서 읽는 일은 첫 쪽이 계속 달라져 끝이 나므로, 읽기만 하는 일(내려받기)로 본다.
        let dir = test_dir("s3-stuck");
        let d = dir.clone();
        let events = run_job("cancel", move |run| download(&c, "b", "d/", &[item("d/")], &d, run));
        let _ = std::fs::remove_dir_all(&dir);
        let last = events.last().unwrap();
        assert_eq!(last.state, "failed");
        assert!(last.error.contains("되풀이"), "{}", last.error);
        assert!(fake.count("GET /b/?") <= 3, "{:?}", fake.log());
        // 앞머리를 못 알아듣는 저장소: 폴더 하나를 지우려다 다른 것을 지우지 않는다.
        let fake = Fake::start();
        fake.put("b", "d/1", b"x");
        fake.put("b", "other", b"x");
        fake.set_broken_list("no-prefix");
        let client = client_for(&fake, Signature::V4);
        let events = run_job("cancel", move |run| delete(&client, "b", &[item("d/")], run));
        assert_eq!(events.last().unwrap().state, "failed");
        assert_eq!(fake.keys("b"), vec!["d/1", "other"]);
    }

    #[test]
    fn 놓일_자리에_객체가_훨씬_많으면_목록을_다_읽지_않고_하나씩_묻는다() {
        let fake = Fake::start();
        for index in 0..3000 {
            fake.put("b", &format!("big/{index:04}"), b"x");
        }
        let dir = test_dir("s3-many");
        let mut paths = Vec::new();
        for index in 0..60 {
            let path = dir.join(format!("new-{index:02}.txt"));
            std::fs::write(&path, "x").unwrap();
            paths.push(path.to_string_lossy().into_owned());
        }
        fake.put("b", "big/new-07.txt", b"old");
        let client = client_for(&fake, Signature::V4);
        fake.clear_log();
        let events = run_job("skip", move |run| upload(&client, "b", "big/", &paths, run));
        assert_eq!(events.iter().find(|e| e.state == "conflict").unwrap().conflicts, vec!["big/new-07.txt"]);
        assert_eq!((events.last().unwrap().state, events.last().unwrap().skipped), ("done", 1));
        // 60개를 올리려고 3,000개짜리 폴더를 다 읽지 않는다: 목록은 값어치만큼(3쪽)만 읽고 나머지는 하나씩 묻는다.
        assert_eq!(fake.count("GET /b/?"), 3, "{:?}", fake.log().iter().filter(|l| l.starts_with("GET")).take(8).collect::<Vec<_>>());
        assert!(fake.count("HEAD /b/big/") <= 60);
        assert_eq!(fake.get("b", "big/new-07.txt").unwrap(), b"old");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 연달아_실패하면_작업을_멈춘다() {
        let fake = Fake::start();
        for index in 0..10 {
            fake.put("b", &format!("d/{index}"), b"x");
        }
        fake.set_deny("PUT");
        let client = client_for(&fake, Signature::V4);
        // 지우기는 묶음에서 하나도 지우지 못한 때만 연달아 실패로 센다: 낱개 셋이 거절돼도(가짜 저장소에서는 일어나지
        // 않지만 세는 법은 같다) 멈추지 않고, 여기서는 복사가 셋 연달아 거절돼 멈춘다.
        let plan = CopyPlan { bucket: "b".into(), base: "d/".into(), items: vec![item("d/")], to_bucket: "b".into(), to_prefix: "e/".into(), remove: true, rename: None };
        fake.clear_log();
        let events = run_job("cancel", move |run| copy(&client, &plan, run));
        let last = events.last().unwrap();
        assert_eq!(last.state, "failed");
        assert!(last.error.contains("연달아 실패"), "{}", last.error);
        assert_eq!(fake.count("PUT /b/e/"), FAIL_STREAK as usize);
        // 복사하지 못했으니 원본은 하나도 지우지 않는다.
        assert_eq!(fake.keys("b").len(), 10);
    }

    #[test]
    fn 요청_이력은_최근_100건을_키와_서명을_가려_남긴다() {
        let fake = Fake::start();
        fake.put("b", "a.xml", b"<old/>");
        fake.put("b", "one", b"1");
        fake.put("b", "two", b"2");
        let client = client_for(&fake, Signature::V4);
        client.list("b", "", true, 10, None).unwrap();
        assert!(client.head("b", "없는 것").unwrap_err().missing());
        assert!(client.list("nope", "", true, 10, None).is_err());
        client.put_bytes("b", "a.xml", "<비밀 내용/>".as_bytes(), "application/xml", &[("x-amz-meta-owner".into(), "kim".into())]).unwrap();
        assert!(client.delete_many("b", &["one".to_string(), "two".to_string()]).unwrap().is_empty());

        let history = client.history.lock().unwrap().1.iter().rev().cloned().collect::<Vec<_>>();
        // 새것부터: 지우기, 쓰기, 없는 버킷의 목록, 없는 객체, 목록.
        assert_eq!(history.iter().map(|e| (e.seq, e.method.as_str(), e.status)).collect::<Vec<_>>(), vec![(5, "POST", 200), (4, "PUT", 200), (3, "GET", 404), (2, "HEAD", 404), (1, "GET", 200)]);
        let all = format!("{history:?}");
        assert!(!all.contains(SECRET) && !all.contains(ACCESS), "키가 이력에 남았다");
        for entry in &history {
            let authorization = &entry.request_headers.iter().find(|(name, _)| name == "authorization").expect("Authorization 줄은 남는다").1;
            assert!(authorization.ends_with("Signature=****") && authorization.contains(&format!("Credential={}****/", &ACCESS[..4])), "{authorization}");
            assert!(entry.at > 0 && entry.url.starts_with("http://127.0.0.1:"));
        }
        // 저장소에 시킨 XML과 실패한 답은 남고, 객체의 내용은 남지 않는다.
        assert!(history[0].request_body.contains("<Key>one</Key>") && history[0].url.ends_with("/b/?delete="), "{:?}", history[0]);
        assert_eq!((history[1].sent, history[1].request_body.as_str()), ("<비밀 내용/>".len() as u64, ""));
        assert!(history[1].request_headers.contains(&("x-amz-meta-owner".to_string(), "kim".to_string())));
        assert!(history[2].response_body.contains("NoSuchBucket"), "{:?}", history[2]);
        assert!(!all.contains("비밀 내용"));
        assert!(history[4].response_headers.iter().any(|(name, _)| name == "content-type") && history[4].response_body.is_empty());

        // 닿지 못한 요청도 남는다.
        let gone = Client::for_test(Endpoint::parse("http://127.0.0.1:1", true).unwrap(), Signature::V2, ACCESS, SECRET);
        assert!(gone.buckets().is_err());
        let entry = gone.history.lock().unwrap().1[0].clone();
        assert!(entry.status == 0 && entry.problem.contains("닿지 못했습니다"), "{entry:?}");
        assert_eq!(entry.request_headers.iter().find(|(name, _)| name == "authorization").unwrap().1, format!("AWS {}****:****", &ACCESS[..4]));

        // 100건을 넘으면 오래된 것부터 버린다.
        for _ in 0..HISTORY_MAX {
            client.exists("b", "a.xml").unwrap();
        }
        let history = client.history.lock().unwrap();
        assert_eq!((history.1.len(), history.1[0].seq, history.1[HISTORY_MAX - 1].seq), (HISTORY_MAX, 6, 105));
        drop(history);

        // 화면이 부르는 명령: 새것부터 돌려주고, 지우면 비며, 그 뒤의 요청은 번호를 이어 센다.
        let handle = NEXT.fetch_add(1, Ordering::Relaxed);
        CLIENTS.lock().unwrap().insert(handle, client.clone());
        let listed = s3_history(handle).unwrap();
        assert_eq!((listed.len(), listed[0].seq, listed[HISTORY_MAX - 1].seq), (HISTORY_MAX, 105, 6));
        s3_history_clear(handle).unwrap();
        assert!(s3_history(handle).unwrap().is_empty());
        client.exists("b", "a.xml").unwrap();
        assert_eq!(s3_history(handle).unwrap().iter().map(|e| e.seq).collect::<Vec<_>>(), vec![106]);
        s3_disconnect(handle);
        assert!(s3_history(handle).is_err() && s3_history_clear(handle).is_err());
        assert_eq!(clip(&"가".repeat(HISTORY_BODY)).lines().next().unwrap().chars().count(), HISTORY_BODY / 3);
    }

    #[test]
    fn 이_pc의_파일_이름으로_쓸_수_없는_키를_다듬는다() {
        assert_eq!(safe_name("a:b*c?.txt"), "a_b_c_.txt");
        assert_eq!(safe_name("끝에 점. . "), "끝에 점");
        assert_eq!(safe_name("nul"), "_nul");
        assert_eq!(safe_name("COM1.txt"), "_COM1.txt");
        assert_eq!(safe_name("COMMON"), "COMMON");
        assert_eq!(safe_name("con .txt"), "_con .txt");
        assert_eq!(safe_name("CONIN$"), "_CONIN$");
        assert_eq!(safe_name("LPT¹.log"), "_LPT¹.log");
        assert_eq!(safe_name("COM10"), "COM10");
        assert_eq!(safe_name(".."), "_");
        let targets = plan_download(vec![Remote { key: "p/../x".into(), size: 1 }, Remote { key: "p/a/".into(), size: 0 }, Remote { key: "p/".into(), size: 0 }], "p/", Path::new(r"C:\dest"));
        assert_eq!(targets.iter().map(|t| (t.path.to_string_lossy().into_owned(), t.folder)).collect::<Vec<_>>(), vec![(r"C:\dest\_\x".to_string(), false), (r"C:\dest\a".to_string(), true)]);
        assert_eq!(content_type_of("a/b.JSON"), "application/json");
        assert_eq!(content_type_of("이름없음"), "application/octet-stream");
        assert_eq!(trim_partial_utf8(&"가나".as_bytes()[..5]), "가".as_bytes());
    }

    /// 실제 저장소로 기본 동작을 한 번씩 확인한다(평소에는 돌지 않는다). 요금이 드는 저장소일 수 있어 요청은 쉰 번 안팎,
    /// 올리는 양은 5MB 남짓, 받는 양은 몇 KB로 묶었다.
    ///
    /// 환경 변수 `CYPHER_S3_LIVE_ENDPOINT`(저장소의 전체 주소)·`CYPHER_S3_LIVE_ACCESS`·`CYPHER_S3_LIVE_SECRET`·
    /// `CYPHER_S3_LIVE_BUCKET`을 주고 `cargo test -p cypher -- --ignored 실제_저장소 --nocapture`로 돌린다
    /// (`CYPHER_S3_LIVE_VHOST=1`이면 경로 방식 대신 가상 호스트 방식 주소로). 키는 어디에도 적지 않는다.
    /// 버킷 안에 `cypher-live-…/` 폴더를 만들어 그 안에서만 쓰고, 끝나면(중간에 실패해도) 그 폴더를 지운다.
    /// 버킷에 원래 있던 것은 고치지 않는다(맨 위 층을 다섯 줄 읽어 수만 센다).
    #[test]
    #[ignore]
    fn 실제_저장소로_기본_동작을_확인한다() {
        fn note(bad: &mut Vec<String>, name: &str, result: Result<String, String>) {
            match result {
                Ok(text) => eprintln!("[통과] {name}: {text}"),
                Err(why) => {
                    eprintln!("[실패] {name}: {why}");
                    bad.push(name.to_string());
                }
            }
        }
        fn finished(events: &[JobEvent]) -> Result<String, String> {
            let last = events.last().unwrap();
            if last.state == "done" && last.failed_count == 0 {
                Ok(format!("{}개, {}바이트", last.done, last.bytes))
            } else {
                Err(format!("{} {} {:?}", last.state, last.error, last.failed))
            }
        }
        let env = |name: &str| std::env::var(format!("CYPHER_S3_LIVE_{name}")).unwrap_or_else(|_| panic!("환경 변수 CYPHER_S3_LIVE_{name}이 없다"));
        let (endpoint, access, secret, bucket) = (env("ENDPOINT"), env("ACCESS"), env("SECRET"), env("BUCKET"));
        let path_style = std::env::var("CYPHER_S3_LIVE_VHOST").is_err();
        let dir = test_dir("s3-live");
        // 접속 정보를 앱이 하는 그대로 저장했다가(키 암호화) 풀어 접속한다. 저장한 파일은 바로 지운다.
        let connect = |secret: &str| -> Client {
            let path = dir.join("connections.json");
            let input: vault::ConnInput =
                serde_json::from_value(serde_json::json!({ "name": "실제", "endpoint": endpoint, "path_style": path_style, "access": access, "secret": secret })).unwrap();
            let saved = vault::save_at(&path, &input).unwrap();
            let opened = vault::open_at(&path, &saved.id, None).unwrap();
            std::fs::remove_file(&path).unwrap();
            Client::new(opened).unwrap()
        };
        let client = Arc::new(connect(&secret));
        let stamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
        let root = format!("cypher-live-{stamp}/");
        eprintln!("주소 {endpoint} · 지역 {} · 버킷 {bucket} · 쓰는 폴더 {root}", client.region);
        let mut bad: Vec<String> = Vec::new();
        let text = |fail: Fail| format!("{} {} {}", fail.status, fail.code, fail.text);

        // 틀린 Secret key는 서명 오류로 거절된다.
        let wrong = connect("틀린-Secret-key");
        note(&mut bad, "틀린 Secret key", match wrong.list(&bucket, &root, true, 1, None) {
            Ok(_) => Err("거절되지 않았다".into()),
            Err(fail) if fail.status == 403 => Ok(format!("{} {} — {}", fail.status, fail.code, fail.text)),
            Err(fail) => Err(text(fail)),
        });
        note(&mut bad, "버킷 목록", client.buckets().map_err(text).map(|all| format!("{}개, 이 버킷이 보이는지: {}", all.len(), all.iter().any(|b| b.name == bucket))));
        note(&mut bad, "맨 위 층 목록", client.list(&bucket, "", true, 5, None).map_err(text).map(|page| {
            format!(
                "폴더 {} · 객체 {} · 더 있음 {} · 키를 %XX로 받기 {} · 구분자 {}",
                page.prefixes.len(), page.objects.len(), page.next.is_some(),
                !client.plain_keys.load(Ordering::Relaxed), !client.no_delimiter.load(Ordering::Relaxed)
            )
        }));

        let steps = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let bad = &mut bad;
            let local = dir.join("올릴 것");
            std::fs::create_dir_all(local.join("폴더/하위")).unwrap();
            std::fs::create_dir_all(local.join("폴더/빈 폴더")).unwrap();
            std::fs::write(local.join("폴더/a.txt"), "A").unwrap();
            std::fs::write(local.join("폴더/하위/b 1.txt"), "B1").unwrap();
            std::fs::write(local.join("가.txt"), "가나다").unwrap();
            std::fs::write(local.join("메모 +&=#.txt"), "첫 줄\r\n둘째 줄\r\n").unwrap();
            let paths: Vec<String> = ["폴더", "가.txt", "메모 +&=#.txt"].iter().map(|name| local.join(name).to_string_lossy().into_owned()).collect();
            let (c, b, r) = (client.clone(), bucket.clone(), root.clone());
            note(bad, "올리기(폴더째)", finished(&run_job("cancel", move |run| upload(&c, &b, &r, &paths, run))));

            note(bad, "목록(두 줄씩 이어 읽기)", (|| {
                let mut names = Vec::new();
                let mut tokens = Vec::new();
                let mut next: Option<String> = None;
                for _ in 0..5 {
                    let page = page_of(client.list(&bucket, &root, true, 2, next.as_deref()).map_err(text)?, &root);
                    names.extend(page.entries.into_iter().map(|entry| (entry.name, entry.folder)));
                    match page.next {
                        Some(token) => {
                            tokens.push(token.chars().take(2).collect::<String>());
                            next = Some(token);
                        }
                        None => break,
                    }
                }
                names.sort();
                let wanted = vec![("가.txt".to_string(), false), ("메모 +&=#.txt".to_string(), false), ("폴더".to_string(), true)];
                if names == wanted { Ok(format!("이어 읽기 표의 종류 {tokens:?}")) } else { Err(format!("{names:?}")) }
            })());

            let memo = format!("{root}메모 +&=#.txt");
            note(bad, "글 미리보기(범위 읽기)", preview(&client, &bucket, &memo).map_err(text).and_then(|shown| {
                let line = format!("{} {:?} {} 크기 {} 종류 {:?} etag {:?}", shown.kind, shown.text, shown.eol, shown.info.size, shown.info.content_type, shown.info.etag);
                if (shown.kind, shown.text.as_str(), shown.eol.as_str(), shown.info.size) == ("text", "첫 줄\n둘째 줄\n", "CRLF", 21) { Ok(line) } else { Err(line) }
            }));
            note(bad, "고쳐 쓰기(종류·메타데이터)", (|| {
                client.put_bytes(&bucket, &memo, "고친 글\n".as_bytes(), "text/plain; charset=utf-8", &[("x-amz-meta-owner".into(), "cypher".into())]).map_err(text)?;
                let info = info_of(&memo, &client.head(&bucket, &memo).map_err(text)?);
                let line = format!("크기 {} 종류 {:?} 메타 {:?}", info.size, info.content_type, info.meta);
                if info.size == 11 && info.content_type.starts_with("text/plain") && info.meta.contains(&("x-amz-meta-owner".to_string(), "cypher".to_string())) { Ok(line) } else { Err(line) }
            })());
            note(bad, "빈 객체 미리보기", preview(&client, &bucket, &format!("{root}폴더/빈 폴더/")).map_err(text).and_then(|shown| {
                let line = format!("{} 크기 {} 종류 {:?}", shown.kind, shown.info.size, shown.info.content_type);
                if shown.kind == "text" && shown.info.size == 0 { Ok(line) } else { Err(line) }
            }));

            let plan = |key: &str, to: &str, remove: bool, rename: Option<&str>| CopyPlan {
                bucket: bucket.clone(), base: root.clone(), items: vec![item(key)], to_bucket: bucket.clone(), to_prefix: to.into(), remove, rename: rename.map(str::to_string),
            };
            let (c, p) = (client.clone(), plan(&format!("{root}폴더/"), &format!("{root}복사/"), false, None));
            note(bad, "폴더 복사", finished(&run_job("cancel", move |run| copy(&c, &p, run))).and_then(|line| {
                if client.exists(&bucket, &format!("{root}복사/폴더/하위/b 1.txt")).map_err(text)? { Ok(line) } else { Err("복사한 것이 없다".into()) }
            }));
            let (c, p) = (client.clone(), plan(&format!("{root}가.txt"), &root, true, Some("나 +.txt")));
            note(bad, "이름 바꾸기", finished(&run_job("cancel", move |run| copy(&c, &p, run))).and_then(|line| {
                let (new, old) = (client.exists(&bucket, &format!("{root}나 +.txt")).map_err(text)?, client.exists(&bucket, &format!("{root}가.txt")).map_err(text)?);
                if new && !old { Ok(line) } else { Err(format!("새 이름 {new}, 옛 이름 {old}")) }
            }));

            // 나눠 올리기: 5MB 조각 하나와 1,000바이트 조각 하나(저장소가 받는 가장 작은 모양).
            let big = format!("{root}큰 파일.bin");
            note(bad, "나눠 올리기", (|| {
                let data: Vec<u8> = (0..(5usize << 20) + 1000).map(|i| (i % 251) as u8).collect();
                let upload = client.begin_multipart(&bucket, &big, vec![("content-type".into(), "application/octet-stream".into())])?;
                let sent = (|| {
                    let mut parts = Vec::new();
                    for (index, chunk) in data.chunks(5 << 20).enumerate() {
                        let mut request = client.object("PUT", &bucket, &big);
                        request.query = Client::part_query(&upload, Some(index as u32 + 1));
                        request.payload_sha256 = client.hash(chunk);
                        let response = client.send(&request, Body::Bytes(chunk)).map_err(text)?;
                        parts.push((index as u32 + 1, response.header("etag").unwrap_or("").to_string()));
                    }
                    client.complete_multipart(&bucket, &big, &upload, &parts)
                })();
                sent.map_err(|why| client.abort_multipart(&bucket, &big, &upload, why))?;
                // 뒤 조각이 제자리에 붙었는지: 끝 1,000바이트만 읽어 견준다.
                let mut request = client.object("GET", &bucket, &big);
                request.headers.push(("range".into(), format!("bytes={}-", 5usize << 20)));
                let response = client.send(&request, Body::None).map_err(text)?;
                let info = info_of(&big, &response);
                let mut tail = Vec::new();
                response.into_reader().take(4096).read_to_end(&mut tail).map_err(|e| e.to_string())?;
                let line = format!("크기 {} etag {:?}", info.size, info.etag);
                if info.size == data.len() as u64 && tail == data[5 << 20..] { Ok(line) } else { Err(format!("{line}, 끝 {}바이트", tail.len())) }
            })());

            let dest = dir.join("받은 것");
            std::fs::create_dir_all(&dest).unwrap();
            let (c, b, r, d) = (client.clone(), bucket.clone(), root.clone(), dest.clone());
            let events = run_job("cancel", move |run| download(&c, &b, &r, &[item(&format!("{r}복사/")), item(&format!("{r}나 +.txt"))], &d, run));
            note(bad, "내려받기(폴더째)", finished(&events).and_then(|line| {
                let read = |name: &str| std::fs::read_to_string(dest.join(name)).unwrap_or_else(|e| format!("<{e}>"));
                let got = (read("복사/폴더/a.txt"), read("복사/폴더/하위/b 1.txt"), read("나 +.txt"), dest.join("복사/폴더/빈 폴더").is_dir());
                if got == ("A".to_string(), "B1".to_string(), "가나다".to_string(), true) { Ok(line) } else { Err(format!("{got:?}")) }
            }));
            note(bad, "없는 객체", client.exists(&bucket, &format!("{root}없는 것")).map_err(text).and_then(|found| if found { Err("있다고 답했다".into()) } else { Ok("없다고 답했다".into()) }));
        }));

        // 치우기: 중간에 실패했어도 만든 폴더는 지운다.
        let (c, b, r) = (client.clone(), bucket.clone(), root.clone());
        let events = run_job("cancel", move |run| delete(&c, &b, &[item(&r)], run));
        note(&mut bad, "폴더째 지우기", finished(&events).map(|line| format!("{line} · 한 번에 지우기 {}", !client.single_delete.load(Ordering::Relaxed))));
        let left = client.list(&bucket, &root, false, 1000, None).map_err(text).map(|page| page.objects.into_iter().map(|object| object.key).collect::<Vec<_>>());
        note(&mut bad, "지운 뒤 남은 것", match left {
            Ok(keys) if keys.is_empty() => Ok("없음".into()),
            Ok(keys) => {
                // 묶어 지우기가 남긴 것이 있으면 하나씩 지워 본다(남으면 요금이 든다).
                let stuck: Vec<String> = keys.iter().filter(|key| client.delete_one(&bucket, key).is_err()).cloned().collect();
                Err(format!("{}개가 남아 있었다. 하나씩 지운 뒤에도 남은 것: {stuck:?}", keys.len()))
            }
            Err(why) => Err(why),
        });
        // 버전 관리가 켜진 버킷이면 지운 것도 옛 판으로 남아 자리를 차지한다: 이 시험이 만든 것의 판까지 지운다.
        note(&mut bad, "남은 옛 판 치우기", sweep_versions(&client, &bucket, &root));
        let _ = std::fs::remove_dir_all(&dir);
        if let Err(panic) = steps {
            std::panic::resume_unwind(panic);
        }
        assert!(bad.is_empty(), "실패한 것: {bad:?}");
    }

    /// `prefix`로 시작하는 키의 판(버전)과 지움 표시를 모두 지운다(시험이 만든 것을 치울 때만 쓴다).
    /// 버전 관리를 모르는 저장소(판 목록을 거절한다)에서는 아무것도 하지 않는다.
    fn sweep_versions(client: &Client, bucket: &str, prefix: &str) -> Result<String, String> {
        assert!(prefix.starts_with("cypher-live-"), "시험이 만든 것만 치운다");
        let request = Request {
            method: "GET",
            bucket: Some(bucket.to_string()),
            query: vec![("prefix".into(), prefix.to_string()), ("versions".into(), String::new())],
            ..Request::default()
        };
        let body = match client.send(&request, Body::None) {
            Ok(response) => text_of(response).map_err(|fail| fail.text)?,
            Err(fail) if matches!(fail.status, 400 | 404 | 405 | 501) => return Ok(format!("판 목록을 모르는 저장소({} {})", fail.status, fail.code)),
            Err(fail) => return Err(fail.text),
        };
        // 판 목록의 뿌리 이름은 저장소마다 다르다(AWS는 `ListVersionsResult`, Ncloud Storage는 `ListBucketResult`).
        let root = api::expect_root(&body, "ListVersionsResult").or_else(|_| api::expect_root(&body, "ListBucketResult"))?;
        let found: Vec<(String, String)> = root
            .children
            .iter()
            .filter(|node| matches!(node.name.as_str(), "Version" | "DeleteMarker"))
            .map(|node| (node.value("Key").to_string(), node.value("VersionId").to_string()))
            .filter(|(key, id)| key.starts_with(prefix) && !id.is_empty())
            .collect();
        let mut stuck = Vec::new();
        for (key, id) in &found {
            let mut request = client.object("DELETE", bucket, key);
            request.query.push(("versionId".into(), id.clone()));
            if let Err(fail) = client.send(&request, Body::None) {
                stuck.push(format!("{key} ({})", fail.text));
            }
        }
        if stuck.is_empty() { Ok(format!("판 {}개를 지웠다(더 있음: {})", found.len(), root.value("IsTruncated"))) } else { Err(format!("지우지 못한 판: {stuck:?}")) }
    }

    /// 앞선 실제 저장소 시험이 남긴 옛 판을 치운다(버전 관리가 켜진 버킷에서만 남는다. 시험이 도중에 죽었을 때 쓴다).
    /// 환경 변수는 위 시험과 같고, 서명에 넣을 지역을 `CYPHER_S3_LIVE_REGION`으로 더 준다.
    #[test]
    #[ignore]
    fn 실제_저장소에_남은_시험의_옛_판을_치운다() {
        let env = |name: &str| std::env::var(format!("CYPHER_S3_LIVE_{name}")).unwrap_or_else(|_| panic!("환경 변수 CYPHER_S3_LIVE_{name}이 없다"));
        let endpoint = Endpoint::parse(&env("ENDPOINT"), std::env::var("CYPHER_S3_LIVE_VHOST").is_err()).unwrap();
        let info = vault::ConnInfo { id: String::new(), name: String::new(), endpoint: env("ENDPOINT"), region: env("REGION"), path_style: true, signature: "v4".into(), insecure: false, locked: false };
        let opened = vault::Opened { info, endpoint, signature: Signature::V4, keys: vault::Keys { access: env("ACCESS"), secret: env("SECRET") } };
        let client = Client::new(opened).unwrap();
        eprintln!("{}", sweep_versions(&client, &env("BUCKET"), "cypher-live-").unwrap());
    }
}
