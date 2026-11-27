//! Claude 세션에 넘기는 설정 파일(`--settings`) 만들기.
//!
//! Cypher는 모델·effort·완료 신호를 받으려고 상태줄과 훅 자리에 전달 프로그램을 넣는다.
//! 그런데 Claude Code의 상태줄 자리는 하나뿐이라, 사용자가 이미 쓰던 상태줄(HUD)이 있으면
//! 가려진다. 그래서 그 세션에서 원래 쓰였을 상태줄 명령을 찾아 전달 프로그램에 알려 주고,
//! 전달 프로그램이 정보를 Cypher로 넘긴 뒤 그 명령을 이어서 실행해 출력을 그대로 내보내게 한다.

use crate::extras::claude_home;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// 사용자가 원래 쓰던 상태줄 설정.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusLine {
    pub command: String,
    padding: Option<Value>,
    /// 적혀 있던 `refreshInterval` 값 그대로(쓸 수 있는 값인지는 `refresh_interval`이 가린다).
    refresh: Option<Value>,
}

/// 원래 상태줄이 정해 둔 갱신 주기(초) 가운데 Claude Code가 받아들이는 것만: 1 이상의 수(소수도 됨).
/// 0·음수·수가 아닌 값은 Claude Code가 "주기 없음"으로 읽는다. 그런 값을 1초로 올려 넣으면 쉬는 세션마다 1초에 한 번씩
/// 상태줄이 돌게 되므로(프로세스 넷), Cypher도 주기 없음으로 둔다.
fn refresh_interval(original: Option<&StatusLine>) -> Option<&Value> {
    original?.refresh.as_ref().filter(|value| value.as_f64().is_some_and(|seconds| seconds >= 1.0))
}

/// 설정 파일 하나에서 명령형 상태줄을 읽는다. Cypher의 전달 프로그램을 가리키는 것은 뺀다.
fn read_status_line(file: &Path) -> Option<StatusLine> {
    let text = std::fs::read_to_string(file).ok()?;
    let value: Value = serde_json::from_str(text.strip_prefix('\u{feff}').unwrap_or(&text)).ok()?;
    let line = value.get("statusLine")?;
    if line.get("type").and_then(Value::as_str).is_some_and(|t| t != "command") {
        return None;
    }
    let command = line.get("command")?.as_str()?.trim();
    if command.is_empty() || command.contains("cypher-bridge") {
        return None;
    }
    Some(StatusLine {
        command: command.to_string(),
        padding: line.get("padding").cloned(),
        refresh: line.get("refreshInterval").cloned(),
    })
}

/// 이 폴더에서 연 세션에 원래 적용됐을 상태줄. Claude Code의 우선순위대로
/// 폴더의 개인 설정 → 폴더 설정 → 사용자 공통 설정 순으로 찾는다.
pub fn original_status_line(cwd: &Path) -> Option<StatusLine> {
    let mut files = vec![
        cwd.join(".claude").join("settings.local.json"),
        cwd.join(".claude").join("settings.json"),
    ];
    files.extend(claude_home().map(|home| home.join("settings.json")));
    files.iter().find_map(|file| read_status_line(file))
}

/// 파일을 쓰는 도구(훅의 matcher 모양). 이 도구들의 직후 신호로 "세션이 파일을 썼다"는 것을 안다(`status.rs`).
const WRITING_TOOLS: &str = "Write|Edit|MultiEdit";

