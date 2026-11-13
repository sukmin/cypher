//! `.http` 요청 파일(IntelliJ HTTP Client 형식의 부분집합) 읽기와 실행 기록 만들기.
//!
//! ```text
//! @host = https://api.example.com        ← 변수
//!
//! ### 로그인                              ← 요청 구분과 이름
//! POST {{host}}/login
//! Content-Type: application/json
//!
//! {"id": "kim"}
//! ```
//!
//! `{{$timestamp}}` 같은 내장 값과 `{{$hmac sha256 hex 비밀키 메시지}}` 같은 계산도 쓸 수 있다
//! (서명이 필요한 API용. 아래 "내장 값" 참고).
//!
//! 본문은 다음 `###`까지이며, `> ` 또는 `<> `로 시작하는 줄을 만나면 거기서 끝난다
//! (실행 기록의 응답 부분이 다시 읽힐 때 본문으로 섞이지 않게 하는 규칙).

use serde::Serialize;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HttpRequest {
    pub name: String,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    /// 이 요청이 시작하는 줄(0부터). `###` 줄이거나, 첫 요청이면 파일 첫 줄.
    pub line: usize,
    /// 이 요청 구역이 끝나는 줄의 다음 줄(다음 `###` 줄이거나 파일 끝). 글이 줄 바꿈으로 끝나면
    /// 편집기에 보이는 맨 끝의 빈 줄까지가 마지막 구역이다.
    pub end: usize,
    /// 값을 채우지 못한 까닭들(모르는 알고리즘, 없는 환경 변수, 주소가 아닌 글자 등). 비어 있어야 보낼 수 있다.
    pub problems: Vec<String>,
    /// `{{$env …}}`로 읽어 온 값들. 기록과 목록에 그대로 남지 않게 가리는 데 쓴다.
    #[serde(skip)]
    pub secrets: Vec<String>,
}

impl HttpRequest {
    /// 글 속의 비밀 값(환경 변수에서 읽은 값)을 `***`로 가린다.
    pub fn masked(&self, text: &str) -> String {
        let mut out = text.to_string();
        for secret in self.secrets.iter().filter(|s| s.len() >= 4) {
            out = out.replace(secret.as_str(), "***");
        }
        out
    }
}

const METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
/// 기록에 값을 남기지 않는 헤더(인증 정보).
const SECRET_HEADERS: [&str; 5] = ["authorization", "proxy-authorization", "cookie", "set-cookie", "x-api-key"];
const MAX_RECORD_BODY: usize = 20_000;

/// 처음 만들어 주는 요청 파일.
pub const TEMPLATE: &str = r#"# HTTP 요청 파일 (IntelliJ .http 형식)
# - 요청은 '###'으로 구분하고, 그 뒤 글자가 요청 이름입니다.
# - '@이름 = 값'으로 변수를 정하고 '{{이름}}'으로 씁니다.
# - 실행하면 요청과 응답이 같은 폴더의 history.http에 기록됩니다.
# - Claude 세션은 http_list / http_run 도구로 이 파일의 요청을 바로 실행할 수 있습니다.
# - 서명(HMAC)이 필요한 API는 맨 아래 "서명(HMAC) 예시"처럼 내장 값을 씁니다.
#   ({{$timestamp}}, {{$uuid}}, {{$method}}, {{$pathQuery}}, {{$body}}, {{$env 이름}}, {{$hash …}}, {{$hmac …}}. README의 "HTTP 요청 파일" 참고)

@host = https://httpbin.org

### 조회 예시
GET {{host}}/get?name=test
Accept: application/json

### 전송 예시
POST {{host}}/post
Content-Type: application/json

{
  "message": "안녕하세요"
}

### 서명(HMAC) 예시
# 서명할 글(message)을 "시각.메서드.경로와 쿼리.본문"으로 만들고, 비밀키(secret)로 HMAC-SHA256 서명을 붙입니다.
# 실제 비밀키는 파일에 적지 말고 환경 변수에서 읽으세요:  @secret = {{$env MY_API_SECRET}}
# 응답(httpbin이 받은 헤더를 그대로 돌려줌)에서 X-Signature 값을 확인할 수 있습니다.
@secret = sample-secret-key
@message = {{$timestamp}}.{{$method}}.{{$pathQuery}}.{{$body}}
POST {{host}}/post?demo=hmac
Content-Type: application/json
X-Timestamp: {{$timestamp}}
X-Signature: {{$hmac sha256 hex secret message}}

{
  "amount": 1000
}
"#;

fn var_line(line: &str) -> Option<(String, String)> {
    let (key, value) = line.trim().strip_prefix('@')?.split_once('=')?;
    let key = key.trim();
    let valid = !key.is_empty() && key.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'));
    valid.then(|| (key.to_string(), value.trim().to_string()))
}

fn is_comment(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with('#') || line.starts_with("//")
}

/// 글 맨 앞의 BOM(메모장 등이 붙이는 U+FEFF)을 뗀다. 첫 줄 안에 있는 글자라 줄 번호는 달라지지 않는다.
/// 떼지 않으면 첫 줄의 `@변수`·`### 이름`·요청 줄을 알아보지 못한다(`trim`은 이 글자를 지우지 않는다).
fn without_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

/// 요청 파일의 요청을 모두 읽는다. 변수와 내장 값은 지금 시각 기준으로 채워진 상태로 돌려준다.
pub fn parse(text: &str) -> Vec<HttpRequest> {
    parse_at(text, Now::system())
}

/// 정해진 시각을 기준으로 읽는다.
pub fn parse_at(text: &str, now: Now) -> Vec<HttpRequest> {
    let text = without_bom(text);
    let vars: HashMap<String, String> = text.lines().filter_map(var_line).collect();

    let mut requests = Vec::new();
    let mut name = String::new();
    let mut start = 0;
    let mut block: Vec<&str> = Vec::new();
    let mut total = 0;
    for (number, line) in text.lines().enumerate() {
        total = number + 1;
        match line.strip_prefix("###") {
            Some(rest) => {
                requests.extend(parse_block(&name, start, number, &block, &vars, now));
                name = rest.trim().to_string();
                start = number;
                block.clear();
            }
            None => block.push(line),
        }
    }
    // 글이 줄 바꿈으로 끝나면 편집기에는 그 뒤에 빈 줄이 하나 더 보인다. 마지막 구역이 그 줄까지 덮게 한다
    // (마지막 요청 아래의 빈 줄에서 실행해도 그 요청이 나가도록).
    if text.ends_with('\n') {
        total += 1;
    }
    requests.extend(parse_block(&name, start, total.max(start + 1), &block, &vars, now));
    requests
}

