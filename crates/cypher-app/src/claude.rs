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
    refresh: Option<u64>,
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
        refresh: line.get("refreshInterval").and_then(Value::as_u64),
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

/// 세션 하나의 설정 내용: 상태줄과 훅을 전달 프로그램으로 지정한다.
/// 원래 상태줄이 있으면 그 여백을 따르고, 더 자주 갱신하던 쪽에 맞춘다.
fn settings_json(bridge: &str, original: Option<&StatusLine>) -> Value {
    // 훅: 요청 시작(UserPromptSubmit), 응답 완료(Stop), 입력·승인 대기(Notification),
    // 서브에이전트 시작·종료(SubagentStart·SubagentStop).
    let hook = json!([{ "hooks": [{ "type": "command", "command": bridge }] }]);
    let refresh = original.and_then(|o| o.refresh).map_or(5, |r| r.clamp(1, 5));
    let mut status = json!({ "type": "command", "command": bridge, "refreshInterval": refresh });
    if let Some(padding) = original.and_then(|o| o.padding.clone()) {
        status["padding"] = padding;
    }
    json!({
        "statusLine": status,
        "hooks": {
            "UserPromptSubmit": hook, "Stop": hook, "Notification": hook,
            "SubagentStart": hook, "SubagentStop": hook
        }
    })
}

/// 세션용 설정 파일을 쓴다. 사용자의 settings.json은 건드리지 않고 `--settings`로만 넘긴다.
/// 파일 이름에 실행 번호와 세션 번호를 넣어 다른 창·다른 세션과 겹치지 않게 한다.
pub fn write_settings(bridge: &str, session: u32, original: Option<&StatusLine>) -> Option<PathBuf> {
    let dir = crate::data_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("claude-settings-{}-{session}.json", std::process::id()));
    std::fs::write(&path, settings_json(bridge, original).to_string()).ok()?;
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
    fn 설정에는_전달_프로그램을_넣고_원래_여백과_갱신_주기를_따른다() {
        let plain = settings_json("C:/app/cypher-bridge.exe", None);
        assert_eq!(plain["statusLine"]["command"], "C:/app/cypher-bridge.exe");
        assert_eq!(plain["statusLine"]["refreshInterval"], 5);
        assert!(plain["statusLine"].get("padding").is_none());
        assert_eq!(plain["hooks"]["SubagentStop"][0]["hooks"][0]["command"], "C:/app/cypher-bridge.exe");

        let original = StatusLine { command: "hud".into(), padding: Some(json!(2)), refresh: Some(1) };
        let chained = settings_json("C:/app/cypher-bridge.exe", Some(&original));
        assert_eq!(chained["statusLine"]["command"], "C:/app/cypher-bridge.exe"); // 자리는 전달 프로그램이 차지
        assert_eq!((chained["statusLine"]["padding"].clone(), chained["statusLine"]["refreshInterval"].clone()), (json!(2), json!(1)));

        let slow = StatusLine { command: "hud".into(), padding: None, refresh: Some(60) };
        assert_eq!(settings_json("b", Some(&slow))["statusLine"]["refreshInterval"], 5); // Cypher 표시가 느려지지 않게
    }
}
