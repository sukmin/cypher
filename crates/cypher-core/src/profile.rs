//! 세션 프로필: 어떤 프로그램을 어떤 인자로 띄울지.
//! 기본 제공 프로필에 설정 파일의 `[[profile]]` 항목이 더해진다.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub label: String,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Claude Code 세션 여부. 상태줄·훅 주입과 이미지 붙여넣기 방식이 달라진다.
    #[serde(default)]
    pub claude: bool,
}

fn profile(id: &str, label: &str, program: &str, args: &[&str], claude: bool) -> Profile {
    Profile {
        id: id.into(),
        label: label.into(),
        program: program.into(),
        args: args.iter().map(|a| a.to_string()).collect(),
        claude,
    }
}

/// 기본 제공 프로필.
pub fn builtin() -> Vec<Profile> {
    vec![
        profile("claude", "Claude", "claude", &[], true),
        profile("powershell", "PowerShell", "powershell.exe", &["-NoLogo"], false),
    ]
}

/// 기본 프로필에 사용자 프로필을 합친다. 같은 id면 사용자 것이 우선한다.
pub fn merge(user: Vec<Profile>) -> Vec<Profile> {
    let mut all = builtin();
    for p in user {
        match all.iter_mut().find(|b| b.id == p.id) {
            Some(slot) => *slot = p,
            None => all.push(p),
        }
    }
    all
}

/// 사용자가 적은 명령 한 줄을 프로그램과 인자로 나눈다. 빈칸으로 나누고, 큰따옴표로 감싼 부분은 빈칸이 있어도
/// 한 덩어리다(`"C:\Program Files\x\claude.exe" --model opus`). 역슬래시는 Windows 경로에 쓰이므로 그대로 둔다.
/// 비어 있거나 따옴표가 닫히지 않았으면 None.
pub fn split_command(text: &str) -> Option<(String, Vec<String>)> {
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    let (mut quoted, mut started) = (false, false);
    for ch in text.trim().chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                started = true; // 빈 따옴표("")도 인자 하나다.
            }
            ch if ch.is_whitespace() && !quoted => {
                if started {
                    parts.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            ch => {
                current.push(ch);
                started = true;
            }
        }
    }
    if quoted {
        return None;
    }
    if started {
        parts.push(current);
    }
    let mut parts = parts.into_iter();
    let program = parts.next().filter(|program| !program.is_empty())?;
    Some((program, parts.collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 명령_한_줄을_프로그램과_인자로_나눈다() {
        let split = |text: &str| split_command(text).map(|(program, args)| (program, args.join("|")));
        assert_eq!(split("claude"), Some(("claude".into(), String::new())));
        assert_eq!(split("  claude   --dangerously-skip-permissions  "), Some(("claude".into(), "--dangerously-skip-permissions".into())));
        assert_eq!(
            split(r#""C:\Program Files\tools\claude.exe" --model opus --append-system-prompt "한국어로 답해""#),
            Some((r"C:\Program Files\tools\claude.exe".into(), "--model|opus|--append-system-prompt|한국어로 답해".into()))
        );
        assert_eq!(split(r#"claude --x="a b" """#), Some(("claude".into(), "--x=a b|".into())));
        // 비어 있거나 따옴표가 닫히지 않은 명령은 받지 않는다.
        for bad in ["", "   ", r#"claude "열린 따옴표"#, r#""""#] {
            assert_eq!(split_command(bad), None, "{bad:?}");
        }
    }
}