/// 편집기에서 커서가 있는 줄(0부터)이 속한 요청의 번호. `requests`는 같은 `text`를 읽은 결과여야 한다.
///
/// 그 줄이 어느 요청 구역에도 들어 있지 않으면 None이다(가까운 다른 요청을 대신 고르지 않는다. 엉뚱한 요청이
/// 전송되면 안 되기 때문이다). 단, 파일 머리말(첫 `###` 앞이고 빈 줄·주석·변수 정의뿐인 부분)에 있으면
/// 바로 아래의 첫 구역을 고른다. 그 구역이 요청이 되지 못했으면(주소를 잘못 적은 줄, 설명만 있는 구역)
/// 이때도 None이다. 그 뒤의 다른 요청을 "첫 요청"이라며 대신 보내지 않는다.
pub fn index_at(text: &str, requests: &[HttpRequest], line: usize) -> Option<usize> {
    if let Some(found) = requests.iter().position(|r| r.line <= line && line < r.end) {
        return Some(found);
    }
    let below = preamble_end(text)?;
    (line < below && requests.first()?.line == below).then_some(0)
}

/// 머리말 바로 아래 줄(첫 `###` 줄)의 번호. 그 앞에 빈 줄·주석·변수 정의가 아닌 글이 있으면 머리말이 아니다.
fn preamble_end(text: &str) -> Option<usize> {
    for (number, line) in without_bom(text).lines().enumerate() {
        if line.starts_with("###") {
            return Some(number);
        }
        if !(line.trim().is_empty() || is_comment(line) || var_line(line).is_some()) {
            return None;
        }
    }
    None
}

fn parse_block(
    name: &str,
    line: usize,
    end: usize,
    block: &[&str],
    vars: &HashMap<String, String>,
    now: Now,
) -> Option<HttpRequest> {
    let mut lines =
        block.iter().copied().skip_while(|l| l.trim().is_empty() || is_comment(l) || var_line(l).is_some());
    let first = lines.next()?.trim();
    let (method, target) = match first.split_once(char::is_whitespace) {
        Some((m, rest)) if METHODS.contains(&m.to_ascii_uppercase().as_str()) => (m.to_ascii_uppercase(), rest.trim()),
        _ => ("GET".to_string(), first),
    };
    let target = target.rfind(" HTTP/").map_or(target, |at| &target[..at]);
    // 요청 줄인지는 적힌 모양으로 판단한다: 주소(`http://`, `https://`)나 변수(`{{`)로 시작해야 한다.
    // 그 밖의 글자(설명 등)는 요청이 아니다.
    let lower = target.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://") || target.starts_with("{{")) {
        return None;
    }

    // 값은 주소 → 본문 → 헤더 순으로 채운다. 헤더(서명)에서 주소와 본문을 쓸 수 있게 하려는 순서다.
    let mut scope = Scope::new(vars, now, &method);
    let expanded = scope.expand(target, 0);
    // 실제로 전송될 모양으로 맞춘다(한글·공백 인코딩, `..` 정리, 기본 포트 생략 등).
    // 서명에 쓰는 `$path`·`$query`·`$host`가 전송 내용과 같아야 서버가 계산한 서명과 맞는다.
    let url = match Url::parse(&expanded) {
        Ok(parsed) if matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some() => {
            scope.url = Some(parsed.clone());
            parsed.to_string()
        }
        // 변수 오타 등으로 주소가 되지 못했다. 요청 목록에는 남겨 두고(다른 요청이 대신 실행되지 않게) 보내지는 않는다.
        _ => {
            scope.problems.push(format!("주소가 http:// 또는 https://로 시작하는 올바른 주소가 아닙니다: {expanded}"));
            expanded
        }
    };

    let mut header_lines = Vec::new();
    let mut body: Vec<&str> = Vec::new();
    let mut in_body = false;
    for line in lines {
        if in_body {
            if line.starts_with("> ") || line.starts_with("<> ") {
                break;
            }
            body.push(line);
        } else if line.trim().is_empty() {
            in_body = true;
        } else if !is_comment(line) {
            if let Some((key, value)) = line.split_once(':') {
                header_lines.push((key.trim().to_string(), value.trim()));
            }
        }
    }
    while body.last().is_some_and(|l| l.trim().is_empty()) {
        body.pop();
    }
    let body = scope.expand(&body.join("\n"), 0);
    scope.body = Some(body.clone());
    let headers = header_lines.into_iter().map(|(key, value)| (key, scope.expand(value, 0))).collect();
    let problems = std::mem::take(&mut scope.problems);
    let secrets = std::mem::take(&mut scope.secrets);
    // 이름 없는 요청의 이름: 시각·무작위 값이 든 주소는 읽을 때마다 달라지므로 적힌 그대로를 이름으로 쓴다.
    let default_name = if target.contains("{{$") { format!("{method} {target}") } else { format!("{method} {url}") };
    Some(HttpRequest {
        name: if name.is_empty() { default_name } else { name.to_string() },
        method: method.clone(),
        url,
        headers,
        body,
        line,
        end,
        problems,
        secrets,
    })
}

// ───────────────────────── 내장 값 ─────────────────────────
//
// `{{이름}}`은 변수, `{{$이름 인자…}}`는 내장 값이다. 인자는 변수 이름, `$내장값`, 또는 "따옴표 글"로 적는다
// (따옴표 안에서는 `{{…}}`를 다시 쓸 수 있다).
//
// | 적는 법 | 값 |
// |---|---|
// | `$timestamp` / `$timestampMs` | 지금 시각(Unix 초 / 밀리초). 한 요청 안에서는 같은 값 |
// | `$isoTimestamp` / `$httpDate` | `2026-11-12T08:00:00Z` / `Thu, 12 Nov 2026 08:00:00 GMT` |
// | `$uuid` / `$randomInt` | 무작위 UUID / 0~999. 같은 값을 여러 번 쓰려면 변수에 담는다 |
// | `$method` `$url` `$host` `$path` `$query` `$pathQuery` `$body` | 이 요청의 부분들 |
// | `$env 이름` | 환경 변수(비밀키를 파일에 적지 않으려 할 때) |
// | `$base64 값` / `$base64url 값` | Base64로 바꾼 글 |
// | `$hash 알고리즘 표기 값` | 해시. 알고리즘: sha256, sha1, sha384, sha512, md5 |
// | `$hmac 알고리즘 표기 키 메시지 [key=hex 또는 key=base64]` | HMAC 서명. 표기: hex, hexupper, base64, base64url |

