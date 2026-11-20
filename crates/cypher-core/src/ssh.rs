//! SSH 세션: 접속할 곳을 읽고, 사용자의 `~/.ssh/config`와 `known_hosts`를 해석한다.
//!
//! 기본은 Cypher에 든 SSH(앱의 `sshc` 모듈)로 접속한다. 여기서는 그 접속에 필요한 것 가운데 화면·네트워크와 무관한 것
//! (접속할 곳 나누기, 설정 파일에서 그 호스트에 걸리는 값 고르기, known_hosts에서 호스트 키 찾기)을 다룬다.
//! 설정의 `[ssh] engine = "openssh"`이거나 `~/.ssh/config`에 내장 SSH가 다루지 못하는 설정(ProxyJump 등)이 있으면
//! Windows의 OpenSSH(`ssh.exe`)를 가상 터미널에 띄운다(`ssh_args`, 사용자 이름을 모르면 `login_script`로 먼저 묻는다).
//! 접속할 곳은 주소와 포트만 받는다: 사용자 이름·비밀번호 등은 접속할 때마다 세션 화면에서 묻고, Cypher가 저장하지 않는다.

use serde::{Deserialize, Serialize};

/// 접속할 곳: 주소와 포트만. 화면이 세션을 만들 때 넘기고, 세션을 되살릴 때 쓰려고 그대로 적어 둔다.
/// 사용자 이름은 담지 않는다(접속할 때 세션 화면에서 묻는다). 예전 기록에 있던 키 파일·추가 옵션 칸은 읽지 않고 버린다.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SshTarget {
    /// 호스트(이름이나 IP) 또는 `~/.ssh/config`에 적어 둔 이름. 화면은 보내지 않지만 `사용자@호스트`도 읽는다.
    pub target: String,
    /// 포트(비우면 설정 파일의 값이나 22).
    #[serde(default)]
    pub port: Option<u16>,
}

/// 접속할 곳을 `ssh`에 넘길 인자로 바꾼다. 대상이 비었거나 옵션처럼 보이면(`-`로 시작) 오류다.
pub fn ssh_args(spec: &SshTarget) -> Result<Vec<String>, String> {
    let target = spec.target.trim();
    if target.is_empty() {
        return Err("접속할 주소가 비어 있습니다(예: example.com).".into());
    }
    // 대상 자리에 옵션이 끼어들지 못하게 한다(`-oProxyCommand=…` 같은 것이 대상으로 넘어가면 명령이 실행된다).
    if target.starts_with('-') || target.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(format!("접속할 주소가 올바르지 않습니다: {target} (빈칸 없이 주소만 적으세요)"));
    }
    let mut args = Vec::new();
    if let Some(port) = spec.port.filter(|port| *port != 0) {
        args.push("-p".into());
        args.push(port.to_string());
    }
    args.push(target.to_string());
    Ok(args)
}

/// PowerShell의 작은따옴표 글. PowerShell은 굽은 작은따옴표(‘ ’ ‚ ‛)도 작은따옴표로 읽으므로 모두 두 번 적는다.
fn ps_quote(text: &str) -> String {
    let mut out = String::from("'");
    for c in text.chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}') {
            out.push(c);
        }
        out.push(c);
    }
    out.push('\'');
    out
}

/// ssh.exe로 접속하는데 사용자 이름을 모를 때 띄울 PowerShell 명령: 세션 화면에서 사용자 이름을 물은 뒤(비우면 다시 묻는다)
/// ssh를 띄우고 그 종료 코드로 끝난다. 사용자 이름은 `-l`의 값으로 넘긴다(`-`로 시작해도 옵션으로 읽히지 않는다).
/// 창 제목(사이드바에 보이는 것)은 PowerShell의 경로 대신 `ssh 주소`, 이름을 받은 뒤에는 `ssh 사용자@주소`로 둔다.
/// `args`의 마지막은 접속할 곳이다(`ssh_args`).
pub fn login_script(program: &str, args: &[String]) -> String {
    let target = ps_quote(args.last().map(String::as_str).unwrap_or(""));
    let args: Vec<String> = args.iter().map(|arg| ps_quote(arg)).collect();
    format!(
        "try {{ $Host.UI.RawUI.WindowTitle = 'ssh ' + {target} }} catch {{ }}\n\
         $user = ''\n\
         while (-not $user) {{\n  $line = Read-Host '사용자 이름'\n  if ($null -eq $line) {{ exit 255 }}\n  $user = $line.Trim()\n}}\n\
         try {{ $Host.UI.RawUI.WindowTitle = 'ssh ' + $user + '@' + {target} }} catch {{ }}\n\
         try {{ & {} -l $user {} }} catch {{ Write-Host $_.Exception.Message -ForegroundColor Red; exit 255 }}\n\
         exit $LASTEXITCODE\n",
        ps_quote(program),
        args.join(" ")
    )
}

