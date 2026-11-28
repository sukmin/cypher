//! 시험용 가짜 S3 저장소: 이 컴퓨터의 포트 하나에서 S3 요청을 받아 메모리에 든 객체로 답한다.
//!
//! 시험에서만 쓴다(`s3.rs`의 시험과, 화면 시험용으로 따로 띄우는 서버가 함께 쓴다). 받은 요청의 서명을 실제 저장소처럼
//! 다시 계산해 견주므로(SigV4·V2), 서명에 넣은 주소·헤더와 실제로 보낸 것이 한 글자라도 다르면 거절한다.
//! 받은 요청은 `"GET /버킷/?prefix=…"` 모양으로 적어 두어, 시험이 요청 수를 셀 수 있다.

use cypher_core::s3 as api;
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex, MutexGuard};

pub const ACCESS: &str = "AKIDFAKE0001";
pub const SECRET: &str = "fake/Secret+Key==";

#[derive(Clone, Default)]
struct Stored {
    data: Vec<u8>,
    content_type: String,
    meta: Vec<(String, String)>,
}

#[derive(Default)]
struct State {
    buckets: BTreeMap<String, BTreeMap<String, Stored>>,
    /// 나눠 올리는 중인 것: 번호 → (버킷, 키, 종류, 조각들)
    uploads: HashMap<String, (String, String, String, BTreeMap<u32, Vec<u8>>)>,
    /// 나눠 올리기를 시작할 때 붙인 사용자 메타데이터
    upload_meta: HashMap<String, Vec<(String, String)>>,
    log: Vec<String>,
    v1_only: bool,
    no_multi_delete: bool,
    /// 목록의 `delimiter`·`encoding-type`을 모르는 저장소 흉내: `reject`(거절) | `ignore`(못 본 척)
    plain_list: String,
    /// 고장 난 목록 흉내: `repeat`(이어 읽기를 무시하고 늘 첫 쪽을 준다) | `no-prefix`(앞머리를 무시한다)
    broken_list: String,
    /// 복사를 모르는 저장소 흉내: 복사 요청을 본문 없는 쓰기로 받아 빈 객체를 만들고 빈 답을 준다.
    blind_copy: bool,
    deny: Option<String>,
    counter: u32,
}

pub struct Fake {
    pub port: u16,
    state: Arc<Mutex<State>>,
}

struct Incoming {
    method: String,
    /// 주소의 경로와 `?` 뒤(받은 글자 그대로)
    path: String,
    query: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Incoming {
    fn header(&self, name: &str) -> &str {
        self.headers.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str()).unwrap_or("")
    }

    /// `?` 뒤의 값들(%XX를 푼 것).
    fn params(&self) -> HashMap<String, String> {
        self.query.split('&').filter(|pair| !pair.is_empty()).map(|pair| { let (name, value) = pair.split_once('=').unwrap_or((pair, "")); (decode(name), decode(value)) }).collect()
    }
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn xml(status: u16, body: String) -> Reply {
    Reply { status, headers: vec![("Content-Type".into(), "application/xml".into())], body: format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{body}").into_bytes() }
}

fn error(status: u16, code: &str, message: &str) -> Reply {
    xml(status, format!("<Error><Code>{code}</Code><Message>{}</Message><RequestId>fake</RequestId></Error>", api::xml_escape(message)))
}

fn empty(status: u16) -> Reply {
    Reply { status, headers: Vec::new(), body: Vec::new() }
}

/// 주소의 %XX를 푼다(`+`는 그대로 둔다).
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' && at + 2 < bytes.len() {
            if let Ok(value) = u8::from_str_radix(std::str::from_utf8(&bytes[at + 1..at + 3]).unwrap_or(""), 16) {
                out.push(value);
                at += 3;
                continue;
            }
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 목록 응답의 `EncodingType=url` 표기(빈칸은 `+`).
fn form_encode(text: &str) -> String {
    api::uri_encode(text, true).replace("%20", "+")
}

fn etag(data: &[u8]) -> String {
    format!("\"{}\"", &api::sha256_hex(data)[..32])
}

impl Fake {
    /// 빈 포트에서 띄운다.
    pub fn start() -> Fake {
        Fake::start_at(0)
    }

    pub fn start_at(port: u16) -> Fake {
        let listener = TcpListener::bind(("127.0.0.1", port)).expect("가짜 저장소를 띄울 포트를 열지 못했다");
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(Mutex::new(State::default()));
        let shared = state.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let state = shared.clone();
                std::thread::spawn(move || serve(stream, &state));
            }
        });
        Fake { port, state }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn put(&self, bucket: &str, key: &str, data: &[u8]) {
        self.lock().buckets.entry(bucket.into()).or_default().insert(key.into(), Stored { data: data.to_vec(), ..Stored::default() });
    }

