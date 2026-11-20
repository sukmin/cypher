//! 터미널 세션 하나: 이 컴퓨터의 프로그램(가상 터미널)이거나 내장 SSH의 채널.
//!
//! 화면이 보내는 입력·크기·멈춤·닫기는 어느 쪽이든 같은 명령(`session_write` 등)으로 온다.
//! 출력 기록(사용자가 고른 글자 파일에 세션 출력을 계속 남기기)도 여기서 한다: 출력이 화면으로 가는 길에서 함께 받아
//! 색·커서 이동 같은 제어 글자를 뺀 글로 적는다(cypher_core::termlog).

use crate::sshc::{Conn, SshTerm};
use cypher_core::termlog::PlainText;
use cypher_pty::PtySession;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

enum Kind {
    Pty(PtySession),
    Ssh(SshTerm),
}

pub struct Term {
    kind: Kind,
    pub log: Log,
}

impl Term {
    pub fn pty(session: PtySession, log: Log) -> Self {
        Self { kind: Kind::Pty(session), log }
    }

    pub fn ssh(session: SshTerm, log: Log) -> Self {
        Self { kind: Kind::Ssh(session), log }
    }

    /// 이 컴퓨터의 프로세스 번호(내장 SSH 세션은 없다).
    pub fn pid(&self) -> Option<u32> {
        match &self.kind {
            Kind::Pty(session) => session.pid(),
            Kind::Ssh(_) => None,
        }
    }

    pub fn write(&self, data: &[u8]) -> Result<(), String> {
        match &self.kind {
            Kind::Pty(session) => session.write(data).map_err(|e| e.to_string()),
            Kind::Ssh(session) => {
                session.write(data);
                Ok(())
            }
        }
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        match &self.kind {
            Kind::Pty(session) => session.resize(cols, rows).map_err(|e| e.to_string()),
            Kind::Ssh(session) => {
                session.resize(cols, rows);
                Ok(())
            }
        }
    }

    pub fn set_paused(&self, paused: bool) {
        match &self.kind {
            Kind::Pty(session) => session.set_paused(paused),
            Kind::Ssh(session) => session.set_paused(paused),
        }
    }

    pub fn kill(&self) {
        match &self.kind {
            Kind::Pty(session) => session.kill(),
            Kind::Ssh(session) => session.kill(),
        }
    }

    /// 내장 SSH 세션이면 인증을 마친 그 연결(연결 복제에 쓴다).
    pub fn connection(&self) -> Option<Arc<Conn>> {
        match &self.kind {
            Kind::Ssh(session) => session.connection(),
            Kind::Pty(_) => None,
        }
    }
}

// ───────────────────────── 출력 기록 ─────────────────────────

/// 쓰는 쪽(따로 도는 스레드)에 보내는 것.
enum Chunk {
    Text(String),
    /// 마지막 글을 적고 끝낸다. 다 적으면 `done`으로 알린다.
    End(String, std_mpsc::Sender<()>),
}

/// 쓰는 쪽이 밀려 있을 때 쌓아 둘 조각 수. 넘치면(드라이브가 멈춤) 기록을 멈춘다.
const LOG_BACKLOG: usize = 4096;

struct LogFile {
    text: PlainText,
    path: PathBuf,
    writer: std_mpsc::SyncSender<Chunk>,
    /// 쓰는 쪽이 파일에 적지 못했으면 참.
    failed: Arc<AtomicBool>,
    notify: LogFailed,
}

#[derive(Default)]
struct LogState {
    file: Option<LogFile>,
    /// 세션이 끝나 더는 기록을 시작하지 않는다.
    closed: bool,
}

/// 세션 출력 기록. 꺼져 있으면 아무것도 하지 않는다. 출력을 받는 쪽과 명령 쪽이 나눠 쓴다.
/// 파일에 적는 것은 따로 도는 스레드가 한다: 느리거나 멈춘 드라이브가 출력(화면)과 명령(창)을 붙잡지 않게.
#[derive(Clone, Default)]
pub struct Log(Arc<Mutex<LogState>>);

/// 기록을 적지 못했을 때 부르는 것(파일 경로, 까닭). 화면에 알린다.
pub type LogFailed = Arc<dyn Fn(&Path, &str) + Send + Sync>;

impl Log {
    /// 출력 조각을 받는다(기록 중이면 끝난 줄을 파일에 적는다).
    pub fn feed(&self, bytes: &[u8]) {
        let mut state = self.0.lock().unwrap();
        let Some(log) = state.file.as_mut() else { return };
        if log.failed.load(Ordering::SeqCst) {
            state.file = None;
            return;
        }
        let text = log.text.feed(bytes);
        if text.is_empty() {
            return;
        }
        if let Err(err) = log.writer.try_send(Chunk::Text(text)) {
            // 쓰는 쪽이 멈춰 쌓아 둘 자리가 찼다(또는 이미 끝났다): 기록을 멈춘다.
            if matches!(err, std_mpsc::TrySendError::Full(_)) && !log.failed.swap(true, Ordering::SeqCst) {
                crate::log(&format!("출력 기록이 밀려 멈춥니다: {}", log.path.display()));
                (log.notify)(&log.path, "파일에 쓰는 것이 출력을 따라가지 못합니다(드라이브가 느리거나 멈췄습니다)");
            }
            log.failed.store(true, Ordering::SeqCst);
            state.file = None;
        }
    }

