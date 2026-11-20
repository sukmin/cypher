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
    /// 프로그램을 파일로 찾지 못했을 때 PowerShell의 명령(별칭·함수)으로 실행해 볼지.
    /// 사용자가 적어 준 실행 명령(Claude 실행 명령)에 쓴다: PowerShell 프로필에 만들어 둔 별칭·함수도 띄울 수 있게.
    pub shell_fallback: bool,
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

/// 띄울 프로그램을 찾은 결과.
#[derive(Debug, PartialEq)]
enum Launch {
    /// 실행 파일: 그 경로로 바로 띄운다.
    Exe(PathBuf),
    /// 배치 파일(.cmd, .bat): 명령 해석기로 띄운다.
    Batch(PathBuf),
    /// PowerShell 스크립트(.ps1): PowerShell로 띄운다.
    Script(PathBuf),
    /// 파일로 찾지 못했다: 적힌 그대로 띄운다(가상 터미널 쪽의 찾기에 맡긴다).
    AsGiven,
}

fn extension_is(path: &std::path::Path, names: &[&str]) -> bool {
    path.extension().is_some_and(|ext| names.iter().any(|name| ext.eq_ignore_ascii_case(name)))
}

fn launch_of(path: PathBuf) -> Launch {
    if extension_is(&path, &["cmd", "bat"]) {
        Launch::Batch(path)
    } else if extension_is(&path, &["ps1"]) {
        Launch::Script(path)
    } else {
        Launch::Exe(path)
    }
}

/// 이름이 경로로 적혔는지(폴더 구분자나 드라이브 글자가 들었다).
fn is_path(name: &str) -> bool {
    name.contains(['/', '\\', ':'])
}

/// 띄울 프로그램을 찾는다. `path`는 새 프로세스가 받을 PATH, `cwd`는 시작 폴더.
///
/// - 이름만 적힌 것(`claude`, `my-claude`, `claude-2.1`)은 PATH의 폴더를 차례로 보며 PATHEXT의 확장자를 붙인 파일을 찾는다.
///   확장자 없는 같은 이름의 파일은 건너뛴다: npm은 프로그램을 설치할 때 같은 폴더에 `claude`(셸 스크립트)와 `claude.cmd`를
///   함께 두는데, 가상 터미널 쪽의 찾기는 이름 그대로인 파일을 먼저 골라 "올바른 Win32 응용 프로그램이 아닙니다"로 실패한다.
///   어느 폴더에도 없으면 PowerShell 스크립트(`이름.ps1`)를 찾는다.
/// - 확장자까지 적힌 이름(`claude.cmd`, `run.ps1`)은 PATH에서 그 이름 그대로를 찾는다.
/// - 경로로 적힌 것(`C:\tools\claude.cmd`, `.\run.ps1`)은 그 파일을 쓴다(상대 경로는 시작 폴더 기준). 확장자를 빼고 적었으면
///   같은 식으로 확장자를 붙여 본다.
///
/// 찾은 파일이 배치 파일이나 PowerShell 스크립트면 그에 맞는 해석기로 띄워야 한다(그대로는 프로그램으로 실행되지 않거나
/// 인자의 따옴표가 깨진다).
fn find_program(name: &str, path: &std::ffi::OsStr, pathext: &str, cwd: &std::path::Path, scripts: bool) -> Launch {
    // 이름만 적은 것을 PATH에서 PowerShell 스크립트로 찾는 것은 `scripts`일 때만 한다(사용자가 적어 준 실행 명령).
    // 경로로 적은 `.ps1`은 언제나 스크립트로 띄운다.
    let script = if scripts || is_path(name) { ".ps1" } else { "" };
    find_file(name, path, pathext, cwd, script).map_or(Launch::AsGiven, launch_of)
}

