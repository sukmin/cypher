//! `.http` 요청 파일 실행과 기록. 화면(옆 창)과 Claude 세션(MCP 도구)이 같이 쓴다.
//!
//! 요청 파일은 세션 폴더의 `.cypher\http\`, `http\`, 또는 폴더 바로 아래의 `*.http`에서 찾는다.
//! 실행하면 요청과 응답을 요청 파일 옆의 `history.http`에 덧붙인다.

use serde::Serialize;
use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use cypher_core::http::{self, HttpRequest};

const HISTORY: &str = "history.http";
const MAX_BODY_BYTES: u64 = 5 << 20;
const MAX_TOOL_TEXT: usize = 15_000;

#[derive(Serialize)]
pub struct HttpFile {
    path: String,
    name: String,
    requests: Vec<HttpRequest>,
}

#[derive(Serialize)]
pub struct HttpFiles {
    /// 새 요청 파일을 만들 위치.
    dir: String,
    files: Vec<HttpFile>,
    /// 요청 파일이 없을 때 편집기에 채워 줄 예시.
    template: &'static str,
}

#[derive(Serialize)]
pub struct HttpResponse {
    /// "200 OK"
    status: String,
    code: u16,
    elapsed_ms: u64,
    headers: Vec<(String, String)>,
    body: String,
    /// 실제로 보낸 요청 원문(변수가 채워진 요청 줄·헤더·본문).
    request: String,
    /// 이 실행을 적어 둔 기록 파일.
    history: String,
}

/// 요청 파일을 글로 읽는다. 저장은 늘 UTF-8로 하지만, 다른 편집기가 다른 인코딩(CP949 등)으로 저장한 파일도 읽는다.
fn read_text(path: &Path) -> Result<String, String> {
    use cypher_core::textfile;
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    // UTF-8이면 손대지 않고 그대로 준다(줄 끝 형식도 그대로).
    let other = match String::from_utf8(bytes) {
        Ok(text) => return Ok(text),
        Err(not_utf8) => not_utf8.into_bytes(),
    };
    let decoded = textfile::decode(&other)?;
    Ok(match decoded.eol {
        textfile::CRLF => decoded.text.replace('\n', "\r\n"),
        textfile::CR => decoded.text.replace('\n', "\r"),
        _ => decoded.text,
    })
}

fn home_dir(cwd: &Path) -> PathBuf {
    cwd.join(".cypher").join("http")
}

/// 폴더에서 요청 파일을 찾는다(기록 파일은 뺀다).
fn find_files(cwd: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for dir in [home_dir(cwd), cwd.join("http"), cwd.to_path_buf()] {
        let Ok(entries) = fs::read_dir(dir) else { continue };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "http"))
            .filter(|p| p.file_name().is_some_and(|n| n != HISTORY))
            .collect();
        files.sort();
        found.extend(files);
    }
    found
}

fn list(cwd: &Path) -> HttpFiles {
    let files = find_files(cwd)
        .into_iter()
        .map(|path| HttpFile {
            name: path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
            requests: read_text(&path).map(|t| http::parse(&t)).unwrap_or_default(),
            path: path.to_string_lossy().into_owned(),
        })
        .collect();
    HttpFiles { dir: home_dir(cwd).to_string_lossy().into_owned(), files, template: http::TEMPLATE }
}

/// 지금 시각(이 컴퓨터의 현지 시각)을 `2026-11-11 22:40:12` 모양으로.
pub(crate) fn now_text() -> String {
    #[repr(C)]
    #[derive(Default)]
    struct SystemTime {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetLocalTime(time: *mut SystemTime);
    }
    let mut t = SystemTime::default();
    // SAFETY: 유효한 구조체 하나를 채워 받는다.
    unsafe { GetLocalTime(&mut t) };
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.year, t.month, t.day, t.hour, t.minute, t.second)
}

/// 요청을 보내고 응답을 받는다. 4xx·5xx도 오류가 아니라 응답으로 돌려준다.
/// 받은 응답: (상태 코드, "200 OK", 헤더, 본문, 걸린 시간 ms)
type Sent = (u16, String, Vec<(String, String)>, String, u64);

/// 헤더 이름이 이미 있는지(대소문자 무시).
fn has_header(headers: &[(String, String)], name: &str) -> bool {
    headers.iter().any(|(key, _)| key.eq_ignore_ascii_case(name))
}

