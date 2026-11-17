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

/// SSH 세션 프로필의 번호.
pub const SSH_PROFILE: &str = "ssh";

/// 기본 제공 프로필.
pub fn builtin() -> Vec<Profile> {
    vec![
        profile("claude", "Claude", "claude", &[], true),
        profile("powershell", "PowerShell", "powershell.exe", &["-NoLogo"], false),
        // SSH 세션: Windows에 든 OpenSSH로 접속한다. 접속할 곳(사용자@호스트, 포트, 키)은 세션을 만들 때 화면이 넘긴다(ssh.rs).
        profile(SSH_PROFILE, "SSH", "ssh.exe", &[], false),
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

/// 글 안의 `%이름%`을 환경 변수 값으로 바꾸고, 맨 앞의 `~`(뒤에 구분자가 오거나 그것뿐일 때)를 사용자 폴더로 바꾼다.
/// 모르는 변수는 그대로 둔다. `lookup`은 환경 변수를 찾는 함수(시험에서 바꿔 끼운다).
pub fn expand_vars(text: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = text;
    if let Some(after) = rest.strip_prefix('~') {
        if after.is_empty() || after.starts_with(['\\', '/']) {
            if let Some(home) = lookup("USERPROFILE") {
                out.push_str(&home);
                rest = after;
            }
        }
    }
    while let Some(start) = rest.find('%') {
        let after = &rest[start + 1..];
        let value = after.find('%').and_then(|end| {
            let name = &after[..end];
            let known = !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '(' | ')'));
            known.then(|| lookup(name)).flatten().map(|value| (end, value))
        });
        match value {
            Some((end, value)) => {
                out.push_str(&rest[..start]);
                out.push_str(&value);
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[..start + 1]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// 사용자가 적은 실행 명령(`split_command`로 나눈 것)에서 띄울 프로그램을 다듬는다.
/// - 프로그램 자리의 `%USERPROFILE%`·`~` 같은 표기를 푼다.
/// - 빈칸이 든 경로를 따옴표 없이 적었으면(`C:\Program Files\x\claude.exe --model opus`) 빈칸에서 잘려 있다. 프로그램이 경로인데
///   그런 파일이 없으면, 뒤의 인자를 하나씩 이어 붙여 보아 실제로 있는 파일이 되는 데까지를 프로그램으로 본다.
///
/// `is_file`은 그 경로가 파일인지 알려 주는 함수(확장자를 빼고 적은 경로도 받아 줄 수 있게, 부르는 쪽이 정한다).
pub fn locate_program(program: String, args: Vec<String>, lookup: impl Fn(&str) -> Option<String>, is_file: impl Fn(&str) -> bool) -> (String, Vec<String>) {
    let program = expand_vars(&program, lookup);
    let looks_like_path = program.contains(['\\', '/', ':']);
    if !looks_like_path || is_file(&program) {
        return (program, args);
    }
    let mut joined = program.clone();
    for (index, arg) in args.iter().enumerate() {
        // 옵션(`--model`)이 나오면 더는 경로의 일부가 아니다.
        if arg.starts_with('-') {
            break;
        }
        joined.push(' ');
        joined.push_str(arg);
        if is_file(&joined) {
            return (joined, args[index + 1..].to_vec());
        }
    }
    (program, args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 실행_명령의_프로그램_자리를_다듬는다() {
        let lookup = |name: &str| match name.to_ascii_uppercase().as_str() {
            "USERPROFILE" => Some(r"C:\Users\나".to_string()),
            "PROGRAMFILES(X86)" => Some(r"C:\Program Files (x86)".to_string()),
            _ => None,
        };
        let expand = |text: &str| expand_vars(text, lookup);
        assert_eq!(expand(r"%USERPROFILE%\bin\claude.exe"), r"C:\Users\나\bin\claude.exe");
        assert_eq!(expand(r"%userprofile%\a%USERPROFILE%"), r"C:\Users\나\aC:\Users\나");
        assert_eq!(expand(r"%ProgramFiles(x86)%\x"), r"C:\Program Files (x86)\x");
        assert_eq!(expand(r"~\bin\claude"), r"C:\Users\나\bin\claude");
        assert_eq!(expand("~/bin/claude"), r"C:\Users\나/bin/claude");
        assert_eq!(expand("~"), r"C:\Users\나");
        // 모르는 변수, 짝 없는 %, 이름 앞의 ~는 그대로 둔다.
        for same in ["claude", "%없는것%\\x", "100%", "a%b c%d", "~claude", "%%", "%USERPROFILE"] {
            assert_eq!(expand(same), same);
        }

        let files = [r"C:\Program Files\my tools\claude.exe", r"C:\Users\나\bin\claude.cmd", r"C:\a b"];
        let locate = |text: &str| {
            let (program, args) = split_command(text).unwrap();
            let (program, args) = locate_program(program, args, lookup, |path| files.contains(&path));
            (program, args.join("|"))
        };
        // 빈칸이 든 경로를 따옴표 없이 적은 것: 실제로 있는 파일이 되는 데까지 잇는다.
        assert_eq!(locate(r"C:\Program Files\my tools\claude.exe --model opus"), (r"C:\Program Files\my tools\claude.exe".into(), "--model|opus".into()));
        assert_eq!(locate(r"C:\Program Files\my tools\claude.exe"), (r"C:\Program Files\my tools\claude.exe".into(), String::new()));
        assert_eq!(locate(r"~\bin\claude.cmd -c"), (r"C:\Users\나\bin\claude.cmd".into(), "-c".into()));
        // 따옴표로 감싼 것과 이름만 적은 것은 그대로다.
        assert_eq!(locate(r#""C:\a b" x y"#), (r"C:\a b".into(), "x|y".into()));
        assert_eq!(locate("claude my tools"), ("claude".into(), "my|tools".into()));
        // 그런 파일이 끝내 없으면 나눈 그대로 둔다(옵션 뒤로는 잇지 않는다).
        assert_eq!(locate(r"C:\없는 폴더\claude.exe --x"), (r"C:\없는".into(), r"폴더\claude.exe|--x".into()));
        assert_eq!(locate(r"C:\Program --x Files\my tools\claude.exe"), (r"C:\Program".into(), r"--x|Files\my|tools\claude.exe".into()));
    }

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