/// PowerShell의 `-EncodedCommand`에 넘길 글(UTF-16LE를 base64로). 명령줄의 따옴표 풀이를 거치지 않는다.
pub fn encoded_command(script: &str) -> String {
    use base64::Engine;
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
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

// ───────────────────────── 내장 SSH: 접속할 곳 ─────────────────────────

/// `사용자@호스트` 또는 `호스트`를 나눈다. 사용자 이름에 `@`가 들 수 있으므로(도메인 계정) 마지막 `@`에서 나눈다.
pub fn split_target(target: &str) -> (Option<&str>, &str) {
    let target = target.trim();
    match target.rsplit_once('@') {
        Some((user, host)) if !user.is_empty() => (Some(user), host),
        Some((_, host)) => (None, host),
        None => (None, target),
    }
}

/// `~/.ssh/config`에서 한 호스트에 걸리는 값. 같은 이름이 여러 번 걸리면 ssh처럼 먼저 나온 것을 쓴다
/// (IdentityFile만 걸린 것을 모두 차례로 모은다).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HostSettings {
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_files: Vec<String>,
    /// 참이면 에이전트의 키 가운데 IdentityFile에 적은 것만 쓴다(먼저 나온 값).
    pub identities_only: Option<bool>,
    /// HashKnownHosts: 참이면 known_hosts에 호스트 이름 대신 해시를 적는다.
    pub hash_known_hosts: Option<bool>,
    /// 내장 SSH가 따르지 못하는 설정(포트 포워딩, 원격 명령, 따질 수 없는 Match 조건 등)이 걸렸으면 그 설정.
    /// 그때는 무시하지 않고 OpenSSH로 접속한다.
    pub unsupported: Option<String>,
    /// ProxyJump·ProxyCommand(내장 SSH가 다루지 못한다). `키 값` 그대로.
    pub proxy: Option<String>,
    /// StrictHostKeyChecking 값(소문자). `yes`면 모르는 호스트에 접속하지 않는다.
    pub strict_host_key_checking: Option<String>,
    /// UserKnownHostsFile에 적은 파일들(`none`이면 빈 목록이 아니라 `["none"]`).
    pub user_known_hosts: Vec<String>,
    /// ConnectTimeout(초).
    pub connect_timeout: Option<u64>,
}

