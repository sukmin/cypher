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
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// 본체가 신호를 처리하고 연결을 닫기를 기다리는 시간. 본체가 멈춰 있어도 Claude Code를 오래 붙잡지 않는다.
const ACK_WAIT: Duration = Duration::from_millis(1500);

/// 원래 상태줄 명령이 끝나기를 기다리는 시간. 넘기면 그 명령이 띄운 것까지 함께 끝낸다.
/// Claude Code는 상태줄이 아직 돌고 있을 때 다음 갱신이 오면 그 실행을 버리지만(프로세스 트리를 부모 번호를 따라 끝낸다),
/// 다음 갱신이 없으면 오래(기본 10분) 기다린다. 그리고 부모 번호를 따라 끝내는 방식은 부모가 먼저 끝난 프로세스
/// (Git Bash가 띄운 프로그램이 그렇다)에 닿지 않는다. 멈춘 명령을 그대로 두면 남는 것이 갱신마다 쌓일 수 있다.
const ORIGINAL_WAIT: Duration = Duration::from_secs(10);

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
        // 본체가 접속만 받고 읽지 못하는 동안에는 큰 신호(Write·Read 도구의 신호에는 파일 내용이 든다)의 쓰기가
        // 끝없이 막힌다. 쓰기에도 시간 제한을 걸어 Claude Code를 오래 붙잡지 않는다.
        let _ = stream.set_write_timeout(Some(ACK_WAIT));
        // 한 덩어리로 만들어 한 번에 보낸다(`write!`는 조각마다 따로 보낸다).
        let message = format!("{token}\n{session}\n{input}");
        if stream.write_all(message.as_bytes()).is_ok() {
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
    shell.stdout(Stdio::inherit()).stderr(Stdio::null());
    run_bounded(&mut shell, input, ORIGINAL_WAIT);
}

/// 명령을 띄워 표준 입력으로 `input`을 넘기고 `limit` 안에 끝나기를 기다린다. 제때 끝났으면 true.
/// 넘기면 그 명령과 그 명령이 띄운 프로세스를 모두 끝낸다(제때 끝난 명령이 뒤에 남겨 둔 프로세스는 건드리지 않는다).
fn run_bounded(command: &mut Command, input: &str, limit: Duration) -> bool {
    let Ok(mut child) = command.stdin(Stdio::piped()).spawn() else { return false };
    // 그 명령이 띄우는 프로세스를 한 묶음(작업 개체)에 담아 둔다. 시간을 넘기면 묶음째 끝낸다.
    #[cfg(windows)]
    let job = job::Job::holding(&child);
    // 입력은 다른 스레드에서 써 넣는다. 명령이 입력을 읽지 않으면 큰 입력의 쓰기가 막히는데, 여기서 기다리면
    // 시간 제한을 재기도 전에 그 명령이 끝날 때까지 붙잡힌다. 다 쓰면(또는 명령이 끝나 쓰지 못하면) 입력을 닫는다.
    if let Some(mut stdin) = child.stdin.take() {
        let input = input.to_owned();
        std::thread::spawn(move || {
            let _ = stdin.write_all(input.as_bytes());
        });
    }
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => break,
        }
    }
    #[cfg(windows)]
    if let Some(job) = &job {
        job.kill();
    }
    kill_tree(&mut child);
    false
}

/// Windows의 작업 개체(Job Object): 프로세스와 그 프로세스가 띄우는 모든 프로세스를 한 묶음으로 다룬다.
///
/// 부모 번호를 따라 내려가며 끝내는 방식(`taskkill /T`)은 부모가 먼저 끝난 프로세스에 닿지 않는다. Git Bash(MSYS)의
/// 프로그램은 뜨는 방식 때문에 Windows에서 보는 부모가 곧 사라지고, `start /b`로 띄우고 끝난 셸의 자식도 그렇다.
/// 작업 개체에 든 프로세스가 띄운 프로세스는 부모가 끝나도 그 묶음에 남으므로, 묶음째 끝내면 모두 끝난다.
#[cfg(windows)]
mod job {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;

    extern "system" {
        fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> *mut c_void;
        fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
        fn TerminateJobObject(job: *mut c_void, exit_code: u32) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }

    pub struct Job(*mut c_void);

    impl Job {
        /// 그 프로세스를 새 묶음에 넣는다. 넣지 못하면(이미 든 묶음이 겹쳐 넣기를 막는 등) None: 그때는 부모 번호를 따라 끝낸다.
        /// 묶음에는 아무 제한도 걸지 않는다: 제때 끝난 명령이 남겨 둔 프로세스는 손잡이를 닫아도 그대로 돈다.
        pub fn holding(child: &std::process::Child) -> Option<Job> {
            // SAFETY: 이름 없는 작업 개체를 만들어, 살아 있는 자식 프로세스의 손잡이(child가 쥐고 있다)를 넣는다.
            // 만든 손잡이는 Job이 쥐고 있다가 버려질 때 한 번 닫는다.
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() {
                    return None;
                }
                let job = Job(job);
                (AssignProcessToJobObject(job.0, child.as_raw_handle() as *mut c_void) != 0).then_some(job)
            }
        }

