//! 전달 프로그램(cypher-bridge)이 접속하는 로컬 통로.
//!
//! 접속마다 첫 줄은 확인값, 둘째 줄은 종류다. 둘째 줄에 적힌 세션 번호는 보낸 쪽이 스스로 적은 값이므로 믿지 않고,
//! 접속한 프로세스가 실제로 그 세션이 띄운 프로세스인지 확인한다(`peer.rs`). 아니면 받지 않는다.
//! 한 번 보내고 끊는 신호는 다 처리한 뒤에 연결을 닫는다. 전달 프로그램은 그때까지 기다렸다가 끝나므로,
//! Claude Code는 앱이 신호를 처리한 뒤에야 다음 일(파일 쓰기 등)을 한다.
//! - 세션 번호: 뒤따르는 내용이 Claude Code 상태줄 JSON 또는 훅 JSON(한 번 보내고 끊는다).
//!   훅 가운데 Write 도구의 직전·직후 신호는 화면에 그대로 넘기지 않고, 새로 만든 파일을 가려
//!   `session-file`로 알린다(`created.rs`).
//! - `mcp <세션 번호>`: Cypher 도구용 MCP 연결(계속 이어진다)

use serde::Serialize;
use serde_json::Value;
use std::io::{BufRead, BufReader, Read};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, Serialize)]
struct StatusPayload {
    id: u32,
    model: Option<String>,
    effort: Option<String>,
    /// 컨텍스트 사용률(%).
    context: Option<f64>,
    cwd: Option<String>,
    /// 세션이 시작된 폴더(이어가기는 이 폴더에서 해야 한다).
    project: Option<String>,
    /// Claude 세션 번호. 다시 시작할 때 이 대화를 이어가는 데 쓴다.
    session: Option<String>,
    worktree: Option<String>,
    /// 이 세션에 든 비용(달러).
    cost: Option<f64>,
    /// 계정 사용 한도(5시간·주간 사용률과 초기화 시각). Claude Code가 준 그대로 넘긴다.
    limits: Option<Value>,
}

/// Claude Code 훅 알림(요청 시작, 응답 완료, 입력·승인 대기).
#[derive(Clone, Serialize)]
struct HookPayload {
    id: u32,
    /// "UserPromptSubmit" | "Stop" | "Notification" | "SubagentStart" | "SubagentStop"
    event: String,
    /// Notification일 때의 종류(예: permission_prompt, idle_prompt).
    kind: Option<String>,
    message: Option<String>,
    /// 이 신호를 낸 서브에이전트의 번호(본 세션이 낸 신호면 없다).
    agent: Option<String>,
    /// 서브에이전트의 종류(예: Explore, general-purpose).
    agent_type: Option<String>,
}

/// Claude 세션이 새로 만든 파일. 화면이 그 세션의 묶음에 종류에 맞는 항목으로 놓는다.
#[derive(Clone, Serialize)]
struct FilePayload {
    id: u32,
    path: String,
    /// "doc" | "image" | "http" | "other"
    kind: &'static str,
}

enum Message {
    Status(StatusPayload),
    Hook(HookPayload),
    /// Write 도구가 파일을 쓰기 직전.
    WillWrite { id: u32, path: PathBuf },
    /// Write 도구가 파일을 쓴 직후. `base`는 그 세션이 일하고 있는 폴더.
    Wrote { id: u32, path: PathBuf, base: Option<PathBuf> },
}

/// 훅 내용 한 번의 크기 한도. Write 도구의 신호에는 쓴 파일의 내용이 통째로 들어 있다.
const MAX_BODY: u64 = 32 << 20;

/// 로컬 포트를 열고 받기 스레드를 띄운다. 열린 포트 번호를 돌려준다.
pub fn start(app: AppHandle, token: String) -> std::io::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (app, token) = (app.clone(), token.clone());
            // MCP 연결은 오래 이어지므로 접속마다 스레드를 따로 둔다.
            thread::spawn(move || accept(app, &token, stream));
        }
    });
    Ok(port)
}

