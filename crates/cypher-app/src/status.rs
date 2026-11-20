//! 전달 프로그램(cypher-bridge)이 접속하는 로컬 통로.
//!
//! 접속마다 첫 줄은 확인값, 둘째 줄은 종류다. 둘째 줄에 적힌 세션 번호는 보낸 쪽이 스스로 적은 값이므로 믿지 않고,
//! 접속한 프로세스가 실제로 그 세션이 띄운 프로세스인지 확인한다(`peer.rs`). 아니면 받지 않는다.
//! 한 번 보내고 끊는 신호는 다 처리한 뒤에 연결을 닫는다. 전달 프로그램은 그때까지 기다렸다가 끝나므로,
//! Claude Code는 앱이 신호를 처리한 뒤에야 다음 일(파일 쓰기 등)을 한다.
//! - 세션 번호: 뒤따르는 내용이 Claude Code 상태줄 JSON 또는 훅 JSON(한 번 보내고 끊는다).
//!   훅 가운데 도구의 직전·직후·실패 신호는 화면에 그대로 넘기지 않는다. 도구 이름과 짧은 설명만 뽑아
//!   `session-tool`로 알리고(사이드바의 "지금 하는 일"), Write 도구의 신호에서는 새로 만든 파일을 가려
//!   `session-file`로 알린다(`created.rs`). 있던 파일을 고쳐 쓴 것(Write, Edit)은 `session-edit`로 알려,
//!   그 파일을 보여 주는 항목이 바로 다시 읽게 한다.
//! - `mcp <세션 번호>`: Cypher 도구용 MCP 연결(계속 이어진다)

use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
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
    /// "UserPromptSubmit" | "Stop" | "Notification" | "SubagentStart" | "SubagentStop" | "SessionEnd"
    event: String,
    /// Notification일 때의 종류(예: permission_prompt, idle_prompt), SessionEnd일 때의 끝난 까닭(예: clear).
    kind: Option<String>,
    message: Option<String>,
    /// 이 신호를 낸 서브에이전트의 번호(본 세션이 낸 신호면 없다).
    agent: Option<String>,
    /// 서브에이전트의 종류(예: Explore, general-purpose).
    agent_type: Option<String>,
    /// 응답 완료(Stop)일 때: 그때 아직 돌고 있는 배경 작업. Claude Code가 알려 주지 않았으면 없다.
    background: Option<Background>,
}

/// 응답 완료(Stop) 신호에 딸려 오는 "아직 돌고 있는 배경 작업"(`background_tasks`)에서 화면이 쓰는 것.
/// 화면은 이것으로 끝났는데도 남아 있는 서브에이전트 표시를 지운다(종료 신호가 오지 않는 경우가 있다).
#[derive(Clone, Serialize, Debug, PartialEq)]
struct Background {
    /// 돌고 있는 서브에이전트·팀원의 번호(번호가 없는 것은 빈 글).
    agents: Vec<String>,
    /// 돌고 있는 워크플로 수. 그 안에서 도는 서브에이전트는 목록에 따로 나오지 않을 수 있다.
    workflows: u32,
}

/// 훅 내용의 `background_tasks`를 읽는다. 그 값이 없으면(옛 버전, 작업 목록을 읽지 못한 때) None.
fn background(v: &Value) -> Option<Background> {
    let tasks = v["background_tasks"].as_array()?;
    let mut found = Background { agents: Vec::new(), workflows: 0 };
    for task in tasks {
        // 목록에는 돌고 있는 것만 온다고 하지만, 끝난 것으로 적혀 있으면 세지 않는다.
        let status = task["status"].as_str().unwrap_or("").to_ascii_lowercase();
        if matches!(status.as_str(), "completed" | "failed" | "killed" | "stopped" | "cancelled" | "canceled" | "done") {
            continue;
        }
        // 종류는 사람이 읽는 이름(subagent, workflow, teammate, shell …)이거나, 모르는 종류면 안에서 쓰는 이름 그대로다.
        let kind = task["type"].as_str().unwrap_or("").to_ascii_lowercase();
        if kind.contains("workflow") {
            found.workflows += 1;
        } else if (kind.contains("agent") || kind.contains("teammate")) && !kind.contains("remote") && !kind.contains("cloud") {
            found.agents.push(task["id"].as_str().unwrap_or("").to_string());
        }
    }
    Some(found)
}

