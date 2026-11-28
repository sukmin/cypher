//! S3 호환 저장소 API의 핵심: 요청 주소 만들기, 서명(SigV4·예전 V2), 응답 XML 읽기.
//!
//! 네트워크와 파일은 건드리지 않는다(요청을 실제로 보내는 일은 앱의 `s3` 모듈이 한다). 그래서 서명은 AWS 문서에
//! 실린 예시 값으로 시험할 수 있다.
//!
//! - 주소는 경로 방식(`https://서버/버킷/키`)과 가상 호스트 방식(`https://버킷.서버/키`)을 모두 만든다.
//!   직접 만든 S3 호환 저장소는 대개 경로 방식이다.
//! - 키는 RFC 3986대로 엄격하게 %XX로 적는다(`A-Za-z0-9-._~`만 그대로). 서명에 넣은 글과 실제로 보내는 글이
//!   한 글자라도 다르면 서버가 거절하므로, 주소를 한 번만 만들어 양쪽에 똑같이 쓴다.
//! - 목록은 ListObjectsV2로 읽되, V2를 모르는 저장소(V1 모양으로 답한다)도 읽을 수 있게 양쪽 응답을 다 받아들인다.

use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

/// 본문이 없는 요청의 본문 해시(빈 글의 SHA-256).
pub const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signature {
    /// AWS Signature Version 4 (기본).
    V4,
    /// AWS Signature Version 2 (오래된 저장소용).
    V2,
}

/// 저장소의 주소. `https://storage.example.com:9000` 같은 글에서 읽는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// `http` 또는 `https`
    pub scheme: String,
    /// Host 헤더에 들어갈 글: 호스트, 기본 포트가 아니면 `:포트`까지.
    pub host: String,
    /// 주소에 경로가 붙어 있으면 그 경로(`/s3` 등, 끝의 `/`는 뗀다). 없으면 빈 글.
    pub base: String,
    /// 경로 방식(`/버킷/키`)으로 부를지. 거짓이면 가상 호스트 방식(`버킷.호스트`).
    pub path_style: bool,
}

impl Endpoint {
    /// 사용자가 적은 주소를 읽는다. 앞에 `http://`·`https://`가 없으면 https로 본다.
    pub fn parse(text: &str, path_style: bool) -> Result<Endpoint, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("저장소 주소를 적어 주세요 (예: https://storage.example.com).".into());
        }
        let full = if text.contains("://") { text.to_string() } else { format!("https://{text}") };
        let url = url::Url::parse(&full).map_err(|e| format!("저장소 주소를 읽을 수 없습니다: {e}"))?;
        let scheme = url.scheme().to_string();
        if scheme != "http" && scheme != "https" {
            return Err("저장소 주소는 http:// 또는 https://로 시작해야 합니다.".into());
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err("주소에 계정을 적지 않습니다. 접근 키와 비밀 키 칸에 적어 주세요.".into());
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err("저장소 주소에는 `?`나 `#` 뒤의 글을 적지 않습니다.".into());
        }
        let name = url.host_str().filter(|h| !h.is_empty()).ok_or("저장소 주소에 호스트가 없습니다.")?;
        // `url`은 기본 포트(80·443)를 적어도 떼어 준다. Host 헤더도 그렇게 보낸다.
        let host = match url.port() {
            Some(port) => format!("{name}:{port}"),
            None => name.to_string(),
        };
        let base = url.path().trim_end_matches('/').to_string();
        Ok(Endpoint { scheme, host, base, path_style })
    }

    /// 화면에 보일 주소.
    pub fn display(&self) -> String {
        format!("{}://{}{}", self.scheme, self.host, self.base)
    }
}

/// RFC 3986의 "그대로 두는 글자"만 남기고 %XX로 적는다. `keep_slash`면 `/`도 그대로 둔다(키의 경로 구분).
pub fn uri_encode(text: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for &byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(byte as char),
            b'/' if keep_slash => out.push('/'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// 목록 응답의 `EncodingType=url`로 적힌 글을 되돌린다(`+`는 빈칸, `%XX`는 그 바이트).
pub fn url_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'+' => out.push(b' '),
            b'%' if at + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[at + 1..at + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(value) => { out.push(value); at += 2; }
                    None => out.push(b'%'),
                }
            }
            other => out.push(other),
        }
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

/// 여러 객체 지우기 요청에 꼭 붙여야 하는 `Content-MD5` 값.
pub fn md5_base64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(md5::Md5::digest(data))
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = <Hmac<Sha256>>::new_from_slice(key).expect("HMAC은 어떤 길이의 키든 받는다");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn hmac_sha1(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = <Hmac<sha1::Sha1>>::new_from_slice(key).expect("HMAC은 어떤 길이의 키든 받는다");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// 협정 세계시의 날짜와 시각.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Utc {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    /// 0 = 일요일
    pub weekday: u32,
}

impl Utc {
    pub fn from(time: SystemTime) -> Utc {
        let secs = time.duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        let days = secs.div_euclid(86400);
        let rest = secs.rem_euclid(86400) as u32;
        // 1970-01-01부터의 날 수를 날짜로 (Howard Hinnant의 civil_from_days).
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let year = yoe + era * 400 + i64::from(month <= 2);
        Utc {
            year, month, day,
            hour: rest / 3600, minute: rest % 3600 / 60, second: rest % 60,
            weekday: (days + 4).rem_euclid(7) as u32, // 1970-01-01은 목요일
        }
    }

    /// `20130524`
    pub fn date(&self) -> String {
        format!("{:04}{:02}{:02}", self.year, self.month, self.day)
    }

    /// `20130524T000000Z`
    pub fn stamp(&self) -> String {
        format!("{}T{:02}{:02}{:02}Z", self.date(), self.hour, self.minute, self.second)
    }

    /// `Fri, 24 May 2013 00:00:00 GMT`
    pub fn http(&self) -> String {
        const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
        const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
        format!(
            "{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
            DAYS[self.weekday as usize], self.day, MONTHS[(self.month - 1) as usize], self.year, self.hour, self.minute, self.second
        )
    }
}

/// 보낼 요청 하나(서명하기 전).
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub method: &'static str,
    /// 버킷. 없으면 저장소 전체(버킷 목록).
    pub bucket: Option<String>,
    /// 객체의 키(있는 그대로, %XX로 적지 않은 것). 버킷만 다루면 빈 글.
    pub key: String,
    /// 주소의 `?` 뒤(이름, 값). 값이 없는 것(`uploads`, `delete`)은 빈 글로 준다.
    pub query: Vec<(String, String)>,
    /// 함께 보낼 헤더(Content-Type, Content-MD5, Range, x-amz-…). Host·날짜·서명은 `sign`이 붙인다.
    pub headers: Vec<(String, String)>,
    /// 본문의 SHA-256(16진). 본문이 없으면 비워 둔다.
    pub payload_sha256: String,
    /// 객체 하나를 다루는 요청인지. 참인데 키가 비어 있으면 요청을 만들지 않는다
    /// (빈 키의 주소는 버킷 자체를 가리켜, 객체를 지우려던 요청이 버킷을 지우는 요청이 된다).
    pub object: bool,
}

