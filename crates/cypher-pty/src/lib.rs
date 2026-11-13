//! ConPTY 세션: 프로그램을 가상 터미널에 붙여 띄우고 입출력을 중계한다.

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

pub struct SpawnOptions {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// 부모 환경에 더할 변수.
    pub env: Vec<(String, String)>,
    /// 부모 환경에서 뺄 변수.
    pub env_remove: Vec<String>,
    /// 새 프로세스의 PATH 맨 앞에 더할 폴더.
    pub path_first: Option<PathBuf>,
    pub cols: u16,
    pub rows: u16,
}

pub struct PtySession {
    /// 프로세스가 끝나면 None으로 바뀐다(ConPTY를 닫아야 읽기 스레드가 끝난다).
    master: Arc<Mutex<Option<Box<dyn MasterPty + Send>>>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    /// 참이면 읽기 스레드가 멈춘다(화면이 출력을 못 따라갈 때의 흐름 제어).
    gate: Arc<(Mutex<bool>, Condvar)>,
    /// 띄운 프로그램의 프로세스 번호(사용량 집계용).
    pid: Option<u32>,
}

fn set_gate(gate: &(Mutex<bool>, Condvar), paused: bool) {
    *gate.0.lock().unwrap() = paused;
    gate.1.notify_all();
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize { rows: rows.max(1), cols: cols.max(1), pixel_width: 0, pixel_height: 0 }
}

impl PtySession {
    /// 프로그램을 띄운다. `on_output`은 읽기 스레드에서, `on_exit`는 남은 출력을
    /// 모두 전달한 뒤 한 번 불린다(종료 코드를 알 수 없으면 None).
    ///
    /// 주의: ConPTY는 시작 직후 커서 위치 질의(ESC[6n)를 보내고 답이 올 때까지
    /// 프로그램을 진행시키지 않는다. 출력을 받는 쪽(xterm.js)이 답을 돌려줘야 한다.
    pub fn spawn(
        opts: SpawnOptions,
        mut on_output: impl FnMut(&[u8]) + Send + 'static,
        on_exit: impl FnOnce(Option<u32>) + Send + 'static,
    ) -> Result<Self> {
        let pair = native_pty_system()
            .openpty(size(opts.cols, opts.rows))
            .context("가상 터미널(ConPTY)을 만들지 못했습니다")?;

        let mut cmd = CommandBuilder::new(&opts.program);
        cmd.args(&opts.args);
        cmd.cwd(&opts.cwd);
        for key in &opts.env_remove {
            cmd.env_remove(key);
        }
        for (key, value) in &opts.env {
            cmd.env(key, value);
        }
        if let Some(dir) = &opts.path_first {
            // 새 프로세스가 받을 PATH(가상 터미널이 새로 꾸린 환경의 것) 앞에 폴더를 더한다.
            let base = cmd.get_env("PATH").map(|value| value.to_os_string()).or_else(|| std::env::var_os("PATH")).unwrap_or_default();
            let mut path = dir.clone().into_os_string();
            path.push(";");
            path.push(base);
            cmd.env("PATH", path);
        }

        let mut child = pair
            .slave
            .spawn_command(cmd)
            .with_context(|| format!("프로그램을 실행하지 못했습니다: {}", opts.program))?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().context("출력 통로를 열지 못했습니다")?;
        let writer = pair.master.take_writer().context("입력 통로를 열지 못했습니다")?;
        let killer = child.clone_killer();
        let pid = child.process_id();
        let master = Arc::new(Mutex::new(Some(pair.master)));

        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let gate_for_reader = gate.clone();
        let reader_thread = thread::spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                {
                    let (lock, signal) = &*gate_for_reader;
                    let mut paused = lock.lock().unwrap();
                    while *paused {
                        paused = signal.wait(paused).unwrap();
                    }
                }
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => on_output(&buf[..n]),
                }
            }
        });

        let master_for_wait = master.clone();
        let gate_for_wait = gate.clone();
        thread::spawn(move || {
            let code = child.wait().ok().map(|status| status.exit_code());
            // 멈춰 둔 읽기를 풀어야 남은 출력을 비우고 끝낼 수 있다.
            set_gate(&gate_for_wait, false);
            // Windows에서는 자식이 끝나도 ConPTY를 닫기 전까지 읽기가 끝나지 않는다.
            master_for_wait.lock().unwrap().take();
            let _ = reader_thread.join();
            on_exit(code);
        });

        Ok(Self { master, writer: Mutex::new(writer), killer: Mutex::new(killer), gate, pid })
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// 출력 읽기를 멈추거나 다시 시작한다. 멈추면 프로그램 쪽 출력이 자연히 밀린다.
    pub fn set_paused(&self, paused: bool) {
        set_gate(&self.gate, paused);
    }

    pub fn write(&self, data: &[u8]) -> Result<()> {
        let mut writer = self.writer.lock().unwrap();
        writer.write_all(data)?;
        writer.flush()?;
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        if let Some(master) = self.master.lock().unwrap().as_ref() {
            master.resize(size(cols, rows))?;
        }
        Ok(())
    }

    pub fn kill(&self) {
        set_gate(&self.gate, false);
        let _ = self.killer.lock().unwrap().kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn 명령을_실행하고_출력과_종료_코드를_받는다() {
        let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>();
        let (exit_tx, exit_rx) = mpsc::channel();
        let session = PtySession::spawn(
            SpawnOptions {
                program: "cmd.exe".into(),
                args: vec!["/c".into(), "echo cypher-ok& exit 3".into()],
                cwd: std::env::temp_dir(),
                env: vec![],
                env_remove: vec![],
                path_first: None,
                cols: 80,
                rows: 24,
            },
            move |bytes| {
                let _ = out_tx.send(bytes.to_vec());
            },
            move |code| {
                let _ = exit_tx.send(code);
            },
        )
        .unwrap();

        // ConPTY는 시작할 때 커서 위치를 묻고(ESC[6n) 답을 기다린다.
        // 실제 앱에서는 xterm.js가 답하므로, 시험에서는 여기서 터미널 역할을 대신한다.
        let mut output = Vec::new();
        let mut answered = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let code = loop {
            if let Ok(code) = exit_rx.try_recv() {
                break code;
            }
            assert!(std::time::Instant::now() < deadline, "종료 알림이 오지 않음");
            if let Ok(chunk) = out_rx.recv_timeout(Duration::from_millis(50)) {
                output.extend(chunk);
                if !answered && output.windows(4).any(|w| w == b"\x1b[6n") {
                    session.write(b"\x1b[1;1R").unwrap();
                    answered = true;
                }
            }
        };
        assert_eq!(code, Some(3));
        output.extend(out_rx.try_iter().flatten());
        assert!(String::from_utf8_lossy(&output).contains("cypher-ok"));
        // 끝난 세션에 크기 변경·멈춤을 해도 오류가 나지 않아야 한다.
        session.resize(100, 30).unwrap();
        session.set_paused(true);
        session.kill();
    }
}