    pub fn put_with(&self, bucket: &str, key: &str, data: &[u8], content_type: &str, meta: &[(&str, &str)]) {
        let meta = meta.iter().map(|(name, value)| (name.to_string(), value.to_string())).collect();
        self.lock().buckets.entry(bucket.into()).or_default().insert(key.into(), Stored { data: data.to_vec(), content_type: content_type.into(), meta });
    }

    pub fn meta(&self, bucket: &str, key: &str) -> Vec<(String, String)> {
        self.lock().buckets.get(bucket).and_then(|objects| objects.get(key)).map(|stored| stored.meta.clone()).unwrap_or_default()
    }

    /// 목록이 고장 난 저장소처럼 굴게 한다: `repeat`(늘 첫 쪽을 준다) | `no-prefix`(앞머리를 무시한다).
    pub fn set_broken_list(&self, mode: &str) {
        self.lock().broken_list = mode.to_string();
    }

    /// 복사를 모르는 저장소처럼 굴게 한다(빈 객체를 만들고 빈 답을 준다).
    pub fn set_blind_copy(&self, on: bool) {
        self.lock().blind_copy = on;
    }

    /// 목록 요청의 `delimiter`·`encoding-type`을 모르는 저장소처럼 굴게 한다: `reject`(400으로 거절) | `ignore`(못 본 척).
    pub fn set_plain_list(&self, mode: &str) {
        self.lock().plain_list = mode.to_string();
    }

    pub fn get(&self, bucket: &str, key: &str) -> Option<Vec<u8>> {
        self.lock().buckets.get(bucket)?.get(key).map(|stored| stored.data.clone())
    }

    pub fn content_type(&self, bucket: &str, key: &str) -> String {
        self.lock().buckets.get(bucket).and_then(|objects| objects.get(key)).map(|stored| stored.content_type.clone()).unwrap_or_default()
    }

    pub fn keys(&self, bucket: &str) -> Vec<String> {
        self.lock().buckets.get(bucket).map(|objects| objects.keys().cloned().collect()).unwrap_or_default()
    }

    pub fn log(&self) -> Vec<String> {
        self.lock().log.clone()
    }

    pub fn clear_log(&self) {
        self.lock().log.clear();
    }

    /// 그 글로 시작하는 요청을 몇 번 받았는지.
    pub fn count(&self, start: &str) -> usize {
        self.lock().log.iter().filter(|line| line.starts_with(start)).count()
    }

    /// 끝나지 않은 나눠 올리기의 수.
    pub fn uploads(&self) -> usize {
        self.lock().uploads.len()
    }

    /// ListObjectsV2를 모르는 저장소처럼 답하게 한다.
    pub fn set_v1_only(&self, on: bool) {
        self.lock().v1_only = on;
    }

    /// 여러 객체를 한 번에 지우는 요청을 모르는 저장소처럼 답하게 한다.
    pub fn set_no_multi_delete(&self, on: bool) {
        self.lock().no_multi_delete = on;
    }

    /// 그 메서드의 요청을 권한 없음으로 거절하게 한다.
    pub fn set_deny(&self, method: &str) {
        self.lock().deny = Some(method.to_string());
    }
}

