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
    /// 읽기 스레드를 멈춰 세우는 문(화면이 출력을 못 따라갈 때의 흐름 제어).
    gate: Arc<(Mutex<Gate>, Condvar)>,
    /// 띄운 프로그램의 프로세스 번호(사용량 집계용).
    pid: Option<u32>,
}

#[derive(Default)]
struct Gate {
    /// 참이면 읽기 스레드가 멈춘다.
    paused: bool,
    /// 프로그램이 끝났거나 끝내라고 했다. 이 뒤로는 멈추지 않는다: 남은 출력을 끝까지 읽어야 읽기 스레드가 끝나고,
    /// 그래야 종료를 알릴 수 있다. 화면이 닫힌 세션에 뒤늦게 보낸 "멈춤"이 읽기를 영영 세워 두면 안 된다.
    ended: bool,
}

fn set_gate(gate: &(Mutex<Gate>, Condvar), paused: bool) {
    let mut state = gate.0.lock().unwrap();
    state.paused = paused && !state.ended;
    drop(state);
    gate.1.notify_all();
}

/// 문을 열고 다시는 닫히지 않게 한다.
fn end_gate(gate: &(Mutex<Gate>, Condvar)) {
    let mut state = gate.0.lock().unwrap();
    (state.paused, state.ended) = (false, true);
    drop(state);
    gate.1.notify_all();
}

/// 이름만 적힌 프로그램(`claude`)을 `path`(새 프로세스가 받을 PATH)에서 찾는다. 경로가 붙었거나 확장자가 있는 이름은
/// 찾지 않고 적힌 그대로 띄운다.
///
/// PATHEXT의 확장자를 붙인 파일만 찾는다. 확장자 없는 같은 이름의 파일은 건너뛴다: npm은 프로그램을 설치할 때 같은 폴더에
/// `claude`(셸 스크립트)와 `claude.cmd`를 함께 두는데, 가상 터미널 쪽의 찾기는 이름 그대로인 파일을 먼저 골라
/// "올바른 Win32 응용 프로그램이 아닙니다"로 실패한다.
fn find_program(name: &str, path: &std::ffi::OsStr, pathext: &str) -> Option<PathBuf> {
    let given = std::path::Path::new(name);
    if name.is_empty() || name.contains(['/', '\\', ':']) || given.extension().is_some() {
        return None;
    }
    let extensions: Vec<&str> = pathext.split(';').map(str::trim).filter(|ext| ext.starts_with('.')).collect();
    std::env::split_paths(path)
        .filter(|dir| dir.is_absolute())
        .flat_map(|dir| extensions.iter().map(move |ext| dir.join(format!("{name}{ext}"))).collect::<Vec<_>>())
        .find(|candidate| candidate.is_file())
}