/// ssh의 호스트 이름 패턴(`*`, `?`)이 맞는지. 대/소문자는 가리지 않는다.
fn glob(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.to_lowercase().chars().collect(), text.to_lowercase().chars().collect());
    // 별표 하나로 돌아가는 탐욕 일치(별표 둘 이상도 이것으로 충분하다).
    let (mut pi, mut ti, mut star, mut mark) = (0usize, 0usize, None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(at) = star {
            pi = at + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

/// `Host` 줄의 패턴 목록이 그 이름에 걸리는지: `!`로 시작하는 패턴에 걸리면 무조건 아니고, 나머지 가운데 하나라도 걸려야 한다.
fn host_line_matches(patterns: &str, host: &str) -> bool {
    let mut hit = false;
    for pattern in patterns.split([' ', '\t', ',']).map(|p| p.trim_matches('"')).filter(|p| !p.is_empty()) {
        if let Some(negated) = pattern.strip_prefix('!') {
            if glob(negated, host) {
                return false;
            }
        } else if glob(pattern, host) {
            hit = true;
        }
    }
    hit
}

/// 설정 줄 하나를 키(소문자)와 값으로 나눈다. 주석·빈 줄이면 None.
fn config_line(line: &str) -> Option<(String, &str)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (key, value) = line.split_once(|c: char| c.is_whitespace() || c == '=')?;
    let value = value.trim().trim_start_matches('=').trim();
    Some((key.to_ascii_lowercase(), value))
}

/// `~/.ssh/config`의 글에서 그 호스트(접속할 때 적은 이름)에 걸리는 값을 모은다.
/// `include`는 `Include`에 적힌 값 하나를 받아 그 파일들의 글을 돌려준다(상대 경로는 `~/.ssh` 기준, 와일드카드 풀기는 부르는 쪽 몫).
/// `Match`는 `all`·`host`·`originalhost` 조건만 따진다. 그 밖의 조건(`exec`, `user` 등) 아래에 설정이 있으면
/// 걸리는지 알 수 없으므로 `unsupported`에 적는다(부르는 쪽은 OpenSSH로 접속한다).
pub fn host_settings(text: &str, host: &str, include: &dyn Fn(&str) -> Vec<String>) -> HostSettings {
    let mut found = HostSettings::default();
    read_settings(text, host, include, &mut found, true, 0);
    found
}

/// `Match` 줄의 조건이 걸리는지. 따질 수 없는 조건이 있으면 Err.
fn match_line(value: &str, host: &str, found: &HostSettings) -> Result<bool, ()> {
    let words: Vec<&str> = value.split_whitespace().collect();
    let mut all = !words.is_empty();
    let mut i = 0;
    while i < words.len() {
        let (negate, name) = match words[i].strip_prefix('!') {
            Some(name) => (true, name),
            None => (false, words[i]),
        };
        let hit = match name.to_ascii_lowercase().as_str() {
            "all" => true,
            criterion @ ("host" | "originalhost") => {
                i += 1;
                let patterns = words.get(i).ok_or(())?.trim_matches('"');
                // host는 HostName을 푼 이름에, originalhost는 접속할 때 적은 이름에 맞춘다.
                let target = match (criterion, found.hostname.as_deref()) {
                    ("host", Some(name)) => name.replace("%h", host).replace("%%", "%"),
                    _ => host.to_string(),
                };
                host_line_matches(patterns, &target)
            }
            _ => return Err(()),
        };
        if hit == negate {
            all = false;
        }
        i += 1;
    }
    Ok(all)
}

/// 내장 SSH가 따르지 못하는 설정이면 화면에 보일 이름(`LocalForward 8080 localhost:80`).
fn unsupported_setting(key: &str, value: &str) -> Option<String> {
    let yes = value.eq_ignore_ascii_case("yes");
    let name = match key {
        "localforward" => "LocalForward",
        "remoteforward" => "RemoteForward",
        "dynamicforward" => "DynamicForward",
        "hostkeyalias" => "HostKeyAlias",
        "certificatefile" => "CertificateFile",
        "pkcs11provider" if !value.eq_ignore_ascii_case("none") => "PKCS11Provider",
        "securitykeyprovider" => "SecurityKeyProvider",
        "remotecommand" if !value.eq_ignore_ascii_case("none") => "RemoteCommand",
        "forwardagent" if yes || value.contains(['/', '\\', '$']) => "ForwardAgent",
        "forwardx11" if yes => "ForwardX11",
        "requesttty" if value.eq_ignore_ascii_case("no") => "RequestTTY",
        _ => return None,
    };
    Some(format!("{name} {value}"))
}

fn read_settings(text: &str, host: &str, include: &dyn Fn(&str) -> Vec<String>, found: &mut HostSettings, mut active: bool, depth: u32) {
    // 따질 수 없는 Match 조건(그 구역에 설정이 있으면 걸리는지 모른다).
    let mut unknown: Option<String> = None;
    for line in text.strip_prefix('\u{feff}').unwrap_or(text).lines() {
        let Some((key, value)) = config_line(line) else { continue };
        let unquoted = value.trim_matches('"');
        match key.as_str() {
            "host" => {
                active = host_line_matches(value, host);
                unknown = None;
            }
            "match" => match match_line(value, host, found) {
                Ok(hit) => {
                    active = hit;
                    unknown = None;
                }
                Err(()) => {
                    active = false;
                    unknown = Some(value.to_string());
                }
            },
            _ if !active => {
                if let Some(criteria) = &unknown {
                    found.unsupported.get_or_insert_with(|| format!("Match {criteria}"));
                }
            }
            "include" if depth < 16 => {
                for part in value.split_whitespace().map(|p| p.trim_matches('"')) {
                    for included in include(part) {
                        read_settings(&included, host, include, found, true, depth + 1);
                    }
                }
            }
            "hostname" if found.hostname.is_none() => found.hostname = Some(unquoted.to_string()),
            "user" if found.user.is_none() => found.user = Some(unquoted.to_string()),
            "port" if found.port.is_none() => found.port = unquoted.parse().ok(),
            "identityfile" => {
                if !found.identity_files.iter().any(|known| known == unquoted) {
                    found.identity_files.push(unquoted.to_string());
                }
            }
            "identitiesonly" if found.identities_only.is_none() => found.identities_only = Some(unquoted.eq_ignore_ascii_case("yes")),
            "hashknownhosts" if found.hash_known_hosts.is_none() => found.hash_known_hosts = Some(unquoted.eq_ignore_ascii_case("yes")),
            "proxyjump" | "proxycommand" if found.proxy.is_none() && !unquoted.eq_ignore_ascii_case("none") => {
                found.proxy = Some(format!("{key} {unquoted}"));
            }
            "stricthostkeychecking" if found.strict_host_key_checking.is_none() => {
                found.strict_host_key_checking = Some(unquoted.to_ascii_lowercase());
            }
            "userknownhostsfile" if found.user_known_hosts.is_empty() => {
                found.user_known_hosts = value.split_whitespace().map(|p| p.trim_matches('"').to_string()).collect();
            }
            "connecttimeout" if found.connect_timeout.is_none() => found.connect_timeout = unquoted.parse().ok(),
            _ => {
                if found.unsupported.is_none() {
                    found.unsupported = unsupported_setting(&key, unquoted);
                }
            }
        }
    }
}

/// 설정 파일의 경로 값(`~/.ssh/id_work`, `%d/.ssh/%h.key`)을 푼다.
/// `%d` 홈 폴더, `%u` 이 컴퓨터의 사용자, `%h` 접속할 호스트, `%r` 원격 사용자, `%%` 글자 %.
pub fn expand_path(value: &str, home: &str, local_user: &str, host: &str, remote_user: &str) -> String {
    let value = value.trim().trim_matches('"');
    let value = if value == "~" {
        home.to_string()
    } else if let Some(rest) = value.strip_prefix("~/").or_else(|| value.strip_prefix("~\\")) {
        format!("{home}\\{rest}")
    } else {
        value.to_string()
    };
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('d') => out.push_str(home),
            Some('u') => out.push_str(local_user),
            Some('h') => out.push_str(host),
            Some('r') => out.push_str(remote_user),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out.replace('/', "\\")
}

// ───────────────────────── 내장 SSH: known_hosts ─────────────────────────

/// known_hosts에서 본 그 호스트의 키.
#[derive(Debug, Clone, PartialEq)]
pub enum HostKey {
    /// 같은 키가 적혀 있다.
    Known,
    /// 같은 종류의 다른 키가 적혀 있다(서버가 바뀌었거나 누군가 가로채고 있다). `line`은 1부터.
    Changed { line: usize, algorithm: String },
    /// 이 키가 `@revoked`로 막혀 있다.
    Revoked { line: usize },
    /// 적혀 있지 않다. `others`는 같은 호스트에 적혀 있는 다른 종류의 키들.
    Unknown { others: Vec<String> },
}

/// known_hosts에 쓰는 호스트 이름: 기본 포트가 아니면 `[호스트]:포트`. 호스트 이름은 소문자로 맞춘다(ssh와 같다).
pub fn known_host_name(host: &str, port: u16) -> String {
    let host = host.to_ascii_lowercase();
    if port == 22 { host } else { format!("[{host}]:{port}") }
}

/// known_hosts의 호스트 칸(쉼표로 나뉜 이름·패턴·해시)이 그 이름에 걸리는지.
fn known_hosts_matches(field: &str, name: &str) -> bool {
    use hmac::{Hmac, Mac};
    let mut hit = false;
    for entry in field.split(',') {
        if let Some(hashed) = entry.strip_prefix("|1|") {
            // |1|소금|해시: HMAC-SHA1(소금, 이름) == 해시
            use base64::Engine;
            let engine = base64::engine::general_purpose::STANDARD;
            let Some((salt, hash)) = hashed.split_once('|') else { continue };
            let (Ok(salt), Ok(hash)) = (engine.decode(salt), engine.decode(hash)) else { continue };
            let Ok(mut mac) = Hmac::<sha1::Sha1>::new_from_slice(&salt) else { continue };
            mac.update(name.as_bytes());
            if mac.verify_slice(&hash).is_ok() {
                hit = true;
            }
        } else if let Some(negated) = entry.strip_prefix('!') {
            if glob(negated, name) {
                return false;
            }
        } else if glob(entry, name) {
            hit = true;
        }
    }
    hit
}

/// known_hosts의 글에서 그 호스트(이름, 포트)의 키를 찾는다. `algorithm`·`key`는 서버가 보인 키의 종류(`ssh-ed25519`)와
/// base64 글이다. 줄의 칸은 빈칸이나 탭으로 나뉘고, `@cert-authority` 줄은 보지 않는다(인증서는 다루지 않는다).
pub fn check_known_host(text: &str, host: &str, port: u16, algorithm: &str, key: &str) -> HostKey {
    let name = known_host_name(host, port);
    let mut others: Vec<String> = Vec::new();
    let mut changed: Option<(usize, String)> = None;
    let mut known = false;
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_whitespace();
        let mut first = fields.next().unwrap_or("");
        let marker = first.strip_prefix('@').map(str::to_ascii_lowercase);
        if marker.is_some() {
            first = fields.next().unwrap_or("");
        }
        let (Some(kind), Some(blob)) = (fields.next(), fields.next()) else { continue };
        if marker.as_deref() == Some("cert-authority") || !known_hosts_matches(first, &name) {
            continue;
        }
        if marker.as_deref() == Some("revoked") {
            if blob == key {
                return HostKey::Revoked { line: index + 1 };
            }
            continue;
        }
        if kind == algorithm {
            if blob == key {
                known = true;
            } else if changed.is_none() {
                changed = Some((index + 1, kind.to_string()));
            }
        } else if !others.iter().any(|other| other == kind) {
            others.push(kind.to_string());
        }
    }
    // 같은 키가 한 줄이라도 있으면 아는 호스트다(같은 이름에 키를 여럿 적어 둘 수 있다).
    if known {
        return HostKey::Known;
    }
    match changed {
        Some((line, algorithm)) => HostKey::Changed { line, algorithm },
        None => HostKey::Unknown { others },
    }
}