/// 서명을 마친 요청: 그대로 보내면 된다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signed {
    pub method: &'static str,
    pub url: String,
    /// 보낼 헤더 전부(Host 포함).
    pub headers: Vec<(String, String)>,
}

/// 키의 한 마디가 `.`이나 `..`뿐인지. 그런 키는 주소로 적으면 "지금 폴더"·"위 폴더"로 풀려 다른 객체를 가리킨다.
pub fn has_dot_segment(key: &str) -> bool {
    key.split('/').any(|part| part == "." || part == "..")
}

/// 헤더 값을 서명 규칙대로 다듬는다: 앞뒤 빈칸을 떼고, 이어진 빈칸은 하나로 줄인다. 보내는 값도 이것으로 한다.
fn squeeze(value: &str) -> String {
    value.split(' ').filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" ")
}

/// 이 요청이 가리키는 호스트와 경로·쿼리(이미 %XX로 적힌 것).
fn target(endpoint: &Endpoint, request: &Request) -> Result<(String, String, String), String> {
    let mut host = endpoint.host.clone();
    let mut path = endpoint.base.clone();
    match &request.bucket {
        Some(bucket) => {
            // 버킷 이름은 주소의 호스트(가상 호스트 방식)나 경로의 한 마디가 된다. 이름에 쓸 수 있는 글자만 받아
            // `@`·`#`·`/` 같은 글자로 다른 서버나 다른 경로를 가리키게 하지 못하게 한다.
            if bucket.is_empty() || !bucket.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_')) {
                return Err(format!("버킷 이름이 올바르지 않습니다: {bucket:?}"));
            }
            if request.object && request.key.is_empty() {
                return Err("객체의 키가 비어 있습니다.".into());
            }
            if endpoint.path_style {
                path.push('/');
                path.push_str(&uri_encode(bucket, false));
            } else {
                host = format!("{bucket}.{host}");
            }
            path.push('/');
            path.push_str(&uri_encode(&request.key, true));
        }
        None => {
            if !request.key.is_empty() {
                return Err("버킷 없이 객체를 가리킬 수 없습니다.".into());
            }
            path.push('/');
        }
    }
    // 키의 한 마디가 `.`이나 `..`이면 주소로 적을 수 없다: HTTP 주소 규칙이 그 마디를 "지금 폴더"·"위 폴더"로 풀어 버려
    // 다른 객체를 가리키게 된다. 엉뚱한 객체를 읽거나 지우지 않게 여기서 막는다.
    if has_dot_segment(&request.key) {
        return Err(format!("키에 `.`이나 `..`뿐인 마디가 있어 다룰 수 없습니다: {}", request.key));
    }
    let mut pairs: Vec<(String, String)> =
        request.query.iter().map(|(name, value)| (uri_encode(name, false), uri_encode(value, false))).collect();
    pairs.sort();
    let query = pairs.iter().map(|(name, value)| format!("{name}={value}")).collect::<Vec<_>>().join("&");
    Ok((host, path, query))
}

/// SigV4의 마지막 단계: 비밀 키에서 그날·그 지역의 서명 키를 끌어내어 "서명할 글"에 서명한다(16진 글).
pub fn v4_signature(secret: &str, date: &str, region: &str, to_sign: &str) -> String {
    let mut key = hmac_sha256(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    for part in [region, "s3", "aws4_request"] {
        key = hmac_sha256(&key, part.as_bytes());
    }
    hex(&hmac_sha256(&key, to_sign.as_bytes()))
}

/// 예전 서명 방식(V2)의 마지막 단계: "서명할 글"을 비밀 키로 HMAC-SHA1 서명한다(Base64 글).
pub fn v2_signature(secret: &str, to_sign: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(hmac_sha1(secret.as_bytes(), to_sign.as_bytes()))
}

/// 요청에 서명한다. `now`는 요청을 보내는 때(서버 시계와 15분 넘게 다르면 서버가 거절한다).
pub fn sign(
    endpoint: &Endpoint, access: &str, secret: &str, region: &str, signature: Signature, request: &Request, now: SystemTime,
) -> Result<Signed, String> {
    let (host, path, query) = target(endpoint, request)?;
    let url = format!("{}://{}{}{}{}", endpoint.scheme, host, path, if query.is_empty() { "" } else { "?" }, query);
    let time = Utc::from(now);
    let mut headers: Vec<(String, String)> =
        request.headers.iter().map(|(name, value)| (name.to_ascii_lowercase(), squeeze(value))).collect();
    headers.retain(|(name, _)| name != "host" && name != "authorization" && name != "x-amz-date" && name != "x-amz-content-sha256");
    headers.push(("host".into(), host.clone()));

    let authorization = match signature {
        Signature::V4 => {
            let payload = if request.payload_sha256.is_empty() { EMPTY_SHA256 } else { request.payload_sha256.as_str() };
            headers.push(("x-amz-content-sha256".into(), payload.to_string()));
            headers.push(("x-amz-date".into(), time.stamp()));
            headers.sort();
            let canonical_headers: String = headers.iter().map(|(name, value)| format!("{name}:{value}\n")).collect();
            let signed = headers.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>().join(";");
            let canonical = format!("{}\n{}\n{}\n{}\n{}\n{}", request.method, path, query, canonical_headers, signed, payload);
            let scope = format!("{}/{}/s3/aws4_request", time.date(), region);
            let to_sign = format!("AWS4-HMAC-SHA256\n{}\n{}\n{}", time.stamp(), scope, sha256_hex(canonical.as_bytes()));
            let signature = v4_signature(secret, &time.date(), region, &to_sign);
            format!("AWS4-HMAC-SHA256 Credential={access}/{scope}, SignedHeaders={signed}, Signature={signature}")
        }
        Signature::V2 => {
            headers.push(("x-amz-date".into(), time.http()));
            headers.sort();
            let find = |wanted: &str| headers.iter().find(|(name, _)| name == wanted).map(|(_, value)| value.as_str()).unwrap_or("");
            let amz: String =
                headers.iter().filter(|(name, _)| name.starts_with("x-amz-")).map(|(name, value)| format!("{name}:{value}\n")).collect();
            // 서명에 넣는 자원: 버킷과 경로(가상 호스트 방식이어도 버킷을 앞에 붙인다), 그리고 정해진 하위 자원만.
            let mut resource = String::new();
            if !endpoint.path_style {
                if let Some(bucket) = &request.bucket {
                    resource.push('/');
                    resource.push_str(bucket);
                }
            }
            resource.push_str(&path);
            const SUB: [&str; 9] = ["acl", "delete", "location", "partNumber", "uploadId", "uploads", "versionId", "versioning", "versions"];
            let mut sub: Vec<&(String, String)> = request.query.iter().filter(|(name, _)| SUB.contains(&name.as_str())).collect();
            sub.sort();
            for (index, (name, value)) in sub.iter().enumerate() {
                resource.push(if index == 0 { '?' } else { '&' });
                resource.push_str(name);
                if !value.is_empty() {
                    resource.push('=');
                    resource.push_str(value);
                }
            }
            // x-amz-date를 보내므로 Date 줄은 비운다.
            let to_sign = format!("{}\n{}\n{}\n\n{}{}", request.method, find("content-md5"), find("content-type"), amz, resource);
            format!("AWS {access}:{}", v2_signature(secret, &to_sign))
        }
    };
    headers.push(("authorization".into(), authorization));
    Ok(Signed { method: request.method, url, headers })
}

// ───────────────────────── XML ─────────────────────────

/// XML 요소 하나. S3의 응답은 속성을 쓰지 않으므로 이름·글·자식만 둔다(이름의 `ns:` 앞머리는 뗀다).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Node {
    pub name: String,
    pub text: String,
    pub children: Vec<Node>,
}