/// 접속한 프로세스를 띄운 세션의 번호. 어느 세션의 자손도 아니면 None.
/// 접속한 쪽(전달 프로그램)은 답을 기다리는 동안만 살아 있으므로, 조상부터 따라 올라간 뒤에 세션 목록을 본다.
fn owner_of(app: &AppHandle, stream: &TcpStream) -> Option<u32> {
    let pid = crate::peer::pid_of(stream.peer_addr().ok()?, stream.local_addr().ok()?)?;
    let chain = crate::peer::lineage(pid);
    let roots: Vec<(u32, u32)> = app
        .state::<crate::AppState>()
        .sessions
        .lock()
        .unwrap()
        .iter()
        .filter_map(|(&id, session)| Some((id, session.pid()?)))
        .collect();
    crate::peer::session_of(&chain, &roots)
}

/// 받지 않은 접속을 기록한다. 같은 사유(적힌 번호, 실제 세션)는 실행 중에 한 번만 남긴다
/// (조상 줄이 늘 끊기는 환경이면 상태줄 신호마다 거절되므로, 그대로 두면 기록이 끝없이 쌓인다).
fn log_refused(claimed: Option<u32>, owner: Option<u32>) {
    /// (적힌 번호, 실제 세션)
    type Reason = (Option<u32>, Option<u32>);
    static SEEN: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<Reason>>> = std::sync::LazyLock::new(Default::default);
    let mut seen = SEEN.lock().unwrap();
    if seen.len() < 200 && seen.insert((claimed, owner)) {
        crate::log(&format!("통로: 받지 않은 접속(적힌 세션 {claimed:?}, 실제 {owner:?}). 같은 사유는 다시 적지 않습니다."));
    }
}

fn accept(app: AppHandle, token: &str, stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let Ok(writer) = stream.try_clone() else { return };
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() || line.trim_end() != token {
        return; // 확인값이 다르면 아무것도 하지 않는다.
    }
    // 확인값이 맞는 접속만 누가 보냈는지 알아본다(전달 프로그램은 답을 기다리고 있으므로 아직 살아 있다).
    let owner = owner_of(&app, reader.get_ref());
    line.clear();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let kind = line.trim_end();
    // 적힌 세션 번호가 접속한 프로세스의 세션과 다르면(다른 세션인 척하거나, 세션 밖의 프로세스) 받지 않는다.
    let claimed = kind.strip_prefix("mcp").unwrap_or(kind).trim().parse::<u32>().ok();
    if owner.is_none() || claimed != owner {
        log_refused(claimed, owner);
        return;
    }
    // `mcp <세션 번호>`: Cypher 도구용 연결.
    if let Some(rest) = kind.strip_prefix("mcp").filter(|rest| rest.starts_with(' ')) {
        let session = rest.trim().parse::<u32>().ok();
        // MCP 연결은 요청 사이에 오래 조용할 수 있다. 읽는 쪽 손잡이의 시간 제한을 직접 푼다
        // (복제한 손잡이에서 풀어도 읽는 쪽에는 적용되지 않는다).
        let _ = reader.get_ref().set_read_timeout(None);
        crate::mcp::serve(app, session, reader, writer);
        return;
    }
    let mut body = String::new();
    if reader.take(MAX_BODY).read_to_string(&mut body).is_err() {
        return;
    }
    let _ = match parse(kind, &body) {
        Some(Message::Status(payload)) => app.emit("session-status", payload),
        Some(Message::Hook(payload)) => app.emit("session-hook", payload),
        Some(Message::WillWrite { id, path }) => {
            crate::created::before_write(id, &path);
            Ok(())
        }
        Some(Message::Wrote { id, path, base }) => match crate::created::after_write(id, &path, base.as_deref()) {
            Some(kind) => app.emit("session-file", FilePayload { id, path: path.to_string_lossy().into_owned(), kind }),
            None => Ok(()),
        },
        None => Ok(()),
    };
    // 여기서 연결이 닫힌다. 전달 프로그램은 이것을 "처리했다"는 답으로 삼고 끝난다.
}

