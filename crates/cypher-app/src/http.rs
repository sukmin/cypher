//! `.http` 요청 파일 실행과 기록. 화면(옆 창)과 Claude 세션(MCP 도구)이 같이 쓴다.
//!
//! 요청 파일은 세션 폴더의 `.cypher\http\`, `http\`, 또는 폴더 바로 아래의 `*.http`에서 찾는다.
//! 실행하면 요청과 응답을 요청 파일 옆의 `history.http`에 덧붙인다.

use serde::Serialize;
use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use cypher_core::http::{self, HttpRequest};

const HISTORY: &str = "history.http";
/// 응답 본문을 받는 한도. 넘으면 여기까지만 받고 잘렸다고 알린다(화면의 문구도 이 크기를 말한다).
const MAX_BODY_BYTES: u64 = 5 << 20;
const MAX_TOOL_TEXT: usize = 15_000;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// 저장하려는 파일이 읽은 뒤 다른 곳에서 바뀌어 저장하지 않았을 때의 오류.
/// 화면(ui/http.js의 `CHANGED_ELSEWHERE`)이 이 문구로 알아보므로 바꿀 때는 함께 바꾼다.
const CHANGED_ELSEWHERE: &str = "파일이 다른 곳에서 바뀌었습니다";

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
    /// 리다이렉트를 따라갔을 때 응답을 받은 최종 주소(환경 변수에서 읽은 값은 가린다). 따라가지 않았으면 없다.
    final_url: Option<String>,
    /// 응답 본문이 한도(5MB)를 넘어 앞부분만 받았는지.
    truncated: bool,
}

/// 읽은 파일 내용.
#[derive(Serialize)]
pub struct HttpText {
    text: String,
    /// 읽은 때의 내용 지문. 저장할 때 돌려받아, 그 사이 다른 곳에서 바뀌었는지 알아본다.
    stamp: String,
}

/// 저장한 결과.
#[derive(Debug, Serialize)]
pub struct HttpSaved {
    path: String,
    /// 저장한 내용의 지문(다음 저장 때 돌려받는다).
    stamp: String,
}

/// 파일 내용의 지문. 읽은 뒤 다른 곳에서 바뀌었는지 알아보는 데만 쓴다(고친 시각과 달리 내용이 같으면 같다).
fn stamp_of(bytes: &[u8]) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// 요청 파일을 글로 읽는다. 저장은 늘 UTF-8로 하지만, 다른 편집기가 다른 인코딩(CP949 등)으로 저장한 파일도 읽는다.
fn read_text(path: &Path) -> Result<String, String> {
    fs::read(path).map_err(|e| e.to_string()).and_then(decode_text)
}