impl Node {
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|node| node.name == name)
    }

    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |node| node.name == name)
    }

    /// 그 이름의 자식에 든 글(없으면 빈 글).
    pub fn value(&self, name: &str) -> &str {
        self.child(name).map(|node| node.text.as_str()).unwrap_or("")
    }
}

fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest.find(';').filter(|end| *end <= 12) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let ch = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .map(|hex| u32::from_str_radix(hex, 16))
                .or_else(|| name.strip_prefix('#').map(|dec| dec.parse::<u32>()))
                .and_then(|code| code.ok())
                .and_then(char::from_u32),
        };
        match ch {
            Some(ch) => { out.push(ch); rest = &rest[end + 1..]; }
            None => { out.push('&'); rest = &rest[1..]; }
        }
    }
    out.push_str(rest);
    out
}

/// XML로 적을 글(요소 안)로 바꾼다.
pub fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\r' => out.push_str("&#13;"),
            other => out.push(other),
        }
    }
    out
}

/// XML 문서를 읽어 맨 위 요소를 돌려준다. S3 응답에 나오는 만큼만 안다(선언, 주석, CDATA, 스스로 닫는 요소, 문자 참조).
pub fn parse_xml(text: &str) -> Result<Node, String> {
    const DEPTH: usize = 64;
    let broken = || "응답 XML을 읽을 수 없습니다.".to_string();
    let mut stack: Vec<Node> = vec![Node::default()];
    let mut rest = text.trim_start_matches('\u{feff}');
    while !rest.is_empty() {
        let Some(open) = rest.find('<') else {
            break;
        };
        if open > 0 {
            let top = stack.last_mut().expect("맨 아래 칸은 늘 있다");
            top.text.push_str(&unescape(&rest[..open]));
        }
        rest = &rest[open..];
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = &after[after.find("-->").ok_or_else(broken)? + 3..];
        } else if let Some(after) = rest.strip_prefix("<![CDATA[") {
            let end = after.find("]]>").ok_or_else(broken)?;
            stack.last_mut().expect("맨 아래 칸은 늘 있다").text.push_str(&after[..end]);
            rest = &after[end + 3..];
        } else if rest.starts_with("<?") || rest.starts_with("<!") {
            rest = &rest[rest.find('>').ok_or_else(broken)? + 1..];
        } else if let Some(after) = rest.strip_prefix("</") {
            let end = after.find('>').ok_or_else(broken)?;
            if stack.len() < 2 {
                return Err(broken());
            }
            let mut node = stack.pop().expect("위에서 길이를 봤다");
            let closing = after[..end].trim();
            if closing.rsplit(':').next().unwrap_or(closing) != node.name {
                return Err(broken());
            }
            // 자식이 있는 요소의 글은 들여쓰기뿐이다.
            if !node.children.is_empty() {
                node.text.clear();
            }
            stack.last_mut().expect("맨 아래 칸은 늘 있다").children.push(node);
            rest = &after[end + 1..];
        } else {
            let end = rest.find('>').ok_or_else(broken)?;
            let inner = &rest[1..end];
            let closed = inner.ends_with('/');
            let name = inner.trim_end_matches('/').split(|c: char| c.is_whitespace()).next().unwrap_or("");
            let name = name.rsplit(':').next().unwrap_or(name).to_string();
            if name.is_empty() || stack.len() > DEPTH {
                return Err(broken());
            }
            let node = Node { name, ..Node::default() };
            if closed {
                stack.last_mut().expect("맨 아래 칸은 늘 있다").children.push(node);
            } else {
                stack.push(node);
            }
            rest = &rest[end + 1..];
        }
    }
    if stack.len() != 1 {
        return Err(broken());
    }
    stack.pop().and_then(|mut root| root.children.pop()).ok_or_else(broken)
}