/// 실제로 보낼 헤더: 요청 파일에 적힌 것에, 빠져 있으면 기본 헤더를 더한다.
/// 기본값을 여기서 직접 정해 두어야 화면에 보여 주는 요청 원문과 실제 전송 내용이 같아진다.
fn wire_headers(request: &HttpRequest) -> Vec<(String, String)> {
    let mut headers = request.headers.clone();
    if !has_header(&headers, "User-Agent") {
        headers.push(("User-Agent".into(), concat!("Cypher/", env!("CARGO_PKG_VERSION")).into()));
    }
    if !has_header(&headers, "Accept") {
        headers.push(("Accept".into(), "*/*".into()));
    }
    headers
}

/// 보낸 요청을 HTTP 원문 모양으로 적는다. Host·Accept-Encoding·Content-Length는 전송 계층이 붙이는 값이다.
fn raw_request(request: &HttpRequest, headers: &[(String, String)]) -> String {
    let (target, host) = match tauri::Url::parse(&request.url) {
        Ok(url) => {
            let mut target = url.path().to_string();
            if let Some(query) = url.query() {
                target.push('?');
                target.push_str(query);
            }
            let host = url.host_str().unwrap_or_default();
            (target, url.port().map_or_else(|| host.to_string(), |port| format!("{host}:{port}")))
        }
        Err(_) => (request.url.clone(), String::new()),
    };
    let mut lines = vec![format!("{} {target} HTTP/1.1", request.method)];
    if !has_header(headers, "Host") {
        lines.push(format!("Host: {host}"));
    }
    lines.extend(headers.iter().map(|(key, value)| format!("{key}: {value}")));
    if !has_header(headers, "Accept-Encoding") {
        lines.push("Accept-Encoding: gzip".into());
    }
    if !request.body.is_empty() && !has_header(headers, "Content-Length") {
        lines.push(format!("Content-Length: {}", request.body.len()));
    }
    lines.push(String::new());
    lines.push(request.body.clone());
    lines.join("\n")
}

fn send(request: &HttpRequest, headers: &[(String, String)]) -> Result<Sent, String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(60)).redirects(5).build();
    let mut call = agent.request(&request.method, &request.url);
    for (key, value) in headers {
        call = call.set(key, value);
    }
    let started = Instant::now();
    let result = if request.body.is_empty() { call.call() } else { call.send_string(&request.body) };
    let response = match result {
        Ok(response) | Err(ureq::Error::Status(_, response)) => response,
        Err(e) => return Err(format!("요청을 보내지 못했습니다: {e}")),
    };
    let code = response.status();
    let status = format!("{code} {}", response.status_text());
    let headers: Vec<(String, String)> = response
        .headers_names()
        .into_iter()
        .filter_map(|name| Some((name.clone(), response.header(&name)?.to_string())))
        .collect();
    let mut bytes = Vec::new();
    response.into_reader().take(MAX_BODY_BYTES).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let elapsed = started.elapsed().as_millis() as u64;
    // 응답 글은 UTF-8로 읽고, 아니면 다른 인코딩(CP949 등)으로 읽어 본다.
    let body = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(e) => match cypher_core::textfile::decode(e.as_bytes()) {
            Ok(decoded) if !decoded.lossy => decoded.text,
            _ => format!("(글자가 아닌 데이터 {}바이트)", e.as_bytes().len()),
        },
    };
    Ok((code, status, headers, body, elapsed))
}

/// 보기 좋게: JSON이면 들여쓴다.
fn pretty(body: &str) -> String {
    match serde_json::from_str::<Value>(body) {
        Ok(value) if value.is_object() || value.is_array() => serde_json::to_string_pretty(&value).unwrap_or_else(|_| body.to_string()),
        _ => body.to_string(),
    }
}

/// 요청 파일에서 이름(또는 1부터 센 번호)으로 고른 요청을 실행하고 기록한다.
pub fn run(file: &Path, pick: &str) -> Result<HttpResponse, String> {
    let text = read_text(file).map_err(|e| format!("요청 파일을 읽지 못했습니다: {e}"))?;
    let requests = http::parse(&text);
    let request = requests
        .iter()
        .find(|r| r.name == pick)
        .or_else(|| pick.trim().parse::<usize>().ok().and_then(|n| requests.get(n.checked_sub(1)?)))
        .ok_or_else(|| {
            let names: Vec<&str> = requests.iter().map(|r| r.name.as_str()).collect();
            format!("'{pick}' 요청이 없습니다. 이 파일의 요청: {}", names.join(", "))
        })?;
    execute(file, request)
}