/// `find_program`의 찾기. `script`는 PATHEXT에 없어도 마지막에 찾아볼 스크립트의 확장자(`.ps1`)다.
fn find_file(name: &str, path: &std::ffi::OsStr, pathext: &str, cwd: &std::path::Path, script: &str) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    let given = std::path::Path::new(name);
    let known: Vec<&str> = pathext.split(';').map(str::trim).filter(|ext| ext.starts_with('.')).collect();
    // 이름에 이미 실행할 수 있는 확장자가 붙어 있는지(`claude-2.1`의 `.1`은 확장자가 아니다).
    let complete = given
        .extension()
        .is_some_and(|ext| known.iter().chain(&[script]).any(|known| !known.is_empty() && known[1..].eq_ignore_ascii_case(&ext.to_string_lossy())));
    // 그 자리(폴더와 이름)에서 붙여 볼 확장자들로 파일을 찾는다.
    let at = |base: &std::path::Path, extensions: &[&str]| -> Option<PathBuf> {
        if complete {
            return base.is_file().then(|| base.to_path_buf());
        }
        extensions.iter().map(|ext| PathBuf::from(format!("{}{ext}", base.display()))).find(|candidate| candidate.is_file())
    };
    let bases: Vec<PathBuf> = if is_path(name) {
        vec![if given.is_absolute() { given.to_path_buf() } else { cwd.join(given) }]
    } else {
        std::env::split_paths(path).filter(|dir| dir.is_absolute()).map(|dir| dir.join(name)).collect()
    };
    // PATHEXT의 확장자를 모든 폴더에서 먼저 찾고, 없을 때만 PowerShell 스크립트를 찾는다.
    bases.iter().find_map(|base| at(base, &known)).or_else(|| if complete || script.is_empty() { None } else { bases.iter().find_map(|base| at(base, &[script])) })
}