/// 값을 채울 때의 기준 시각(Unix 밀리초).
#[derive(Debug, Clone, Copy)]
pub struct Now(pub u128);

impl Now {
    pub fn system() -> Self {
        Now(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis()))
    }

    /// (년, 월, 일, 시, 분, 초, 요일 0=일요일) — 협정 세계시.
    fn civil(self) -> (i64, u32, u32, u32, u32, u32, usize) {
        let secs = (self.0 / 1000) as i64;
        let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400) as u32);
        // 날짜 계산: Howard Hinnant의 civil_from_days.
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let year = yoe + era * 400 + i64::from(month <= 2);
        (year, month, day, rest / 3600, rest / 60 % 60, rest % 60, (days + 4).rem_euclid(7) as usize)
    }

    fn iso(self) -> String {
        let (y, mo, d, h, mi, s, _) = self.civil();
        format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
    }

    fn http_date(self) -> String {
        const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
        const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
        let (y, mo, d, h, mi, s, weekday) = self.civil();
        format!("{}, {d:02} {} {y:04} {h:02}:{mi:02}:{s:02} GMT", DAYS[weekday], MONTHS[mo as usize - 1])
    }
}

/// 무작위 64비트(표준 라이브러리의 해시 씨앗을 빌린다).
fn random_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    std::collections::hash_map::RandomState::new().build_hasher().finish()
}

fn uuid_v4() -> String {
    let (a, b) = (random_u64(), random_u64());
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        a >> 32,
        (a >> 16) & 0xffff,
        a & 0xfff,
        0x8000 | ((b >> 48) & 0x3fff),
        b & 0xffff_ffff_ffff
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok()).collect()
}

/// 계산 결과를 글로 적는다.
fn render(bytes: &[u8], format: &str) -> Result<String, String> {
    use base64::Engine;
    match format.to_ascii_lowercase().as_str() {
        "hex" => Ok(hex(bytes)),
        "hexupper" => Ok(hex(bytes).to_ascii_uppercase()),
        "base64" => Ok(base64::engine::general_purpose::STANDARD.encode(bytes)),
        "base64url" => Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)),
        other => Err(format!("모르는 표기입니다: {other} (hex, hexupper, base64, base64url 중 하나)")),
    }
}

const ALGORITHMS: &str = "sha256, sha1, sha384, sha512, md5 중 하나";

fn digest(algorithm: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    use sha2::Digest;
    Ok(match algorithm.to_ascii_lowercase().as_str() {
        "sha256" => sha2::Sha256::digest(data).to_vec(),
        "sha384" => sha2::Sha384::digest(data).to_vec(),
        "sha512" => sha2::Sha512::digest(data).to_vec(),
        "sha1" => sha1::Sha1::digest(data).to_vec(),
        "md5" => md5::Md5::digest(data).to_vec(),
        other => return Err(format!("모르는 알고리즘입니다: {other} ({ALGORITHMS})")),
    })
}

fn hmac(algorithm: &str, key: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    use hmac::{Hmac, Mac};
    macro_rules! sign {
        ($hash:ty) => {{
            let mut mac = <Hmac<$hash>>::new_from_slice(key).map_err(|e| e.to_string())?;
            mac.update(data);
            mac.finalize().into_bytes().to_vec()
        }};
    }
    Ok(match algorithm.to_ascii_lowercase().as_str() {
        "sha256" => sign!(sha2::Sha256),
        "sha384" => sign!(sha2::Sha384),
        "sha512" => sign!(sha2::Sha512),
        "sha1" => sign!(sha1::Sha1),
        "md5" => sign!(md5::Md5),
        other => return Err(format!("모르는 알고리즘입니다: {other} ({ALGORITHMS})")),
    })
}

/// `{{` 뒤의 글에서, 짝이 맞는 `}}`의 위치. 안쪽에 겹쳐 쓴 `{{…}}`는 건너뛴다.
/// 내장 값(`$`로 시작)에서는 "따옴표 글" 안의 `}}`를 닫는 표시로 보지 않는다.
fn matching_close(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let quoting = text.trim_start().starts_with('$');
    let (mut depth, mut i) = (0usize, 0usize);
    while i + 1 < bytes.len() {
        if quoting && bytes[i] == b'"' {
            // 닫는 따옴표 뒤로 건너뛴다. 닫는 따옴표가 없으면 따옴표가 아닌 글자로 본다.
            i += bytes[i + 1..].iter().position(|b| *b == b'"').map_or(1, |close| close + 2);
            continue;
        }
        match (bytes[i], bytes[i + 1]) {
            (b'{', b'{') => {
                depth += 1;
                i += 2;
            }
            (b'}', b'}') if depth == 0 => return Some(i),
            (b'}', b'}') => {
                depth -= 1;
                i += 2;
            }
            _ => i += 1,
        }
    }
    None
}

/// 내장 값의 인자를 나눈다: 공백으로 구분하고, "따옴표 글"은 하나로 본다(따옴표째로 돌려준다).
fn tokens(text: &str) -> Result<Vec<&str>, String> {
    let mut found = Vec::new();
    let mut rest = text.trim_start();
    while !rest.is_empty() {
        let end = match rest.strip_prefix('"') {
            Some(quoted) => {
                let end = quoted.find('"').ok_or("닫는 따옴표가 없습니다")? + 2;
                if rest[end..].chars().next().is_some_and(|c| !c.is_whitespace()) {
                    return Err("따옴표 글 뒤에 다른 글자가 붙어 있습니다(인자는 공백으로 나눕니다)".into());
                }
                end
            }
            None => rest.find(char::is_whitespace).unwrap_or(rest.len()),
        };
        found.push(&rest[..end]);
        rest = rest[end..].trim_start();
    }
    Ok(found)
}

const MAX_DEPTH: usize = 12;
/// 내장 값의 이름들.
const BUILTINS: [&str; 18] = [
    "timestamp", "timestampMs", "isoTimestamp", "httpDate", "uuid", "random.uuid", "randomInt", "method", "url", "host",
    "path", "query", "pathQuery", "body", "env", "base64", "base64url", "hash",
];
const HMAC: &str = "hmac";

/// `{{…}}` 하나를 채운 결과.
enum Filled {
    Value(String),
    /// 우리 내장 값인데 계산하지 못했다(까닭은 `problems`에 적었다). 적힌 그대로 남긴다.
    Failed,
    /// 변수도 내장 값도 아니다(다른 도구의 틀 문법이거나 그냥 중괄호). 건드리지 않는다.
    NotOurs,
}