fn serve(stream: TcpStream, state: &Mutex<State>) {
    let Ok(mut out) = stream.try_clone() else { return };
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let mut parts = line.split_whitespace();
        let (method, target) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
        let mut headers = Vec::new();
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header).unwrap_or(0) == 0 {
                return;
            }
            let header = header.trim_end();
            if header.is_empty() {
                break;
            }
            if let Some((name, value)) = header.split_once(':') {
                headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
            }
        }
        let length: usize = headers.iter().find(|(name, _)| name == "content-length").and_then(|(_, value)| value.parse().ok()).unwrap_or(0);
        let mut body = vec![0u8; length];
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        let (path, query) = target.split_once('?').map(|(p, q)| (p.to_string(), q.to_string())).unwrap_or((target.clone(), String::new()));
        let incoming = Incoming { method, path, query, headers, body };
        let reply = {
            let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
            state.log.push(format!("{} {}", incoming.method, target));
            match verify(&incoming) {
                Err(reply) => reply,
                Ok(()) if state.deny.as_deref() == Some(incoming.method.as_str()) => error(403, "AccessDenied", "Access Denied"),
                Ok(()) => handle(&mut state, &incoming),
            }
        };
        let reason = match reply.status { 200 => "OK", 204 => "No Content", 206 => "Partial Content", 403 => "Forbidden", 404 => "Not Found", 416 => "Range Not Satisfiable", _ => "Status" };
        let mut head = format!("HTTP/1.1 {} {}\r\nContent-Length: {}\r\n", reply.status, reason, reply.body.len());
        for (name, value) in &reply.headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        if out.write_all(head.as_bytes()).is_err() {
            return;
        }
        if incoming.method != "HEAD" && out.write_all(&reply.body).is_err() {
            return;
        }
        let _ = out.flush();
    }
}

/// 서명을 저장소처럼 다시 계산해 견준다.
fn verify(incoming: &Incoming) -> Result<(), Reply> {
    let mismatch = || error(403, "SignatureDoesNotMatch", "The request signature we calculated does not match the signature you provided.");
    let authorization = incoming.header("authorization");
    if let Some(rest) = authorization.strip_prefix("AWS4-HMAC-SHA256 ") {
        let field = |name: &str| rest.split(',').map(str::trim).find_map(|part| part.strip_prefix(name)).unwrap_or("").to_string();
        let (credential, signed, signature) = (field("Credential="), field("SignedHeaders="), field("Signature="));
        let mut scope = credential.splitn(2, '/');
        if scope.next() != Some(ACCESS) {
            return Err(error(403, "InvalidAccessKeyId", "The AWS Access Key Id you provided does not exist in our records."));
        }
        let scope = scope.next().unwrap_or("");
        let mut scope_parts = scope.split('/');
        let (date, region) = (scope_parts.next().unwrap_or(""), scope_parts.next().unwrap_or(""));
        let payload = incoming.header("x-amz-content-sha256");
        if payload != "UNSIGNED-PAYLOAD" && payload != api::sha256_hex(&incoming.body) {
            return Err(error(400, "XAmzContentSHA256Mismatch", "The provided 'x-amz-content-sha256' header does not match what was computed."));
        }
        let mut pairs: Vec<&str> = incoming.query.split('&').filter(|pair| !pair.is_empty()).collect();
        pairs.sort();
        // 저장소처럼 값의 이어진 빈칸을 하나로 줄여 서명을 다시 계산한다(받은 값이 줄여져 있지 않으면 어긋난다).
        let squeeze = |value: &str| value.split(' ').filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" ");
        let headers: String = signed.split(';').map(|name| format!("{name}:{}\n", squeeze(incoming.header(name)))).collect();
        let canonical = format!("{}\n{}\n{}\n{}\n{}\n{}", incoming.method, incoming.path, pairs.join("&"), headers, signed, payload);
        let to_sign = format!("AWS4-HMAC-SHA256\n{}\n{}\n{}", incoming.header("x-amz-date"), scope, api::sha256_hex(canonical.as_bytes()));
        return if api::v4_signature(SECRET, date, region, &to_sign) == signature && signed.split(';').any(|name| name == "host") { Ok(()) } else { Err(mismatch()) };
    }
    if let Some(rest) = authorization.strip_prefix("AWS ") {
        let (access, signature) = rest.split_once(':').unwrap_or(("", ""));
        if access != ACCESS {
            return Err(error(403, "InvalidAccessKeyId", "The AWS Access Key Id you provided does not exist in our records."));
        }
        let mut amz: Vec<&(String, String)> = incoming.headers.iter().filter(|(name, _)| name.starts_with("x-amz-")).collect();
        amz.sort();
        let amz: String = amz.iter().map(|(name, value)| format!("{name}:{value}\n")).collect();
        const SUB: [&str; 9] = ["acl", "delete", "location", "partNumber", "uploadId", "uploads", "versionId", "versioning", "versions"];
        let mut sub: Vec<String> = incoming
            .query
            .split('&')
            .filter_map(|pair| { let (name, value) = pair.split_once('=').unwrap_or((pair, "")); SUB.contains(&name).then(|| if value.is_empty() { name.to_string() } else { format!("{name}={}", decode(value)) }) })
            .collect();
        sub.sort();
        let resource = format!("{}{}{}", incoming.path, if sub.is_empty() { "" } else { "?" }, sub.join("&"));
        let to_sign = format!("{}\n{}\n{}\n\n{}{}", incoming.method, incoming.header("content-md5"), incoming.header("content-type"), amz, resource);
        return if api::v2_signature(SECRET, &to_sign) == signature { Ok(()) } else { Err(mismatch()) };
    }
    Err(error(403, "AccessDenied", "Anonymous access is forbidden."))
}

