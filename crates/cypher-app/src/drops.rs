//! 끌어다 놓기: 본창에 놓인 파일의 경로를 알아내고, 어떤 항목으로 만들지 가려 준다.
//!
//! 본창은 글자·링크·그림까지 받으려고 화면(웹뷰) 쪽의 끌어다 놓기를 쓴다. 그 방식에서는 놓인 파일의
//! 내용만 보이고 경로는 보이지 않으므로, 화면이 파일 객체를 웹뷰 메시지에 실어 보내면 여기서 경로를 읽어
//! `drop-paths` 사건으로 돌려준다(터미널에 경로를 넣거나, 메모를 그 파일에 연결하려면 경로가 필요하다).

use serde::Serialize;
use std::fs;
use std::io::Read;
use std::path::Path;
use tauri::{AppHandle, Emitter, Manager};

/// 메모로 열 수 있는 글자 파일 크기(메모의 한도와 같다).
const MAX_TEXT_BYTES: u64 = 8 << 20;
/// 화면이 파일 객체와 함께 보내는 메시지의 머리말. 뒤에 요청 번호가 붙는다.
const MESSAGE_PREFIX: &str = "cypher-drop:";

/// 본창 웹뷰가 보내는 "놓인 파일" 메시지를 받기 시작한다. 본창을 만든 뒤 한 번 부른다.
pub fn watch(app: &AppHandle) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2File, ICoreWebView2WebMessageReceivedEventArgs2};
    use webview2_com::{take_pwstr, WebMessageReceivedEventHandler};
    use windows_core::{Interface, PWSTR};

    let Some(main) = app.get_webview("main") else { return };
    let app = app.clone();
    let attached = main.with_webview(move |platform| {
        let handler = WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            // 우리 메시지는 `cypher-drop:<번호>` 글자다. 글자가 아닌 메시지는 앞선 처리기(앱 명령용)가 오류로
            // 끝내면서 여기까지 오지 않으므로 글자로 보낸다. 그 밖의 글자 메시지는 우리 것이 아니다.
            let mut text = PWSTR::null();
            // SAFETY: 살아 있는 사건 인자에서 값을 읽는다. 받은 문자열은 take_pwstr가 풀어 준다.
            if unsafe { args.TryGetWebMessageAsString(&mut text) }.is_err() {
                return Ok(());
            }
            let text = take_pwstr(text);
            let Some(id) = text.strip_prefix(MESSAGE_PREFIX).and_then(|id| id.parse::<u64>().ok()) else { return Ok(()) };

            // 메시지에 딸려 온 파일 객체에서 경로를 읽는다. 경로가 없는 것(다른 프로그램이 내용만 준 파일)은 빈 글.
            let mut paths: Vec<String> = Vec::new();
            // SAFETY: 살아 있는 사건 인자와 그 안의 객체 목록을 읽는다.
            let objects = args.cast::<ICoreWebView2WebMessageReceivedEventArgs2>().and_then(|more| unsafe { more.AdditionalObjects() });
            if let Ok(objects) = objects {
                let mut count = 0u32;
                // SAFETY: 위와 같다.
                unsafe { objects.Count(&mut count)? };
                for index in 0..count {
                    // SAFETY: 위와 같다. 받은 문자열은 take_pwstr가 풀어 준다.
                    let path = unsafe { objects.GetValueAtIndex(index) }
                        .and_then(|object| object.cast::<ICoreWebView2File>())
                        .and_then(|file| {
                            let mut path = PWSTR::null();
                            unsafe { file.Path(&mut path)? };
                            Ok(take_pwstr(path))
                        })
                        .unwrap_or_default();
                    paths.push(path);
                }
            }
            let _ = app.emit_to("main", "drop-paths", serde_json::json!({ "id": id, "paths": paths }));
            Ok(())
        }));
        let mut token = 0i64;
        // SAFETY: 창 스레드에서 살아 있는 웹뷰에 처리기를 단다.
        let added = unsafe { platform.controller().CoreWebView2().and_then(|core| core.add_WebMessageReceived(&handler, &mut token)) };
        if let Err(problem) = added {
            crate::log(&format!("끌어다 놓기 준비 실패: {}", problem.message()));
        }
    });
    if let Err(problem) = attached {
        crate::log(&format!("끌어다 놓기 준비 실패: {problem}"));
    }
}

