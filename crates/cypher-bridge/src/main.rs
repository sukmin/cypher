//! Claude Code와 Cypher 본체 사이의 전달 프로그램. 두 가지로 쓰인다.
//!
//! - 인자 없이: 상태줄·훅 명령. 표준 입력으로 받은 JSON을 본체로 넘기고 아무것도 출력하지 않는다.
//! - `mcp`: 브라우저 조작 도구를 내주는 MCP 서버. 표준 입출력과 본체 사이에서 줄을 그대로 나른다.
//!
//! 본체가 세션을 띄울 때 넣어 준 환경 변수가 없으면 조용히 끝난다.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

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
        let _ = write!(stream, "{token}\n{session}\n{input}");
    }
}

/// MCP: 표준 입력의 줄은 본체로, 본체의 줄은 표준 출력으로 나른다.
fn relay_mcp() {
    let Some((mut stream, token)) = connect() else {
        eprintln!("Cypher 본체에 연결하지 못했습니다");
        std::process::exit(1);
    };
    if writeln!(stream, "{token}\nmcp").is_err() {
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