/// Claude 세션이 새로 만든 파일. 화면이 그 세션의 묶음에 종류에 맞는 항목으로 놓는다.
#[derive(Clone, Serialize)]
struct FilePayload {
    id: u32,
    path: String,
    /// "doc" | "image" | "http" | "table" | "html" | "other"
    kind: &'static str,
}

/// Claude 세션이 고쳐 쓴 파일. 화면이 그 세션의 묶음에서 이 파일을 보여 주는 항목을 다시 읽게 한다.
#[derive(Clone, Serialize)]
struct EditPayload {
    id: u32,
    path: String,
}

/// Claude 세션이 지금 쓰는 도구. 화면이 사이드바의 그 세션 줄에 보여 준다.
/// 도구에 넘긴 내용(파일 내용, 긴 명령)은 넘기지 않고 짧게 줄인 설명만 넘긴다.
#[derive(Clone, Serialize, Debug, PartialEq)]
struct ToolPayload {
    id: u32,
    /// "start"(쓰기 직전) | "end"(끝남) | "fail"(실패했거나 중단됨)
    phase: &'static str,
    /// 보여 줄 도구 이름: Bash, Edit … (MCP 도구는 `mcp__서버__도구`의 도구 부분)
    tool: String,
    /// 한눈에 볼 설명: 명령의 첫 줄, 파일 이름, 찾는 글 …
    detail: String,
    /// 풍선말에 보일 조금 더 긴 설명: 전체 경로, 명령 앞부분 …
    full: String,
    /// 이 도구 호출의 번호. 시작과 끝을 짝짓는 데 쓴다.
    use_id: Option<String>,
    /// 이 도구를 쓴 서브에이전트의 번호(본 세션이 쓴 것이면 없다).
    agent: Option<String>,
}

enum Message {
    Status(StatusPayload),
    Hook(HookPayload),
    /// Write 도구가 파일을 쓰기 직전.
    WillWrite { id: u32, path: PathBuf },
    /// Write 도구가 파일을 쓴 직후. `base`는 그 세션이 일하고 있는 폴더.
    Wrote { id: u32, path: PathBuf, base: Option<PathBuf> },
    /// Edit 도구가 있던 파일을 고친 직후.
    Edited { id: u32, path: PathBuf },
}

/// 훅 내용 한 번의 크기 한도. Write 도구의 신호에는 쓴 파일의 내용이 통째로 들어 있다.
const MAX_BODY: u64 = 32 << 20;
/// 접속 머리의 한 줄(확인값, 세션 번호)에서 읽는 최대 길이.
const HEAD_LINE_MAX: u64 = 256;

/// 로컬 포트를 열고 받기 스레드를 띄운다. 열린 포트 번호를 돌려준다.
pub fn start(app: AppHandle, token: String) -> std::io::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            // 받기가 계속 실패하는 동안(손잡이·버퍼 부족 등) 쉬지 않고 돌며 CPU를 다 쓰지 않게 잠깐 쉰다.
            let Ok(stream) = stream else {
                thread::sleep(std::time::Duration::from_millis(50));
                continue;
            };
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
    static SEEN: LazyLock<Mutex<HashSet<Reason>>> = LazyLock::new(Default::default);
    let mut seen = SEEN.lock().unwrap();
    if seen.len() < 200 && seen.insert((claimed, owner)) {
        crate::log(&format!("통로: 받지 않은 접속(적힌 세션 {claimed:?}, 실제 {owner:?}). 같은 사유는 다시 적지 않습니다."));
    }
}

/// 접속의 둘째 줄: 무엇을 하러 왔고, 몇 번 세션이라고 하는지.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    /// `mcp <세션 번호>`: Cypher 도구용 연결(계속 이어진다).
    Mcp(u32),
    /// `<세션 번호>`: 상태줄이나 훅 신호 하나(한 번 보내고 끊는다).
    Signal(u32),
}