/// PowerShell에 넘길 글 한 덩어리: 작은따옴표로 감싸고 안의 작은따옴표는 겹쳐 적는다.
fn powershell_quote(text: &str) -> String {
    // PowerShell은 굽은 따옴표(‘ ’ ‚ ‛)도 작은따옴표로 읽는다. 모두 겹쳐 적어야 글자 그대로 넘어간다
    // (겹쳐 적지 않으면 그 자리에서 따옴표가 닫혀 뒤의 글이 명령으로 읽힌다).
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('\'');
    for ch in text.chars() {
        quoted.push(ch);
        if matches!(ch, '\'' | '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}') {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    quoted
}

/// 파일로 찾지 못한 이름을 PowerShell의 명령(별칭·함수)으로 실행하는 명령 글. 끝나면 그 명령의 종료 코드로 끝낸다.
fn shell_command(program: &str, args: &[String]) -> String {
    let mut line = format!("& {}", powershell_quote(program));
    for arg in args {
        line.push(' ');
        line.push_str(&powershell_quote(arg));
    }
    // 프로그램이 낸 종료 코드가 있으면 그것으로, 없으면(별칭·함수가 프로그램을 띄우지 않았거나 그런 명령이 없다) 성공 여부로 끝낸다.
    format!("{line}; $ok = $?; if ($null -ne $LASTEXITCODE) {{ exit $LASTEXITCODE }} elseif (-not $ok) {{ exit 1 }}")
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
        // 가상 터미널 라이브러리는 전체 경로가 아닌 프로그램 이름을 PATH에서 다시 찾으면서 PATHEXT 항목마다 첫 글자(`.`)를
        // 떼는데, 빈 항목(`.EXE;.CMD;`처럼 끝에 `;`가 있을 때)에서 패닉한다. 넘기는 환경에서 빈 항목을 뺀다.
        if let Some(extensions) = env.get_env("PATHEXT").map(|value| value.to_string_lossy().into_owned()) {
            let kept: Vec<&str> = extensions.split(';').map(str::trim).filter(|ext| ext.starts_with('.') && ext.len() > 1).collect();
            if kept.len() != extensions.split(';').count() {
                env.env("PATHEXT", kept.join(";"));
            }
        }
        let found = find_program(
            &opts.program,
            env.get_env("PATH").unwrap_or_default(),
            &env.get_env("PATHEXT").map(|value| value.to_string_lossy().into_owned()).unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into()),
            &opts.cwd,
            opts.shell_fallback,
        );
        let mut cmd = env;
        let mut args = opts.args.clone();
        match found {
            // 배치 파일은 명령 해석기로 띄운다. `call`을 앞에 두는 것은 `/c` 뒤가 따옴표로 시작하면 명령 해석기가
            // 맨 앞과 맨 뒤의 따옴표를 떼어 버려, 공백이 든 경로와 따옴표로 감싼 인자가 함께 있을 때 깨지기 때문이다.
            Launch::Batch(path) => {
                let shell = cmd.get_env("ComSpec").map(|value| value.to_os_string()).unwrap_or_else(|| "cmd.exe".into());
                *cmd.get_argv_mut() = vec![shell, "/d".into(), "/c".into(), "call".into(), path.into_os_string()];
            }
            // PowerShell 스크립트는 PowerShell로 띄운다. 실행 정책은 건드리지 않는다(사용자의 셸에서 돌던 스크립트면 여기서도 돈다.
            // 정책이 막으면 PowerShell이 그 까닭을 세션 화면에 알려 준다).
            Launch::Script(path) => {
                *cmd.get_argv_mut() = vec!["powershell.exe".into(), "-NoLogo".into(), "-File".into(), path.into_os_string()];
            }
            Launch::Exe(path) => *cmd.get_argv_mut() = vec![path.into_os_string()],
            // 파일로 찾지 못한 이름: PowerShell 프로필에 만들어 둔 별칭·함수일 수 있다. PowerShell의 명령으로 실행해 본다
            // (그것도 아니면 PowerShell이 "인식되지 않는 명령"이라고 알려 주고 끝난다).
            Launch::AsGiven if opts.shell_fallback && !is_path(&opts.program) => {
                *cmd.get_argv_mut() = vec!["powershell.exe".into(), "-NoLogo".into(), "-Command".into(), shell_command(&opts.program, &args).into()];
                args.clear();
            }
            Launch::AsGiven => {}
        }
        cmd.args(&args);
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
            shell_fallback: false,
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

    #[test]
    fn 띄울_프로그램을_이름과_경로로_찾는다() {
        // 파일 이름만 보므로 내용은 글 한 줄이고, 확장자는 프로그램처럼 보이지 않게 지어낸 것을 쓴다(.run, .pack, 스크립트 자리는 .note).
        // 시험 파일은 빌드 폴더에 만들고 끝나면 지운다.
        let root = std::env::current_exe().unwrap().parent().unwrap().join(format!("cypher find 시험-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (first, second) = (root.join("a b"), root.join("c"));
        for dir in [&first, &second] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let file = |path: PathBuf| std::fs::write(path, "프로그램 찾기 시험에 쓰는 글 파일입니다.").unwrap();
        file(first.join("tool")); // 확장자 없는 같은 이름(건너뛴다)
        file(second.join("tool.run"));
        file(second.join("tool-2.1.run"));
        file(second.join("wrap.pack"));
        file(second.join("script.note"));
        file(first.join("both.note"));
        file(second.join("both.run"));
        let path = std::env::join_paths([first.clone(), second.clone()]).unwrap();
        let find = |name: &str| find_file(name, &path, ".run;.pack", &second, ".note");
        assert_eq!(find("tool"), Some(second.join("tool.run")));
        assert_eq!(find("tool.run"), Some(second.join("tool.run")));
        // 이름에 점이 있어도 실행 파일의 확장자가 아니면 확장자를 붙여 찾는다.
        assert_eq!(find("tool-2.1"), Some(second.join("tool-2.1.run")));
        assert_eq!(find("wrap"), Some(second.join("wrap.pack")));
        // PowerShell 스크립트: 이름만 적어도, 확장자까지 적어도 찾는다.
        assert_eq!(find("script"), Some(second.join("script.note")));
        assert_eq!(find("script.note"), Some(second.join("script.note")));
        // 뒤쪽 폴더에 있더라도 PATHEXT의 확장자가 PowerShell 스크립트보다 먼저다.
        assert_eq!(find("both"), Some(second.join("both.run")));
        assert_eq!(find("both.note"), Some(first.join("both.note")));
        // 경로로 적은 것: 절대 경로, 확장자를 뺀 경로, 시작 폴더 기준의 상대 경로.
        assert_eq!(find(&second.join("tool.run").to_string_lossy()), Some(second.join("tool.run")));
        assert_eq!(find(&second.join("script").to_string_lossy()), Some(second.join("script.note")));
        assert_eq!(find(r".\script.note"), Some(second.join(r".\script.note")));
        // 없는 것은 적힌 그대로 둔다.
        for missing in ["nothing", "tool.none", "", r"C:\없는 폴더\x.run"] {
            assert_eq!(find(missing), None, "{missing}");
        }
        // 스크립트를 찾지 않는 경우(`script`가 빈 글): 이름만 적은 것은 PATHEXT의 확장자로만 찾는다.
        let plain = |name: &str| find_file(name, &path, ".run;.pack", &second, "");
        assert_eq!(plain("script"), None);
        assert_eq!(plain("both"), Some(second.join("both.run")));
        assert_eq!(plain("tool"), Some(second.join("tool.run")));
        let _ = std::fs::remove_dir_all(&root);
        // 굽은 따옴표가 든 인자도 글자 그대로 넘어간다.
        assert_eq!(powershell_quote("a’; b ‘c"), "'a’’; b ‘‘c'");
        assert_eq!(launch_of(PathBuf::from(r"C:\x\Claude.CMD")), Launch::Batch(PathBuf::from(r"C:\x\Claude.CMD")));
        assert_eq!(launch_of(PathBuf::from("run.bat")), Launch::Batch(PathBuf::from("run.bat")));
        assert_eq!(launch_of(PathBuf::from("wrap.PS1")), Launch::Script(PathBuf::from("wrap.PS1")));
        assert_eq!(launch_of(PathBuf::from("claude.exe")), Launch::Exe(PathBuf::from("claude.exe")));
        assert_eq!(find_program("cypher-없는-프로그램", std::ffi::OsStr::new(""), ".EXE", &second, true), Launch::AsGiven);

        assert_eq!(
            shell_command("cc", &["--settings".into(), r"C:\a b\it's.json".into()]),
            r"& 'cc' '--settings' 'C:\a b\it''s.json'; $ok = $?; if ($null -ne $LASTEXITCODE) { exit $LASTEXITCODE } elseif (-not $ok) { exit 1 }"
        );
    }

    #[test]
    fn 파일로_찾지_못한_이름은_powershell의_명령으로_실행한다() {
        // PowerShell에 원래 있는 별칭(echo)으로 확인한다. 별칭은 파일이 아니라서 PATH에서는 찾을 수 없다.
        let mut opts = options("echo", &["cypher shell ok"]);
        opts.shell_fallback = true;
        let (_session, code, output) = run(opts);
        assert!(output.contains("cypher shell ok"), "{output}");
        assert_eq!(code, Some(0));
        // 그런 명령이 없으면 PowerShell이 알려 주고 실패로 끝난다.
        let mut opts = options("cypher-이런-명령은-없다", &[]);
        opts.shell_fallback = true;
        let (_session, code, _output) = run(opts);
        assert_eq!(code, Some(1));
        // 프로그램이 낸 종료 코드는 그대로 전한다(PowerShell의 함수로 감싼 프로그램처럼).
        let mut opts = options("Invoke-Expression", &["cmd /c exit 7"]);
        opts.shell_fallback = true;
        let (_session, code, _output) = run(opts);
        assert_eq!(code, Some(7));
    }
}