/// 서버가 돌려준 오류: `<Error><Code>…</Code><Message>…</Message></Error>`.
pub fn parse_error(body: &str) -> Option<(String, String)> {
    let root = parse_xml(body).ok()?;
    (root.name == "Error").then(|| (root.value("Code").to_string(), root.value("Message").to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bucket {
    pub name: String,
    pub created: String,
}

pub fn parse_buckets(body: &str) -> Result<Vec<Bucket>, String> {
    let root = parse_xml(body)?;
    if root.name != "ListAllMyBucketsResult" {
        return Err(format!("버킷 목록이 아닌 응답입니다: <{}>", root.name));
    }
    Ok(root
        .child("Buckets")
        .map(|buckets| {
            buckets
                .all("Bucket")
                .map(|bucket| Bucket { name: bucket.value("Name").to_string(), created: bucket.value("CreationDate").to_string() })
                .filter(|bucket| !bucket.name.is_empty())
                .collect()
        })
        .unwrap_or_default())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    pub key: String,
    pub size: u64,
    /// 고친 때(ISO 8601 글 그대로).
    pub modified: String,
    pub etag: String,
    pub storage_class: String,
}

/// 목록 한 쪽.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListPage {
    /// 구분자로 묶인 "폴더"(끝에 `/`가 붙은 전체 키 앞머리).
    pub prefixes: Vec<String>,
    pub objects: Vec<Object>,
    /// 뒤에 더 있는지.
    pub truncated: bool,
    /// 다음 쪽을 읽을 때 넘길 값: `t:<이어 읽기 표>`(V2) 또는 `m:<마지막 키>`(V1 모양으로 답한 저장소).
    pub next: Option<String>,
}

/// ListObjectsV2(또는 V1) 응답을 읽는다.
pub fn parse_list(body: &str) -> Result<ListPage, String> {
    let root = parse_xml(body)?;
    if root.name != "ListBucketResult" {
        return Err(format!("객체 목록이 아닌 응답입니다: <{}>", root.name));
    }
    let encoded = root.value("EncodingType").eq_ignore_ascii_case("url");
    let decode = |text: &str| if encoded { url_decode(text) } else { text.to_string() };
    let objects: Vec<Object> = root
        .all("Contents")
        .map(|node| Object {
            key: decode(node.value("Key")),
            size: node.value("Size").trim().parse().unwrap_or(0),
            modified: node.value("LastModified").trim().to_string(),
            etag: node.value("ETag").trim().trim_matches('"').to_string(),
            storage_class: node.value("StorageClass").trim().to_string(),
        })
        .collect();
    let prefixes: Vec<String> = root.all("CommonPrefixes").map(|node| decode(node.value("Prefix"))).filter(|prefix| !prefix.is_empty()).collect();
    let truncated = root.value("IsTruncated").trim().eq_ignore_ascii_case("true");
    let token = root.value("NextContinuationToken");
    let next = if !truncated {
        None
    } else if !token.is_empty() {
        Some(format!("t:{token}"))
    } else {
        // V1 모양의 답: NextMarker가 있으면 그것, 없으면 이번 쪽에서 가장 뒤에 오는 이름부터 이어 읽는다.
        let marker = decode(root.value("NextMarker"));
        // 마지막이 폴더(묶음)면 그 폴더 뒤부터 읽는다: 폴더 이름 그대로면 그 안의 키가 다시 나와 같은 폴더가 또 보인다.
        let last_object = objects.iter().map(|object| object.key.as_str()).max();
        let last_prefix = prefixes.iter().map(String::as_str).max();
        let last = match (last_object, last_prefix) {
            (Some(object), Some(prefix)) if prefix > object => format!("{prefix}\u{10ffff}"),
            (None, Some(prefix)) => format!("{prefix}\u{10ffff}"),
            (Some(object), _) => object.to_string(),
            (None, None) => String::new(),
        };
        let from = if marker.is_empty() { last } else { marker };
        (!from.is_empty()).then(|| format!("m:{from}"))
    };
    Ok(ListPage { prefixes, objects, truncated: truncated && next.is_some(), next })
}

/// 목록 요청의 `?` 뒤. `delimiter`가 참이면 `/`로 묶어 그 층만 읽는다(거짓이면 앞머리 아래 전부).
/// `encode`가 참이면 키를 %XX로 적어 달라고 한다(XML에 적을 수 없는 글자가 든 키도 읽을 수 있다).
pub fn list_query(prefix: &str, delimiter: bool, limit: u32, next: Option<&str>, encode: bool) -> Vec<(String, String)> {
    let mut query: Vec<(String, String)> = vec![("max-keys".into(), limit.clamp(1, 1000).to_string())];
    if encode {
        query.push(("encoding-type".into(), "url".into()));
    }
    if !prefix.is_empty() {
        query.push(("prefix".into(), prefix.to_string()));
    }
    if delimiter {
        query.push(("delimiter".into(), "/".into()));
    }
    match next.and_then(|next| next.split_once(':')) {
        // V1 모양으로 답하는 저장소는 `marker`로 이어 읽는다.
        Some(("m", marker)) => query.push(("marker".into(), marker.to_string())),
        Some(("t", token)) => {
            query.push(("list-type".into(), "2".into()));
            query.push(("continuation-token".into(), token.to_string()));
        }
        // 그 이름 뒤부터 읽는다(폴더로 묶어 주지 않는 저장소에서 폴더 하나를 건너뛸 때).
        Some(("a", after)) => {
            query.push(("list-type".into(), "2".into()));
            query.push(("start-after".into(), after.to_string()));
        }
        _ => query.push(("list-type".into(), "2".into())),
    }
    query
}

/// 구분자(`/`)로 묶어 주지 않는 저장소의 답을 폴더 모양으로 묶는다: `prefix` 아래의 이름에 `/`가 든 객체는
/// 그 첫 마디까지를 폴더로 친다. 묶어 주는 저장소의 답은 그대로 지나간다(그런 객체가 없다).
/// 한 쪽의 마지막 객체가 폴더 안의 것이었으면, 다음 쪽은 그 폴더를 통째로 건너뛰고 읽는다(폴더 안을 다 훑지 않는다).
pub fn group_by_slash(page: ListPage, prefix: &str) -> ListPage {
    let ListPage { mut prefixes, objects, truncated, mut next } = page;
    let mut kept = Vec::with_capacity(objects.len());
    let mut last_folder: Option<String> = None;
    for object in objects {
        let folder = object.key.strip_prefix(prefix).and_then(|rest| rest.find('/').map(|at| format!("{prefix}{}", &rest[..=at])));
        match folder {
            // 폴더 자신을 나타내는 빈 객체(`폴더/`)가 한 층 아래의 것이면 그것도 폴더 한 줄이다.
            Some(folder) => {
                if !prefixes.contains(&folder) {
                    prefixes.push(folder.clone());
                }
                last_folder = Some(folder);
            }
            None => {
                kept.push(object);
                last_folder = None;
            }
        }
    }
    if let (true, Some(folder)) = (truncated, &last_folder) {
        // 그 폴더로 시작하는 어떤 키보다도 뒤에 오는 글.
        let after = format!("{folder}\u{10ffff}");
        next = Some(if next.as_deref().is_some_and(|token| token.starts_with("m:")) { format!("m:{after}") } else { format!("a:{after}") });
    }
    ListPage { prefixes, objects: kept, truncated, next }
}

/// 조각 복사(UploadPartCopy)의 답에서 그 조각의 확인 값(ETag)을 꺼낸다.
pub fn parse_copy_part(body: &str) -> Result<String, String> {
    let root = expect_root(body, "CopyPartResult")?;
    let etag = root.value("ETag").trim();
    if etag.is_empty() {
        return Err("저장소가 복사한 조각의 확인 값(ETag)을 돌려주지 않았습니다.".into());
    }
    Ok(etag.to_string())
}

/// 여러 객체 지우기 요청의 본문.
pub fn delete_body(keys: &[String]) -> String {
    let mut body = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Delete><Quiet>true</Quiet>");
    for key in keys {
        body.push_str("<Object><Key>");
        body.push_str(&xml_escape(key));
        body.push_str("</Key></Object>");
    }
    body.push_str("</Delete>");
    body
}

/// 여러 객체 지우기 요청의 본문(XML)에 적을 수 있는 키인지. 제어 문자가 든 키는 XML로 적을 수 없어 하나씩 지운다.
pub fn xml_safe(key: &str) -> bool {
    key.chars().all(|ch| (ch >= ' ' || matches!(ch, '\t' | '\n' | '\r')) && !matches!(ch, '\u{fffe}' | '\u{ffff}'))
}

/// 응답이 기다리던 것인지 확인하고 맨 위 요소를 돌려준다. 저장소가 오류를 적었으면 그 오류를, 빈 답이나 엉뚱한 답이면
/// 그렇다고 알린다. 복사·나눠 올리기 맺기·여러 객체 지우기는 200으로 답한 뒤 본문에 오류를 적기도 하고, 그 요청을
/// 모르는 저장소는 빈 답으로 "성공"이라고 하기도 한다(그 답을 믿고 원본을 지우면 안 된다).
pub fn expect_root(body: &str, name: &str) -> Result<Node, String> {
    let root = parse_xml(body).map_err(|_| format!("저장소가 알 수 없는 답을 돌려줬습니다(<{name}>을 기다렸는데 {}).", if body.trim().is_empty() { "빈 답" } else { "XML이 아닌 답" }))?;
    if root.name == "Error" {
        return Err(error_text(root.value("Code"), root.value("Message")));
    }
    if root.name != name {
        return Err(format!("저장소가 알 수 없는 답을 돌려줬습니다(<{name}>을 기다렸는데 <{}>).", root.name));
    }
    Ok(root)
}

/// 여러 객체 지우기의 답에서 지우지 못한 것들: (키, 까닭).
pub fn parse_delete_result(body: &str) -> Result<Vec<(String, String)>, String> {
    let root = expect_root(body, "DeleteResult")?;
    Ok(root.all("Error").map(|node| (node.value("Key").to_string(), error_text(node.value("Code"), node.value("Message")))).collect())
}

/// 큰 파일 나눠 올리기를 시작한 답에서 올리기 번호를 꺼낸다.
pub fn parse_upload_id(body: &str) -> Result<String, String> {
    let root = parse_xml(body)?;
    let id = root.value("UploadId").trim();
    if root.name != "InitiateMultipartUploadResult" || id.is_empty() {
        return Err("나눠 올리기를 시작하지 못했습니다(응답에 올리기 번호가 없습니다).".into());
    }
    Ok(id.to_string())
}

/// 나눠 올리기를 마치는 요청의 본문. `parts`는 (조각 번호, ETag).
pub fn complete_body(parts: &[(u32, String)]) -> String {
    let mut body = String::from("<CompleteMultipartUpload>");
    for (number, etag) in parts {
        body.push_str(&format!("<Part><PartNumber>{number}</PartNumber><ETag>{}</ETag></Part>", xml_escape(etag)));
    }
    body.push_str("</CompleteMultipartUpload>");
    body
}

/// 서버의 오류 번호와 설명을 사람이 읽을 글로.
pub fn error_text(code: &str, message: &str) -> String {
    let hint = match code {
        "AccessDenied" | "AllAccessDisabled" => "권한이 없습니다",
        "InvalidAccessKeyId" => "접근 키가 올바르지 않습니다",
        "SignatureDoesNotMatch" => "서명이 맞지 않습니다(비밀 키, 지역, 서명 방식, 주소 방식을 확인하세요)",
        "NoSuchBucket" => "그런 버킷이 없습니다",
        "NoSuchKey" => "그런 객체가 없습니다",
        "RequestTimeTooSkewed" => "이 컴퓨터의 시계가 서버와 너무 다릅니다",
        "BucketNotEmpty" => "버킷이 비어 있지 않습니다",
        "BucketAlreadyExists" | "BucketAlreadyOwnedByYou" => "같은 이름의 버킷이 이미 있습니다",
        "PermanentRedirect" | "AuthorizationHeaderMalformed" => "지역이나 주소가 맞지 않습니다",
        "EntityTooLarge" => "한 번에 다룰 수 있는 크기를 넘었습니다",
        "SlowDown" | "ServiceUnavailable" => "저장소가 바쁩니다. 잠시 뒤에 다시 해 보세요",
        "InvalidObjectState" => "보관 등급(Deep Archive 등)에 있는 객체라 먼저 복원해야 읽을 수 있습니다",
        "InvalidBucketName" => "버킷 이름이 규칙에 맞지 않습니다(3~63자, 영어 소문자·숫자·`-`·`.`, 처음과 끝은 소문자나 숫자)",
        "TooManyBuckets" => "만들 수 있는 버킷 수를 넘었습니다",
        "BucketNotEmptyInUnversioned" => "버킷이 비어 있지 않습니다",
        "XAmzContentSHA256Mismatch" => "보낸 내용이 서명할 때의 내용과 다릅니다(올리는 사이에 파일이 바뀌었을 수 있습니다)",
        "NoSuchUpload" => "나눠 올리던 작업이 저장소에 없습니다(시간이 지나 치워졌을 수 있습니다)",
        "PreconditionFailed" => "조건이 맞지 않아 하지 않았습니다",
        _ => "",
    };
    match (hint.is_empty(), code.is_empty(), message.is_empty()) {
        (false, _, true) => format!("{hint} ({code})"),
        (false, _, false) => format!("{hint} ({code}: {message})"),
        (true, false, false) => format!("{code}: {message}"),
        (true, false, true) => code.to_string(),
        (true, true, _) => message.to_string(),
    }
}

// ───────────────────────── 주소 줄 ─────────────────────────

/// 주소 줄에 적은 곳: 버킷과 그 안의 키(폴더면 끝이 `/`, 버킷 목록이면 둘 다 빈 글).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub bucket: String,
    pub key: String,
}

/// `s3://버킷/폴더/파일`, `버킷/폴더/`, `/버킷` 같은 글을 읽는다. 빈 글이나 `s3://`는 버킷 목록이다.
pub fn parse_location(text: &str) -> Result<Location, String> {
    let text = text.trim();
    let lower = text.get(..5).map(str::to_ascii_lowercase);
    let rest = if lower.as_deref() == Some("s3://") { &text[5..] } else { text };
    if rest.contains("://") {
        return Err("s3://버킷/경로 모양으로 적어 주세요.".into());
    }
    let rest = rest.trim_start_matches('/');
    let (bucket, key) = rest.split_once('/').unwrap_or((rest, ""));
    Ok(Location { bucket: bucket.to_string(), key: key.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const ACCESS: &str = "AKIAIOSFODNN7EXAMPLE";
    const SECRET: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";

    /// AWS 문서의 예시가 쓰는 때: 2013-05-24 00:00:00 UTC.
    fn doc_time() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_369_353_600)
    }

    fn aws() -> Endpoint {
        Endpoint { scheme: "https".into(), host: "s3.amazonaws.com".into(), base: String::new(), path_style: false }
    }

    fn header<'a>(signed: &'a Signed, name: &str) -> &'a str {
        signed.headers.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str()).unwrap_or("")
    }

    #[test]
    fn 날짜를_서명에_쓰는_모양으로_적는다() {
        let time = Utc::from(doc_time());
        assert_eq!(time.stamp(), "20130524T000000Z");
        assert_eq!(time.http(), "Fri, 24 May 2013 00:00:00 GMT");
        assert_eq!(Utc::from(UNIX_EPOCH).http(), "Thu, 01 Jan 1970 00:00:00 GMT");
        // 윤년의 2월 29일과 해의 마지막 순간
        assert_eq!(Utc::from(UNIX_EPOCH + Duration::from_secs(1_709_208_000)).stamp(), "20240229T120000Z");
        assert_eq!(Utc::from(UNIX_EPOCH + Duration::from_secs(1_767_225_599)).stamp(), "20251231T235959Z");
    }

    #[test]
    fn aws_문서의_객체_읽기_예시와_같은_서명이_나온다() {
        let request = Request {
            method: "GET",
            bucket: Some("examplebucket".into()),
            key: "test.txt".into(),
            headers: vec![("Range".into(), "bytes=0-9".into())],
            ..Request::default()
        };
        let signed = sign(&aws(), ACCESS, SECRET, "us-east-1", Signature::V4, &request, doc_time()).unwrap();
        assert_eq!(signed.url, "https://examplebucket.s3.amazonaws.com/test.txt");
        assert_eq!(
            header(&signed, "authorization"),
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
             SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, \
             Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
        assert_eq!(header(&signed, "x-amz-content-sha256"), EMPTY_SHA256);
    }

    #[test]
    fn aws_문서의_객체_쓰기_예시와_같은_서명이_나온다() {
        let body = b"Welcome to Amazon S3.";
        let request = Request {
            method: "PUT",
            bucket: Some("examplebucket".into()),
            key: "test$file.text".into(),
            headers: vec![
                ("Date".into(), "Fri, 24 May 2013 00:00:00 GMT".into()),
                ("x-amz-storage-class".into(), "REDUCED_REDUNDANCY".into()),
            ],
            payload_sha256: sha256_hex(body),
            ..Request::default()
        };
        let signed = sign(&aws(), ACCESS, SECRET, "us-east-1", Signature::V4, &request, doc_time()).unwrap();
        assert_eq!(signed.url, "https://examplebucket.s3.amazonaws.com/test%24file.text");
        assert!(header(&signed, "authorization").ends_with("Signature=98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd"), "{}", header(&signed, "authorization"));
    }

    #[test]
    fn aws_문서의_목록_예시와_같은_서명이_나온다() {
        let request = Request {
            method: "GET",
            bucket: Some("examplebucket".into()),
            query: vec![("prefix".into(), "J".into()), ("max-keys".into(), "2".into())],
            ..Request::default()
        };
        let signed = sign(&aws(), ACCESS, SECRET, "us-east-1", Signature::V4, &request, doc_time()).unwrap();
        assert_eq!(signed.url, "https://examplebucket.s3.amazonaws.com/?max-keys=2&prefix=J");
        assert!(header(&signed, "authorization").ends_with("Signature=34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"), "{}", header(&signed, "authorization"));

        // 값이 없는 하위 자원은 `이름=`로 적는다.
        let request = Request { method: "GET", bucket: Some("examplebucket".into()), query: vec![("lifecycle".into(), String::new())], ..Request::default() };
        let signed = sign(&aws(), ACCESS, SECRET, "us-east-1", Signature::V4, &request, doc_time()).unwrap();
        assert_eq!(signed.url, "https://examplebucket.s3.amazonaws.com/?lifecycle=");
        assert!(header(&signed, "authorization").ends_with("Signature=fea454ca298b7da1c68078a5d1bdbfbbe0d65c699e0f91ac7a200a0136783543"), "{}", header(&signed, "authorization"));
    }

    #[test]
    fn 예전_서명_방식도_aws_문서의_예시와_같다() {
        // 문서의 예시는 Date 헤더를 쓰지만 여기서는 x-amz-date를 보내므로, 서명할 글을 직접 견준다:
        // 같은 키로 문서의 글에 서명하면 문서의 값이 나오는지(HMAC-SHA1·Base64)와, 우리가 만든 글의 모양을 따로 본다.
        let doc = "GET\n\n\nTue, 27 Mar 2007 19:36:42 +0000\n/awsexamplebucket1/photos/puppy.jpg";
        assert_eq!(base64::engine::general_purpose::STANDARD.encode(hmac_sha1(SECRET.as_bytes(), doc.as_bytes())), "qgk2+6Sv9/oM7G3qLEjTH1a1l1g=");

        let endpoint = Endpoint { scheme: "http".into(), host: "storage.local:9000".into(), base: String::new(), path_style: true };
        let request = Request {
            method: "POST",
            bucket: Some("b".into()),
            key: "큰 파일.bin".into(),
            query: vec![("uploadId".into(), "abc".into()), ("partNumber".into(), "2".into()), ("max-keys".into(), "5".into())],
            headers: vec![("Content-Type".into(), "application/xml".into()), ("x-amz-meta-a".into(), "1".into())],
            ..Request::default()
        };
        let signed = sign(&endpoint, "AK", "SK", "us-east-1", Signature::V2, &request, doc_time()).unwrap();
        let expect = "POST\n\napplication/xml\n\nx-amz-date:Fri, 24 May 2013 00:00:00 GMT\nx-amz-meta-a:1\n/b/%ED%81%B0%20%ED%8C%8C%EC%9D%BC.bin?partNumber=2&uploadId=abc";
        let wanted = base64::engine::general_purpose::STANDARD.encode(hmac_sha1(b"SK", expect.as_bytes()));
        assert_eq!(header(&signed, "authorization"), format!("AWS AK:{wanted}"));
        assert_eq!(signed.url, "http://storage.local:9000/b/%ED%81%B0%20%ED%8C%8C%EC%9D%BC.bin?max-keys=5&partNumber=2&uploadId=abc");
    }

    #[test]
    fn 헤더_값의_이어진_빈칸은_하나로_줄여_서명하고_보낸다() {
        let endpoint = Endpoint::parse("http://h", true).unwrap();
        let with = |value: &str| {
            let request = Request { method: "PUT", bucket: Some("b".into()), key: "k".into(), headers: vec![("x-amz-meta-note".into(), value.into())], ..Request::default() };
            sign(&endpoint, "a", "s", "r", Signature::V4, &request, doc_time()).unwrap()
        };
        let (wide, narrow) = (with("  hello   big  world "), with("hello big world"));
        assert_eq!(header(&wide, "x-amz-meta-note"), "hello big world");
        assert_eq!(header(&wide, "authorization"), header(&narrow, "authorization"));
        assert!(xml_safe("보통 키/a b.txt") && !xml_safe("a\u{1}b") && xml_safe("tab\there"));
    }

    #[test]
    fn 주소는_경로_방식과_가상_호스트_방식으로_만든다() {
        let path = Endpoint::parse("http://10.0.0.5:9000/", true).unwrap();
        assert_eq!(path, Endpoint { scheme: "http".into(), host: "10.0.0.5:9000".into(), base: String::new(), path_style: true });
        let request = Request { method: "GET", bucket: Some("logs".into()), key: "2024/a b+c/한글.txt".into(), ..Request::default() };
        let signed = sign(&path, "a", "s", "us-east-1", Signature::V4, &request, doc_time()).unwrap();
        assert_eq!(signed.url, "http://10.0.0.5:9000/logs/2024/a%20b%2Bc/%ED%95%9C%EA%B8%80.txt");
        assert_eq!(header(&signed, "host"), "10.0.0.5:9000");

        // 주소에 경로가 붙은 저장소(게이트웨이 뒤)와 기본 포트
        let behind = Endpoint::parse("storage.example.com:443/s3/", true).unwrap();
        assert_eq!(behind.display(), "https://storage.example.com/s3");
        let signed = sign(&behind, "a", "s", "us-east-1", Signature::V4, &Request { method: "GET", ..Request::default() }, doc_time()).unwrap();
        assert_eq!(signed.url, "https://storage.example.com/s3/");

        let hosted = Endpoint::parse("https://s3.example.com", false).unwrap();
        let signed = sign(&hosted, "a", "s", "us-east-1", Signature::V4, &request, doc_time()).unwrap();
        assert!(signed.url.starts_with("https://logs.s3.example.com/2024/"));

        assert!(Endpoint::parse("ftp://x", true).is_err());
        assert!(Endpoint::parse("https://me:pw@x", true).is_err());
        assert!(Endpoint::parse("", true).is_err());
    }

    #[test]
    fn 주소로_적을_수_없는_키는_거절한다() {
        let endpoint = Endpoint::parse("http://h", true).unwrap();
        for key in ["a/../b", "./a", "a/.", ".."] {
            let request = Request { method: "DELETE", bucket: Some("b".into()), key: key.into(), ..Request::default() };
            assert!(sign(&endpoint, "a", "s", "r", Signature::V4, &request, doc_time()).is_err(), "{key}");
        }
        // 점이 든 보통 이름은 된다.
        for key in ["a/..b", ".git/config", "a/...", "a//b"] {
            let request = Request { method: "GET", bucket: Some("b".into()), key: key.into(), ..Request::default() };
            assert!(sign(&endpoint, "a", "s", "r", Signature::V4, &request, doc_time()).is_ok(), "{key}");
        }
        // 버킷 이름에 쓸 수 없는 글자(다른 서버·경로를 가리키게 할 수 있는 것)와, 키가 빈 객체 요청은 거절한다.
        let hosted = Endpoint::parse("https://s3.example.com", false).unwrap();
        for bucket in ["a/b", "x@evil.example#", "a b", "", "a?b", "a%2Fb"] {
            let request = Request { method: "GET", bucket: Some(bucket.into()), ..Request::default() };
            assert!(sign(&endpoint, "a", "s", "r", Signature::V4, &request, doc_time()).is_err(), "{bucket}");
            assert!(sign(&hosted, "a", "s", "r", Signature::V4, &request, doc_time()).is_err(), "{bucket}");
        }
        let empty = Request { method: "DELETE", bucket: Some("b".into()), object: true, ..Request::default() };
        assert!(sign(&endpoint, "a", "s", "r", Signature::V4, &empty, doc_time()).is_err());
        assert!(sign(&endpoint, "a", "s", "r", Signature::V4, &Request { object: false, ..empty }, doc_time()).is_ok());
    }

    #[test]
    fn 만든_주소는_주소_해석기를_거쳐도_그대로다() {
        // 보내는 쪽(ureq)은 주소를 `url`로 한 번 읽는다. 그때 글자가 바뀌면 서명이 어긋난다.
        let endpoint = Endpoint::parse("https://h.example", true).unwrap();
        let key = "폴더/a b/!$&'()*+,;=:@[]{}|\\^`<>#?%25\"~_-.x";
        let request = Request {
            method: "GET",
            bucket: Some("bkt".into()),
            key: key.into(),
            query: vec![("prefix".into(), "가 나/+&=".into()), ("uploads".into(), String::new())],
            ..Request::default()
        };
        let signed = sign(&endpoint, "a", "s", "r", Signature::V4, &request, doc_time()).unwrap();
        assert_eq!(url::Url::parse(&signed.url).unwrap().as_str(), signed.url);
    }

    #[test]
    fn 목록_응답을_읽는다() {
        let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Name>logs</Name><Prefix>2024/</Prefix><KeyCount>3</KeyCount><MaxKeys>2</MaxKeys><Delimiter>/</Delimiter>
  <EncodingType>url</EncodingType><IsTruncated>true</IsTruncated>
  <NextContinuationToken>1ueGcxLPRx1Tr/XYExHnhbYLgveDs2J/wm36Hy4vbOwM=</NextContinuationToken>
  <Contents><Key>2024/a+b%2Bc.txt</Key><LastModified>2024-01-02T03:04:05.000Z</LastModified><ETag>&quot;abc&quot;</ETag><Size>1234567890123</Size><StorageClass>STANDARD</StorageClass></Contents>
  <Contents><Key>2024/%ED%95%9C%EA%B8%80.log</Key><LastModified>2024-01-03T00:00:00Z</LastModified><ETag>"d-2"</ETag><Size>0</Size></Contents>
  <CommonPrefixes><Prefix>2024/01/</Prefix></CommonPrefixes>
  <CommonPrefixes><Prefix>2024/a%26b/</Prefix></CommonPrefixes>
</ListBucketResult>"#;
        let page = parse_list(body).unwrap();
        assert_eq!(page.prefixes, vec!["2024/01/", "2024/a&b/"]);
        assert_eq!(page.objects.len(), 2);
        assert_eq!(page.objects[0], Object { key: "2024/a b+c.txt".into(), size: 1_234_567_890_123, modified: "2024-01-02T03:04:05.000Z".into(), etag: "abc".into(), storage_class: "STANDARD".into() });
        assert_eq!(page.objects[1].key, "2024/한글.log");
        assert!(page.truncated);
        assert_eq!(page.next.as_deref(), Some("t:1ueGcxLPRx1Tr/XYExHnhbYLgveDs2J/wm36Hy4vbOwM="));
        assert!(list_query("2024/", true, 500, page.next.as_deref(), true).contains(&("continuation-token".to_string(), "1ueGcxLPRx1Tr/XYExHnhbYLgveDs2J/wm36Hy4vbOwM=".to_string())));
        // 묶어 주는 저장소의 답은 폴더로 묶기를 거쳐도 그대로다.
        assert_eq!(group_by_slash(page.clone(), "2024/"), page);
    }

    #[test]
    fn 예전_모양의_목록_응답도_이어_읽을_수_있다() {
        // V2를 모르는 저장소: 이어 읽기 표가 없다. 인코딩 표시도 없으면 글자를 풀지 않는다.
        let body = "<ListBucketResult><IsTruncated>true</IsTruncated><Contents><Key>a+b%20.txt</Key><Size>1</Size></Contents><CommonPrefixes><Prefix>z/</Prefix></CommonPrefixes></ListBucketResult>";
        let page = parse_list(body).unwrap();
        assert_eq!(page.objects[0].key, "a+b%20.txt");
        // 마지막이 폴더라 그 폴더 뒤부터 이어 읽는다.
        assert_eq!(page.next.as_deref(), Some("m:z/\u{10ffff}"));
        let query = list_query("", true, 5000, page.next.as_deref(), true);
        assert!(query.contains(&("marker".to_string(), "z/\u{10ffff}".to_string())));
        let object_last = "<ListBucketResult><IsTruncated>true</IsTruncated><Contents><Key>zz</Key></Contents><CommonPrefixes><Prefix>z/</Prefix></CommonPrefixes></ListBucketResult>";
        assert_eq!(parse_list(object_last).unwrap().next.as_deref(), Some("m:zz"));
        assert!(query.contains(&("max-keys".to_string(), "1000".to_string())));
        assert!(!query.iter().any(|(name, _)| name == "list-type"));

        let with_marker = "<ListBucketResult><IsTruncated>true</IsTruncated><NextMarker>m/x</NextMarker><Contents><Key>a</Key></Contents></ListBucketResult>";
        assert_eq!(parse_list(with_marker).unwrap().next.as_deref(), Some("m:m/x"));
        // 끝까지 읽었으면 다음이 없다. 빈 목록도 읽는다.
        let done = parse_list("<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>").unwrap();
        assert_eq!(done, ListPage::default());
        assert!(parse_list("<Error><Code>NoSuchBucket</Code></Error>").is_err());
    }

    #[test]
    fn 묶어_주지_않는_저장소의_목록을_폴더_모양으로_묶는다() {
        let object = |key: &str| Object { key: key.into(), size: 1, modified: String::new(), etag: String::new(), storage_class: String::new() };
        // 구분자를 모르는 저장소: 아래의 것까지 모두 객체로 온다.
        let flat = ListPage {
            prefixes: vec![],
            objects: vec![object("d/"), object("d/a.txt"), object("d/sub/"), object("d/sub/1"), object("d/sub/2"), object("d/z.txt"), object("d/zz/9")],
            truncated: true,
            next: Some("t:token".into()),
        };
        let grouped = group_by_slash(flat, "d/");
        assert_eq!(grouped.prefixes, vec!["d/sub/", "d/zz/"]);
        assert_eq!(grouped.objects.iter().map(|o| o.key.as_str()).collect::<Vec<_>>(), vec!["d/", "d/a.txt", "d/z.txt"]);
        // 마지막 객체가 폴더 안의 것이었으니 다음 쪽은 그 폴더 뒤부터 읽는다.
        assert_eq!(grouped.next.as_deref(), Some("a:d/zz/\u{10ffff}"));
        let query = list_query("d/", true, 500, grouped.next.as_deref(), false);
        assert!(query.contains(&("start-after".to_string(), "d/zz/\u{10ffff}".to_string())));
        assert!(query.contains(&("list-type".to_string(), "2".to_string())));
        assert!(!query.iter().any(|(name, _)| name == "encoding-type" || name == "continuation-token"));

        // 마지막 객체가 그 층의 것이면 저장소가 준 표로 이어 읽는다. 예전 모양의 저장소는 `marker`로 건너뛴다.
        let page = ListPage { prefixes: vec![], objects: vec![object("x/1"), object("y")], truncated: true, next: Some("t:token".into()) };
        assert_eq!(group_by_slash(page, "").next.as_deref(), Some("t:token"));
        let page = ListPage { prefixes: vec![], objects: vec![object("y"), object("z/1")], truncated: true, next: Some("m:z/1".into()) };
        assert_eq!(group_by_slash(page, "").next.as_deref(), Some("m:z/\u{10ffff}"));
        let page = ListPage { prefixes: vec![], objects: vec![object("z/1")], truncated: false, next: None };
        assert_eq!(group_by_slash(page, "").next, None);

        assert_eq!(parse_copy_part("<CopyPartResult><LastModified>x</LastModified><ETag>&quot;abc&quot;</ETag></CopyPartResult>").unwrap(), "\"abc\"");
        assert!(parse_copy_part("<Error><Code>InvalidObjectState</Code></Error>").unwrap_err().contains("복원"));
        assert!(parse_copy_part("<CopyPartResult/>").is_err());
    }

    #[test]
    fn 버킷_목록과_오류와_그_밖의_응답을_읽는다() {
        let body = "<ListAllMyBucketsResult><Owner><ID>x</ID></Owner><Buckets><Bucket><Name>a</Name><CreationDate>2020-01-01T00:00:00.000Z</CreationDate></Bucket><Bucket><Name>b-2</Name></Bucket></Buckets></ListAllMyBucketsResult>";
        let buckets = parse_buckets(body).unwrap();
        assert_eq!(buckets.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), vec!["a", "b-2"]);
        assert_eq!(parse_buckets("<ListAllMyBucketsResult><Buckets/></ListAllMyBucketsResult>").unwrap(), vec![]);

        let error = "<?xml version=\"1.0\"?><Error><Code>AccessDenied</Code><Message>Access Denied &amp; logged</Message><RequestId>1</RequestId></Error>";
        assert_eq!(parse_error(error), Some(("AccessDenied".into(), "Access Denied & logged".into())));
        assert_eq!(parse_error("<html>502</html>"), None);
        assert_eq!(parse_error("not xml"), None);
        // 기다리던 답이 아니면(오류, 빈 답, 엉뚱한 답) 성공으로 치지 않는다.
        assert!(expect_root(error, "CopyObjectResult").unwrap_err().contains("권한이 없습니다"));
        assert!(expect_root("<CopyObjectResult><ETag>x</ETag></CopyObjectResult>", "CopyObjectResult").is_ok());
        assert!(expect_root("", "CopyObjectResult").unwrap_err().contains("빈 답"));
        assert!(expect_root("<html>ok</html>", "CopyObjectResult").unwrap_err().contains("<html>"));
        assert!(expect_root("OK", "CompleteMultipartUploadResult").is_err());
        assert!(parse_delete_result("").is_err());
        assert!(parse_delete_result("<Whatever/>").is_err());

        assert_eq!(parse_upload_id("<InitiateMultipartUploadResult><Bucket>b</Bucket><Key>k</Key><UploadId>VXBsb2Fk</UploadId></InitiateMultipartUploadResult>").unwrap(), "VXBsb2Fk");
        assert!(parse_upload_id("<InitiateMultipartUploadResult/>").is_err());

        let failed = parse_delete_result("<DeleteResult><Deleted><Key>a</Key></Deleted><Error><Key>b&lt;1&gt;</Key><Code>AccessDenied</Code><Message>no</Message></Error></DeleteResult>").unwrap();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].0, "b<1>");
        assert!(parse_delete_result("<DeleteResult/>").unwrap().is_empty());
    }

    #[test]
    fn 요청_본문의_특수_글자를_바꿔_적는다() {
        let body = delete_body(&["a&b<c>.txt".to_string(), "따옴표\"'".to_string()]);
        assert!(body.contains("<Key>a&amp;b&lt;c&gt;.txt</Key>"));
        let root = parse_xml(&body).unwrap();
        assert_eq!(root.all("Object").map(|o| o.value("Key").to_string()).collect::<Vec<_>>(), vec!["a&b<c>.txt", "따옴표\"'"]);
        assert_eq!(md5_base64(b""), "1B2M2Y8AsgTpgAmY7PhCfg==");
        assert_eq!(complete_body(&[(1, "\"a\"".into()), (2, "b".into())]), "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>&quot;a&quot;</ETag></Part><Part><PartNumber>2</PartNumber><ETag>b</ETag></Part></CompleteMultipartUpload>");
    }

    #[test]
    fn xml의_주석과_cdata와_문자_참조를_읽는다() {
        let root = parse_xml("\u{feff}<?xml version=\"1.0\"?><!-- c --><a:R xmlns:a=\"x\"><K attr=\"1\">x &#65;&#x42; &unknown; &amp;</K><E/><C><![CDATA[<raw&>]]></C></a:R>").unwrap();
        assert_eq!(root.name, "R");
        assert_eq!(root.value("K"), "x AB &unknown; &");
        assert_eq!(root.value("E"), "");
        assert_eq!(root.value("C"), "<raw&>");
        for broken in ["", "<a>", "</a>", "<a><b></a>", "<a></b>", "plain"] {
            assert!(parse_xml(broken).is_err(), "{broken}");
        }
        assert_eq!(parse_xml("<n:a><n:b>1</n:b ></n:a>").unwrap().value("b"), "1");
        assert!(parse_xml(&"<a>".repeat(200)).is_err());
    }

    #[test]
    fn 주소_줄의_글을_버킷과_키로_나눈다() {
        let at = |text: &str| parse_location(text).map(|l| (l.bucket, l.key)).unwrap();
        assert_eq!(at("s3://logs/2024/a.txt"), ("logs".into(), "2024/a.txt".into()));
        assert_eq!(at("S3://logs/2024/"), ("logs".into(), "2024/".into()));
        assert_eq!(at("logs"), ("logs".into(), String::new()));
        assert_eq!(at(" /logs/a b/ "), ("logs".into(), "a b/".into()));
        assert_eq!(at("s3://"), (String::new(), String::new()));
        assert_eq!(at(""), (String::new(), String::new()));
        assert!(parse_location("https://x/y").is_err());
    }

    #[test]
    fn 퍼센트_글자를_되돌린다() {
        assert_eq!(url_decode("a+b%2Bc%2F%ED%95%9C"), "a b+c/한");
        assert_eq!(url_decode("100%"), "100%");
        assert_eq!(url_decode("%zz%4"), "%zz%4");
        assert_eq!(uri_encode("a b/c~_-.", true), "a%20b/c~_-.");
        assert_eq!(uri_encode("a/b", false), "a%2Fb");
    }
}
