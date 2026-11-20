//! SSH 세션: 접속할 곳을 `ssh` 명령의 인자로 바꾸고, 사용자의 `~/.ssh/config`에서 적어 둔 호스트를 읽는다.
//!
//! Cypher는 SSH를 직접 구현하지 않는다. Windows에 든 OpenSSH(`ssh.exe`)를 가상 터미널에 붙여 띄운다.
//! 그래서 키·에이전트·`~/.ssh/config`·known_hosts는 평소 명령줄에서 쓰던 그대로 적용되고, 비밀번호는 Cypher가 저장하지 않는다.

use crate::profile::split_command;
use serde::{Deserialize, Serialize};

/// 접속할 곳. 화면이 세션을 만들 때 넘기고, 세션을 되살릴 때 쓰려고 그대로 적어 둔다.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SshTarget {
    /// `사용자@호스트`, 호스트만, 또는 `~/.ssh/config`에 적어 둔 이름.
    pub target: String,
    /// 포트(비우면 ssh의 기본값이나 설정 파일의 값).
    #[serde(default)]
    pub port: Option<u16>,
    /// 개인 키 파일(비우면 ssh가 알아서 고른다).
    #[serde(default)]
    pub identity: Option<String>,
    /// 그 밖에 붙일 ssh 옵션(`-J 경유서버`, `-L 8080:localhost:80` 등). 명령줄에 적듯이 빈칸으로 나눠 적는다.
    #[serde(default)]
    pub options: Option<String>,
}

/// 접속할 곳을 `ssh`에 넘길 인자로 바꾼다. 대상이 비었거나 옵션처럼 보이면(`-`로 시작) 오류다.
pub fn ssh_args(spec: &SshTarget) -> Result<Vec<String>, String> {
    let target = spec.target.trim();
    if target.is_empty() {
        return Err("접속할 곳이 비어 있습니다(예: user@example.com).".into());
    }
    // 대상 자리에 옵션이 끼어들지 못하게 한다(`-oProxyCommand=…` 같은 것이 대상으로 넘어가면 명령이 실행된다).
    if target.starts_with('-') || target.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(format!("접속할 곳이 올바르지 않습니다: {target} (빈칸 없이 user@host 꼴로 적으세요. 옵션은 \"추가 옵션\" 칸에 적습니다)"));
    }
    let mut args = Vec::new();
    if let Some(options) = spec.options.as_deref().map(str::trim).filter(|text| !text.is_empty()) {
        let (first, rest) = split_command(options).ok_or("추가 옵션의 큰따옴표가 닫히지 않았습니다.")?;
        args.push(first);
        args.extend(rest);
    }
    if let Some(port) = spec.port.filter(|port| *port != 0) {
        args.push("-p".into());
        args.push(port.to_string());
    }
    if let Some(identity) = spec.identity.as_deref().map(str::trim).filter(|text| !text.is_empty()) {
        args.push("-i".into());
        args.push(identity.to_string());
    }
    args.push(target.to_string());
    Ok(args)
}

/// `~/.ssh/config`에 적어 둔 호스트 하나.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ConfigHost {
    /// `Host` 줄의 이름(이 이름으로 접속하면 ssh가 아래 값을 알아서 쓴다).
    pub name: String,
    pub hostname: String,
    pub user: String,
    pub port: String,
}