impl Kind {
    fn read(line: &str) -> Option<Kind> {
        let line = line.trim();
        match line.strip_prefix("mcp ") {
            Some(session) => session.trim().parse().ok().map(Kind::Mcp),
            None => line.parse().ok().map(Kind::Signal),
        }
    }

    /// 보낸 쪽이 적은 세션 번호.
    fn session(self) -> u32 {
        match self {
            Kind::Mcp(session) | Kind::Signal(session) => session,
        }
    }
}

fn accept(app: AppHandle, token: &str, stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let Ok(writer) = stream.try_clone() else { return };
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    // 머리의 두 줄(확인값, 세션 번호)은 짧다. 줄바꿈 없이 끝없이 보내는 접속이 메모리를 채우지 못하게 길이를 묶는다.
    if (&mut reader).take(HEAD_LINE_MAX).read_line(&mut line).is_err() || line.trim_end() != token {
        return; // 확인값이 다르면 아무것도 하지 않는다.
    }
    // 확인값이 맞는 접속만 누가 보냈는지 알아본다(전달 프로그램은 답을 기다리고 있으므로 아직 살아 있다).
    let owner = owner_of(&app, reader.get_ref());
    line.clear();
    if (&mut reader).take(HEAD_LINE_MAX).read_line(&mut line).is_err() {
        return;
    }
    // 적힌 세션 번호가 접속한 프로세스의 세션과 다르면(다른 세션인 척하거나, 세션 밖의 프로세스) 받지 않는다.
    let kind = Kind::read(&line);
    let claimed = kind.map(Kind::session);
    if owner.is_none() || claimed != owner {
        log_refused(claimed, owner);
        return;
    }
    let id = match kind {
        Some(Kind::Mcp(session)) => {
            // MCP 연결은 요청 사이에 오래 조용할 수 있다. 읽는 쪽 손잡이의 시간 제한을 직접 푼다
            // (복제한 손잡이에서 풀어도 읽는 쪽에는 적용되지 않는다).
            let _ = reader.get_ref().set_read_timeout(None);
            return crate::mcp::serve(app, Some(session), reader, writer);
        }
        Some(Kind::Signal(id)) => id,
        None => return,
    };
    let mut body = String::new();
    if reader.take(MAX_BODY).read_to_string(&mut body).is_err() {
        return;
    }
    let Ok(value) = serde_json::from_str::<Value>(&body) else { return };
    // 도구 신호를 무엇에 쓸지는 설정을 따른다. 지금 쓰는 도구를 보여 주려고 모든 도구에 훅을 걸었을 때에도,
    // 만든 파일 놓기를 꺼 두었으면 Write 도구의 신호로 파일을 놓지 않는다.
    let (activity, files) = {
        let state = app.state::<crate::AppState>();
        (state.config.session.activity, state.config.session.show_created != "off")
    };
    if activity {
        if let Some(tool) = tool_signal(id, &value) {
            let _ = app.emit("session-tool", tool);
        }
    }
    let _ = match read(id, &value) {
        Some(Message::Status(payload)) => app.emit("session-status", payload),
        Some(Message::Hook(payload)) => app.emit("session-hook", payload),
        Some(Message::WillWrite { id, path }) if files => {
            crate::created::before_write(id, &path);
            Ok(())
        }
        Some(Message::Wrote { id, path, base }) => {
            let fresh = if files { crate::created::after_write(id, &path, base.as_deref()) } else { None };
            let path = path.to_string_lossy().into_owned();
            match fresh {
                Some(kind) => app.emit("session-file", FilePayload { id, path, kind }),
                // 사이드바에 새로 놓을 파일이 아니면(있던 파일을 고쳐 썼다 등) 그 파일을 보여 주는 항목만 다시 읽게 한다.
                None => app.emit("session-edit", EditPayload { id, path }),
            }
        }
        Some(Message::Edited { id, path }) => app.emit("session-edit", EditPayload { id, path: path.to_string_lossy().into_owned() }),
        _ => Ok(()),
    };
    // 여기서 연결이 닫힌다. 전달 프로그램은 이것을 "처리했다"는 답으로 삼고 끝난다.
}