/// 세션 하나의 설정 내용: 상태줄과 훅을 전달 프로그램으로 지정한다.
/// `bridge`는 셸이 읽을 호출문이다(경로에 셸이 달리 읽는 글자가 있으면 따옴표로 감싸 온다).
/// 원래 상태줄이 있으면 그 여백과 갱신 주기를 따른다. Cypher가 따로 거는 갱신 주기는 없다.
/// `watch_files`이면 Write 도구의 직전 훅과 파일을 쓰는 도구의 직후 훅을 건다(세션이 새로 만든 파일을 사이드바에 놓고,
/// 고쳐 쓴 파일을 보여 주는 항목을 다시 읽게 하는 데 쓴다).
/// `activity`이면 모든 도구의 직전·직후·실패 훅을 건다(사이드바의 그 줄에 지금 쓰는 도구를 보여 주는 데 쓴다).
/// 둘 다 아니면 도구 훅을 걸지 않는다.
fn settings_json(bridge: &str, original: Option<&StatusLine>, watch_files: bool, activity: bool) -> Value {
    // 훅: 요청 시작(UserPromptSubmit), 응답 완료(Stop), 입력·승인 대기(Notification),
    // 서브에이전트 시작·종료(SubagentStart·SubagentStop), Claude Code 종료(SessionEnd: 셸 세션에서 직접 띄운
    // Claude가 끝났을 때 그 줄의 모델·상태 표시를 지우는 데 쓴다).
    // 훅은 셸을 거치지 않고 전달 프로그램을 바로 띄운다(`args`가 있으면 Claude Code가 `command`를 실행 파일로 보고 그대로 띄운다).
    // 셸을 거치면 Windows에서는 신호 하나마다 Git Bash가 두 번 떠서, 바로 띄울 때보다 CPU가 다섯 배쯤 든다.
    // 도구 훅을 걸면 도구를 쓸 때마다 두 번씩 오므로 이 차이가 크다. `args`를 모르는 옛 Claude Code는 이 칸을 버리고
    // `command`를 셸로 돌리므로, 경로가 셸에 평범하면(보통의 경우) 그때도 전처럼 동작한다.
    let program = bridge.trim_matches('"');
    let run = json!({ "type": "command", "command": program, "args": [] });
    let hook = json!([{ "hooks": [run.clone()] }]);
    // 상태줄에는 셸을 거치지 않는 형식이 없다(Claude Code는 상태줄 명령을 늘 셸로 돌린다).
    let mut status = json!({ "type": "command", "command": bridge });
    // 상태줄은 일이 생길 때마다 Claude Code가 돌린다(세션 시작, 새 답변, `/compact`, 권한 모드 변경, 사용 한도 초기화 시각).
    // 모델·컨텍스트·비용·한도도 그때 바뀌므로, Cypher는 쉬는 동안 다시 돌리는 주기(`refreshInterval`)를 걸지 않는다.
    // Windows에서 상태줄은 늘 Git Bash로 돌아서 한 번 돌 때마다 프로세스가 넷(Git Bash 실행기, 콘솔 호스트, bash,
    // 전달 프로그램) 뜨고 백신 검사도 따라붙는다. 주기를 걸면 쉬는 세션마다 그만큼 CPU를 계속 쓴다(예전에는 5초마다 돌렸다).
    // 원래 상태줄이 주기를 정해 두었으면(시계를 보이는 HUD 등) 그 주기만 따른다.
    // 그래서 쉬는 동안 `/model`·`/effort`로 바꾼 것은 다음 답변이 올 때 사이드바에 보인다(이 둘은 상태줄을 다시 돌리지 않는다).
    if let Some(refresh) = refresh_interval(original) {
        status["refreshInterval"] = refresh.clone();
    }
    if let Some(padding) = original.and_then(|o| o.padding.clone()) {
        status["padding"] = padding;
    }
    let mut settings = json!({
        "statusLine": status,
        "hooks": {
            "UserPromptSubmit": hook, "Stop": hook, "Notification": hook,
            "SubagentStart": hook, "SubagentStop": hook, "SessionEnd": hook
        }
    });
    let tools = |matcher: &str| json!([{ "matcher": matcher, "hooks": [run.clone()] }]);
    if activity {
        // 지금 쓰는 도구를 보여 주려면 모든 도구의 직전·직후를 본다. 실패하거나 중단된 도구에는 직후 신호 대신
        // 실패 신호가 온다. 받아야 "하는 중" 표시가 남지 않는다.
        for event in ["PreToolUse", "PostToolUse", "PostToolUseFailure"] {
            settings["hooks"][event] = tools("*");
        }
    } else if watch_files {
        // 세션이 쓴 파일만 보려면 파일을 쓰는 도구만 본다. 새로 만든 파일인지는 Write 도구의 직전(그 파일이 이미 있었는지)과
        // 직후(실제로 생겼는지)로 가리고, 있던 파일을 고친 것은 직후 신호로 안다(그 파일을 보여 주는 항목이 다시 읽는다).
        settings["hooks"]["PreToolUse"] = tools("Write");
        settings["hooks"]["PostToolUse"] = tools(WRITING_TOOLS);
    }
    settings
}