/// 요청 하나의 값을 채우는 자리. 변수는 요청마다 한 번만 계산해 같은 값을 쓴다.
struct Scope<'a> {
    vars: &'a HashMap<String, String>,
    cache: HashMap<String, String>,
    /// 지금 값을 계산하고 있는 변수들(자기 자신이나 서로를 쓰는 변수를 알아채는 데 쓴다).
    resolving: Vec<String>,
    now: Now,
    method: &'a str,
    url: Option<Url>,
    body: Option<String>,
    problems: Vec<String>,
    secrets: Vec<String>,
}

impl<'a> Scope<'a> {
    fn new(vars: &'a HashMap<String, String>, now: Now, method: &'a str) -> Self {
        Self {
            vars,
            cache: HashMap::new(),
            resolving: Vec::new(),
            now,
            method,
            url: None,
            body: None,
            problems: Vec::new(),
            secrets: Vec::new(),
        }
    }

    /// 글 속의 `{{…}}`를 값으로 바꾼다. 변수도 내장 값도 아닌 것은 적힌 그대로 둔다.
    fn expand(&mut self, text: &str, depth: usize) -> String {
        if depth > MAX_DEPTH || !text.contains("{{") {
            return text.to_string();
        }
        let mut out = String::new();
        let mut rest = text;
        while let Some(open) = rest.find("{{") {
            out.push_str(&rest[..open]);
            let after = &rest[open + 2..];
            // 우리 것이 아니면 `{` 한 글자만 넘기고 그다음부터 다시 본다. 통째로 건너뛰면 `{"a":{{{x}}}}`처럼
            // JSON 중괄호와 붙어 있는 변수나, 떠도는 `{{` 뒤의 변수를 놓친다.
            let skip_one = |out: &mut String| {
                out.push('{');
                &rest[open + 1..]
            };
            let Some(close) = matching_close(after) else {
                if let Some(name) = after.trim_start().strip_prefix('$').map(builtin_name) {
                    if is_builtin(name) {
                        self.problems.push(format!("{{{{${name} …: 닫는 `}}}}`가 없습니다"));
                    }
                }
                rest = skip_one(&mut out);
                continue;
            };
            let inner = &after[..close];
            match self.fill(inner.trim(), depth) {
                Filled::Value(value) => {
                    out.push_str(&value);
                    rest = &after[close + 2..];
                }
                Filled::Failed => {
                    out.push_str("{{");
                    out.push_str(inner);
                    out.push_str("}}");
                    rest = &after[close + 2..];
                }
                Filled::NotOurs => rest = skip_one(&mut out),
            }
        }
        out.push_str(rest);
        out
    }

    /// `{{…}}` 안에 적힌 것을 채운다.
    fn fill(&mut self, expression: &str, depth: usize) -> Filled {
        let Some(call) = expression.strip_prefix('$') else {
            return self.variable(expression, depth).map_or(Filled::NotOurs, Filled::Value);
        };
        let name = builtin_name(call);
        if !is_builtin(name) {
            // 다른 도구의 틀 문법(`{{ $labels.instance }}`, `{{$json.name}}`, `{{$random.integer(1, 9)}}` 등)은 그대로 보낸다.
            // 점·괄호가 없는 단순한 이름은 내장 값 이름을 잘못 적은 것으로 보고 알린다.
            if name.is_empty() || name.contains(['.', '(', '[']) {
                return Filled::NotOurs;
            }
            self.problems.push(format!("{{{{{expression}}}}}: 모르는 내장 값입니다: ${name}"));
            return Filled::Failed;
        }
        match self.builtin(name, &call[name.len()..], depth) {
            Ok(value) => Filled::Value(value),
            Err(problem) => {
                self.problems.push(format!("{{{{{expression}}}}}: {problem}"));
                Filled::Failed
            }
        }
    }

    fn variable(&mut self, name: &str, depth: usize) -> Option<String> {
        if let Some(value) = self.cache.get(name) {
            return Some(value.clone());
        }
        // 자기 자신이나 서로를 쓰는 변수(`@a = {{a}}/v1`)는 더 풀지 않고 적힌 그대로 둔다.
        if depth > MAX_DEPTH || self.resolving.iter().any(|busy| busy == name) {
            return None;
        }
        let raw = self.vars.get(name)?;
        self.resolving.push(name.to_string());
        let value = self.expand(raw, depth + 1);
        self.resolving.pop();
        self.cache.insert(name.to_string(), value.clone());
        Some(value)
    }

    /// 인자 하나의 값: "따옴표 글"(안의 `{{…}}`는 채운다), `$내장값`, 변수 이름.
    /// 변수 이름을 잘못 적은 것을 그 글자 그대로 키나 메시지로 쓰지 않도록, 없는 이름은 오류로 본다.
    fn argument(&mut self, token: &str, depth: usize) -> Result<String, String> {
        if let Some(quoted) = token.strip_prefix('"') {
            return Ok(self.expand(quoted.strip_suffix('"').unwrap_or(quoted), depth + 1));
        }
        if let Some(call) = token.strip_prefix('$') {
            if !is_builtin(call) {
                return Err(format!("모르는 내장 값입니다: ${call}"));
            }
            return self.builtin(call, "", depth + 1);
        }
        self.variable(token, depth + 1)
            .ok_or_else(|| format!("변수 {token}이(가) 없습니다. 글자 그대로 쓰려면 \"{token}\"처럼 따옴표로 감싸세요"))
    }