/// 상태줄 JSON과 훅 JSON이 같은 통로로 온다. 훅에는 `hook_event_name`이 있다.
fn parse(session: &str, body: &str) -> Option<Message> {
    let id = session.trim().parse().ok()?;
    let v: Value = serde_json::from_str(body).ok()?;
    let text = |v: &Value| v.as_str().map(str::to_string);
    if let Some(event) = text(&v["hook_event_name"]) {
        // 도구 직전·직후 신호: Write 도구가 쓰는 파일만 본다. 그 밖의 도구 신호는 버린다.
        if event == "PreToolUse" || event == "PostToolUse" {
            if v["tool_name"].as_str() != Some("Write") {
                return None;
            }
            let path = written_path(&v)?;
            return Some(if event == "PreToolUse" {
                Message::WillWrite { id, path }
            } else {
                Message::Wrote { id, path, base: local_path(v["cwd"].as_str().unwrap_or("")) }
            });
        }
        return Some(Message::Hook(HookPayload {
            id,
            event,
            kind: text(&v["notification_type"]),
            message: text(&v["message"]),
            agent: text(&v["agent_id"]),
            agent_type: text(&v["agent_type"]),
        }));
    }
    Some(Message::Status(StatusPayload {
        id,
        model: text(&v["model"]["display_name"]).or_else(|| text(&v["model"]["id"])),
        effort: text(&v["effort"]["level"]),
        context: v["context_window"]["used_percentage"].as_f64(),
        cwd: text(&v["workspace"]["current_dir"]).or_else(|| text(&v["cwd"])),
        project: text(&v["workspace"]["project_dir"]),
        session: text(&v["session_id"]),
        worktree: text(&v["worktree"]["name"]).or_else(|| text(&v["workspace"]["git_worktree"])),
        cost: v["cost"]["total_cost_usd"].as_f64(),
        limits: v.get("rate_limits").filter(|l| l.is_object()).cloned(),
    }))
}

