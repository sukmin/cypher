//! 전달 프로그램(cypher-bridge)이 접속하는 로컬 통로.
//!
//! 접속마다 첫 줄은 확인값, 둘째 줄은 종류다.
//! - 세션 번호: 뒤따르는 내용이 Claude Code 상태줄 JSON 또는 훅 JSON(한 번 보내고 끊는다)
//! - `mcp <세션 번호>`: Cypher 도구용 MCP 연결(계속 이어진다)

use serde::Serialize;
use serde_json::Value;
use std::io::{BufRead, BufReader, Read};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

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

enum Message {
    Status(StatusPayload),
    Hook(HookPayload),
}

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

fn accept(app: AppHandle, token: &str, stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let Ok(writer) = stream.try_clone() else { return };
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() || line.trim_end() != token {
        return; // 확인값이 다르면 아무것도 하지 않는다.
    }
    line.clear();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let kind = line.trim_end();
    // `mcp` 또는 `mcp <세션 번호>`: 어느 Claude 세션이 연 연결인지 함께 온다(예전 전달 프로그램은 번호를 보내지 않는다).
    if let Some(rest) = kind.strip_prefix("mcp").filter(|rest| rest.is_empty() || rest.starts_with(' ')) {
        let session = rest.trim().parse::<u32>().ok();
        // MCP 연결은 요청 사이에 오래 조용할 수 있다. 읽는 쪽 손잡이의 시간 제한을 직접 푼다
        // (복제한 손잡이에서 풀어도 읽는 쪽에는 적용되지 않는다).
        let _ = reader.get_ref().set_read_timeout(None);
        crate::mcp::serve(app, session, reader, writer);
        return;
    }
    let mut body = String::new();
    if reader.take(1 << 20).read_to_string(&mut body).is_err() {
        return;
    }
    let _ = match parse(kind, &body) {
        Some(Message::Status(payload)) => app.emit("session-status", payload),
        Some(Message::Hook(payload)) => app.emit("session-hook", payload),
        None => Ok(()),
    };
}

/// 상태줄 JSON과 훅 JSON이 같은 통로로 온다. 훅에는 `hook_event_name`이 있다.
fn parse(session: &str, body: &str) -> Option<Message> {
    let id = session.trim().parse().ok()?;
    let v: Value = serde_json::from_str(body).ok()?;
    let text = |v: &Value| v.as_str().map(str::to_string);
    if let Some(event) = text(&v["hook_event_name"]) {
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

#[cfg(test)]
mod tests {
    use super::*;

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