        /// 묶음에 든 프로세스를 모두 끝낸다.
        pub fn kill(&self) {
            // SAFETY: 이 Job이 쥐고 있는 살아 있는 손잡이다.
            unsafe {
                TerminateJobObject(self.0, 1);
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: 이 Job이 만든 손잡이를 한 번만 닫는다.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

/// 프로세스와 그 자손을 부모 번호를 따라 내려가며 끝낸다. 셸만 끝내면 셸이 띄운 프로그램(상태줄 HUD 등)은 그대로 남는다.
/// 작업 개체에 넣지 못했을 때와, 넣기 전의 짧은 사이에 뜬 프로세스를 위해 묶음째 끝낸 뒤에도 한 번 더 한다.
fn kill_tree(child: &mut Child) {
    #[cfg(windows)]
    {
        // 이름만으로 부르지 않고 Windows 폴더의 것을 직접 부른다.
        let taskkill = std::env::var_os("SystemRoot").map(|root| PathBuf::from(root).join("System32").join("taskkill.exe"));
        if let Some(taskkill) = taskkill.filter(|path| path.is_file()) {
            let _ = Command::new(taskkill)
                .args(["/T", "/F", "/PID", &child.id().to_string()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// MCP: 표준 입력의 줄은 본체로, 본체의 줄은 표준 출력으로 나른다.
fn relay_mcp() {
    let Some((mut stream, token)) = connect() else {
        eprintln!("Cypher 본체에 연결하지 못했습니다");
        std::process::exit(1);
    };
    // 어느 Claude 세션이 띄운 것인지 함께 알린다(같은 워크스페이스의 브라우저·항목을 찾는 데 쓴다).
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

/// Cypher의 인자를 붙이지 않는 하위 명령: 대화를 여는 것이 아니라 Claude Code를 관리하는 명령들.
const MANAGING: [&str; 10] = ["mcp", "update", "upgrade", "doctor", "install", "config", "plugin", "auth", "setup-token", "migrate-installer"];

/// 대화를 여는 호출인지(Cypher의 설정·도구를 붙일지). 관리 명령과 버전·도움말만 묻는 호출에는 붙이지 않는다.
fn opens_session(args: &[std::ffi::OsString]) -> bool {
    let text: Vec<String> = args.iter().map(|arg| arg.to_string_lossy().into_owned()).collect();
    let managing = text.first().is_some_and(|first| MANAGING.contains(&first.as_str()));
    let asking = text.iter().any(|arg| matches!(arg.as_str(), "--version" | "-v" | "--help" | "-h"));
    !managing && !asking
}

/// 같은 폴더인지(표기만 다른 것은 같은 것으로 본다).
fn same_dir(a: &std::path::Path, b: &std::path::Path) -> bool {
    let plain = |p: &std::path::Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase();
    plain(a) == plain(b)
}

/// 진짜 `claude`의 파일 이름. 한 폴더 안에서는 이 순서로 찾는다.
const CLAUDE_NAMES: [&str; 3] = ["claude.exe", "claude.cmd", "claude.bat"];

/// PATH에서 진짜 `claude`를 찾는다. 이 프로그램이 놓인 폴더(`own`)는 건너뛴다.
fn find_claude(path: &std::ffi::OsStr, own: &std::path::Path) -> Option<PathBuf> {
    find_in_path(path, own, &CLAUDE_NAMES)
}

/// PATH의 폴더를 차례로 보며 `names` 가운데 처음 나오는 파일을 찾는다. `own` 폴더는 건너뛴다.
fn find_in_path(path: &std::ffi::OsStr, own: &std::path::Path, names: &[&str]) -> Option<PathBuf> {
    std::env::split_paths(path)
        .filter(|dir| !dir.as_os_str().is_empty() && !same_dir(dir, own))
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|file| file.is_file())
}

/// Ctrl+C를 이 프로그램이 받아 끝나지 않게 한다(받는 것은 Claude Code다). 자식 프로세스에는 이어지지 않는다.
#[cfg(windows)]
fn keep_running_on_ctrl_c() {
    extern "system" fn handled(_kind: u32) -> i32 {
        1
    }
    extern "system" {
        fn SetConsoleCtrlHandler(handler: Option<extern "system" fn(u32) -> i32>, add: i32) -> i32;
    }
    // SAFETY: 넘기는 함수는 프로그램이 끝날 때까지 살아 있고, 아무 상태도 건드리지 않는다.
    unsafe {
        SetConsoleCtrlHandler(Some(handled), 1);
    }
}

#[cfg(not(windows))]
fn keep_running_on_ctrl_c() {}

/// `claude`라는 이름으로 불렸을 때(끼움 프로그램): 진짜 Claude Code를 Cypher의 설정 파일(상태줄·훅)과 도구 설정을
/// 붙여 띄우고, 끝날 때까지 기다렸다가 그 종료 코드를 돌려준다.
///
/// Cypher는 셸 세션의 PATH 맨 앞에 이 프로그램의 사본이 `claude.exe`로 든 폴더를 넣는다. 그래서 그 셸에서 사용자가
/// `claude`를 치면(별칭·함수·스크립트가 이름으로 부르는 것 포함) 이 프로그램이 먼저 불린다. 인자는 받은 그대로 넘긴다.
fn run_claude(own: &std::path::Path) -> i32 {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let own_dir = own.parent().unwrap_or(own);
    let Some(real) = find_claude(&path, own_dir) else {
        eprintln!("Cypher: claude 명령을 찾지 못했습니다. Claude Code가 PATH에 있어야 합니다.");
        return 127;
    };
    let given: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let mut command = Command::new(&real);
    if opens_session(&given) {
        // `--mcp-config`는 값을 여러 개 받으므로 사용자의 인자 바로 앞에 두지 않는다(첫 인자를 값으로 삼킨다).
        if let Some(mcp) = std::env::var_os("CYPHER_CLAUDE_MCP").filter(|value| !value.is_empty()) {
            command.arg("--mcp-config").arg(mcp);
        }
        if let Some(settings) = std::env::var_os("CYPHER_CLAUDE_SETTINGS").filter(|value| !value.is_empty()) {
            command.arg("--settings").arg(settings);
        }
    }
    command.args(&given);
    // 이 Claude Code가 스스로 다시 띄우는 claude(도구로 실행하는 것 등)는 이 프로그램을 거치지 않게 한다:
    // 같은 세션 번호로 신호가 겹쳐 오지 않도록 PATH에서 이 폴더를 빼고, 붙일 파일의 경로도 넘기지 않는다.
    let rest = std::env::join_paths(std::env::split_paths(&path).filter(|dir| !same_dir(dir, own_dir))).unwrap_or(path);
    command.env("PATH", rest).env_remove("CYPHER_CLAUDE_SETTINGS").env_remove("CYPHER_CLAUDE_MCP");
    keep_running_on_ctrl_c();
    match command.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(error) => {
            eprintln!("Cypher: {}을(를) 실행하지 못했습니다: {error}", real.display());
            126
        }
    }
}

fn main() {
    // `claude`라는 이름의 사본으로 불렸으면 끼움 프로그램으로 동작한다.
    if let Ok(own) = std::env::current_exe() {
        if own.file_stem().is_some_and(|stem| stem.eq_ignore_ascii_case("claude")) {
            std::process::exit(run_claude(&own));
        }
    }
    match std::env::args().nth(1).as_deref() {
        Some("mcp") => relay_mcp(),
        _ => forward_status(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn 대화를_여는_호출에만_인자를_붙인다() {
        let opens = |args: &[&str]| opens_session(&args.iter().map(OsString::from).collect::<Vec<_>>());
        assert!(opens(&[]) && opens(&["--dangerously-skip-permissions"]) && opens(&["고쳐 줘", "--model", "opus"]));
        assert!(opens(&["-p", "질문"]) && opens(&["--resume", "abc"]) && opens(&["-c"]));
        // 관리 명령과 버전·도움말에는 붙이지 않는다.
        for args in [&["mcp", "list"][..], &["update"], &["doctor"], &["--version"], &["-h"], &["config", "list"]] {
            assert!(!opens(args), "{args:?}");
        }
        // 프롬프트가 관리 명령과 같은 낱말로 시작해도, 첫 인자 전체가 그 낱말일 때만 관리 명령이다.
        assert!(opens(&["update the readme"]));
    }

    #[test]
    fn 진짜_claude는_이_프로그램의_폴더를_건너뛰고_찾는다() {
        // 시험용 폴더는 임시 폴더가 아니라 빌드 폴더 아래에 두고, 프로그램처럼 보이는 파일(.exe, .cmd)은 만들지 않는다
        // (임시 폴더에 내용 없는 가짜 실행 파일을 만들면 Windows의 백신이 잡을 수 있다). 찾는 이름만 바꿔 같은 길을 본다.
        let names = ["tool.one", "tool.two", "tool.three"];
        let find = |path: &std::ffi::OsStr, own: &std::path::Path| find_in_path(path, own, &names);
        let root = std::env::current_exe().unwrap().parent().unwrap().join("test-files").join("cypher-shim-test");
        let _ = std::fs::remove_dir_all(&root);
        let (shim, first, second) = (root.join("shim"), root.join("a b"), root.join("c"));
        for dir in [&shim, &first, &second] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(shim.join("tool.one"), "").unwrap();
        std::fs::write(second.join("tool.one"), "").unwrap();
        let path = std::env::join_paths([shim.clone(), first.clone(), second.clone()]).unwrap();
        assert_eq!(find(&path, &shim), Some(second.join("tool.one")));
        // 앞선 폴더의 둘째 이름이 뒤 폴더의 첫째 이름보다 먼저다(PATH 순서). 표기가 달라도 자기 폴더는 건너뛴다.
        std::fs::write(first.join("tool.two"), "").unwrap();
        let upper = PathBuf::from(shim.to_string_lossy().to_uppercase().replace('\\', "/") + "/");
        assert_eq!(find(&path, &upper), Some(first.join("tool.two")));
        assert_eq!(find(&std::env::join_paths([shim.clone()]).unwrap(), &shim), None);
        // 진짜 claude는 이 이름들로, 이 순서로 찾는다.
        assert_eq!(CLAUDE_NAMES, ["claude.exe", "claude.cmd", "claude.bat"]);
        assert_eq!(find_claude(&path, &shim), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(windows)]
    fn 원래_상태줄이_제때_끝나지_않으면_그_명령이_띄운_것까지_끝낸다() {
        use std::os::windows::process::CommandExt;
        // 표시 파일은 임시 폴더가 아니라 빌드 폴더 아래에 둔다.
        let root = std::env::current_exe().unwrap().parent().unwrap().join("test-files").join("cypher-bounded-test");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // 제때 끝나는 명령은 끝날 때까지 기다린다.
        let mut quick = Command::new("cmd");
        quick.raw_arg("/C exit 0").stdout(Stdio::null());
        assert!(run_bounded(&mut quick, "{}", Duration::from_secs(10)));
        // 셸(cmd)이 띄운 셸이 2초쯤 뒤에 표시 파일을 쓴다. 시간을 넘겨 셸만 끝냈다면 그 아래 셸이 남아 파일을 쓴다.
        let mut slow = Command::new("cmd");
        slow.raw_arg(r#"/C "cmd /C "ping -n 3 127.0.0.1 >nul & echo x>late.txt"""#).current_dir(&root).stdout(Stdio::null());
        let started = Instant::now();
        assert!(!run_bounded(&mut slow, "{}", Duration::from_millis(300)));
        assert!(started.elapsed() < Duration::from_secs(2), "시간을 넘긴 뒤 바로 돌아와야 한다: {:?}", started.elapsed());
        std::thread::sleep(Duration::from_secs(3));
        assert!(!root.join("late.txt").exists(), "끝내지 못한 자손 프로세스가 남아 파일을 썼다");

        // 부모가 먼저 끝난 자손도 끝낸다(Git Bash가 띄운 프로그램, `start /b`로 띄우고 끝난 셸의 자식이 그렇다).
        // 가운데 셸은 맨 아래 셸을 띄우고 바로 끝나므로, 부모 번호를 따라 내려가서는 맨 아래 셸에 닿지 못한다.
        // 따옴표가 겹치지 않게 아래 두 단계의 명령은 환경 변수로 넘긴다(셸이 제 단계에서 풀어 쓴다).
        let mut orphan = Command::new("cmd");
        orphan
            .env("CYPHER_TEST_INNER", "ping -n 3 127.0.0.1 >nul & echo x>orphan.txt")
            .env("CYPHER_TEST_MIDDLE", r#"start /b cmd /C "%CYPHER_TEST_INNER%""#)
            .raw_arg(r#"/C "start /b cmd /C "%CYPHER_TEST_MIDDLE%" & ping -n 30 127.0.0.1 >nul""#)
            .current_dir(&root)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let started = Instant::now();
        assert!(!run_bounded(&mut orphan, "{}", Duration::from_millis(800)));
        assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
        std::thread::sleep(Duration::from_secs(4));
        assert!(!root.join("orphan.txt").exists(), "부모가 먼저 끝난 자손 프로세스가 남아 파일을 썼다");

        // 입력을 읽지 않는 명령에 큰 입력을 넘겨도 시간 제한은 지킨다(입력 쓰기가 막혀도 기다리지 않는다).
        let mut deaf = Command::new("cmd");
        deaf.raw_arg("/C ping -n 8 127.0.0.1 >nul").stdout(Stdio::null());
        let started = Instant::now();
        assert!(!run_bounded(&mut deaf, &"x".repeat(1 << 20), Duration::from_millis(300)));
        assert!(started.elapsed() < Duration::from_secs(2), "큰 입력이 막혀 시간 제한을 넘겼다: {:?}", started.elapsed());
        let _ = std::fs::remove_dir_all(&root);
    }
}