    /// 기록을 시작한다. 그 파일이 있으면 새로 쓴다(저장 창이 바꿀지 물었다). 이미 기록 중이면 앞의 것을 마치고 바꾼다.
    /// 적다가 실패하면 `failed`를 한 번 부르고 기록을 멈춘다.
    pub fn start(&self, path: &Path, label: &str, failed: LogFailed) -> std::io::Result<()> {
        if self.0.lock().unwrap().closed {
            return Err(std::io::Error::other("세션이 끝났습니다"));
        }
        self.stop();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut file = BufWriter::new(File::create(path)?);
        write!(file, "### Cypher 세션 출력 기록 · {label} · 시작 {}\r\n", crate::http::now_text())?;
        file.flush()?;
        let (writer, chunks) = std_mpsc::sync_channel::<Chunk>(LOG_BACKLOG);
        let flag = Arc::new(AtomicBool::new(false));
        let (thread_flag, thread_path, notify) = (flag.clone(), path.to_path_buf(), failed.clone());
        std::thread::Builder::new().name("cypher-session-log".into()).spawn(move || {
            for chunk in chunks {
                let (text, done) = match chunk {
                    Chunk::Text(text) => (text, None),
                    Chunk::End(text, done) => (text, Some(done)),
                };
                if !thread_flag.load(Ordering::SeqCst) {
                    if let Err(err) = file.write_all(text.as_bytes()).and_then(|_| file.flush()) {
                        thread_flag.store(true, Ordering::SeqCst);
                        crate::log(&format!("출력 기록을 적지 못해 멈춥니다: {} ({err})", thread_path.display()));
                        failed(&thread_path, &err.to_string());
                    }
                }
                if let Some(done) = done {
                    let _ = done.send(());
                    return;
                }
            }
        })?;
        let mut state = self.0.lock().unwrap();
        if state.closed {
            // 여는 사이에 세션이 끝났다.
            let _ = writer.try_send(Chunk::End(format!("### 기록 끝 {}\r\n", crate::http::now_text()), std_mpsc::channel().0));
            return Err(std::io::Error::other("세션이 끝났습니다"));
        }
        state.file = Some(LogFile { text: PlainText::new(), path: path.to_path_buf(), writer, failed: flag, notify });
        Ok(())
    }

    /// 기록을 마친다(끝나지 않은 줄과 마친 시각을 적는다). 기록 중이던 파일의 경로(적다가 실패했으면 None).
    /// `wait`만큼 다 적기를 기다린다(바로 그 파일을 열어 볼 수 있게).
    fn finish(&self, wait: Duration) -> Option<PathBuf> {
        let mut log = self.0.lock().unwrap().file.take()?;
        let rest = log.text.finish();
        let (done, finished) = std_mpsc::channel();
        // 기다리지 않고 넣는다: 쓰는 쪽이 멈춘 드라이브에 걸려 쌓아 둘 자리가 찼으면 끝 표시는 못 적은 것으로 친다
        // (여기서 기다리면 세션 종료 알림과 기록 끄기 명령이 돌아오지 않는다).
        let sent = log.writer.try_send(Chunk::End(format!("{rest}### 기록 끝 {}\r\n", crate::http::now_text()), done)).is_ok();
        if !sent {
            log.failed.store(true, Ordering::SeqCst);
        }
        if sent && !wait.is_zero() {
            let _ = finished.recv_timeout(wait);
        }
        (!log.failed.load(Ordering::SeqCst)).then_some(log.path)
    }

    /// 기록을 마친다(명령에서: 다 적을 때까지 잠깐 기다린다).
    pub fn stop(&self) -> Option<PathBuf> {
        self.finish(Duration::from_secs(2))
    }

    /// 세션이 끝났다: 기록을 마치고, 그 뒤로는 시작하지 않는다(기다리지 않는다).
    pub fn close(&self) {
        self.0.lock().unwrap().closed = true;
        self.finish(Duration::ZERO);
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.0.lock().unwrap().file.as_ref().map(|log| log.path.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quiet() -> LogFailed {
        Arc::new(|_: &Path, _: &str| {})
    }

    #[test]
    fn 출력을_글자로_기록하고_멈춘다() {
        let dir = std::env::current_exe().unwrap().parent().unwrap().join("test-files").join(format!("termlog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("기록").join("세션.log");
        let log = Log::default();
        log.feed(b"before start\n"); // 기록 전 출력은 남지 않는다.
        log.start(&path, "deploy@web", quiet()).unwrap();
        assert_eq!(log.path().as_deref(), Some(path.as_path()));
        log.feed("\x1b[32m$ ls\x1b[0m\r\n파일 하나\r\n$ ".as_bytes());
        assert_eq!(log.stop().as_deref(), Some(path.as_path()));
        log.feed(b"after stop\n");
        assert!(log.stop().is_none() && log.path().is_none());
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.split("\r\n").collect();
        assert!(lines[0].starts_with("### Cypher 세션 출력 기록 · deploy@web · 시작 "), "{text}");
        assert_eq!(&lines[1..4], ["$ ls", "파일 하나", "$"]);
        assert!(lines[4].starts_with("### 기록 끝 ") && !text.contains("before") && !text.contains("after"));
        // 다시 시작하면 새로 쓴다.
        log.start(&path, "again", quiet()).unwrap();
        log.stop();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("파일 하나"));
        // 세션이 끝나면 마치고, 그 뒤로는 시작하지 않는다.
        log.start(&path, "closing", quiet()).unwrap();
        log.feed(b"last line\n");
        log.close();
        assert!(log.path().is_none() && log.start(&path, "late", quiet()).is_err());
        for _ in 0..50 {
            if std::fs::read_to_string(&path).unwrap().contains("### 기록 끝") {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("last line") && text.contains("### 기록 끝"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