fn handle(state: &mut State, incoming: &Incoming) -> Reply {
    let params = incoming.params();
    let path = decode(&incoming.path);
    let path = path.trim_start_matches('/');
    let (bucket, key) = path.split_once('/').unwrap_or((path, ""));
    let method = incoming.method.as_str();
    if bucket.is_empty() {
        let rows: String = state.buckets.keys().map(|name| format!("<Bucket><Name>{}</Name><CreationDate>2024-01-01T00:00:00.000Z</CreationDate></Bucket>", api::xml_escape(name))).collect();
        return xml(200, format!("<ListAllMyBucketsResult><Owner><ID>fake</ID></Owner><Buckets>{rows}</Buckets></ListAllMyBucketsResult>"));
    }
    if key.is_empty() {
        if method == "PUT" {
            if state.buckets.contains_key(bucket) {
                return error(409, "BucketAlreadyOwnedByYou", "Your previous request to create the named bucket succeeded and you already own it.");
            }
            state.buckets.insert(bucket.to_string(), BTreeMap::new());
            return empty(200);
        }
        let Some(objects) = state.buckets.get_mut(bucket) else {
            return error(404, "NoSuchBucket", "The specified bucket does not exist");
        };
        return match method {
            "DELETE" if objects.is_empty() => { state.buckets.remove(bucket); empty(204) }
            "DELETE" => error(409, "BucketNotEmpty", "The bucket you tried to delete is not empty"),
            "POST" if params.contains_key("delete") => {
                if state.no_multi_delete {
                    return error(501, "NotImplemented", "A header you provided implies functionality that is not implemented");
                }
                if incoming.header("content-md5") != api::md5_base64(&incoming.body) {
                    return error(400, "InvalidDigest", "The Content-MD5 you specified was invalid.");
                }
                let Ok(root) = api::parse_xml(&String::from_utf8_lossy(&incoming.body)) else {
                    return error(400, "MalformedXML", "The XML you provided was not well-formed");
                };
                for object in root.all("Object") {
                    objects.remove(object.value("Key"));
                }
                xml(200, "<DeleteResult></DeleteResult>".into())
            }
            "GET" | "HEAD" if !state.broken_list.is_empty() => {
                let mut params = params.clone();
                if state.broken_list == "repeat" {
                    // 이어 읽기 표를 무시한다: 늘 첫 쪽과 같은 표를 준다.
                    params.remove("continuation-token");
                    params.remove("start-after");
                    params.remove("marker");
                    params.insert("max-keys".into(), "2".into());
                } else {
                    params.remove("prefix");
                }
                list(objects, &params, state.v1_only)
            }
            "GET" | "HEAD" if state.plain_list == "reject" && (params.contains_key("delimiter") || params.contains_key("encoding-type")) => {
                error(400, "InvalidArgument", "Unsupported query parameter")
            }
            "GET" | "HEAD" if !state.plain_list.is_empty() => {
                let mut params = params.clone();
                params.remove("delimiter");
                params.remove("encoding-type");
                list(objects, &params, state.v1_only)
            }
            "GET" | "HEAD" => list(objects, &params, state.v1_only),
            _ => error(405, "MethodNotAllowed", "The specified method is not allowed against this resource."),
        };
    }
    if method == "POST" && params.contains_key("uploads") {
        state.counter += 1;
        let id = format!("upload-{}", state.counter);
        state.uploads.insert(id.clone(), (bucket.to_string(), key.to_string(), incoming.header("content-type").to_string(), BTreeMap::new()));
        state.upload_meta.insert(id.clone(), incoming.headers.iter().filter(|(name, _)| name.starts_with("x-amz-meta-")).cloned().collect());
        return xml(200, format!("<InitiateMultipartUploadResult><Bucket>{}</Bucket><Key>{}</Key><UploadId>{id}</UploadId></InitiateMultipartUploadResult>", api::xml_escape(bucket), api::xml_escape(key)));
    }
    if let Some(id) = params.get("uploadId") {
        if !state.uploads.contains_key(id) {
            return error(404, "NoSuchUpload", "The specified upload does not exist.");
        }
        return match method {
            "PUT" => {
                let number: u32 = params.get("partNumber").and_then(|n| n.parse().ok()).unwrap_or(0);
                // 조각 복사: 본문 대신 다른 객체의 한 구간을 조각으로 삼는다.
                let source = decode(incoming.header("x-amz-copy-source"));
                let data = if source.is_empty() {
                    incoming.body.clone()
                } else {
                    let (from_bucket, from_key) = source.trim_start_matches('/').split_once('/').unwrap_or(("", ""));
                    let Some(found) = state.buckets.get(from_bucket).and_then(|objects| objects.get(from_key)) else {
                        return error(404, "NoSuchKey", "The specified key does not exist.");
                    };
                    let range = incoming.header("x-amz-copy-source-range").strip_prefix("bytes=").and_then(|range| range.split_once('-')).and_then(|(a, b)| Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?)));
                    match range {
                        Some((a, b)) if a <= b && b < found.data.len() => found.data[a..=b].to_vec(),
                        Some(_) => return error(400, "InvalidArgument", "The x-amz-copy-source-range value must be within the source object."),
                        None => found.data.clone(),
                    }
                };
                let tag = etag(&data);
                state.uploads.get_mut(id).expect("위에서 봤다").3.insert(number, data);
                if source.is_empty() {
                    Reply { status: 200, headers: vec![("ETag".into(), tag)], body: Vec::new() }
                } else {
                    xml(200, format!("<CopyPartResult><ETag>{}</ETag></CopyPartResult>", api::xml_escape(&tag)))
                }
            }
            "POST" => {
                let (bucket, key, content_type, parts) = state.uploads.remove(id).expect("위에서 봤다");
                let data: Vec<u8> = parts.into_values().flatten().collect();
                let meta = state.upload_meta.remove(id).unwrap_or_default();
                state.buckets.entry(bucket).or_default().insert(key, Stored { data, content_type, meta });
                xml(200, "<CompleteMultipartUploadResult><ETag>\"done\"</ETag></CompleteMultipartUploadResult>".into())
            }
            "DELETE" => { state.uploads.remove(id); empty(204) }
            _ => error(405, "MethodNotAllowed", "The specified method is not allowed against this resource."),
        };
    }
    if method == "PUT" {
        let source = if state.blind_copy { "" } else { incoming.header("x-amz-copy-source") };
        let stored = if source.is_empty() {
            let meta = incoming
                .headers
                .iter()
                .filter(|(name, _)| name.starts_with("x-amz-meta-") || matches!(name.as_str(), "cache-control" | "content-disposition" | "content-language" | "content-encoding"))
                .cloned()
                .collect();
            Stored { data: incoming.body.clone(), content_type: incoming.header("content-type").to_string(), meta }
        } else {
            let source = decode(source);
            let (from_bucket, from_key) = source.trim_start_matches('/').split_once('/').unwrap_or(("", ""));
            match state.buckets.get(from_bucket).and_then(|objects| objects.get(from_key)) {
                Some(found) => found.clone(),
                None => return error(404, "NoSuchKey", "The specified key does not exist."),
            }
        };
        let Some(objects) = state.buckets.get_mut(bucket) else {
            return error(404, "NoSuchBucket", "The specified bucket does not exist");
        };
        let tag = etag(&stored.data);
        objects.insert(key.to_string(), stored);
        return if source.is_empty() { Reply { status: 200, headers: vec![("ETag".into(), tag)], body: Vec::new() } } else { xml(200, format!("<CopyObjectResult><ETag>{}</ETag></CopyObjectResult>", api::xml_escape(&tag))) };
    }
    let Some(objects) = state.buckets.get_mut(bucket) else {
        return error(404, "NoSuchBucket", "The specified bucket does not exist");
    };
    match method {
        "DELETE" => { objects.remove(key); empty(204) }
        "GET" | "HEAD" => {
            let Some(found) = objects.get(key) else {
                return error(404, "NoSuchKey", "The specified key does not exist.");
            };
            let total = found.data.len();
            let mut headers = vec![
                ("Content-Type".to_string(), if found.content_type.is_empty() { "binary/octet-stream".to_string() } else { found.content_type.clone() }),
                ("Last-Modified".to_string(), "Tue, 02 Jan 2024 03:04:05 GMT".to_string()),
                ("ETag".to_string(), etag(&found.data)),
            ];
            headers.extend(found.meta.iter().cloned());
            let range = incoming.header("range").strip_prefix("bytes=").and_then(|range| range.split_once('-')).and_then(|(from, to)| Some((from.parse::<usize>().ok()?, to.parse::<usize>().ok())));
            match range {
                Some((from, _)) if from >= total => error(416, "InvalidRange", "The requested range is not satisfiable"),
                Some((from, to)) => {
                    let end = to.unwrap_or(total - 1).min(total - 1);
                    headers.push(("Content-Range".into(), format!("bytes {from}-{end}/{total}")));
                    Reply { status: 206, headers, body: found.data[from..=end].to_vec() }
                }
                None => Reply { status: 200, headers, body: found.data.clone() },
            }
        }
        _ => error(405, "MethodNotAllowed", "The specified method is not allowed against this resource."),
    }
}