/// `~/.ssh/config`의 글에서 호스트를 읽는다. 여러 호스트에 걸리는 이름(`*`, `?`, `!`가 든 것)은 뺀다.
/// 한 `Host` 줄에 이름이 여럿이면 각각 올린다. `Match` 구역의 값은 읽지 않는다.
pub fn config_hosts(text: &str) -> Vec<ConfigHost> {
    let mut hosts: Vec<ConfigHost> = Vec::new();
    // 지금 읽고 있는 `Host` 구역에 속한 호스트들의 자리. `Match` 구역이나 `Host` 앞의 줄이면 비어 있다.
    let mut current: Vec<usize> = Vec::new();
    // BOM이 붙은 파일(Windows PowerShell의 `Set-Content -Encoding UTF8` 등)도 ssh는 읽는다. 떼지 않으면 첫 줄의 키가
    // `host`와 맞지 않아 첫 `Host` 구역이 통째로 빠진다.
    for line in text.strip_prefix('\u{feff}').unwrap_or(text).lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // `키 값` 또는 `키=값`.
        let (key, value) = match line.split_once(|c: char| c.is_whitespace() || c == '=') {
            Some((key, value)) => (key.to_ascii_lowercase(), value.trim().trim_start_matches('=').trim()),
            None => continue,
        };
        let value = value.trim_matches('"');
        match key.as_str() {
            "host" => {
                current.clear();
                for name in value.split_whitespace().map(|name| name.trim_matches('"')) {
                    if name.is_empty() || name.contains(['*', '?', '!']) || hosts.iter().any(|host| host.name == name) {
                        continue;
                    }
                    current.push(hosts.len());
                    hosts.push(ConfigHost { name: name.to_string(), ..Default::default() });
                }
            }
            "match" => current.clear(),
            // 같은 값이 거듭 나오면 ssh처럼 먼저 나온 것을 쓴다.
            "hostname" | "user" | "port" => {
                for &index in &current {
                    let slot = match key.as_str() {
                        "hostname" => &mut hosts[index].hostname,
                        "user" => &mut hosts[index].user,
                        _ => &mut hosts[index].port,
                    };
                    if slot.is_empty() {
                        *slot = value.to_string();
                    }
                }
            }
            _ => {}
        }
    }
    hosts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(target: &str) -> SshTarget {
        SshTarget { target: target.into(), ..Default::default() }
    }

    #[test]
    fn 접속할_곳을_ssh_인자로_바꾼다() {
        assert_eq!(ssh_args(&spec("me@example.com")).unwrap(), ["me@example.com"]);
        assert_eq!(ssh_args(&spec("  web1  ")).unwrap(), ["web1"]);
        let full = SshTarget {
            target: "me@10.0.0.5".into(),
            port: Some(2222),
            identity: Some(r"C:\Users\나\.ssh\id work".into()),
            options: Some(r#"-J jump@gate -o "ServerAliveInterval 30""#.into()),
        };
        assert_eq!(
            ssh_args(&full).unwrap(),
            ["-J", "jump@gate", "-o", "ServerAliveInterval 30", "-p", "2222", "-i", r"C:\Users\나\.ssh\id work", "me@10.0.0.5"]
        );
        // 빈 값은 없는 것과 같다.
        let blank = SshTarget { target: "h".into(), port: Some(0), identity: Some("  ".into()), options: Some(String::new()) };
        assert_eq!(ssh_args(&blank).unwrap(), ["h"]);
        // 대상 자리에 옵션이나 빈칸이 든 것은 받지 않는다.
        for bad in ["", "   ", "-oProxyCommand=calc", "me@host -p 22", "a\tb", "a\nb"] {
            assert!(ssh_args(&spec(bad)).is_err(), "{bad:?}");
        }
        let open = SshTarget { target: "h".into(), options: Some(r#"-o "열린 따옴표"#.into()), ..Default::default() };
        assert!(ssh_args(&open).is_err());
    }

    #[test]
    fn bom이_붙은_설정_파일의_첫_호스트도_읽는다() {
        let hosts = config_hosts("\u{feff}Host first\n    HostName 10.0.0.1\nHost second\n");
        assert_eq!(hosts.iter().map(|host| host.name.as_str()).collect::<Vec<_>>(), ["first", "second"]);
        assert_eq!(hosts[0].hostname, "10.0.0.1");
    }

    #[test]
    fn 설정_파일의_호스트를_읽는다() {
        let text = "\
# 주석
Host *
    ServerAliveInterval 30

Host web1 web2
    HostName 10.0.0.5
    User deploy
    Port 2222
    User ignored

host=db
  hostname = db.internal
Host \"따옴표 없는 이름\" *.example.com !bad
Host web1
    User again
Match host gate
    User matched
Host gate
    HostName gate.example.com
";
        let hosts = config_hosts(text);
        let names: Vec<&str> = hosts.iter().map(|host| host.name.as_str()).collect();
        assert_eq!(names, ["web1", "web2", "db", "따옴표", "없는", "이름", "gate"]);
        let host = |name: &str| hosts.iter().find(|host| host.name == name).unwrap().clone();
        assert_eq!(host("web1"), ConfigHost { name: "web1".into(), hostname: "10.0.0.5".into(), user: "deploy".into(), port: "2222".into() });
        assert_eq!(host("web2").user, "deploy");
        assert_eq!(host("db").hostname, "db.internal");
        // `Match` 구역의 값은 어느 호스트에도 붙지 않는다.
        assert_eq!(host("gate"), ConfigHost { name: "gate".into(), hostname: "gate.example.com".into(), ..Default::default() });
        assert!(config_hosts("").is_empty());
        assert!(config_hosts("HostName x\nUser y").is_empty());
    }
}