/// 도구 직전·직후·실패 신호인지.
fn is_tool_event(event: &str) -> bool {
    matches!(event, "PreToolUse" | "PostToolUse" | "PostToolUseFailure")
}

/// 글을 한 줄로 만들고(줄바꿈·잇단 공백은 한 칸) `max`자까지만 남긴다. 잘랐으면 끝에 …을 붙인다.
fn brief(text: &str, max: usize) -> String {
    let mut out = String::new();
    let mut count = 0;
    for word in text.split_whitespace() {
        let gap = (count > 0).then_some(' ');
        for ch in gap.into_iter().chain(word.chars()) {
            if count == max {
                out.push('…');
                return out;
            }
            out.push(ch);
            count += 1;
        }
    }
    out
}

/// 경로의 마지막 이름(파일 이름).
fn file_name(path: &str) -> &str {
    path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(path)
}

const DETAIL_MAX: usize = 60;
const FULL_MAX: usize = 300;

/// 도구 이름과 넘긴 값에서 보여 줄 이름, 짧은 설명, 조금 더 긴 설명을 뽑는다.
/// 넘긴 값의 모양을 모르는 도구(MCP 도구 등)는 흔한 이름의 글자 값 하나를 설명으로 쓴다.
fn describe_tool(name: &str, input: &Value) -> (String, String, String) {
    let field = |key: &str| input[key].as_str().map(str::trim).filter(|text| !text.is_empty());
    // MCP 도구: `mcp__서버__도구`. 도구 이름만 보이고, 따로 보일 설명이 없으면 서버 이름을 설명으로 쓴다.
    let (label, server) = match name.strip_prefix("mcp__").and_then(|rest| rest.rsplit_once("__")) {
        Some((server, tool)) if !tool.is_empty() => (tool, Some(server)),
        _ => (name, None),
    };
    let path = ["file_path", "notebook_path"].into_iter().find_map(field);
    let (detail, full) = if let Some(path) = path {
        (file_name(path).to_string(), path.to_string())
    } else if let Some(text) = ["command", "pattern", "query", "url", "description", "skill", "path", "prompt", "title", "name", "text"]
        .into_iter()
        .find_map(field)
    {
        // 명령은 첫 줄만 짧게 보이고, 풍선말에는 앞부분을 조금 더 보인다.
        (text.lines().find(|line| !line.trim().is_empty()).unwrap_or(text).to_string(), text.to_string())
    } else {
        let server = server.unwrap_or_default().to_string();
        (server.clone(), server)
    };
    (label.to_string(), brief(&detail, DETAIL_MAX), brief(&full, FULL_MAX))
}

/// 도구 직전·직후·실패 신호에서 화면에 보일 것만 뽑는다. 도구 신호가 아니면 None.
fn tool_signal(id: u32, v: &Value) -> Option<ToolPayload> {
    let phase = match v["hook_event_name"].as_str()? {
        "PreToolUse" => "start",
        "PostToolUse" => "end",
        "PostToolUseFailure" => "fail",
        _ => return None,
    };
    let name = v["tool_name"].as_str().map(str::trim).filter(|name| !name.is_empty())?;
    let (tool, detail, full) = describe_tool(name, &v["tool_input"]);
    let text = |v: &Value| v.as_str().filter(|text| !text.is_empty()).map(str::to_string);
    Some(ToolPayload { id, phase, tool, detail, full, use_id: text(&v["tool_use_id"]), agent: text(&v["agent_id"]) })
}

/// 상태줄 JSON과 훅 JSON이 같은 통로로 온다. 훅에는 `hook_event_name`이 있다.
#[cfg(test)]
fn parse(id: u32, body: &str) -> Option<Message> {
    read(id, &serde_json::from_str(body).ok()?)
}