/// known_hosts에 덧붙일 줄(줄 바꿈 없이).
pub fn known_host_line(host: &str, port: u16, algorithm: &str, key: &str) -> String {
    format!("{} {algorithm} {key}", known_host_name(host, port))
}

/// HashKnownHosts yes일 때 덧붙일 줄: 호스트 이름 대신 `|1|base64(소금)|base64(HMAC-SHA1(소금, 이름))`(`ssh-keygen -H`와 같은 꼴).
pub fn known_host_line_hashed(host: &str, port: u16, algorithm: &str, key: &str, salt: &[u8; 20]) -> String {
    use base64::Engine;
    use hmac::{Hmac, Mac};
    let engine = base64::engine::general_purpose::STANDARD;
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(salt).expect("HMAC는 어떤 길이의 키든 받는다");
    mac.update(known_host_name(host, port).as_bytes());
    format!("|1|{}|{} {algorithm} {key}", engine.encode(salt), engine.encode(mac.finalize().into_bytes()))
}

/// 그 호스트에 대해 known_hosts에 적혀 있는 키의 종류들(`@revoked`·`@cert-authority` 줄은 빼고).
/// 접속할 때 서버에 이 종류의 키를 먼저 달라고 한다(이미 믿은 서버를 처음 보는 서버로 묻지 않게).
pub fn known_host_algorithms(text: &str, host: &str, port: u16) -> Vec<String> {
    match check_known_host(text, host, port, "", "") {
        HostKey::Unknown { others } => others,
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 접속할_곳을_사용자와_호스트로_나눈다() {
        assert_eq!(split_target("me@example.com"), (Some("me"), "example.com"));
        assert_eq!(split_target(" web1 "), (None, "web1"));
        assert_eq!(split_target("dom@corp@host"), (Some("dom@corp"), "host"));
        assert_eq!(split_target("@host"), (None, "host"));
    }

    #[test]
    fn 설정_파일에서_그_호스트의_값을_고른다() {
        let text = "\
IdentityFile ~/.ssh/global
Include extra
Host web*  !web-old
    HostName %h.example.com
    User deploy
    Port 2222
    IdentityFile ~/.ssh/web
Host web1
    User ignored
    ProxyJump gate
Host *
    User fallback
    IdentityFile \"~/.ssh/my key\"
    StrictHostKeyChecking Yes
    UserKnownHostsFile ~/.ssh/kh1 ~/.ssh/kh2
Match host zz
    User matched
Include never
";
        // 걸리지 않는 구역 안의 Include는 읽지 않는다("never"를 읽으면 시험이 실패한다).
        let include = |name: &str| if name == "never" { panic!("걸리지 않는 구역의 Include를 읽었다") } else if name == "extra" { vec!["Host web1\n  ConnectTimeout 7\n  IdentityFile ~/.ssh/from-include\n".to_string()] } else { Vec::new() };
        let web1 = host_settings(text, "web1", &include);
        assert_eq!(web1.hostname.as_deref(), Some("%h.example.com"));
        assert_eq!(web1.user.as_deref(), Some("deploy"));
        assert_eq!(web1.port, Some(2222));
        assert_eq!(web1.identity_files, ["~/.ssh/global", "~/.ssh/from-include", "~/.ssh/web", "~/.ssh/my key"]);
        assert_eq!(web1.proxy.as_deref(), Some("proxyjump gate"));
        assert_eq!(web1.strict_host_key_checking.as_deref(), Some("yes"));
        assert_eq!(web1.user_known_hosts, ["~/.ssh/kh1", "~/.ssh/kh2"]);
        assert_eq!(web1.connect_timeout, Some(7));
        // `!` 패턴에 걸리면 그 줄은 걸리지 않는다. Match 구역의 값은 쓰지 않는다.
        let old = host_settings(text, "web-old", &include);
        assert_eq!((old.user.as_deref(), old.port, old.hostname), (Some("fallback"), None, None));
        let other = host_settings(text, "x", &include);
        assert_eq!(other.user.as_deref(), Some("fallback"));
        assert!(host_settings("", "a", &include) == HostSettings::default());
        // 대/소문자를 가리지 않고, `?`는 한 글자다.
        assert_eq!(host_settings("Host DB?\n User u\n", "db1", &include).user.as_deref(), Some("u"));
        assert!(host_settings("Host DB?\n User u\n", "db12", &include).user.is_none());
        // ProxyCommand none은 경유하지 않는다는 뜻이다.
        assert!(host_settings("ProxyCommand none\n", "a", &include).proxy.is_none());
        assert!(glob("*.example.*", "a.example.com") && !glob("*.example", "a.example.com") && glob("**", ""));
    }

    #[test]
    fn 따르지_못하는_설정과_match_조건을_알아본다() {
        let none = |_: &str| Vec::new();
        let text = "Host web\n  LocalForward 8080 localhost:80\nHost db\n  RemoteCommand none\n  ForwardAgent no\n  IdentitiesOnly yes\n  IdentitiesOnly no\n  HashKnownHosts yes\nMatch host *.corp\n  User corp\nMatch originalhost alias !host gate.corp\n  Port 2200\nMatch exec \"test -f x\"\n  ProxyJump j\n";
        assert_eq!(host_settings(text, "web", &none).unsupported.as_deref(), Some("LocalForward 8080 localhost:80"));
        let db = host_settings(text, "db", &none);
        // none·no는 따르지 못할 것이 없다. IdentitiesOnly·HashKnownHosts는 먼저 나온 값.
        assert_eq!(db.identities_only, Some(true));
        assert_eq!(db.hash_known_hosts, Some(true));
        // exec 조건은 따질 수 없다: 그 아래에 설정이 있으므로 OpenSSH로 넘긴다.
        assert_eq!(db.unsupported.as_deref(), Some("Match exec \"test -f x\""));
        // host·originalhost 조건은 따진다.
        assert_eq!(host_settings(text, "a.corp", &none).user.as_deref(), Some("corp"));
        assert_eq!(host_settings(text, "alias", &none).port, Some(2200));
        assert_eq!(host_settings("Match host *.corp\n  User corp\n", "a.com", &none).user, None);
        // 따질 수 없는 Match라도 그 아래에 설정이 없으면 상관없다.
        assert_eq!(host_settings("Match exec true\nHost x\n  User u\n", "x", &none).unsupported, None);
        // 해시로 적는 줄은 ssh가 읽는 꼴이고, 그 이름에 걸린다.
        let line = known_host_line_hashed("Example.com", 2222, "ssh-ed25519", "AAAAk", &[3u8; 20]);
        assert!(line.starts_with("|1|") && !line.contains("example"));
        assert_eq!(check_known_host(&line, "example.com", 2222, "ssh-ed25519", "AAAAk"), HostKey::Known);
        assert_eq!(known_host_algorithms("a ssh-ed25519 K1\na ecdsa-sha2-nistp256 K2\n@revoked a ssh-rsa K3\nb ssh-rsa K4\n", "a", 22), ["ssh-ed25519", "ecdsa-sha2-nistp256"]);
    }

    #[test]
    fn 설정_파일의_경로를_푼다() {
        let home = r"C:\Users\나";
        assert_eq!(expand_path("~/.ssh/id_ed25519", home, "me", "web", "deploy"), r"C:\Users\나\.ssh\id_ed25519");
        assert_eq!(expand_path("\"%d/.ssh/%h-%r.key\"", home, "me", "web", "deploy"), r"C:\Users\나\.ssh\web-deploy.key");
        assert_eq!(expand_path("C:/keys/100%%_%u", home, "me", "web", "deploy"), r"C:\keys\100%_me");
        assert_eq!(expand_path("~", home, "me", "h", "r"), home);
    }

    #[test]
    fn known_hosts에서_호스트_키를_찾는다() {
        // 해시된 줄은 `ssh-keygen -H`가 만드는 꼴: |1|base64(소금)|base64(HMAC-SHA1(소금, 이름))
        use base64::Engine;
        use hmac::{Hmac, Mac};
        let salt = [7u8; 20];
        let mut mac = Hmac::<sha1::Sha1>::new_from_slice(&salt).unwrap();
        mac.update(b"[hashed.example]:2200");
        let engine = base64::engine::general_purpose::STANDARD;
        let hashed = format!("|1|{}|{}", engine.encode(salt), engine.encode(mac.finalize().into_bytes()));
        let text = format!(
            "# 주석\nexample.com,10.0.0.1 ssh-ed25519 AAAAkey1\n[example.com]:2222\tssh-ed25519\tAAAAkey2 comment\n{hashed} ssh-rsa AAAArsa\n*.wild.net ecdsa-sha2-nistp256 AAAAec\n@revoked bad.example ssh-ed25519 AAAAbad\n@cert-authority *.example.com ssh-ed25519 AAAAca\nmulti.example ssh-ed25519 AAAAold\nmulti.example ssh-ed25519 AAAAnew\n"
        );
        let check = |host: &str, port: u16, kind: &str, key: &str| check_known_host(&text, host, port, kind, key);
        assert_eq!(check("example.com", 22, "ssh-ed25519", "AAAAkey1"), HostKey::Known);
        assert_eq!(check("EXAMPLE.com", 22, "ssh-ed25519", "AAAAkey1"), HostKey::Known);
        assert_eq!(check("10.0.0.1", 22, "ssh-ed25519", "AAAAkey1"), HostKey::Known);
        assert_eq!(check("example.com", 2222, "ssh-ed25519", "AAAAkey2"), HostKey::Known);
        assert_eq!(check("example.com", 22, "ssh-ed25519", "AAAAother"), HostKey::Changed { line: 2, algorithm: "ssh-ed25519".into() });
        assert_eq!(check("example.com", 22, "ssh-rsa", "AAAArsa"), HostKey::Unknown { others: vec!["ssh-ed25519".into()] });
        assert_eq!(check("hashed.example", 2200, "ssh-rsa", "AAAArsa"), HostKey::Known);
        assert_eq!(check("hashed.example", 22, "ssh-rsa", "AAAArsa"), HostKey::Unknown { others: vec![] });
        assert_eq!(check("a.wild.net", 22, "ecdsa-sha2-nistp256", "AAAAec"), HostKey::Known);
        assert_eq!(check("bad.example", 22, "ssh-ed25519", "AAAAbad"), HostKey::Revoked { line: 6 });
        // 인증 기관 줄은 호스트 키로 보지 않는다.
        assert_eq!(check("x.example.com", 22, "ssh-ed25519", "AAAAca"), HostKey::Unknown { others: vec![] });
        // 같은 이름에 키가 여럿이면 하나라도 같으면 안다.
        assert_eq!(check("multi.example", 22, "ssh-ed25519", "AAAAnew"), HostKey::Known);
        assert_eq!(known_host_line("Web.Example", 22, "ssh-ed25519", "AAAA"), "web.example ssh-ed25519 AAAA");
        assert_eq!(known_host_line("::1", 2022, "ssh-ed25519", "AAAA"), "[::1]:2022 ssh-ed25519 AAAA");
    }

    fn spec(target: &str) -> SshTarget {
        SshTarget { target: target.into(), ..Default::default() }
    }

    #[test]
    fn 접속할_곳을_ssh_인자로_바꾼다() {
        assert_eq!(ssh_args(&spec("example.com")).unwrap(), ["example.com"]);
        assert_eq!(ssh_args(&spec("  web1  ")).unwrap(), ["web1"]);
        assert_eq!(ssh_args(&SshTarget { target: "10.0.0.5".into(), port: Some(2222) }).unwrap(), ["-p", "2222", "10.0.0.5"]);
        // 포트 0은 없는 것과 같다.
        assert_eq!(ssh_args(&SshTarget { target: "h".into(), port: Some(0) }).unwrap(), ["h"]);
        // 대상 자리에 옵션이나 빈칸이 든 것은 받지 않는다.
        for bad in ["", "   ", "-oProxyCommand=calc", "host -p 22", "a\tb", "a\nb"] {
            assert!(ssh_args(&spec(bad)).is_err(), "{bad:?}");
        }
        // 예전 기록의 키 파일·추가 옵션 칸은 읽지 않는다(사용자 이름·옵션은 저장하지 않는다).
        let old: SshTarget = toml::from_str("target = 'h'\nport = 22\nidentity = 'C:\\k'\noptions = '-J gate'\nname = '웹'\n").unwrap();
        assert_eq!(old, SshTarget { target: "h".into(), port: Some(22) });
    }

    #[test]
    fn 사용자_이름을_물은_뒤_ssh를_띄우는_명령을_만든다() {
        let script = login_script(r"C:\Program Files\OpenSSH\ssh.exe", &["-p".into(), "2222".into(), "it's’host".into()]);
        assert!(script.contains("Read-Host '사용자 이름'"), "{script}");
        // 작은따옴표(굽은 것 포함)는 두 번 적어 글 밖으로 나가지 못한다.
        assert!(script.contains(r"& 'C:\Program Files\OpenSSH\ssh.exe' -l $user '-p' '2222' 'it''s’’host'"), "{script}");
        assert!(script.ends_with("exit $LASTEXITCODE\n"));
        // 창 제목은 접속할 곳(마지막 인자)으로 둔다.
        assert!(script.contains("$Host.UI.RawUI.WindowTitle = 'ssh ' + $user + '@' + 'it''s’’host'"), "{script}");
        // UTF-16LE를 base64로: "a한" → 61 00 5C D5
        assert_eq!(encoded_command("a한"), "YQBc1Q==");
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
