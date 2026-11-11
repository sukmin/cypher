//! 화면에서 부르는 명령: 설정, 세션 관리, 링크 처리.

use crate::AppState;
use serde::Serialize;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Emitter, Manager, State};
use cypher_core::config::Config;
use cypher_core::links::{self, LinkSpan, Target};
use cypher_pty::{PtySession, SpawnOptions};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// PowerShell이 프롬프트를 그릴 때마다 현재 폴더를 알려 주게 한다(OSC 9;9).
/// 상대경로 링크의 기준 폴더가 `cd`를 따라가는 데 쓴다. 큰따옴표를 쓰지 않는다.
const POWERSHELL_CWD_HOOK: &str = "$global:__cypherPrompt = $function:prompt; \
function global:prompt { \
$l = $executionContext.SessionState.Path.CurrentLocation; \
if ($l.Provider.Name -eq 'FileSystem') { \
[Console]::Write(([string][char]27) + ']9;9;' + $l.ProviderPath + ([string][char]7)) }; \
& $global:__cypherPrompt }";

#[derive(Clone, Serialize)]
struct ExitPayload {
    id: u32,
    code: Option<u32>,
}

#[derive(Serialize)]
pub struct ConfigPayload {
    config: Config,
    /// 설정 파일을 읽지 못했을 때의 설명(이때는 기본값으로 동작한다).
    error: Option<String>,
    path: String,
}

#[tauri::command]
pub fn app_config(state: State<'_, AppState>) -> ConfigPayload {
    ConfigPayload {
        config: state.config.clone(),
        error: state.config_error.clone(),
        path: state.config_path.to_string_lossy().into_owned(),
    }
}

/// 설정 파일을 메모장으로 연다.
#[tauri::command]
pub fn open_config(state: State<'_, AppState>) -> Result<(), String> {
    Command::new("notepad.exe").arg(&state.config_path).spawn().map(drop).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn home_dir() -> String {
    std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\".into())
}

/// 시작하자마자 열 세션. 환경 변수 `CYPHER_AUTOSTART=<프로필>`이 있으면
/// 실행 위치 폴더에서 그 프로필을 연다(바로가기·시험용).
#[tauri::command]
pub fn autostart(state: State<'_, AppState>) -> Option<(String, String)> {
    let profile = std::env::var("CYPHER_AUTOSTART").ok().filter(|p| state.config.profile(p).is_some())?;
    let cwd = std::env::current_dir().ok()?;
    Some((profile, cwd.to_string_lossy().into_owned()))
}

/// Windows 알림을 띄우고 작업 표시줄 아이콘을 깜빡인다.
/// 화면 쪽에서 창이 뒤에 있을 때만 부른다.
#[tauri::command]
pub fn notify(app: AppHandle, title: String, body: String) {
    use tauri_plugin_notification::NotificationExt;
    let _ = app.notification().builder().title(title).body(body).show();
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.request_user_attention(Some(tauri::UserAttentionType::Informational));
    }
}

#[tauri::command]
pub async fn pick_folder(start: Option<String>, title: Option<String>) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let title = title.unwrap_or_else(|| "세션을 시작할 폴더".into());
        let mut dialog = rfd::FileDialog::new().set_title(title);
        if let Some(dir) = start.filter(|d| Path::new(d).is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        dialog.pick_folder().map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .ok()
    .flatten()
}