fn decode_text(bytes: Vec<u8>) -> Result<String, String> {
    use cypher_core::textfile;
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

/// 요청 파일을 두는 폴더들.
fn dirs(cwd: &Path) -> [PathBuf; 3] {
    [home_dir(cwd), cwd.join("http"), cwd.to_path_buf()]
}

/// 폴더에서 요청 파일을 찾는다(기록 파일은 뺀다).
fn find_files(cwd: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for dir in dirs(cwd) {
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

/// 받은 응답.
struct Sent {
    code: u16,
    /// "200 OK"
    status: String,
    headers: Vec<(String, String)>,
    body: String,
    elapsed_ms: u64,
    /// 리다이렉트를 따라갔으면 응답을 받은 최종 주소.
    final_url: Option<String>,
    /// 본문이 한도를 넘어 앞부분만 받았는지.
    truncated: bool,
}

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

/// 본문을 `limit` 바이트까지 읽는다. 더 있었으면(한 바이트 더 읽어 보아 안다) 한도에서 자르고 참을 함께 돌려준다.
fn read_limited(reader: impl Read, limit: u64) -> std::io::Result<(Vec<u8>, bool)> {
    let mut bytes = Vec::new();
    reader.take(limit + 1).read_to_end(&mut bytes)?;
    let truncated = bytes.len() as u64 > limit;
    if truncated {
        bytes.truncate(limit as usize);
    }
    Ok((bytes, truncated))
}

/// 응답 본문을 글로 바꾼다. UTF-8로 읽고, 아니면 다른 인코딩(CP949 등)으로 읽어 본다.
/// 한도에서 잘린 본문은 끝이 글자 중간일 수 있으므로 온전한 글자까지만 쓴다.
fn body_text(mut bytes: Vec<u8>, truncated: bool) -> String {
    if truncated {
        if let Err(e) = std::str::from_utf8(&bytes) {
            // 끝에서 글자가 덜 끝난 경우에만 그 앞까지로 줄인다(가운데가 깨진 것은 UTF-8이 아닌 글이다).
            if e.error_len().is_none() {
                bytes.truncate(e.valid_up_to());
            }
        }
    }
    let other = match String::from_utf8(bytes) {
        Ok(text) => return text,
        Err(not_utf8) => not_utf8.into_bytes(),
    };
    let decode = |bytes: &[u8]| cypher_core::textfile::decode(bytes).ok().filter(|decoded| !decoded.lossy).map(|decoded| decoded.text);
    decode(&other)
        // 두 바이트 글자(CP949)의 가운데에서 잘렸을 수 있다. 마지막 한 바이트를 빼고 다시 읽어 본다.
        .or_else(|| truncated.then(|| decode(&other[..other.len().saturating_sub(1)])).flatten())
        .unwrap_or_else(|| format!("(글자가 아닌 데이터 {}바이트)", other.len()))
}

/// 요청을 보내고 응답을 받는다. 4xx·5xx도 오류가 아니라 응답으로 돌려준다.
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
    // 리다이렉트는 따라가되, 응답을 준 주소가 요청 주소와 다르면 알려 준다.
    let final_url = Some(response.get_url()).filter(|url| *url != request.url).map(str::to_string);
    // 같은 이름의 헤더가 여러 개(Set-Cookie 등)면 값마다 한 줄로 적는다.
    let mut names: Vec<String> = Vec::new();
    for name in response.headers_names() {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    let headers: Vec<(String, String)> =
        names.iter().flat_map(|name| response.all(name).into_iter().map(move |value| (name.clone(), value.to_string()))).collect();
    let (bytes, truncated) = read_limited(response.into_reader(), MAX_BODY_BYTES).map_err(|e| e.to_string())?;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    Ok(Sent { code, status, headers, body: body_text(bytes, truncated), elapsed_ms, final_url, truncated })
}

/// git 명령을 창 없이(콘솔 창이 깜빡이지 않게) 실행한다. git이 없으면 None.
fn git(dir: &Path, args: &[&str]) -> Option<std::process::Output> {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "core.quotepath=false"])
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null()) // 입력을 기다리며 멈추지 않게 한다.
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()
}

/// 기록 파일이 저장소에 올라가지 않게 해 둔다(요청·응답 내용이 담기기 때문이다). 사용자의 `.gitignore`는 고치지 않는다.
///
/// - git 저장소 안: 아직 무시되지 않으면 저장소의 `info/exclude`(이 컴퓨터에만 있는 무시 목록)에 그 파일을 올린다.
///   그 폴더에 `.gitignore`가 이미 있어도(저장소 맨 위 폴더가 그렇다) 빠지지 않는다.
/// - 저장소가 아니거나 git이 없음: 그 폴더에 `.gitignore`가 없을 때만 만들어 둔다(나중에 저장소가 되어도 올라가지 않게).
fn keep_out_of_git(history: &Path) {
    let Some(dir) = history.parent() else { return };
    // `git check-ignore -q`: 무시되면 0, 무시되지 않으면 1, 저장소가 아니면 128.
    match git(dir, &["check-ignore", "-q", HISTORY]).and_then(|output| output.status.code()) {
        Some(0) => {}
        Some(1) => exclude_locally(dir),
        _ => {
            let ignore = dir.join(".gitignore");
            if !ignore.exists() {
                let _ = fs::write(ignore, format!("{HISTORY}\n"));
            }
        }
    }
}

