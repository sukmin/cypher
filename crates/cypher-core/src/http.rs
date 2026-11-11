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
//! 본문은 다음 `###`까지이며, `> ` 또는 `<> `로 시작하는 줄을 만나면 거기서 끝난다
//! (실행 기록의 응답 부분이 다시 읽힐 때 본문으로 섞이지 않게 하는 규칙).

use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HttpRequest {
    pub name: String,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    /// 이 요청이 시작하는 줄(0부터). `###` 줄이거나, 첫 요청이면 파일 첫 줄.
    pub line: usize,
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

fn substitute(text: &str, vars: &HashMap<String, String>) -> String {
    let mut out = text.to_string();
    for (key, value) in vars {
        out = out.replace(&format!("{{{{{key}}}}}"), value);
    }
    out
}

/// 요청 파일의 요청을 모두 읽는다. 변수는 값이 채워진 상태로 돌려준다.
pub fn parse(text: &str) -> Vec<HttpRequest> {
    let mut vars: HashMap<String, String> = text.lines().filter_map(var_line).collect();
    // 변수가 다른 변수를 쓰는 경우(@url = {{host}}/api)를 한 번 풀어 준다.
    let resolved: Vec<(String, String)> = vars.iter().map(|(k, v)| (k.clone(), substitute(v, &vars))).collect();
    vars.extend(resolved);

    let mut requests = Vec::new();
    let mut name = String::new();
    let mut start = 0;
    let mut block: Vec<&str> = Vec::new();
    for (number, line) in text.lines().enumerate() {
        match line.strip_prefix("###") {
            Some(rest) => {
                requests.extend(parse_block(&name, start, &block, &vars));
                name = rest.trim().to_string();
                start = number;
                block.clear();
            }
            None => block.push(line),
        }
    }
    requests.extend(parse_block(&name, start, &block, &vars));
    requests
}

/// 편집기에서 커서가 있는 줄(0부터)이 속한 요청의 번호. 요청보다 위(변수·설명 부분)에 있으면 첫 요청.
pub fn index_at(requests: &[HttpRequest], line: usize) -> Option<usize> {
    if requests.is_empty() {
        return None;
    }
    Some(requests.iter().rposition(|r| r.line <= line).unwrap_or(0))
}

fn parse_block(name: &str, line: usize, block: &[&str], vars: &HashMap<String, String>) -> Option<HttpRequest> {
    let mut lines =
        block.iter().copied().skip_while(|l| l.trim().is_empty() || is_comment(l) || var_line(l).is_some());
    let first = lines.next()?.trim();
    let (method, target) = match first.split_once(char::is_whitespace) {
        Some((m, rest)) if METHODS.contains(&m.to_ascii_uppercase().as_str()) => (m.to_ascii_uppercase(), rest.trim()),
        _ => ("GET".to_string(), first),
    };
    let target = target.rfind(" HTTP/").map_or(target, |at| &target[..at]);
    let url = substitute(target, vars);
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return None; // 주소가 아닌 글자(설명 등)는 요청이 아니다.
    }

    let mut headers = Vec::new();
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
                headers.push((key.trim().to_string(), substitute(value.trim(), vars)));
            }
        }
    }
    while body.last().is_some_and(|l| l.trim().is_empty()) {
        body.pop();
    }
    Some(HttpRequest {
        name: if name.is_empty() { format!("{method} {url}") } else { name.to_string() },
        method,
        url,
        headers,
        body: substitute(&body.join("\n"), vars),
        line,
    })
}

fn masked<'a>(key: &str, value: &'a str) -> &'a str {
    if SECRET_HEADERS.contains(&key.to_ascii_lowercase().as_str()) {
        "***"
    } else {
        value
    }
}

/// 실행 한 번의 기록. 그 자체로 `.http` 형식이라 요청 부분은 다시 실행할 수 있고,
/// 응답은 `<>` 줄 아래 주석으로 남는다. 인증 헤더 값은 `***`로 가린다.
pub fn record(
    request: &HttpRequest,
    when: &str,
    status: &str,
    elapsed_ms: u64,
    response_headers: &[(String, String)],
    response_body: &str,
) -> String {
    let mut out = format!("### {when} {}\n{} {}\n", request.name, request.method, request.url);
    for (key, value) in &request.headers {
        out.push_str(&format!("{key}: {}\n", masked(key, value)));
    }
    if !request.body.is_empty() {
        out.push_str(&format!("\n{}\n", request.body));
    }
    out.push_str(&format!("\n<> {status} · {elapsed_ms}ms\n"));
    for (key, value) in response_headers {
        out.push_str(&format!("# {key}: {}\n", masked(key, value)));
    }
    out.push_str("#\n");
    let shown: String = response_body.chars().take(MAX_RECORD_BODY).collect();
    for line in shown.lines() {
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
        let requests = parse(TEMPLATE);
        assert_eq!(requests.len(), 2);
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
        };
        let text = record(
            &request,
            "2026-11-11 22:40:12",
            "200 OK",
            132,
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
        let lines: Vec<&str> = TEMPLATE.lines().collect();
        let first = lines.iter().position(|l| l.starts_with("### 조회")).unwrap();
        let second = lines.iter().position(|l| l.starts_with("### 전송")).unwrap();
        assert_eq!((requests[0].line, requests[1].line), (first, second));
        assert_eq!(index_at(&requests, 0), Some(0)); // 맨 위 설명 부분
        assert_eq!(index_at(&requests, first + 1), Some(0));
        assert_eq!(index_at(&requests, second), Some(1));
        assert_eq!(index_at(&requests, 999), Some(1));
        assert_eq!(index_at(&[], 3), None);
    }

    #[test]
    fn 주소가_아닌_글자는_요청이_아니다() {
        assert!(parse("그냥 메모\n다음 줄").is_empty());
        assert!(parse("### 빈 요청\n# 주석만\n").is_empty());
    }
}