    fn builtin(&mut self, name: &str, rest: &str, depth: usize) -> Result<String, String> {
        let args = tokens(rest)?;
        let count = |min: usize, max: usize, usage: &str| {
            if (min..=max).contains(&args.len()) { Ok(()) } else { Err(format!("쓰는 법: {{{{${usage}}}}}")) }
        };
        let url = |scope: &Self| scope.url.clone().ok_or_else(|| "이 자리에서는 아직 쓸 수 없는 값입니다".to_string());
        match name {
            "env" => {
                count(1, 1, "env 이름")?;
                let value = std::env::var(args[0]).map_err(|_| format!("환경 변수 {}가 없습니다", args[0]))?;
                self.secrets.push(value.clone());
                return Ok(value);
            }
            "base64" | "base64url" => {
                count(1, 1, "base64 값")?;
                let value = self.argument(args[0], depth)?;
                return render(value.as_bytes(), name);
            }
            "hash" => {
                count(3, 3, "hash sha256 hex 값")?;
                let value = self.argument(args[2], depth)?;
                return render(&digest(args[0], value.as_bytes())?, args[1]);
            }
            HMAC => {
                count(4, 5, "hmac sha256 hex 키 메시지")?;
                let key = self.argument(args[2], depth)?;
                let message = self.argument(args[3], depth)?;
                let key = match args.get(4).copied() {
                    None | Some("key=text") => key.into_bytes(),
                    Some("key=hex") => unhex(&key).ok_or("키가 16진수 글이 아닙니다")?,
                    Some("key=base64") => {
                        use base64::Engine;
                        base64::engine::general_purpose::STANDARD.decode(key.trim()).map_err(|_| "키가 Base64 글이 아닙니다")?
                    }
                    Some(other) => return Err(format!("모르는 옵션입니다: {other} (key=text, key=hex, key=base64 중 하나)")),
                };
                return render(&hmac(args[0], &key, message.as_bytes())?, args[1]);
            }
            _ => count(0, 0, name)?, // 아래는 인자를 받지 않는 값들
        }
        match name {
            "timestamp" => Ok((self.now.0 / 1000).to_string()),
            "timestampMs" => Ok(self.now.0.to_string()),
            "isoTimestamp" => Ok(self.now.iso()),
            "httpDate" => Ok(self.now.http_date()),
            "uuid" | "random.uuid" => Ok(uuid_v4()),
            "randomInt" => Ok((random_u64() % 1000).to_string()),
            "method" => Ok(self.method.to_string()),
            // 주소의 부분들은 실제로 전송되는 모양 그대로다(인코딩된 경로·쿼리, 기본 포트는 빠진 호스트).
            "url" => url(self).map(|url| url.to_string()),
            "host" => url(self).map(|url| {
                let host = url.host_str().unwrap_or_default();
                url.port().map_or_else(|| host.to_string(), |port| format!("{host}:{port}"))
            }),
            "path" => url(self).map(|url| url.path().to_string()),
            "query" => url(self).map(|url| url.query().unwrap_or_default().to_string()),
            "pathQuery" => url(self).map(|url| match url.query() {
                Some(query) => format!("{}?{query}", url.path()),
                None => url.path().to_string(),
            }),
            "body" => self.body.clone().ok_or_else(|| "이 자리에서는 아직 쓸 수 없는 값입니다".to_string()),
            other => Err(format!("모르는 내장 값입니다: ${other}")),
        }
    }
}

/// `$` 뒤에 적힌 내장 값의 이름(첫 공백 앞까지).
fn builtin_name(call: &str) -> &str {
    call.split(char::is_whitespace).next().unwrap_or("")
}

fn is_builtin(name: &str) -> bool {
    name == HMAC || BUILTINS.contains(&name)
}

fn masked_header<'a>(key: &str, value: &'a str) -> &'a str {
    if SECRET_HEADERS.contains(&key.to_ascii_lowercase().as_str()) {
        "***"
    } else {
        value
    }
}