/// 저장소의 `info/exclude`에 이 폴더의 기록 파일을 가리키는 줄을 덧붙인다(이미 있으면 그대로 둔다).
fn exclude_locally(dir: &Path) {
    let answer = |args: &[&str]| {
        let output = git(dir, args).filter(|output| output.status.success())?;
        Some(String::from_utf8_lossy(&output.stdout).trim_end_matches(['\r', '\n']).to_string())
    };
    // 저장소 맨 위 기준의 이 폴더("http/" 꼴, 맨 위 폴더면 빈 글)와 exclude 파일의 위치(워크트리에서도 맞는 경로).
    let (Some(prefix), Some(exclude)) = (answer(&["rev-parse", "--show-prefix"]), answer(&["rev-parse", "--git-path", "info/exclude"]))
    else {
        return;
    };
    let exclude = dir.join(exclude); // 상대 경로로 알려 주면 이 폴더 기준이다.
    // 이 파일 하나만 가리키게 맨 위 기준(`/`)으로 적고, 무시 규칙에서 뜻이 있는 글자는 글자 그대로 읽히게 한다.
    let mut rule = String::from("/");
    for c in format!("{prefix}{HISTORY}").chars() {
        if matches!(c, '\\' | '*' | '?' | '[') {
            rule.push('\\');
        }
        rule.push(c);
    }
    let old = fs::read(&exclude).map(|bytes| String::from_utf8_lossy(&bytes).into_owned()).unwrap_or_default();
    if old.lines().any(|line| line.trim_end() == rule) {
        return;
    }
    if let Some(parent) = exclude.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let gap = if old.is_empty() || old.ends_with('\n') { "" } else { "\n" };
    let _ = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&exclude)
        .and_then(|mut file| file.write_all(format!("{gap}{rule}\n").as_bytes()));
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
    let index = http::index_at(&text, &requests, line)
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
    let Sent { code, status, headers, body, elapsed_ms, final_url, truncated } = send(request, &sent_headers)?;
    let body = pretty(&body);

    let history = file.with_file_name(HISTORY);
    let entry = http::record(request, &now_text(), &status, elapsed_ms, final_url.as_deref(), &headers, &body);
    let appended = fs::OpenOptions::new().create(true).append(true).open(&history).and_then(|mut f| f.write_all(entry.as_bytes()));
    if appended.is_ok() {
        keep_out_of_git(&history);
    }
    Ok(HttpResponse {
        status,
        code,
        elapsed_ms,
        headers,
        body,
        request: raw,
        history: history.to_string_lossy().into_owned(),
        final_url: final_url.map(|url| request.masked(&url)),
        truncated,
    })
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
        || (file.file_name().is_some_and(|n| n == HISTORY) && dirs(cwd).iter().any(|dir| file.parent() == Some(dir)))
}