/// 편집기에서 커서가 있는 줄(0부터)의 요청을 실행하고 기록한다.
pub fn run_at(file: &Path, line: usize) -> Result<HttpResponse, String> {
    let text = read_text(file).map_err(|e| format!("요청 파일을 읽지 못했습니다: {e}"))?;
    let requests = http::parse(&text);
    let index = http::index_at(&requests, line)
        .ok_or("이 줄에는 보낼 수 있는 요청이 없습니다. '### 이름' 아래에 'GET https://…'처럼 http:// 또는 https://로 시작하는 주소(또는 그런 주소가 되는 변수)를 적어 주세요.")?;
    execute(file, &requests[index])
}

fn execute(file: &Path, request: &HttpRequest) -> Result<HttpResponse, String> {
    // 서명 같은 내장 값을 계산하지 못했으면 엉뚱한 값으로 보내지 않고 까닭을 알린다.
    if !request.problems.is_empty() {
        return Err(format!("요청을 보내지 않았습니다. 값을 채우지 못한 곳이 있습니다:\n{}", request.problems.join("\n")));
    }
    let sent_headers = wire_headers(request);
    let raw = raw_request(request, &sent_headers);
    let (code, status, headers, body, elapsed_ms) = send(request, &sent_headers)?;
    let body = pretty(&body);

    let history = file.with_file_name(HISTORY);
    let entry = http::record(request, &now_text(), &status, elapsed_ms, &headers, &body);
    let appended = fs::OpenOptions::new().create(true).append(true).open(&history).and_then(|mut f| f.write_all(entry.as_bytes()));
    if appended.is_ok() {
        // 기록에는 응답 내용이 담기므로 저장소에 올라가지 않게 해 둔다.
        let ignore = file.with_file_name(".gitignore");
        if !ignore.exists() {
            let _ = fs::write(ignore, format!("{HISTORY}\n"));
        }
    }
    Ok(HttpResponse { status, code, elapsed_ms, headers, body, request: raw, history: history.to_string_lossy().into_owned() })
}

// ───────────────────────── 화면에서 부르는 명령 ─────────────────────────

#[tauri::command]
pub async fn http_files(cwd: String) -> HttpFiles {
    list(Path::new(&cwd))
}


/// 이 폴더에서 읽을 수 있는 파일인지: 찾은 요청 파일이거나 그 옆의 기록 파일.
fn readable(cwd: &Path, file: &Path) -> bool {
    let files = find_files(cwd);
    files.iter().any(|p| p == file)
        || (file.file_name().is_some_and(|n| n == HISTORY)
            && [home_dir(cwd), cwd.join("http"), cwd.to_path_buf()].iter().any(|dir| file.parent() == Some(dir)))
}

/// 저장할 곳을 정한다. 이미 있는 요청 파일은 이 폴더에서 찾은 것만 덮어쓸 수 있고,
/// 새 파일은 `.cypher\http\<이름>.http`에 만든다.
fn save_target(cwd: &Path, file: Option<&str>, name: Option<&str>) -> Result<PathBuf, String> {
    if let Some(file) = file {
        let path = PathBuf::from(file);
        return find_files(cwd).into_iter().find(|p| *p == path).ok_or_else(|| "이 폴더의 요청 파일이 아닙니다".to_string());
    }
    let stem = name.unwrap_or("requests").trim().trim_end_matches(".http");
    let valid = !stem.is_empty()
        && stem != "history"
        && !stem.starts_with('.')
        && stem.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ' '));
    if !valid {
        return Err("파일 이름에는 글자·숫자·공백·`-`·`_`·`.`만 쓸 수 있습니다".into());
    }
    let target = home_dir(cwd).join(format!("{stem}.http"));
    // 새 이름을 정해 만드는 경우, 이미 있는 파일을 예시 내용으로 덮어쓰지 않는다.
    if name.is_some() && target.exists() {
        return Err(format!("같은 이름의 요청 파일이 이미 있습니다: {stem}.http (위쪽 목록에서 그 파일을 고르세요)"));
    }
    Ok(target)
}