/// 실행 한 번의 기록. 그 자체로 `.http` 형식이라 요청 부분은 다시 실행할 수 있고,
/// 응답은 `<>` 줄 아래 주석으로 남는다. 인증 헤더 값과 환경 변수에서 읽은 값은 `***`로 가린다.
/// `final_url`은 리다이렉트를 따라가 응답을 받은 주소다(요청 주소와 같으면 None). `<>` 줄 끝에 `→ 주소`로 적는다.
pub fn record(
    request: &HttpRequest,
    when: &str,
    status: &str,
    elapsed_ms: u64,
    final_url: Option<&str>,
    response_headers: &[(String, String)],
    response_body: &str,
) -> String {
    let mut out = format!("### {when} {}\n{} {}\n", request.masked(&request.name), request.method, request.masked(&request.url));
    for (key, value) in &request.headers {
        out.push_str(&format!("{key}: {}\n", masked_header(key, &request.masked(value))));
    }
    if !request.body.is_empty() {
        out.push_str(&format!("\n{}\n", request.masked(&request.body)));
    }
    out.push_str(&format!("\n<> {status} · {elapsed_ms}ms"));
    if let Some(url) = final_url {
        out.push_str(&format!(" → {}", request.masked(url)));
    }
    out.push('\n');
    for (key, value) in response_headers {
        out.push_str(&format!("# {key}: {}\n", masked_header(key, value)));
    }
    out.push_str("#\n");
    let shown: String = response_body.chars().take(MAX_RECORD_BODY).collect();
    for line in request.masked(&shown).lines() {
        out.push_str(&format!("# {line}\n"));
    }
    if response_body.chars().count() > MAX_RECORD_BODY {
        out.push_str("# … (이하 생략)\n");
    }
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 기본_요청_파일을_읽는다() {
        let requests = parse_at(TEMPLATE, Now(1_794_470_400_000));
        assert_eq!(requests.len(), 3);
        assert!(requests.iter().all(|r| r.problems.is_empty()), "{requests:?}");
        // 서명 예시: 시각.메서드.경로와 쿼리.본문을 비밀키로 서명한다.
        let signed = &requests[2];
        assert_eq!((signed.name.as_str(), signed.url.as_str()), ("서명(HMAC) 예시", "https://httpbin.org/post?demo=hmac"));
        let message = format!("1794470400.POST./post?demo=hmac.{}", signed.body);
        let signature = signed.headers.iter().find(|(key, _)| key == "X-Signature").unwrap();
        assert_eq!(signature.1, hex(&hmac("sha256", b"sample-secret-key", message.as_bytes()).unwrap()));
        assert_eq!(requests[0].name, "조회 예시");
        assert_eq!((requests[0].method.as_str(), requests[0].url.as_str()), ("GET", "https://httpbin.org/get?name=test"));
        assert_eq!(requests[0].headers, vec![("Accept".to_string(), "application/json".to_string())]);
        assert_eq!(requests[1].method, "POST");
        assert_eq!(requests[1].body, "{\n  \"message\": \"안녕하세요\"\n}");
    }

    #[test]
    fn 변수와_이름_없는_요청과_버전_표기() {
        let text = "@base = https://a.example\n@api = {{base}}/v1\n@token = abc\n\nhttps://a.example/ping\n\n###\n# 설명\nDELETE {{api}}/items/3 HTTP/1.1\nAuthorization: Bearer {{token}}\n";
        let requests = parse(text);
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].name, "GET https://a.example/ping");
        assert_eq!(requests[1].url, "https://a.example/v1/items/3");
        assert_eq!(requests[1].headers[0].1, "Bearer abc");
        assert_eq!(requests[1].name, "DELETE https://a.example/v1/items/3");
    }

    #[test]
    fn 기록은_다시_읽어도_요청만_나온다() {
        let request = HttpRequest {
            name: "로그인".into(),
            method: "POST".into(),
            url: "https://a.example/login".into(),
            headers: vec![("Content-Type".into(), "application/json".into()), ("Authorization".into(), "Bearer 비밀".into())],
            body: "{\"id\":\"kim\"}".into(),
            line: 0,
            end: 5,
            problems: Vec::new(),
            secrets: Vec::new(),
        };
        let text = record(
            &request,
            "2026-11-11 22:40:12",
            "200 OK",
            132,
            None,
            &[("content-type".into(), "application/json".into()), ("set-cookie".into(), "sid=1".into())],
            "{\"ok\":true}\n### 응답 속 구분선처럼 보이는 줄",
        );
        assert!(text.contains("<> 200 OK · 132ms"));
        assert!(text.contains("# {\"ok\":true}"));
        assert!(!text.contains("비밀") && !text.contains("sid=1")); // 인증 정보는 남기지 않는다.

        let again = parse(&text);
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].name, "2026-11-11 22:40:12 로그인");
        assert_eq!(again[0].body, request.body);
        assert_eq!(again[0].headers[1], ("Authorization".to_string(), "***".to_string()));
    }

    #[test]
    fn 커서가_있는_줄의_요청을_찾는다() {
        let requests = parse(TEMPLATE);
        let index_at = |requests: &[HttpRequest], line| index_at(TEMPLATE, requests, line);
        let lines: Vec<&str> = TEMPLATE.lines().collect();
        let first = lines.iter().position(|l| l.starts_with("### 조회")).unwrap();
        let second = lines.iter().position(|l| l.starts_with("### 전송")).unwrap();
        assert_eq!((requests[0].line, requests[1].line), (first, second));
        assert_eq!(index_at(&requests, 0), Some(0)); // 맨 위 설명 부분
        assert_eq!(index_at(&requests, first + 1), Some(0));
        assert_eq!(index_at(&requests, second), Some(1));
        assert_eq!(index_at(&requests, lines.len() - 1), Some(2));
        assert_eq!(index_at(&requests, 999), None); // 파일 밖의 줄
        assert_eq!(index_at(&[], 3), None);
    }

    #[test]
    fn 값을_못_채운_요청_자리에서_다른_요청이_실행되지_않는다() {
        // "조회"의 주소는 변수 오타로 채워지지 않는다. 그 줄에서 실행해도 앞의 "지우기"가 대신 나가면 안 된다.
        let text = "@host = https://a.example\n### 지우기\nDELETE {{host}}/items/1\n\n### 조회\nGET {{hsot}}/items\n\n### 설명만\n그냥 글자\n\n### 상대 주소\nGET /items\n";
        let requests = parse(text);
        let index_at = |requests: &[HttpRequest], line| index_at(text, requests, line);
        let names: Vec<&str> = requests.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["지우기", "조회"]);
        assert!(requests[0].problems.is_empty());
        assert!(requests[1].problems[0].contains("올바른 주소가 아닙니다"), "{:?}", requests[1].problems);
        assert_eq!(index_at(&requests, 2), Some(0));
        assert_eq!(index_at(&requests, 5), Some(1)); // "조회"의 요청 줄
        assert_eq!(index_at(&requests, 8), None); // 요청이 아닌 구역
        assert_eq!(index_at(&requests, 11), None); // 주소가 아닌 요청 줄
        // 환경 변수가 없어 주소를 못 채운 경우도 같다.
        let text = "### 가\nGET https://a.example/1\n### 나\nGET {{$env CYPHER_없는_환경_변수}}/2\n";
        let requests = parse(text);
        assert_eq!(requests.len(), 2);
        assert_eq!(super::index_at(text, &requests, 3), Some(1));
        assert!(!requests[1].problems.is_empty());
    }

    #[test]
    fn 첫_요청이_잘못_적혀_있으면_그_자리에서_다음_요청이_실행되지_않는다() {
        let picks = |text: &str, lines: std::ops::Range<usize>| -> Vec<Option<usize>> {
            let requests = parse(text);
            lines.map(|line| index_at(text, &requests, line)).collect()
        };
        // "조회"는 http://가 빠져 요청이 되지 못했다. 그 구역이나 머리말에서 실행해도 뒤의 "지우기"가 나가면 안 된다.
        let text = "# 설명\n@host = https://a.example\n\n### 조회\nGET localhost:8080/items\n\n### 지우기\nDELETE {{host}}/items/1\n";
        assert_eq!(parse(text).iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["지우기"]);
        assert_eq!(picks(text, 0..6), [None; 6]);
        assert_eq!(picks(text, 6..9), [Some(0); 3]);
        // 설명만 있는 구역이 첫 요청 앞에 있을 때도 같다.
        let text = "@host = https://a.example\n### 메모\n# 나중에 정리\n\n### 지우기\nDELETE {{host}}/items/1\n";
        assert_eq!(picks(text, 0..5), [None, None, None, None, Some(0)]);
        // 머리말(빈 줄·주석·변수뿐)에서는 바로 아래의 첫 요청을 보낸다.
        let text = "// 설명\n@host = https://a.example\n\n### 조회\nGET {{host}}/items\n\n### 지우기\nDELETE {{host}}/items/1\n";
        assert_eq!(picks(text, 0..7), [Some(0), Some(0), Some(0), Some(0), Some(0), Some(0), Some(1)]);
        // `###` 없이 적은 첫 줄이 요청이 되지 못했으면, 그 위아래는 머리말이 아니다.
        let text = "@host = https://a.example\nlocalhost/items\n\n### 지우기\nDELETE {{host}}/items/1\n";
        assert_eq!(picks(text, 0..4), [None, None, None, Some(0)]);
    }

    #[test]
    fn bom이_붙은_파일도_첫_줄부터_읽는다() {
        // 첫 줄이 변수인 경우
        let text = "\u{feff}@host = https://a.example\n### 조회\nGET {{host}}/items\n";
        let requests = parse(text);
        assert_eq!((requests.len(), requests[0].url.as_str()), (1, "https://a.example/items"));
        assert!(requests[0].problems.is_empty(), "{:?}", requests[0].problems);
        assert_eq!(index_at(text, &requests, 0), Some(0)); // 머리말 판정도 같다.
        // 첫 줄이 요청 이름인 경우. 줄 번호는 BOM이 없을 때와 같다.
        let text = "\u{feff}### 조회\nGET https://a.example/items\n\n### 지우기\nDELETE https://a.example/items/1\n";
        let requests = parse(text);
        assert_eq!(requests.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["조회", "지우기"]);
        assert_eq!((requests[0].line, requests[0].end, requests[1].line), (0, 3, 3));
        assert_eq!(index_at(text, &requests, 0), Some(0));
        // 첫 줄이 요청 줄이거나 주석인 경우
        let requests = parse("\u{feff}GET https://a.example/ping\n");
        assert_eq!((requests.len(), requests[0].method.as_str(), requests[0].url.as_str()), (1, "GET", "https://a.example/ping"));
        let text = "\u{feff}# 설명\n### 조회\nGET https://a.example/items\n";
        assert_eq!(index_at(text, &parse(text), 0), Some(0));
    }

    #[test]
    fn 마지막_요청_아래_빈_줄도_그_요청이다() {
        // 글이 줄 바꿈으로 끝나면 편집기에는 2번 줄(빈 줄)이 있다. 거기서 실행해도 마지막 요청이 나간다.
        for text in ["### 가\nGET https://a.example/1\n", "### 가\r\nGET https://a.example/1\r\n"] {
            let requests = parse(text);
            assert_eq!((requests[0].line, requests[0].end), (0, 3), "{text:?}");
            assert_eq!(index_at(text, &requests, 2), Some(0));
            assert_eq!(index_at(text, &requests, 3), None); // 편집기에 없는 줄
        }
        // 줄 바꿈으로 끝나지 않으면 마지막 글자 줄까지다.
        let text = "### 가\nGET https://a.example/1";
        let requests = parse(text);
        assert_eq!(requests[0].end, 2);
        assert_eq!((index_at(text, &requests, 1), index_at(text, &requests, 2)), (Some(0), None));
        // 앞 요청의 범위는 그대로다. 마지막 구역이 요청이 아니면 그 아래 빈 줄에서도 보내지 않는다.
        let text = "### 가\nGET https://a.example/1\n### 나\nGET https://a.example/2\n\n";
        let requests = parse(text);
        assert_eq!((requests[0].end, requests[1].end), (2, 6));
        assert_eq!(index_at(text, &requests, 5), Some(1));
        let text = "### 가\nGET https://a.example/1\n### 메모\n";
        assert_eq!(index_at(text, &parse(text), 3), None);
    }

    #[test]
    fn 서명에_쓰는_주소_부분은_전송되는_모양과_같다() {
        let text = "GET https://user:pw@A.Example:443/a/../검색?q=한 글\nX: {{$host}}|{{$path}}|{{$query}}|{{$pathQuery}}\nY: {{$url}}\n";
        let request = &parse(text)[0];
        assert!(request.problems.is_empty(), "{:?}", request.problems);
        assert_eq!(request.headers[0].1, "a.example|/%EA%B2%80%EC%83%89|q=%ED%95%9C%20%EA%B8%80|/%EA%B2%80%EC%83%89?q=%ED%95%9C%20%EA%B8%80");
        assert_eq!(request.headers[1].1, request.url);
        // 포트가 기본값이 아니면 호스트에 붙고, 빈 쿼리의 물음표도 전송되는 그대로다.
        let request = &parse("GET http://localhost:8080/p?\nX: {{$host}} {{$pathQuery}}\n")[0];
        assert_eq!(request.headers[0].1, "localhost:8080 /p?");
    }

    #[test]
    fn 치환기의_경계_경우() {
        let one = |text: &str| parse(text).remove(0);
        // 자기 자신·서로를 쓰는 변수는 끝없이 불어나지 않는다.
        let request = one("@a = {{a}}{{a}}{{a}}\n@b = {{c}}{{c}}\n@c = {{b}}{{b}}\nPOST https://a.example/x\n\n{{a}} {{b}}\n");
        assert!(request.body.len() < 200, "{}", request.body.len());
        // JSON 중괄호와 붙은 변수, 떠도는 `{{` 뒤의 변수도 채운다.
        assert_eq!(one("@x = \"k\":1\nPOST https://a.example/x\n\n{\"a\":{{{x}}}}\n").body, "{\"a\":{\"k\":1}}");
        assert_eq!(one("@h = H\nPOST https://a.example/x\n\n{\"t\": \"{{\", \"v\": {\"w\": \"{{h}}\"}}\n").body, "{\"t\": \"{{\", \"v\": {\"w\": \"H\"}}");
        // 다른 도구의 틀 문법은 건드리지 않고 그대로 보낸다.
        let foreign = "{\"summary\": \"{{ $labels.instance }} down\", \"n\": \"{{ $json.name }}\", \"r\": \"{{$random.integer(1, 9)}}\"}";
        let request = one(&format!("POST https://a.example/x\n\n{foreign}\n"));
        assert_eq!((request.body.as_str(), request.problems.len()), (foreign, 0));
        // 따옴표 글 안의 `}}`는 닫는 표시가 아니다.
        assert_eq!(one("GET https://a.example/x\nA: {{$base64 \"a}}b\"}}\n").headers[0].1, "YX19Yg==");
        // 잘못 적은 인자는 조용히 다른 값이 되지 않고 문제로 알린다.
        for wrong in [
            "{{$hash sha256 hex \"hello\" \"world\"}}", // 인자가 남음
            "{{$base64 \"abc\"def}}",                    // 따옴표 뒤에 글자
            "{{$base64 \"abc}}",                         // 닫는 따옴표 없음
            "{{$hmac sha256 hex secrte \"msg\"}}",      // 없는 변수 이름
            "{{$timestamp 3}}",                          // 인자를 받지 않는 값
            "{{$timestmap}}",                            // 이름 오타
        ] {
            let request = one(&format!("@secret = k\nGET https://a.example/x\nA: {wrong}\n"));
            assert_eq!(request.problems.len(), 1, "{wrong}: {:?}", request.problems);
        }
        // 시각이 든 이름 없는 요청은 읽을 때마다 이름이 달라지지 않는다.
        let text = "GET https://a.example/x?ts={{$timestamp}}\n";
        assert_eq!(parse_at(text, Now(1_000_000))[0].name, parse_at(text, Now(2_000_000))[0].name);
        assert_ne!(parse_at(text, Now(1_000_000))[0].url, parse_at(text, Now(2_000_000))[0].url);
    }

    #[test]
    fn 환경_변수에서_읽은_값은_기록에_남기지_않는다() {
        std::env::set_var("CYPHER_TEST_SECRET", "s3cr3t-value");
        let request = parse("GET https://a.example/x?api_key={{$env CYPHER_TEST_SECRET}}\nX-Auth-Token: {{$env CYPHER_TEST_SECRET}}\n").remove(0);
        assert!(request.url.contains("s3cr3t-value")); // 실제 전송에는 들어간다.
        let text = record(&request, "2026-11-12 08:00:00", "200 OK", 1, None, &[], "echo s3cr3t-value");
        assert!(!text.contains("s3cr3t-value"), "{text}");
        // 리다이렉트를 따라간 최종 주소는 상태 줄에 남고, 거기 든 비밀 값도 가린다. 다시 읽어도 요청은 하나다.
        let moved = "https://b.example/x?api_key=s3cr3t-value";
        let text = record(&request, "2026-11-12 08:00:00", "200 OK", 1, Some(moved), &[], "");
        assert!(text.contains("\n<> 200 OK · 1ms → https://b.example/x?api_key=***\n"), "{text}");
        assert_eq!(parse(&text).len(), 1);
        assert!(text.contains("api_key=***") && text.contains("X-Auth-Token: ***"));
        assert_eq!(request.masked("주소 https://a.example/x?api_key=s3cr3t-value"), "주소 https://a.example/x?api_key=***");
    }

    #[test]
    fn 내장_값과_hmac_서명() {
        // 2026-11-12 08:00:00 UTC (목요일)
        let now = Now(1_794_470_400_123);
        let text = r#"@host = https://api.example.com
@secret = key
@nonce = {{$uuid}}
@message = {{$timestamp}}{{$method}}{{$pathQuery}}{{$body}}

### 서명
POST {{host}}/v1/orders?side=buy
X-Timestamp: {{$timestamp}}
X-Ms: {{$timestampMs}}
X-Iso: {{$isoTimestamp}}
Date: {{$httpDate}}
X-Nonce: {{nonce}}
X-Nonce-Again: {{nonce}}
X-Host: {{$host}} {{$path}} {{$query}}
X-Body-Hash: {{$hash sha256 hex $body}}
X-Signature: {{$hmac sha256 hex secret message}}
X-Fox: {{$hmac sha256 hex secret "The quick brown fox jumps over the lazy dog"}}
X-Fox64: {{$hmac sha256 base64 "6b6579" "The quick brown fox jumps over the lazy dog" key=hex}}
X-Fox-Sha1: {{$hmac sha1 hexupper secret "The quick brown fox jumps over the lazy dog"}}
X-Basic: Basic {{$base64 "user:{{secret}}"}}
X-Unknown: {{없는변수}}

{"qty": 1, "at": {{$timestamp}}}
"#;
        let request = &parse_at(text, now)[0];
        assert!(request.problems.is_empty(), "{:?}", request.problems);
        let header = |name: &str| request.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str()).unwrap();
        assert_eq!(request.url, "https://api.example.com/v1/orders?side=buy");
        assert_eq!(request.body, r#"{"qty": 1, "at": 1794470400}"#);
        assert_eq!(header("X-Timestamp"), "1794470400");
        assert_eq!(header("X-Ms"), "1794470400123");
        assert_eq!(header("X-Iso"), "2026-11-12T08:00:00Z");
        assert_eq!(header("Date"), "Thu, 12 Nov 2026 08:00:00 GMT");
        assert_eq!(header("X-Host"), "api.example.com /v1/orders side=buy");
        // 변수에 담은 무작위 값은 한 요청 안에서 같다.
        assert_eq!(header("X-Nonce"), header("X-Nonce-Again"));
        assert_eq!(header("X-Nonce").len(), 36);
        // 알려진 값과 대조한다(RFC 예시로 널리 쓰이는 값).
        let fox = "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8";
        assert_eq!(header("X-Fox"), fox);
        assert_eq!(header("X-Fox64"), "97yD9DBThCSxMpjmqm+xQ+9NWaFJRhdZl0edvC0aPNg=");
        assert_eq!(header("X-Fox-Sha1"), "DE7C9B85B8B78AA6BC8A7A36F70A90701C9DB4D9");
        assert_eq!(header("X-Basic"), "Basic dXNlcjprZXk=");
        assert_eq!(header("X-Unknown"), "{{없는변수}}"); // 모르는 변수는 그대로 둔다.
        // 서명 메시지는 시각 + 메서드 + 경로·쿼리 + (값이 채워진) 본문이다.
        let message = format!("1794470400POST/v1/orders?side=buy{}", request.body);
        assert_eq!(header("X-Signature"), hex(&hmac("sha256", b"key", message.as_bytes()).unwrap()));
        assert_eq!(header("X-Body-Hash"), hex(&digest("sha256", request.body.as_bytes()).unwrap()));
        assert_eq!(hex(&digest("sha256", b"").unwrap()), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    #[test]
    fn 계산하지_못한_내장_값은_문제로_알린다() {
        let text = "GET https://a.example/x\nA: {{$hmac sha3 hex \"k\" \"m\"}}\nB: {{$env CYPHER_없는_환경_변수}}\nC: {{$모름}}\nD: {{$hmac sha256 hex \"k\"}}\n";
        let request = &parse(text)[0];
        assert_eq!(request.problems.len(), 4, "{:?}", request.problems);
        assert!(request.problems[0].contains("모르는 알고리즘"));
        assert_eq!(request.headers[0].1, "{{$hmac sha3 hex \"k\" \"m\"}}");
        // 주소 안에서 본문을 쓰는 것처럼 아직 없는 값도 문제다. 자기 자신을 쓰는 변수는 끝없이 돌지 않는다.
        assert!(!parse("GET https://a.example/{{$body}}\n")[0].problems.is_empty());
        assert_eq!(parse("@a = {{a}}\nGET https://a.example/{{a}}\n").len(), 1);
        // JSON 본문의 겹친 중괄호는 변수로 보지 않는다.
        assert_eq!(parse("POST https://a.example/x\n\n{\"a\":{\"b\":{}}}\n")[0].body, "{\"a\":{\"b\":{}}}");
    }

    #[test]
    fn 주소가_아닌_글자는_요청이_아니다() {
        assert!(parse("그냥 메모\n다음 줄").is_empty());
        assert!(parse("### 빈 요청\n# 주석만\n").is_empty());
    }
}