fn is_batch(path: &std::path::Path) -> bool {
    path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat"))
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

        // 환경은 프로그램 이름과 상관없이 같으므로, 먼저 환경을 꾸려 새 프로세스가 받을 PATH에서 프로그램을 찾는다.
        let mut env = CommandBuilder::new(&opts.program);
        for key in &opts.env_remove {
            env.env_remove(key);
        }
        for (key, value) in &opts.env {
            env.env(key, value);
        }
        if let Some(dir) = &opts.path_first {
            // 새 프로세스가 받을 PATH(가상 터미널이 새로 꾸린 환경의 것) 앞에 폴더를 더한다.
            let base = env.get_env("PATH").map(|value| value.to_os_string()).or_else(|| std::env::var_os("PATH")).unwrap_or_default();
            let mut path = dir.clone().into_os_string();
            path.push(";");
            path.push(base);
            env.env("PATH", path);
        }
        let found = find_program(
            &opts.program,
            env.get_env("PATH").unwrap_or_default(),
            &env.get_env("PATHEXT").map(|value| value.to_string_lossy().into_owned()).unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into()),
        );
        let mut cmd = env;
        match found {
            // 배치 파일은 명령 해석기로 띄운다. `call`을 앞에 두는 것은 `/c` 뒤가 따옴표로 시작하면 명령 해석기가
            // 맨 앞과 맨 뒤의 따옴표를 떼어 버려, 공백이 든 경로와 따옴표로 감싼 인자가 함께 있을 때 깨지기 때문이다.
            Some(path) if is_batch(&path) => {
                let shell = cmd.get_env("ComSpec").map(|value| value.to_os_string()).unwrap_or_else(|| "cmd.exe".into());
                *cmd.get_argv_mut() = vec![shell, "/d".into(), "/c".into(), "call".into(), path.into_os_string()];
            }
            Some(path) => *cmd.get_argv_mut() = vec![path.into_os_string()],
            None => {}
        }
        cmd.args(&opts.args);
        cmd.cwd(&opts.cwd);

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

        let gate = Arc::new((Mutex::new(Gate::default()), Condvar::new()));
        let gate_for_reader = gate.clone();
        let reader_thread = thread::spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                {
                    let (lock, signal) = &*gate_for_reader;
                    let mut state = lock.lock().unwrap();
                    while state.paused {
                        state = signal.wait(state).unwrap();
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
            end_gate(&gate_for_wait);
            // Windows에서는 자식이 끝나도 ConPTY를 닫기 전까지 읽기가 끝나지 않는다.
            // 닫기는 잠금을 놓은 뒤에 한다: 닫기가 남은 출력이 비워지기를 기다리는 동안 잠금을 쥐고 있으면,
            // 그 사이의 크기 변경(창 스레드)이 잠금에서 멈춰 서로 기다리게 될 수 있다.
            let closing = master_for_wait.lock().unwrap().take();
            drop(closing);
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
        end_gate(&self.gate);
        let _ = self.killer.lock().unwrap().kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    fn options(program: &str, args: &[&str]) -> SpawnOptions {
        SpawnOptions {
            program: program.into(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            cwd: std::env::temp_dir(),
            env: vec![],
            env_remove: vec![],
            path_first: None,
            cols: 80,
            rows: 24,
        }
    }

    /// 프로그램을 띄워 끝날 때까지 기다린다. (세션, 종료 코드, 출력)
    fn run(opts: SpawnOptions) -> (PtySession, Option<u32>, String) {
        let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>();
        let (exit_tx, exit_rx) = mpsc::channel();
        let session = PtySession::spawn(
            opts,
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
        output.extend(out_rx.try_iter().flatten());
        (session, code, String::from_utf8_lossy(&output).into_owned())
    }

    #[test]
    fn 명령을_실행하고_출력과_종료_코드를_받는다() {
        let (session, code, output) = run(options("cmd.exe", &["/c", "echo cypher-ok& exit 3"]));
        assert_eq!(code, Some(3));
        assert!(output.contains("cypher-ok"));
        // 끝난 세션에 크기 변경·멈춤을 해도 오류가 나지 않아야 한다.
        session.resize(100, 30).unwrap();
        session.set_paused(true);
        assert!(!session.gate.0.lock().unwrap().paused); // 끝난 세션은 멈추지 않는다.
        session.kill();
    }

    #[test]
    fn 확장자_없는_같은_이름의_파일이_있어도_실행_파일을_찾는다() {
        // npm이 설치한 프로그램의 모양: 같은 폴더에 `이름`(셸 스크립트)과 `이름.cmd`가 함께 있다.
        // 시험 파일은 빌드 폴더에 만들고 끝나면 지운다.
        let dir = std::env::current_exe().unwrap().parent().unwrap().join(format!("cypher pty 시험-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ptytool"), "#!/bin/sh\necho sh\n").unwrap();
        std::fs::write(dir.join("ptytool.cmd"), "@echo cmd-ok %1\r\n@exit /b 5\r\n").unwrap();
        let mut opts = options("ptytool", &["arg one"]);
        opts.path_first = Some(dir.clone());
        let (_session, code, output) = run(opts);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(output.contains("cmd-ok"), "{output}");
        assert_eq!(code, Some(5));
    }
}
