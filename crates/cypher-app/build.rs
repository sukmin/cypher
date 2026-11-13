//! 빌드 준비: 앱 명령을 권한 목록에 올리고, 창별 권한 파일을 만든다.
//!
//! 명령을 권한 목록에 올려 두면 권한을 받은 화면만 그 명령을 부를 수 있다.
//! 사이트 항목이 여는 웹 페이지에는 아무 권한도 주지 않으므로, 웹 페이지가
//! 터미널 입력 같은 앱 명령을 부를 수 없다.

use std::path::Path;

/// 본창(세션 화면)이 쓰는 명령.
const MAIN: &[&str] = &[
    "app_config", "open_config", "open_claude_file", "home_dir", "autostart", "notify", "pick_folder",
    "session_create", "session_write", "session_resize", "session_pause", "session_close",
    "link_detect", "link_open",
    "git_changes", "git_diff", "save_text",
    "memory_info", "memory_export", "memory_import", "memory_open",
    "memo_read", "memo_write", "memo_remove", "text_open", "text_save",
    "site_open", "site_bounds", "site_navigate", "site_action", "site_close", "site_capture",
    "image_import", "image_pick", "image_paste", "image_store", "image_fetch", "image_read", "image_info",
    "image_remove", "image_export", "image_copy", "drop_classify",
    "clipboard_read", "paste_preview", "clipboard_write",
    "browser_open", "browser_where", "browser_capture", "browser_engine_info", "browser_latest_version",
    "mcp_reply", "mindmap_read", "mindmap_write", "mindmap_remove", "mindmap_export", "mindmap_pdf",
    "store_dirs", "ui_zoom", "closed_restore",
    "http_files", "http_read", "http_save", "http_run_at",
];

fn permissions(commands: &[&str]) -> String {
    commands.iter().map(|c| format!("\"allow-{}\"", c.replace('_', "-"))).collect::<Vec<_>>().join(", ")
}

/// 내용이 달라졌을 때만 쓴다(매번 쓰면 빌드가 계속 다시 돈다).
fn write_if_changed(path: &Path, content: &str) {
    if std::fs::read_to_string(path).ok().as_deref() != Some(content) {
        std::fs::write(path, content).expect("권한 파일을 쓰지 못했습니다");
    }
}

fn main() {
    let dir = Path::new("capabilities");
    std::fs::create_dir_all(dir).expect("capabilities 폴더를 만들지 못했습니다");
    write_if_changed(
        &dir.join("default.json"),
        &format!(
            "{{\n  \"identifier\": \"default\",\n  \"description\": \"본창 화면 권한 (build.rs가 만든 파일). 같은 창에 붙는 사이트 웹뷰에는 권한이 없다.\",\n  \"webviews\": [\"main\"],\n  \"permissions\": [\"core:default\", {}]\n}}\n",
            permissions(MAIN)
        ),
    );

    let all: &'static [&'static str] = MAIN;
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(all)),
    )
    .expect("Tauri 빌드 준비에 실패했습니다");
}