/// 이 컴퓨터의 드라이브에 있는 전체 경로(`C:\…`)만 받는다. 긴 경로 표시(`\\?\C:\…`)는 떼어 내고,
/// 네트워크 경로(`\\서버\…`)는 받지 않는다(있는지 확인하느라 접속 스레드가 멈출 수 있다).
fn local_path(given: &str) -> Option<PathBuf> {
    let given = given.trim().replace('/', "\\");
    let given = given.strip_prefix(r"\\?\").unwrap_or(&given);
    let path = PathBuf::from(given);
    (!given.starts_with(r"\\") && path.is_absolute()).then_some(path)
}

/// Write 도구가 쓰는 파일의 전체 경로. 상대 경로로 왔으면 그 세션의 지금 폴더를 기준으로 푼다.
fn written_path(v: &Value) -> Option<PathBuf> {
    let given = v["tool_input"]["file_path"].as_str().map(str::trim).filter(|p| !p.is_empty())?;
    if let Some(path) = local_path(given) {
        return Some(path);
    }
    let relative = PathBuf::from(given.replace('/', "\\"));
    if relative.has_root() || relative.components().any(|part| matches!(part, std::path::Component::Prefix(_))) {
        return None; // 네트워크 경로나 드라이브 상대 경로
    }
    Some(local_path(v["cwd"].as_str()?)?.join(relative))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 쓰기_도구의_신호에서_파일_경로를_꺼낸다() {
        let pre = r#"{"hook_event_name":"PreToolUse","tool_name":"Write","cwd":"C:\\ws\\proj","tool_input":{"file_path":"C:/ws/proj/docs/계획.md","content":"계획 글"}}"#;
        let Some(Message::WillWrite { id, path }) = parse("4", pre) else { panic!("쓰기 직전 신호로 읽혀야 한다") };
        assert_eq!((id, path), (4, PathBuf::from(r"C:\ws\proj\docs\계획.md")));

        let post = r#"{"hook_event_name":"PostToolUse","tool_name":"Write","cwd":"C:\\ws\\proj","tool_input":{"file_path":"notes/a.txt"},"tool_response":{"type":"create","filePath":"x"}}"#;
        let Some(Message::Wrote { id, path, base }) = parse("4", post) else { panic!("쓴 직후 신호로 읽혀야 한다") };
        assert_eq!((id, path, base), (4, PathBuf::from(r"C:\ws\proj\notes\a.txt"), Some(PathBuf::from(r"C:\ws\proj"))));

        // 긴 경로 표시는 떼어 내고, 세션 폴더를 모르면 base가 없다.
        let long = r#"{"hook_event_name":"PostToolUse","tool_name":"Write","tool_input":{"file_path":"\\\\?\\C:\\a.md"}}"#;
        let Some(Message::Wrote { path, base, .. }) = parse("4", long) else { panic!("쓴 직후 신호로 읽혀야 한다") };
        assert_eq!((path, base), (PathBuf::from(r"C:\a.md"), None));
        // 다른 도구의 신호, 경로를 알 수 없는 신호, 네트워크 경로·드라이브 상대 경로는 버린다.
        for other in [
            r#"{"hook_event_name":"PostToolUse","tool_name":"Write","tool_input":{"file_path":"\\\\server\\share\\a.md"}}"#,
            r#"{"hook_event_name":"PostToolUse","tool_name":"Write","tool_input":{"file_path":"//server/share/a.md"}}"#,
            r#"{"hook_event_name":"PostToolUse","tool_name":"Write","cwd":"\\\\server\\share","tool_input":{"file_path":"a.md"}}"#,
            r#"{"hook_event_name":"PostToolUse","tool_name":"Write","cwd":"C:\\ws","tool_input":{"file_path":"\\a.md"}}"#,
            r#"{"hook_event_name":"PostToolUse","tool_name":"Write","cwd":"C:\\ws","tool_input":{"file_path":"D:a.md"}}"#,
            r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"ls"}}"#,
            r#"{"hook_event_name":"PostToolUse","tool_name":"Write","tool_input":{}}"#,
            r#"{"hook_event_name":"PostToolUse","tool_name":"Write","tool_input":{"file_path":"상대.md"}}"#,
        ] {
            assert!(parse("4", other).is_none(), "{other}");
        }
    }

    #[test]
    fn 상태줄_내용에서_모델과_effort를_꺼낸다() {
        let body = r#"{"cwd":"C:\\ws","model":{"id":"claude-fable-5-2","display_name":"Fable 5.2"},"effort":{"level":"high"},"context_window":{"used_percentage":8},"rate_limits":{"seven_day":{"used_percentage":86}}}"#;
        let Some(Message::Status(p)) = parse("7", body) else {
            panic!("상태줄로 읽혀야 한다");
        };
        assert_eq!(p.id, 7);
        assert_eq!(p.model.as_deref(), Some("Fable 5.2"));
        assert_eq!(p.effort.as_deref(), Some("high"));
        assert_eq!(p.context, Some(8.0));
        assert_eq!(p.cwd.as_deref(), Some("C:\\ws"));
        assert_eq!(p.limits.unwrap()["seven_day"]["used_percentage"], 86);
    }

    #[test]
    fn 훅_알림을_구분한다() {
        let body = r#"{"hook_event_name":"Notification","notification_type":"permission_prompt","message":"승인이 필요합니다"}"#;
        let Some(Message::Hook(h)) = parse("3", body) else {
            panic!("훅으로 읽혀야 한다");
        };
        assert_eq!((h.id, h.event.as_str()), (3, "Notification"));
        assert_eq!(h.kind.as_deref(), Some("permission_prompt"));
        assert_eq!(h.message.as_deref(), Some("승인이 필요합니다"));
        assert_eq!(h.agent, None); // 본 세션이 낸 신호

        let body = r#"{"hook_event_name":"SubagentStart","agent_id":"agent-abc123","agent_type":"Explore"}"#;
        let Some(Message::Hook(h)) = parse("3", body) else {
            panic!("훅으로 읽혀야 한다");
        };
        assert_eq!((h.event.as_str(), h.agent.as_deref()), ("SubagentStart", Some("agent-abc123")));
    }

    #[test]
    fn 세션_번호나_내용이_잘못되면_버린다() {
        assert!(parse("세션", "{}").is_none());
        assert!(parse("1", "JSON 아님").is_none());
    }
}