/// 세션용 설정 파일을 쓴다. 사용자의 settings.json은 건드리지 않고 `--settings`로만 넘긴다.
/// 파일 이름에 실행 번호와 세션 번호를 넣어 다른 창·다른 세션과 겹치지 않게 한다.
pub fn write_settings(bridge: &str, session: u32, original: Option<&StatusLine>, watch_files: bool, activity: bool) -> Option<PathBuf> {
    let dir = crate::data_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("claude-settings-{}-{session}.json", std::process::id()));
    std::fs::write(&path, settings_json(bridge, original, watch_files, activity).to_string()).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cypher-statusline-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        dir
    }

    #[test]
    fn 폴더_설정에서_원래_상태줄을_찾는다() {
        let dir = project("find");
        std::fs::write(
            dir.join(".claude").join("settings.json"),
            r#"{"statusLine":{"type":"command","command":"bash ~/hud.sh","padding":2,"refreshInterval":1}}"#,
        )
        .unwrap();
        let found = read_status_line(&dir.join(".claude").join("settings.json")).unwrap();
        assert_eq!(found.command, "bash ~/hud.sh");
        assert_eq!(original_status_line(&dir).unwrap().command, "bash ~/hud.sh");

        // 폴더의 개인 설정이 있으면 그쪽이 먼저다.
        std::fs::write(dir.join(".claude").join("settings.local.json"), r#"{"statusLine":{"command":"my-hud"}}"#).unwrap();
        assert_eq!(original_status_line(&dir).unwrap().command, "my-hud");
    }

    #[test]
    fn 쓸_수_없는_상태줄은_건너뛴다() {
        let dir = project("skip");
        let file = dir.join(".claude").join("settings.json");
        for text in [
            r#"{"statusLine":{"type":"command","command":"C:/x/cypher-bridge.exe"}}"#, // 우리 것
            r#"{"statusLine":{"type":"other","command":"x"}}"#,
            r#"{"statusLine":{"type":"command","command":"  "}}"#,
            r#"{"model":"x"}"#,
            "JSON 아님",
        ] {
            std::fs::write(&file, text).unwrap();
            assert_eq!(read_status_line(&file), None, "{text}");
        }
    }

    #[test]
    fn 설정에는_전달_프로그램을_넣고_훅은_셸_없이_띄운다() {
        let plain = settings_json("C:/app/cypher-bridge.exe", None, false, false);
        assert!(plain["hooks"].get("PreToolUse").is_none() && plain["hooks"].get("PostToolUse").is_none());
        // 세션이 쓴 파일을 보려면 Write 도구의 직전 훅과, 파일을 쓰는 도구(고치는 도구 포함)의 직후 훅을 건다.
        let watching = settings_json("C:/app/cypher-bridge.exe", None, true, false);
        assert_eq!(watching["hooks"]["PreToolUse"][0]["matcher"], "Write");
        assert_eq!(watching["hooks"]["PostToolUse"][0]["matcher"], "Write|Edit|MultiEdit");
        for event in ["PreToolUse", "PostToolUse"] {
            assert_eq!(watching["hooks"][event][0]["hooks"][0]["command"], "C:/app/cypher-bridge.exe");
            assert_eq!(watching["hooks"][event][0]["hooks"][0]["args"], json!([]));
        }
        assert!(watching["hooks"].get("PostToolUseFailure").is_none());
        // 지금 쓰는 도구를 보여 주려면 모든 도구의 직전·직후·실패 훅을 건다(만든 파일 놓기를 꺼 두었어도).
        for files in [false, true] {
            let active = settings_json("C:/app/cypher-bridge.exe", None, files, true);
            for event in ["PreToolUse", "PostToolUse", "PostToolUseFailure"] {
                assert_eq!(active["hooks"][event][0]["matcher"], "*");
                assert_eq!(active["hooks"][event][0]["hooks"][0]["command"], "C:/app/cypher-bridge.exe");
            }
        }
        assert_eq!(watching["hooks"]["Stop"], plain["hooks"]["Stop"]);
        assert_eq!(plain["statusLine"]["command"], "C:/app/cypher-bridge.exe");
        assert!(plain["statusLine"].get("args").is_none()); // 상태줄은 셸로만 돈다
        assert!(plain["statusLine"].get("padding").is_none());
        // 훅은 셸을 거치지 않고 전달 프로그램을 바로 띄운다(`args`가 있으면 exec 형식).
        for event in ["UserPromptSubmit", "Stop", "Notification", "SubagentStart", "SubagentStop", "SessionEnd"] {
            let run = &plain["hooks"][event][0]["hooks"][0];
            assert_eq!((run["command"].clone(), run["args"].clone()), (json!("C:/app/cypher-bridge.exe"), json!([])), "{event}");
        }
        // 셸용으로 따옴표를 두른 경로는 상태줄에만 그대로 쓰고, 바로 띄우는 훅에는 따옴표를 떼어 넘긴다.
        let quoted = settings_json("\"C:/없는 폴더/cypher-bridge.exe\"", None, false, false);
        assert_eq!(quoted["statusLine"]["command"], "\"C:/없는 폴더/cypher-bridge.exe\"");
        assert_eq!(quoted["hooks"]["Stop"][0]["hooks"][0]["command"], "C:/없는 폴더/cypher-bridge.exe");
    }

    #[test]
    fn 쉬는_동안의_갱신_주기는_원래_상태줄이_정해_둔_것만_따른다() {
        let refresh = |original: Option<&StatusLine>| settings_json("b", original, false, false)["statusLine"].get("refreshInterval").cloned();
        // 원래 상태줄이 없거나 주기를 정하지 않았으면 주기를 걸지 않는다(일이 생길 때만 Claude Code가 돌린다).
        assert_eq!(refresh(None), None);
        let line = |refresh: Option<Value>| StatusLine { command: "hud".into(), padding: Some(json!(2)), refresh };
        assert_eq!(refresh(Some(&line(None))), None);
        // 정해 두었으면(시계를 보이는 HUD 등) 그 주기를 그대로 따른다. 소수도 Claude Code가 받는 값이다.
        assert_eq!(refresh(Some(&line(Some(json!(1))))), Some(json!(1)));
        assert_eq!(refresh(Some(&line(Some(json!(3600))))), Some(json!(3600)));
        assert_eq!(refresh(Some(&line(Some(json!(1.5))))), Some(json!(1.5)));
        // Claude Code가 "주기 없음"으로 읽는 값(1보다 작은 수, 수가 아닌 값)은 주기를 걸지 않는다.
        // 1초로 올려 넣으면 쉬는 세션마다 1초에 한 번씩 상태줄이 돈다.
        for bad in [json!(0), json!(-5), json!(0.5), json!("5"), json!(null), json!(true)] {
            assert_eq!(refresh(Some(&line(Some(bad.clone())))), None, "{bad}");
        }
        // 설정 파일에서 읽을 때도 같다.
        let dir = project("refresh");
        let file = dir.join(".claude").join("settings.json");
        for (text, want) in [("0", None), ("2", Some(json!(2))), ("\"x\"", None)] {
            std::fs::write(&file, format!(r#"{{"statusLine":{{"command":"hud","refreshInterval":{text}}}}}"#)).unwrap();
            assert_eq!(refresh(read_status_line(&file).as_ref()), want, "{text}");
        }

        let chained = settings_json("C:/app/cypher-bridge.exe", Some(&line(Some(json!(1)))), false, false);
        assert_eq!(chained["statusLine"]["command"], "C:/app/cypher-bridge.exe"); // 자리는 전달 프로그램이 차지
        assert_eq!(chained["statusLine"]["padding"], json!(2));
    }
}