/// 요청 파일(또는 기록 파일)의 내용. 기록이 길면 끝부분만 준다.
#[tauri::command]
pub async fn http_read(cwd: String, file: String) -> Result<String, String> {
    let (cwd, file) = (Path::new(&cwd), Path::new(&file));
    if !readable(cwd, file) {
        return Err("이 폴더의 요청 파일이 아닙니다".into());
    }
    let text = read_text(file)?;
    // 기록 파일만 끝부분을 잘라 준다. 요청 파일은 편집기에 통째로 들어가야 한다(잘린 채 저장되면 안 된다).
    const MAX: usize = 300_000;
    if text.len() <= MAX || file.file_name().is_none_or(|name| name != HISTORY) {
        return Ok(text);
    }
    let mut cut = text.len() - MAX;
    while !text.is_char_boundary(cut) {
        cut += 1;
    }
    Ok(format!("… (앞부분 생략)\n{}", &text[cut..]))
}

/// 편집기 내용을 저장한다. `file`이 없으면 새 파일을 만든다. 저장한 경로를 돌려준다.
#[tauri::command]
pub async fn http_save(cwd: String, file: Option<String>, name: Option<String>, text: String) -> Result<String, String> {
    let target = save_target(Path::new(&cwd), file.as_deref(), name.as_deref())?;
    if let Some(dir) = target.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    fs::write(&target, text).map_err(|e| e.to_string())?;
    Ok(target.to_string_lossy().into_owned())
}

/// 편집기에서 커서가 있는 줄의 요청을 보낸다.
#[tauri::command]
pub async fn http_run_at(file: String, line: usize) -> Result<HttpResponse, String> {
    run_at(Path::new(&file), line)
}

// ───────────────────────── Claude가 쓰는 도구 ─────────────────────────

pub fn tools() -> Value {
    json!([
        { "name": "http_list",
          "description": "폴더의 .http 요청 파일(IntelliJ HTTP Client 형식)과 그 안의 요청 이름을 나열한다. 사용자가 저장해 둔 요청을 쓰기 전에 부른다. 지난 실행의 요청·응답은 같은 폴더의 history.http에 있다.",
          "inputSchema": { "type": "object", "properties": { "folder": { "type": "string", "description": "작업 폴더의 절대 경로" } }, "required": ["folder"] } },
        { "name": "http_run",
          "description": "요청 파일의 요청 하나를 실제로 보내고 응답을 돌려준다. 실행 내용은 history.http에 기록된다. 새 요청이 필요하면 요청 파일에 '### 이름' 블록을 추가한 뒤 실행한다. 서명이 필요한 API는 {{$timestamp}}, {{$body}}, {{$hmac sha256 hex 키변수 메시지변수}} 같은 내장 값을 쓴다.",
          "inputSchema": { "type": "object", "properties": {
              "file": { "type": "string", "description": ".http 파일의 절대 경로" },
              "name": { "type": "string", "description": "요청 이름('###' 뒤의 글자) 또는 1부터 센 번호" } },
            "required": ["file", "name"] } },
    ])
}

