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