fn list(objects: &BTreeMap<String, Stored>, params: &HashMap<String, String>, v1_only: bool) -> Reply {
    let get = |name: &str| params.get(name).map(String::as_str).unwrap_or("");
    let (prefix, delimiter) = (get("prefix"), get("delimiter"));
    let limit: usize = get("max-keys").parse().unwrap_or(1000).min(1000);
    let v2 = !v1_only && get("list-type") == "2";
    let encoded = !v1_only && get("encoding-type") == "url";
    let after = if v2 { get("continuation-token").strip_prefix("T").unwrap_or(get("start-after")) } else { get("marker") };
    let show = |text: &str| api::xml_escape(&if encoded { form_encode(text) } else { text.to_string() });
    let (mut contents, mut prefixes, mut count, mut last, mut truncated) = (String::new(), String::new(), 0usize, String::new(), false);
    let mut group = String::new();
    for (key, stored) in objects.range::<str, _>((std::ops::Bound::Excluded(after), std::ops::Bound::Unbounded)) {
        if !key.starts_with(prefix) {
            continue;
        }
        // 묶음으로 이미 보인 폴더(이어 읽기의 시작점이 그 폴더일 때도)의 아래는 건너뛴다.
        if !delimiter.is_empty() && ((!group.is_empty() && key.starts_with(&group)) || (after.ends_with(delimiter) && key.starts_with(after))) {
            continue;
        }
        if count == limit {
            truncated = true;
            break;
        }
        let rest = &key[prefix.len()..];
        match (!delimiter.is_empty()).then(|| rest.find(delimiter)).flatten() {
            Some(at) => {
                group = format!("{prefix}{}", &rest[..at + delimiter.len()]);
                prefixes.push_str(&format!("<CommonPrefixes><Prefix>{}</Prefix></CommonPrefixes>", show(&group)));
                last = group.clone();
            }
            None => {
                contents.push_str(&format!(
                    "<Contents><Key>{}</Key><LastModified>2024-01-02T03:04:05.000Z</LastModified><ETag>{}</ETag><Size>{}</Size><StorageClass>STANDARD</StorageClass></Contents>",
                    show(key), api::xml_escape(&etag(&stored.data)), stored.data.len()
                ));
                last = key.clone();
            }
        }
        count += 1;
    }
    let mut body = format!("<ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Prefix>{}</Prefix><MaxKeys>{limit}</MaxKeys><IsTruncated>{truncated}</IsTruncated>", show(prefix));
    if encoded {
        body.push_str("<EncodingType>url</EncodingType>");
    }
    if truncated && v2 {
        body.push_str(&format!("<NextContinuationToken>T{}</NextContinuationToken>", api::xml_escape(&last)));
    } else if truncated && !delimiter.is_empty() {
        body.push_str(&format!("<NextMarker>{}</NextMarker>", show(&last)));
    }
    body.push_str(&contents);
    body.push_str(&prefixes);
    body.push_str("</ListBucketResult>");
    xml(200, body)
}