pub fn call(name: &str, args: &Value) -> Result<Vec<Value>, String> {
    let text = match name {
        "http_list" => {
            let folder = args["folder"].as_str().ok_or("folder가 필요합니다")?;
            let found = list(Path::new(folder));
            if found.files.is_empty() {
                format!("요청 파일이 없습니다. {}\\requests.http 를 만들면 됩니다(형식: '### 이름' 아래에 'GET 주소', 헤더, 빈 줄, 본문).", found.dir)
            } else {
                found
                    .files
                    .iter()
                    .map(|f| {
                        let names: Vec<String> = f
                            .requests
                            .iter()
                            .enumerate()
                            .map(|(i, r)| {
                                // 환경 변수에서 읽은 값은 가리고, 값을 못 채운 요청은 표시해 둔다.
                                let note = if r.problems.is_empty() { "" } else { " (값을 채우지 못한 곳이 있어 지금은 보낼 수 없음)" };
                                format!("  {}. {} — {} {}{note}", i + 1, r.masked(&r.name), r.method, r.masked(&r.url))
                            })
                            .collect();
                        format!("{}\n{}", f.path, names.join("\n"))
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n")
            }
        }
        "http_run" => {
            let file = args["file"].as_str().ok_or("file이 필요합니다")?;
            let pick = args["name"].as_str().map(str::to_string).or_else(|| args["name"].as_u64().map(|n| n.to_string()));
            let response = run(Path::new(file), &pick.ok_or("name이 필요합니다")?)?;
            let headers: Vec<String> = response.headers.iter().map(|(k, v)| format!("{k}: {v}")).collect();
            let mut body: String = response.body.chars().take(MAX_TOOL_TEXT).collect();
            if response.body.chars().count() > MAX_TOOL_TEXT {
                body.push_str("\n… (이하 생략. 전체는 기록 파일 참고)");
            }
            format!("{} · {}ms\n{}\n\n{}\n\n기록: {}", response.status, response.elapsed_ms, headers.join("\n"), body, response.history)
        }
        other => return Err(format!("없는 도구입니다: {other}")),
    };
    Ok(vec![json!({ "type": "text", "text": text })])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    /// 요청 하나를 받아 그대로 되돌려 주는 한 번짜리 서버.
    fn echo_server() -> (u16, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line.trim().is_empty() {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
                head.push_str(&line);
            }
            let mut body = vec![0u8; length];
            reader.read_exact(&mut body).unwrap();
            let reply = r#"{"ok":true,"이름":"응답"}"#;
            write!(stream, "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).unwrap();
            format!("{head}\n{}", String::from_utf8(body).unwrap())
        });
        (port, handle)
    }

    #[test]
    fn 요청을_보내고_기록한다() {
        let dir = std::env::temp_dir().join("cypher-http-test").join(".cypher").join("http");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let (port, server) = echo_server();
        let file = dir.join("requests.http");
        fs::write(
            &file,
            format!("@host = http://127.0.0.1:{port}\n\n### 만들기\nPOST {{{{host}}}}/items\nContent-Type: application/json\nAuthorization: Bearer secret-token-123\n\n{{\"name\": \"사과\"}}\n"),
        )
        .unwrap();

        let response = run(&file, "만들기").unwrap();
        assert_eq!((response.code, response.status.as_str()), (201, "201 Created"));
        assert!(response.body.contains("\"ok\": true")); // JSON은 들여써서 보여 준다.
        // 보낸 요청 원문: 변수가 채워진 요청 줄, 붙은 기본 헤더, 본문.
        assert!(response.request.starts_with(&format!("POST /items HTTP/1.1\nHost: 127.0.0.1:{port}\n")));
        assert!(response.request.contains(concat!("User-Agent: Cypher/", env!("CARGO_PKG_VERSION"))));
        assert!(response.request.contains("Content-Length: 18\n\n{\"name\": \"사과\"}"));

        let received = server.join().unwrap();
        assert!(received.starts_with("POST /items HTTP/1.1"));
        // 화면에 보여 준 헤더가 실제로 나갔는지
        let received_lower = received.to_ascii_lowercase();
        assert!(received_lower.contains("user-agent: cypher/") && received_lower.contains("accept: */*"));
        assert!(received_lower.contains("accept-encoding: gzip") && received_lower.contains("content-length: 18"));
        assert!(received.contains("Bearer secret-token-123") && received.ends_with("{\"name\": \"사과\"}"));

        let history = fs::read_to_string(dir.join(HISTORY)).unwrap();
        assert!(history.contains("<> 201 Created") && history.contains("만들기"));
        assert!(!history.contains("secret-token-123"));
        assert_eq!(fs::read_to_string(dir.join(".gitignore")).unwrap(), "history.http\n");

        // 목록에는 요청 파일만 나오고 기록 파일은 빠진다.
        let listed = list(dir.parent().unwrap().parent().unwrap());
        assert_eq!(listed.files.len(), 1);
        assert_eq!(listed.files[0].requests[0].name, "만들기");
        assert!(run(&file, "없는 요청").is_err());

        // 저장 위치: 찾은 요청 파일만 덮어쓰고, 새 파일은 정해진 폴더에만 만든다.
        let cwd = dir.parent().unwrap().parent().unwrap();
        assert_eq!(save_target(cwd, Some(&file.to_string_lossy()), None).unwrap(), file);
        assert!(save_target(cwd, Some(r"C:\Windows\win.ini"), None).is_err());
        assert_eq!(save_target(cwd, None, Some("결제 API")).unwrap(), dir.join("결제 API.http"));
        assert_eq!(save_target(cwd, None, None).unwrap(), dir.join("requests.http"));
        for bad in ["history", "..\\밖", ".숨김", "a/b", ""] {
            assert!(save_target(cwd, None, Some(bad)).is_err(), "{bad}");
        }
        assert!(readable(cwd, &file) && readable(cwd, &dir.join(HISTORY)));
        assert!(!readable(cwd, Path::new(r"C:\Windows\win.ini")));
    }
}
