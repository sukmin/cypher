//! Claude Code와 Cypher 본체 사이의 전달 프로그램. 두 가지로 쓰인다.
//!
//! - 인자 없이: 상태줄·훅 명령. 표준 입력으로 받은 JSON을 본체로 넘긴다.
//!   사용자가 원래 쓰던 상태줄(HUD)이 있으면 그 명령을 이어서 실행해 출력을 그대로 내보낸다.
//! - `mcp`: Cypher 도구(브라우저 조작 등)를 내주는 MCP 서버. 표준 입출력과 본체 사이에서 줄을 그대로 나른다.
//!
//! 본체가 세션을 띄울 때 넣어 준 환경 변수가 없으면 조용히 끝난다.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

/// 본체가 신호를 처리하고 연결을 닫기를 기다리는 시간. 본체가 멈춰 있어도 Claude Code를 오래 붙잡지 않는다.
const ACK_WAIT: Duration = Duration::from_millis(1500);

fn connect() -> Option<(TcpStream, String)> {
    let port = std::env::var("CYPHER_PORT").ok()?.parse::<u16>().ok()?;
    let token = std::env::var("CYPHER_TOKEN").ok()?;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let stream = TcpStream::connect_timeout(&addr, Duration::from_millis(500)).ok()?;
    Some((stream, token))
}

/// 상태줄·훅: 받은 내용을 세션 번호와 함께 한 번 보내고 끝난다.
fn forward_status() {
    let Ok(session) = std::env::var("CYPHER_SESSION") else { return };
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return;
    }
    if let Some((mut stream, token)) = connect() {
        if write!(stream, "{token}\n{session}\n{input}").is_ok() {
            // 다 보냈다고 알리고, 본체가 처리한 뒤 연결을 닫을 때까지 잠깐 기다린다. 이렇게 해야
            // (1) 본체가 이 프로세스가 살아 있는 동안 누가 보낸 것인지 확인할 수 있고,
            // (2) 쓰기 직전 신호라면 본체가 "그 파일이 아직 없다"는 것을 Claude Code가 쓰기 전에 본다.
            let _ = stream.shutdown(std::net::Shutdown::Write);
            let _ = stream.set_read_timeout(Some(ACK_WAIT));
            let _ = stream.read(&mut [0u8; 1]);
        }
    }
    // 상태줄로 불렸을 때만(훅 신호가 아닐 때) 원래 상태줄을 이어서 실행한다.
    if !input.contains("\"hook_event_name\"") {
        if let Ok(original) = std::env::var("CYPHER_STATUSLINE") {
            run_original(&original, &input);
        }
    }
}

/// Claude Code가 상태줄 명령을 실행하는 데 쓰는 Git Bash를 찾는다.
/// `bash`를 이름만으로 실행하면 Windows가 System32의 WSL용 bash를 먼저 고르므로 직접 찾는다.
fn git_bash() -> Option<PathBuf> {
    let listed = ["CLAUDE_CODE_GIT_BASH_PATH", "SHELL"]
        .iter()
        .filter_map(|name| std::env::var_os(name).map(PathBuf::from))
        .find(|path| path.is_file());
    listed.or_else(|| {
        std::env::split_paths(&std::env::var_os("PATH")?)
            .filter(|dir| !dir.to_string_lossy().to_ascii_lowercase().contains("system32"))
            .map(|dir| dir.join("bash.exe"))
            .find(|path| path.is_file())
    })
}

/// 사용자가 원래 쓰던 상태줄 명령을 같은 입력으로 실행하고, 그 출력을 그대로 내보낸다.
/// Claude Code와 같은 셸로 실행해야 명령이 원래대로 풀린다: Git Bash 환경이면 bash, 아니면 PowerShell.
fn run_original(command: &str, input: &str) {
    let mut shell = match git_bash().filter(|_| std::env::var_os("MSYSTEM").is_some()) {
        Some(bash) => {
            let mut shell = Command::new(bash);
            shell.args(["-c", command]);
            shell
        }
        None => {
            let mut shell = Command::new("powershell.exe");
            shell.args(["-NoProfile", "-Command", command]);
            shell
        }
    };
    // 원래 명령이 다시 이 프로그램을 부르더라도 끝없이 이어지지 않게 한다.
    shell.env_remove("CYPHER_STATUSLINE");
    let spawned = shell.stdin(Stdio::piped()).stdout(Stdio::inherit()).stderr(Stdio::null()).spawn();
    let Ok(mut child) = spawned else { return };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }
    let _ = child.wait();
}

/// MCP: 표준 입력의 줄은 본체로, 본체의 줄은 표준 출력으로 나른다.
fn relay_mcp() {
    let Some((mut stream, token)) = connect() else {
        eprintln!("Cypher 본체에 연결하지 못했습니다");
        std::process::exit(1);
    };
    // 어느 Claude 세션이 띄운 것인지 함께 알린다(같은 그룹의 브라우저·항목을 찾는 데 쓴다).
    let session = std::env::var("CYPHER_SESSION").unwrap_or_default();
    if writeln!(stream, "{token}\nmcp {session}").is_err() {
        return;
    }
    let Ok(from_app) = stream.try_clone() else { return };
    std::thread::spawn(move || {
        let mut out = std::io::stdout();
        for line in BufReader::new(from_app).lines() {
            let Ok(line) = line else { break };
            if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
                break;
            }
        }
        std::process::exit(0); // 본체가 끝났다.
    });
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if writeln!(stream, "{line}").and_then(|_| stream.flush()).is_err() {
            break;
        }
    }
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("mcp") => relay_mcp(),
        _ => forward_status(),
    }
}