/// Claude 세션 ID(UUID) 모양인지. 명령 인자로 넘기기 전에 확인한다.
fn is_session_id(s: &str) -> bool {
    s.len() == 36 && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// 워크트리 이름으로 쓸 수 있는지(영문·숫자·`.`·`_`·`-`, 옵션처럼 보이지 않을 것).
fn is_worktree_name(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && !s.starts_with(['-', '.'])
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// 인자 없이 띄우는 PowerShell인지(현재 폴더 알림을 심어도 되는지).
fn wants_powershell_hook(program: &str, args: &[String]) -> bool {
    let name = Path::new(program).file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase());
    matches!(name.as_deref(), Some("powershell" | "pwsh"))
        && !args.iter().any(|a| {
            let a = a.to_ascii_lowercase();
            ["-c", "-command", "-file", "-f", "-encodedcommand", "-noexit"].contains(&a.as_str())
        })
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn session_create(
    app: AppHandle,
    state: State<'_, AppState>,
    profile: String,
    cwd: String,
    cols: u16,
    rows: u16,
    resume: Option<String>,
    worktree: Option<String>,
    on_output: Channel<InvokeResponseBody>,
) -> Result<u32, String> {
    let profile = state.config.profile(&profile).ok_or("알 수 없는 프로필입니다")?;
    let cwd = PathBuf::from(cwd);
    if !cwd.is_dir() {
        return Err(format!("폴더가 없습니다: {}", cwd.display()));
    }

    let id = state.next_id.fetch_add(1, Ordering::Relaxed);
    let mut args = profile.args.clone();
    let mut env: Vec<(String, String)> = vec![
        ("TERM_PROGRAM".into(), "cypher".into()),
        ("COLORTERM".into(), "truecolor".into()),
    ];
    if profile.claude {
        if let Some(session) = resume {
            if !is_session_id(&session) {
                return Err("이어갈 세션 번호가 올바르지 않습니다".into());
            }
            // 이전 대화를 그대로 잇는다. `--continue`와 겹치지 않게 한다.
            args.retain(|a| a != "--continue" && a != "-c");
            args.push("--resume".into());
            args.push(session);
        } else if let Some(name) = worktree {
            if !is_worktree_name(&name) {
                return Err("워크트리 이름은 영문·숫자·`.`·`_`·`-`만 쓸 수 있습니다".into());
            }
            args.push("--worktree".into());
            args.push(name);
        }
        env.push(("FORCE_HYPERLINK".into(), "1".into()));
        if let Some(mcp) = &state.mcp_config {
            // 내장 브라우저 조작 도구를 이 세션에 연결한다.
            args.push("--mcp-config".into());
            args.push(mcp.to_string_lossy().into_owned());
        }
        if let Some(settings) = &state.claude_settings {
            args.push("--settings".into());
            args.push(settings.to_string_lossy().into_owned());
            env.push(("CYPHER_PORT".into(), state.status_port.to_string()));
            env.push(("CYPHER_TOKEN".into(), state.token.clone()));
            env.push(("CYPHER_SESSION".into(), id.to_string()));
        }
    } else if wants_powershell_hook(&profile.program, &args) {
        args.extend(["-NoExit".into(), "-Command".into(), POWERSHELL_CWD_HOOK.into()]);
    }

    let options = SpawnOptions {
        program: profile.program.clone(),
        args,
        cwd,
        env,
        // Cypher 자체가 Claude Code 안에서 실행됐을 때 그 흔적이 새 세션에 넘어가지 않게 한다.
        env_remove: vec![
            "CLAUDECODE".into(),
            "CLAUDE_CODE_ENTRYPOINT".into(),
            "CLAUDE_CODE_CHILD_SESSION".into(), // 남아 있으면 새 세션의 대화 기록 저장이 꺼진다.
        ],
        cols,
        rows,
    };

    // 등록이 끝나기 전에 종료 알림이 먼저 지우려 들지 않도록 잠근 채로 띄운다.
    let mut sessions = state.sessions.lock().unwrap();
    let exit_app = app.clone();
    let session = PtySession::spawn(
        options,
        move |bytes| {
            let _ = on_output.send(InvokeResponseBody::Raw(bytes.to_vec()));
        },
        move |code| {
            exit_app.state::<AppState>().sessions.lock().unwrap().remove(&id);
            let _ = exit_app.emit("session-exit", ExitPayload { id, code });
        },
    )
    .map_err(|e| format!("{e:#}"))?;
    sessions.insert(id, Arc::new(session));
    Ok(id)
}

fn session(state: &State<'_, AppState>, id: u32) -> Result<Arc<PtySession>, String> {
    state.sessions.lock().unwrap().get(&id).cloned().ok_or_else(|| "끝난 세션입니다".to_string())
}

#[tauri::command]
pub fn session_write(state: State<'_, AppState>, id: u32, data: String) -> Result<(), String> {
    session(&state, id)?.write(data.as_bytes()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn session_resize(state: State<'_, AppState>, id: u32, cols: u16, rows: u16) -> Result<(), String> {
    session(&state, id)?.resize(cols, rows).map_err(|e| e.to_string())
}

/// 화면이 출력을 못 따라갈 때 읽기를 잠시 멈춘다.
#[tauri::command]
pub fn session_pause(state: State<'_, AppState>, id: u32, paused: bool) {
    if let Ok(s) = session(&state, id) {
        s.set_paused(paused);
    }
}

#[tauri::command]
pub fn session_close(state: State<'_, AppState>, id: u32) {
    if let Ok(s) = session(&state, id) {
        s.kill();
    }
}

/// 한 줄에서 링크를 찾는다. 느린 드라이브에서 창이 멈추지 않도록 작업 스레드에서 돈다.
#[tauri::command]
pub async fn link_detect(text: String, cwd: String) -> Vec<LinkSpan> {
    links::detect(&text, Path::new(&cwd))
}

/// PATH에서 프로그램을 찾는다. 확장자가 없으면 PATHEXT 순서로 붙여 본다(`code` → `code.cmd`).
fn find_program(name: &str) -> Option<PathBuf> {
    let given = Path::new(name);
    if given.is_absolute() {
        return given.is_file().then(|| given.to_path_buf());
    }
    let exts: Vec<String> = if given.extension().is_some() {
        vec![String::new()]
    } else {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
            .map(str::to_string)
            .collect()
    };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .flat_map(|dir| exts.iter().map(move |ext| dir.join(format!("{name}{ext}"))))
        .find(|candidate| candidate.is_file())
}

fn reveal(file: &Path) -> std::io::Result<()> {
    Command::new("explorer.exe").raw_arg(format!("/select,\"{}\"", file.display())).spawn().map(drop)
}

/// 파일을 "열기": 편집기 설정이 있으면 그 편집기로(줄·칸 포함), 없으면 연결 프로그램으로.
fn open_file(config: &Config, file: &Path, line: Option<u32>, col: Option<u32>) -> std::io::Result<()> {
    if let Some((program, args)) = links::editor_command(&config.link.editor, file, line, col) {
        let program = find_program(&program).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, format!("편집기를 찾지 못했습니다: {program}"))
        })?;
        return Command::new(program).args(args).creation_flags(CREATE_NO_WINDOW).spawn().map(drop);
    }
    open::that_detached(file)
}

/// 링크를 연다. 웹 주소는 브라우저, 폴더는 탐색기, 파일은 탐색기에서 선택해 보여 준다.
/// `open_file`이 참이면 파일을 열되, 실행될 수 있는 파일은 보여 주기만 한다.
#[tauri::command]
pub async fn link_open(
    app: AppHandle,
    target: String,
    cwd: String,
    open_file: bool,
    line: Option<u32>,
    col: Option<u32>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let result = match links::classify(&target, Path::new(&cwd)) {
        None => return Err(format!("열 수 없는 대상입니다: {target}")),
        Some(Target::Url(url)) => open::that_detached(url),
        Some(Target::Dir(dir)) => Command::new("explorer.exe").arg(dir).spawn().map(drop),
        Some(Target::File(file)) if open_file && !links::is_launch_blocked(&file) => {
            self::open_file(&state.config, &file, line, col)
        }
        Some(Target::File(file)) => reveal(&file),
    };
    result.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 세션_번호와_워크트리_이름_검사() {
        assert!(is_session_id("550e8400-e29b-41d4-a716-446655440000"));
        assert!(!is_session_id("--dangerously-skip-permissions"));
        assert!(is_worktree_name("feature-login_2"));
        assert!(!is_worktree_name("-rf"));
        assert!(!is_worktree_name("a b"));
        assert!(!is_worktree_name("../밖"));
        assert!(!is_worktree_name(""));
    }

    #[test]
    fn 파워셸_현재_폴더_알림은_인자_없는_셸에만() {
        let none: Vec<String> = vec!["-NoLogo".into()];
        assert!(wants_powershell_hook("powershell.exe", &none));
        assert!(wants_powershell_hook(r"C:\Program Files\PowerShell\7\pwsh.exe", &none));
        assert!(!wants_powershell_hook("powershell.exe", &["-File".into(), "a.ps1".into()]));
        assert!(!wants_powershell_hook("cmd.exe", &none));
        assert!(!POWERSHELL_CWD_HOOK.contains('"'));
    }

    #[test]
    fn 확장자_없는_프로그램을_찾는다() {
        // cmd는 어느 Windows에나 있다.
        assert!(find_program("cmd").is_some_and(|p| p.is_file()));
        assert!(find_program("cypher-없는-프로그램").is_none());
    }
}