/// 저장할 곳을 정한다. 열어 둔 요청 파일은 이 폴더의 요청 파일 자리(요청 폴더 바로 아래의 `.http` 파일, 기록 파일 제외)일
/// 때만 받는다. 바깥에서 지워졌거나 이름이 바뀌어 지금은 없는 파일도 받아 다시 만든다(고친 내용을 저장할 곳이 있게).
/// 새 파일은 `.cypher\http\<이름>.http`에 만든다.
fn save_target(cwd: &Path, file: Option<&str>, name: Option<&str>) -> Result<PathBuf, String> {
    if let Some(file) = file {
        let path = PathBuf::from(file);
        // 폴더는 글자가 아니라 경로 마디로 견준다. `..`가 낀 경로는 요청 폴더 "바로 아래"가 되지 못해 걸러진다.
        let inside = dirs(cwd).iter().any(|dir| path.parent() == Some(dir));
        let named = path.extension().is_some_and(|e| e == "http")
            && path.file_name().is_some_and(|n| !n.to_string_lossy().eq_ignore_ascii_case(HISTORY));
        let free = path.is_file() || !path.exists(); // 같은 이름의 폴더 등은 안 된다.
        return if inside && named && free { Ok(path) } else { Err("이 폴더의 요청 파일이 아닙니다".into()) };
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
pub async fn http_read(cwd: String, file: String) -> Result<HttpText, String> {
    let (cwd, file) = (Path::new(&cwd), Path::new(&file));
    if !readable(cwd, file) {
        return Err("이 폴더의 요청 파일이 아닙니다".into());
    }
    let bytes = fs::read(file).map_err(|e| e.to_string())?;
    let stamp = stamp_of(&bytes);
    let text = decode_text(bytes)?;
    // 기록 파일만 끝부분을 잘라 준다. 요청 파일은 편집기에 통째로 들어가야 한다(잘린 채 저장되면 안 된다).
    const MAX: usize = 300_000;
    if text.len() <= MAX || file.file_name().is_none_or(|name| name != HISTORY) {
        return Ok(HttpText { text, stamp });
    }
    let mut cut = text.len() - MAX;
    while !text.is_char_boundary(cut) {
        cut += 1;
    }
    Ok(HttpText { text: format!("… (앞부분 생략)\n{}", &text[cut..]), stamp })
}

/// 편집기 내용을 저장한다. `file`이 없으면 새 파일을 만든다. 저장한 경로와 내용 지문을 돌려준다.
/// `stamp`는 편집기가 그 파일을 읽은(또는 마지막으로 저장한) 때의 지문이다. 디스크의 지금 내용과 다르면
/// 저장하지 않고 `CHANGED_ELSEWHERE` 오류를 돌려준다. 확인 없이 덮어쓰려면 `stamp` 없이 부른다.
#[tauri::command]
pub async fn http_save(
    cwd: String,
    file: Option<String>,
    name: Option<String>,
    text: String,
    stamp: Option<String>,
) -> Result<HttpSaved, String> {
    save(Path::new(&cwd), file.as_deref(), name.as_deref(), &text, stamp.as_deref())
}

fn save(cwd: &Path, file: Option<&str>, name: Option<&str>, text: &str, stamp: Option<&str>) -> Result<HttpSaved, String> {
    let target = save_target(cwd, file, name)?;
    // 읽은 뒤 다른 곳(Claude 세션, 다른 편집기)에서 바뀐 파일을 옛 내용으로 덮어쓰지 않는다.
    // 파일이 없어졌으면 견줄 것이 없으니 다시 만든다.
    if let (Some(stamp), Ok(now)) = (stamp, fs::read(&target)) {
        if stamp_of(&now) != stamp {
            return Err(CHANGED_ELSEWHERE.into());
        }
    }
    if let Some(dir) = target.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // 임시 파일에 쓴 뒤 바꿔치기한다(저장 도중 꺼져도 잘린 파일이 남지 않게).
    crate::memo::write_safely(&target, text.as_bytes())?;
    Ok(HttpSaved { path: target.to_string_lossy().into_owned(), stamp: stamp_of(text.as_bytes()) })
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
                // 기록 파일도 긴 응답은 앞부분만 남긴다. 전체가 거기 있다고 말하지 않는다.
                body.push_str("\n… (이하 생략. 기록 파일에도 긴 응답은 앞부분만 남습니다)");
            }
            if response.truncated {
                body.push_str("\n… (응답이 5MB를 넘어 앞부분만 받았습니다)");
            }
            let moved = response.final_url.as_ref().map(|url| format!(" → {url}")).unwrap_or_default();
            format!(
                "{} · {}ms{moved}\n{}\n\n{}\n\n기록: {}",
                response.status,
                response.elapsed_ms,
                headers.join("\n"),
                body,
                response.history
            )
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

    /// 정해 둔 응답을 차례로(연결마다 하나씩) 돌려주는 서버. 받은 요청 줄들을 돌려준다.
    fn reply_server(replies: Vec<String>) -> (u16, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let mut asked = Vec::new();
            for reply in replies {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                let mut line = String::new();
                while reader.read_line(&mut line).unwrap() > 0 && !line.trim().is_empty() {
                    line.clear();
                }
                stream.write_all(reply.as_bytes()).unwrap();
                asked.push(first.trim().to_string());
            }
            asked
        });
        (port, handle)
    }

    /// 시험마다 따로 쓰는 빈 폴더.
    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn ignored(dir: &Path) -> Option<i32> {
        git(dir, &["check-ignore", "-q", HISTORY]).and_then(|output| output.status.code())
    }

    #[test]
    fn 리다이렉트를_따라간_주소와_같은_이름의_헤더를_모두_보여_준다() {
        let dir = test_dir("cypher-http-redirect-test").join("http");
        fs::create_dir_all(&dir).unwrap();
        let (port, server) = reply_server(vec![
            "HTTP/1.1 302 Found\r\nLocation: /new?x=1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
            "HTTP/1.1 200 OK\r\nSet-Cookie: a=1\r\nX-Trace: t1\r\nSet-Cookie: b=2\r\nx-trace: t2\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".into(),
        ]);
        let file = dir.join("requests.http");
        fs::write(&file, format!("\u{feff}### 옮겨 간 주소\nGET http://127.0.0.1:{port}/old\n")).unwrap();

        // BOM이 붙은 파일의 첫 줄 요청을, 그 아래 빈 줄(2번 줄)에서 실행한다.
        let response = run_at(&file, 2).unwrap();
        assert_eq!(server.join().unwrap(), ["GET /old HTTP/1.1", "GET /new?x=1 HTTP/1.1"]);
        assert_eq!((response.code, response.body.as_str(), response.truncated), (200, "ok", false));
        let moved = format!("http://127.0.0.1:{port}/new?x=1");
        assert_eq!(response.final_url.as_deref(), Some(moved.as_str()));
        // 같은 이름의 헤더는 값마다 한 줄이다(첫 값만 되풀이되지 않는다).
        let values = |name: &str| -> Vec<&str> { response.headers.iter().filter(|(key, _)| key == name).map(|(_, value)| value.as_str()).collect() };
        assert_eq!(values("set-cookie"), ["a=1", "b=2"]);
        assert_eq!(values("x-trace"), ["t1", "t2"]);
        assert_eq!(response.headers.len(), 6, "{:?}", response.headers);

        let history = fs::read_to_string(dir.join(HISTORY)).unwrap();
        assert!(history.contains(&format!("ms → {moved}\n")), "{history}");
        assert_eq!(history.matches("# set-cookie: ***\n").count(), 2, "{history}");
    }

    #[test]
    fn 긴_응답은_한도에서_자르고_글자_중간에서_끊지_않는다() {
        // 한도까지만 읽고, 더 있었는지 알려 준다.
        assert_eq!(read_limited(&b"abcdef"[..], 4).unwrap(), (b"abcd".to_vec(), true));
        assert_eq!(read_limited(&b"abcdef"[..], 6).unwrap(), (b"abcdef".to_vec(), false));
        assert_eq!(read_limited(&b"abc"[..], 6).unwrap(), (b"abc".to_vec(), false));
        // "가나다"(글자마다 3바이트)를 7바이트에서 자르면 "다"의 첫 바이트가 남는다. 온전한 글자까지만 쓴다.
        let (bytes, truncated) = read_limited("가나다".as_bytes(), 7).unwrap();
        assert_eq!((bytes.len(), truncated), (7, true));
        assert_eq!(body_text(bytes, truncated), "가나");
        assert_eq!(body_text("가나다".as_bytes().to_vec(), false), "가나다");
        // CP949 글(글자마다 2바이트)도 같다.
        let cp949 = cypher_core::textfile::encode("한글 응답", cypher_core::textfile::CP949, cypher_core::textfile::LF).unwrap();
        assert_eq!(body_text(cp949.clone(), false), "한글 응답");
        assert_eq!(body_text(cp949[..cp949.len() - 1].to_vec(), true), "한글 응");
        // 글자가 아닌 데이터는 잘렸든 아니든 그렇게 알린다.
        assert_eq!(body_text(vec![0, 159, 146, 150, 0, 1], true), "(글자가 아닌 데이터 6바이트)");
    }

    #[test]
    fn 기록_파일은_저장소에_올라가지_않게_해_둔다() {
        let root = test_dir("cypher-http-git-test");
        if !git(&root, &["init", "-q"]).is_some_and(|output| output.status.success()) {
            return; // git이 없는 컴퓨터에서는 확인할 수 없다.
        }
        // 저장소 맨 위에는 `.gitignore`가 이미 있다. 그 파일은 고치지 않고 저장소의 exclude에 올린다.
        let sub = root.join("api [v1]").join("http");
        fs::create_dir_all(&sub).unwrap();
        fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        for dir in [&root, &sub] {
            fs::write(dir.join(HISTORY), "### 기록\n").unwrap();
            assert_eq!(ignored(dir), Some(1));
            keep_out_of_git(&dir.join(HISTORY));
            keep_out_of_git(&dir.join(HISTORY)); // 두 번 해도 한 줄만 는다.
            assert_eq!(ignored(dir), Some(0));
        }
        assert_eq!(fs::read_to_string(root.join(".gitignore")).unwrap(), "*.log\n");
        assert!(!sub.join(".gitignore").exists());
        let exclude = fs::read_to_string(root.join(".git").join("info").join("exclude")).unwrap();
        let added: Vec<&str> = exclude.lines().filter(|line| line.contains(HISTORY)).collect();
        assert_eq!(added, ["/history.http", "/api \\[v1]/http/history.http"]);
        // 그 파일들만 가린다. 다른 폴더의 같은 이름 파일은 그대로다.
        assert_eq!(ignored(&root.join("api [v1]")), Some(1));
    }

    #[test]
    fn 바깥에서_바뀐_파일은_덮어쓰지_않고_없어진_파일은_다시_만든다() {
        let cwd = test_dir("cypher-http-save-test");
        let cwd_text = cwd.to_string_lossy().into_owned();
        let saved = save(&cwd, None, Some("결제"), "GET https://a.example/1\n", None).unwrap();
        let file = PathBuf::from(&saved.path);
        assert_eq!(file, home_dir(&cwd).join("결제.http"));
        // 읽을 때의 지문은 저장할 때 돌려받은 것과 같다.
        let read = tauri::async_runtime::block_on(http_read(cwd_text.clone(), saved.path.clone())).unwrap();
        assert_eq!((read.text.as_str(), &read.stamp), ("GET https://a.example/1\n", &saved.stamp));

        // 그대로면 저장되고 새 지문을 받는다.
        let second = save(&cwd, Some(&saved.path), None, "GET https://a.example/2\n", Some(&saved.stamp)).unwrap();
        assert_ne!(second.stamp, saved.stamp);
        // 다른 곳에서 바뀌었으면(Claude 세션이 요청을 더한 경우) 저장하지 않는다.
        let theirs = "GET https://a.example/2\n\n### 더한 요청\nGET https://a.example/3\n";
        fs::write(&file, theirs).unwrap();
        let refused = save(&cwd, Some(&saved.path), None, "GET https://a.example/내것\n", Some(&second.stamp));
        assert_eq!(refused.unwrap_err(), CHANGED_ELSEWHERE);
        assert_eq!(fs::read_to_string(&file).unwrap(), theirs);
        // 지문 없이 부르면 덮어쓴다(사용자가 "덮어쓰기"를 고른 경우).
        let forced = save(&cwd, Some(&saved.path), None, "GET https://a.example/내것\n", None).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "GET https://a.example/내것\n");

        // 바깥에서 지워진 파일은 다시 만든다. 임시 파일은 남지 않는다.
        fs::remove_file(&file).unwrap();
        assert!(!readable(&cwd, &file));
        save(&cwd, Some(&saved.path), None, "GET https://a.example/다시\n", Some(&forced.stamp)).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "GET https://a.example/다시\n");
        let left: Vec<String> = fs::read_dir(home_dir(&cwd)).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left, ["결제.http"]);

        // 없는 파일을 받는 것은 요청 폴더 바로 아래의 `.http` 자리뿐이다.
        let home = home_dir(&cwd);
        for ok in [home.join("새 파일.http"), cwd.join("http").join("a.http"), cwd.join("b.http")] {
            assert_eq!(save_target(&cwd, Some(&ok.to_string_lossy()), None).unwrap(), ok);
        }
        fs::create_dir_all(home.join("폴더.http")).unwrap();
        for bad in [
            home.join(HISTORY),
            home.join("HISTORY.http"),
            home.join("메모.txt"),
            home.join("폴더.http"),
            home.join("sub").join("a.http"),
            home.join("..").join("a.http"),
            cwd.join("..").join("a.http"),
            cwd.join("http").join("..").join("..").join("a.http"),
            cwd.parent().unwrap().join("a.http"),
            PathBuf::from(r"C:\Windows\a.http"),
            PathBuf::from("a.http"),
        ] {
            assert!(save_target(&cwd, Some(&bad.to_string_lossy()), None).is_err(), "{}", bad.display());
        }
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
        assert_eq!((response.final_url.as_deref(), response.truncated), (None, false)); // 리다이렉트도 잘림도 없었다.
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
        // 저장소가 아닌 폴더에서는 `.gitignore`를 만들어 둔다(이 시험 폴더가 어떤 저장소 안에 놓인 컴퓨터에서는 그 저장소의 exclude에 오른다).
        match ignored(&dir) {
            Some(0) => {}
            _ => assert_eq!(fs::read_to_string(dir.join(".gitignore")).unwrap(), "history.http\n"),
        }

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