fn read(id: u32, v: &Value) -> Option<Message> {
    let text = |v: &Value| v.as_str().map(str::to_string);
    if let Some(event) = text(&v["hook_event_name"]) {
        // 도구 신호에서는 파일을 쓰는 도구가 쓴 파일만 본다(지금 쓰는 도구는 `tool_signal`이 따로 뽑는다).
        // Write 도구는 직전(그 파일이 이미 있었는지)과 직후를, 있던 파일을 고치는 Edit 도구는 직후만 본다.
        if is_tool_event(&event) {
            return match (v["tool_name"].as_str()?, event.as_str()) {
                ("Write", "PreToolUse") => Some(Message::WillWrite { id, path: written_path(v)? }),
                ("Write", "PostToolUse") => {
                    Some(Message::Wrote { id, path: written_path(v)?, base: local_path(v["cwd"].as_str().unwrap_or("")) })
                }
                ("Edit" | "MultiEdit", "PostToolUse") => Some(Message::Edited { id, path: written_path(v)? }),
                _ => None,
            };
        }
        return Some(Message::Hook(HookPayload {
            id,
            background: if event == "Stop" { background(v) } else { None },
            event,
            // SessionEnd에서는 끝난 까닭(clear, logout, prompt_input_exit …)을 종류 자리에 싣는다.
            kind: text(&v["notification_type"]).or_else(|| text(&v["reason"])),
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
    fn 둘째_줄에서_종류와_세션_번호를_읽는다() {
        assert_eq!(Kind::read("3\n"), Some(Kind::Signal(3)));
        assert_eq!(Kind::read("mcp 12\r\n"), Some(Kind::Mcp(12)));
        assert_eq!(Kind::read("mcp 12").map(Kind::session), Some(12));
        // 번호가 없거나 모양이 다르면 받지 않는다.
        for bad in ["", "mcp", "mcp ", "mcpx 3", "mcp3", "mcp 3 4", "-1", "세션"] {
            assert_eq!(Kind::read(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn 쓰기_도구의_신호에서_파일_경로를_꺼낸다() {
        let pre = r#"{"hook_event_name":"PreToolUse","tool_name":"Write","cwd":"C:\\ws\\proj","tool_input":{"file_path":"C:/ws/proj/docs/계획.md","content":"계획 글"}}"#;
        let Some(Message::WillWrite { id, path }) = parse(4, pre) else { panic!("쓰기 직전 신호로 읽혀야 한다") };
        assert_eq!((id, path), (4, PathBuf::from(r"C:\ws\proj\docs\계획.md")));

        let post = r#"{"hook_event_name":"PostToolUse","tool_name":"Write","cwd":"C:\\ws\\proj","tool_input":{"file_path":"notes/a.txt"},"tool_response":{"type":"create","filePath":"x"}}"#;
        let Some(Message::Wrote { id, path, base }) = parse(4, post) else { panic!("쓴 직후 신호로 읽혀야 한다") };
        assert_eq!((id, path, base), (4, PathBuf::from(r"C:\ws\proj\notes\a.txt"), Some(PathBuf::from(r"C:\ws\proj"))));

        // 긴 경로 표시는 떼어 내고, 세션 폴더를 모르면 base가 없다.
        let long = r#"{"hook_event_name":"PostToolUse","tool_name":"Write","tool_input":{"file_path":"\\\\?\\C:\\a.md"}}"#;
        let Some(Message::Wrote { path, base, .. }) = parse(4, long) else { panic!("쓴 직후 신호로 읽혀야 한다") };
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
            assert!(parse(4, other).is_none(), "{other}");
        }
    }

    #[test]
    fn 고치는_도구의_직후_신호에서_고친_파일을_꺼낸다() {
        for tool in ["Edit", "MultiEdit"] {
            let post = format!(
                r#"{{"hook_event_name":"PostToolUse","tool_name":"{tool}","cwd":"C:\\ws\\proj","tool_input":{{"file_path":"docs/설명서.html","old_string":"a","new_string":"b"}}}}"#
            );
            let Some(Message::Edited { id, path }) = parse(4, &post) else { panic!("{tool}: 고친 직후 신호로 읽혀야 한다") };
            assert_eq!((id, path), (4, PathBuf::from(r"C:\ws\proj\docs\설명서.html")));
        }
        // 고치기 직전의 신호, 파일을 쓰지 않는 도구, 경로를 알 수 없는 신호는 버린다.
        for other in [
            r#"{"hook_event_name":"PreToolUse","tool_name":"Edit","tool_input":{"file_path":"C:/ws/a.md"}}"#,
            r#"{"hook_event_name":"PostToolUse","tool_name":"Read","tool_input":{"file_path":"C:/ws/a.md"}}"#,
            r#"{"hook_event_name":"PostToolUse","tool_name":"Edit","tool_input":{"file_path":"//server/share/a.md"}}"#,
            r#"{"hook_event_name":"PostToolUse","tool_name":"Edit","tool_input":{}}"#,
        ] {
            assert!(parse(4, other).is_none(), "{other}");
        }
    }

    #[test]
    fn 상태줄_내용에서_모델과_effort를_꺼낸다() {
        let body = r#"{"cwd":"C:\\ws","model":{"id":"claude-fable-5-2","display_name":"Fable 5.2"},"effort":{"level":"high"},"context_window":{"used_percentage":8},"rate_limits":{"seven_day":{"used_percentage":86}}}"#;
        let Some(Message::Status(p)) = parse(7, body) else {
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
        let Some(Message::Hook(h)) = parse(3, body) else {
            panic!("훅으로 읽혀야 한다");
        };
        assert_eq!((h.id, h.event.as_str()), (3, "Notification"));
        assert_eq!(h.kind.as_deref(), Some("permission_prompt"));
        assert_eq!(h.message.as_deref(), Some("승인이 필요합니다"));
        assert_eq!(h.agent, None); // 본 세션이 낸 신호

        let body = r#"{"hook_event_name":"SubagentStart","agent_id":"agent-abc123","agent_type":"Explore"}"#;
        let Some(Message::Hook(h)) = parse(3, body) else {
            panic!("훅으로 읽혀야 한다");
        };
        assert_eq!((h.event.as_str(), h.agent.as_deref()), ("SubagentStart", Some("agent-abc123")));
        assert_eq!(h.background, None); // 돌고 있는 배경 작업은 응답 완료 신호에서만 읽는다.
    }

    #[test]
    fn 완료_신호에서_아직_돌고_있는_배경_작업을_읽는다() {
        let stop = |body: &str| match parse(3, body) {
            Some(Message::Hook(h)) => h.background,
            _ => panic!("훅으로 읽혀야 한다"),
        };
        // 서브에이전트·팀원은 번호를, 워크플로는 수를 센다. 셸 같은 다른 작업과 끝난 것으로 적힌 것은 세지 않는다.
        let body = r#"{"hook_event_name":"Stop","background_tasks":[
            {"id":"a1","type":"subagent","status":"running","agent_type":"Explore"},
            {"id":"t1","type":"teammate","status":"running"},
            {"type":"local_agent","status":"running"},
            {"id":"w1","type":"workflow","status":"running","name":"review"},
            {"id":"s1","type":"shell","status":"running","command":"tail -f log"},
            {"id":"a2","type":"subagent","status":"completed"}]}"#;
        assert_eq!(stop(body), Some(Background { agents: vec!["a1".into(), "t1".into(), String::new()], workflows: 1 }));
        // 돌고 있는 것이 없으면 빈 목록이고, 목록을 주지 않았으면(옛 버전) 알 수 없다.
        assert_eq!(stop(r#"{"hook_event_name":"Stop","background_tasks":[]}"#), Some(Background { agents: vec![], workflows: 0 }));
        assert_eq!(stop(r#"{"hook_event_name":"Stop"}"#), None);
        assert_eq!(stop(r#"{"hook_event_name":"Stop","background_tasks":"x"}"#), None);
        // 서브에이전트 종료 신호에 딸려 온 목록은 쓰지 않는다(본 세션이 기다리며 돌리는 서브에이전트는 배경 작업이 아니라 그 목록에 없을 수 있다).
        let Some(Message::Hook(h)) = parse(3, r#"{"hook_event_name":"SubagentStop","agent_id":"a1","background_tasks":[]}"#) else {
            panic!("훅으로 읽혀야 한다")
        };
        assert_eq!(h.background, None);
    }

    #[test]
    fn 도구_신호에서_이름과_짧은_설명을_뽑는다() {
        let signal = |body: &str| tool_signal(5, &serde_json::from_str(body).unwrap());
        let shown = |body: &str| signal(body).map(|t| (t.phase, t.tool, t.detail));
        let pre = r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_use_id":"toolu_1","tool_input":{"command":"\n  cargo build   --workspace\necho done","description":"빌드"}}"#;
        let tool = signal(pre).unwrap();
        assert_eq!((tool.id, tool.phase, tool.tool.as_str(), tool.detail.as_str()), (5, "start", "Bash", "cargo build --workspace"));
        assert_eq!(tool.full, "cargo build --workspace echo done");
        assert_eq!((tool.use_id.as_deref(), tool.agent), (Some("toolu_1"), None));

        // 파일을 다루는 도구는 파일 이름을, 풍선말에는 전체 경로를 보인다. 파일 내용은 넘기지 않는다.
        let edit = r#"{"hook_event_name":"PostToolUse","tool_name":"Edit","agent_id":"agent-1","tool_input":{"file_path":"C:\\ws\\ui\\main.js","old_string":"비밀","new_string":"x"}}"#;
        let tool = signal(edit).unwrap();
        assert_eq!((tool.phase, tool.tool.as_str(), tool.detail.as_str(), tool.full.as_str()), ("end", "Edit", "main.js", r"C:\ws\ui\main.js"));
        assert_eq!(tool.agent.as_deref(), Some("agent-1"));
        assert_eq!(
            shown(r#"{"hook_event_name":"PostToolUseFailure","tool_name":"Read","tool_input":{"file_path":"src/없는 파일.rs"},"error":"없음"}"#),
            Some(("fail", "Read".into(), "없는 파일.rs".into()))
        );
        assert_eq!(shown(r#"{"hook_event_name":"PreToolUse","tool_name":"Grep","tool_input":{"pattern":"fn main","path":"src"}}"#), Some(("start", "Grep".into(), "fn main".into())));
        // MCP 도구는 도구 이름만 보이고, 설명할 값이 없으면 서버 이름을 보인다.
        assert_eq!(
            shown(r#"{"hook_event_name":"PreToolUse","tool_name":"mcp__cypher__browser_navigate","tool_input":{"url":"https://example.com/a"}}"#),
            Some(("start", "browser_navigate".into(), "https://example.com/a".into()))
        );
        assert_eq!(shown(r#"{"hook_event_name":"PreToolUse","tool_name":"mcp__cypher__browser_read","tool_input":{}}"#), Some(("start", "browser_read".into(), "cypher".into())));
        assert_eq!(shown(r#"{"hook_event_name":"PreToolUse","tool_name":"TodoWrite","tool_input":{"todos":[{"content":"x"}]}}"#), Some(("start", "TodoWrite".into(), String::new())));
        // 긴 설명은 잘라서 넘긴다.
        let long = format!(r#"{{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{{"command":"{}"}}}}"#, "가".repeat(500));
        let tool = signal(&long).unwrap();
        assert_eq!((tool.detail.chars().count(), tool.full.chars().count()), (DETAIL_MAX + 1, FULL_MAX + 1));
        assert!(tool.detail.ends_with('…'));
        // 도구 신호가 아니거나 도구 이름이 없으면 뽑지 않는다.
        for other in [
            r#"{"hook_event_name":"Stop"}"#,
            r#"{"hook_event_name":"PreToolUse","tool_input":{"command":"ls"}}"#,
            r#"{"model":{"id":"x"}}"#,
        ] {
            assert_eq!(signal(other), None, "{other}");
        }
        // 도구 신호는 훅 알림으로 화면에 넘기지 않는다(파일을 쓰는 도구의 실패 신호도 파일로 치지 않는다).
        assert!(parse(5, pre).is_none());
        assert!(!matches!(parse(5, edit), Some(Message::Hook(_))));
        for tool in ["Write", "Edit"] {
            let failed = format!(r#"{{"hook_event_name":"PostToolUseFailure","tool_name":"{tool}","tool_input":{{"file_path":"C:/a.md"}}}}"#);
            assert!(parse(5, &failed).is_none(), "{tool}");
        }
    }

    #[test]
    fn 세션_번호나_내용이_잘못되면_버린다() {
        assert!(parse(1, "JSON 아님").is_none());
    }
}