/// 놓인 파일 하나를 어떤 항목으로 만들지.
#[derive(Serialize, Debug, PartialEq)]
pub struct Dropped {
    path: String,
    /// 파일 이름(확장자 포함).
    name: String,
    /// "image" | "text" | "table" | "http" | "link" | "folder" | "other"
    kind: &'static str,
    /// 인터넷 바로 가기(.url)일 때 그 주소.
    url: Option<String>,
}

/// 놓인 파일들이 각각 무엇인지 가린다(내용의 앞부분과 확장자로).
#[tauri::command]
pub async fn drop_classify(paths: Vec<String>) -> Vec<Dropped> {
    paths.iter().map(|path| classify(Path::new(path))).collect()
}

fn classify(path: &Path) -> Dropped {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let done = |kind: &'static str, url: Option<String>| Dropped { path: path.to_string_lossy().into_owned(), name: name.clone(), kind, url };
    let Ok(meta) = fs::metadata(path) else { return done("other", None) };
    if meta.is_dir() {
        return done("folder", None);
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let mut head = Vec::new();
    if fs::File::open(path).and_then(|file| file.take(4096).read_to_end(&mut head)).is_err() {
        return done("other", None);
    }
    if crate::image::sniff(&head).is_some() || crate::image::format_of_extension(&ext).is_some() {
        return done("image", None);
    }
    if ext == "url" {
        // 인터넷 바로 가기: `URL=주소` 줄을 찾는다.
        let text = String::from_utf8_lossy(&head);
        let url = text.lines().find_map(|line| line.trim().strip_prefix("URL=")).map(|url| url.trim().to_string());
        return match url {
            Some(url) if url.starts_with("http://") || url.starts_with("https://") => done("link", Some(url)),
            _ => done("other", None),
        };
    }
    if ext == "http" || ext == "rest" {
        return done("http", None);
    }
    // 글자 파일인지: UTF-16(BOM 있음)이 아닌데 0 바이트가 보이면 글자 파일이 아니다.
    let utf16 = head.starts_with(&[0xff, 0xfe]) || head.starts_with(&[0xfe, 0xff]);
    if meta.len() > MAX_TEXT_BYTES || (!utf16 && head.contains(&0)) {
        return done("other", None);
    }
    // 값을 구분자로 나눈 글은 표 항목으로 연다(JSON은 메모로 연다. 표로 보려면 "+ 표"에서 고른다).
    if ext == "csv" || ext == "tsv" {
        return done("table", None);
    }
    done("text", None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 놓인_파일의_종류를_가린다() {
        let dir = std::env::temp_dir().join(format!("cypher-drop-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sub")).unwrap();
        let file = |name: &str, bytes: &[u8]| {
            let path = dir.join(name);
            fs::write(&path, bytes).unwrap();
            path
        };
        let kind = |path: &Path| classify(path).kind;

        assert_eq!(kind(&file("shot.png", b"\x89PNG\r\n\x1a\n....")), "image");
        // 확장자가 틀려도 내용으로 알아본다.
        assert_eq!(kind(&file("really-a-jpeg.txt", &[0xff, 0xd8, 0xff, 0xe0, 0, 16])), "image");
        assert_eq!(kind(&file("notes.md", "# 할 일\n".as_bytes())), "text");
        assert_eq!(kind(&file("no-extension", b"plain")), "text");
        assert_eq!(kind(&file("utf16.txt", &[0xff, 0xfe, b'a', 0, b'b', 0])), "text");
        assert_eq!(kind(&file("api.http", b"GET https://example.com\n")), "http");
        assert_eq!(kind(&file("결과.csv", "이름,값\n가,1\n".as_bytes())), "table");
        assert_eq!(kind(&file("rows.TSV", b"a\tb\n")), "table");
        assert_eq!(kind(&file("data.json", b"[1]")), "text");
        // 글자가 아닌 것은 확장자가 csv여도 표가 아니다.
        assert_eq!(kind(&file("binary.csv", &[1, 0, 2, 0])), "other");
        assert_eq!(kind(&file("tool.exe", &[b'M', b'Z', 0, 0, 3, 0])), "other");
        assert_eq!(kind(&dir.join("sub")), "folder");
        assert_eq!(kind(&dir.join("없는 파일")), "other");

        let link = classify(&file("site.url", b"[InternetShortcut]\r\nURL=https://example.com/a?b=1\r\n"));
        assert_eq!((link.kind, link.url.as_deref()), ("link", Some("https://example.com/a?b=1")));
        assert_eq!(kind(&file("local.url", b"[InternetShortcut]\r\nURL=file:///C:/x\r\n")), "other");
        let _ = fs::remove_dir_all(&dir);
    }
}
