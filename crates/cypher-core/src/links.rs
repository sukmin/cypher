//! 터미널 한 줄에서 웹 주소와 파일 경로를 찾아 실제 대상으로 해석한다.
//!
//! 터미널 출력은 신뢰할 수 없는 입력이다. 그래서
//! - 웹 주소는 http/https만 인정하고,
//! - 경로는 실제로 존재할 때만 링크로 만들며,
//! - 네트워크 경로(`\\서버\…`)는 존재 확인조차 하지 않는다(확인만으로 인증 정보가 나갈 수 있다).

use regex::Regex;
use serde::Serialize;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkKind {
    Url,
    Dir,
    File,
}

/// 한 줄 안의 링크 하나. `start`/`end`는 UTF-16 단위 위치(끝은 제외)라서
/// 화면 쪽 JS 문자열 색인과 그대로 맞는다.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LinkSpan {
    pub start: usize,
    pub end: usize,
    pub kind: LinkKind,
    /// 웹 주소이거나 정리된 절대경로.
    pub target: String,
    pub line: Option<u32>,
    pub col: Option<u32>,
}

/// 클릭 시 실제로 열 대상.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Url(String),
    Dir(PathBuf),
    File(PathBuf),
}

static TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?x)
          (?P<url>https?://[^\s<>"'`\x{2500}-\x{257F}]+)
        | (?P<furl>file:///[^\s<>"'`\x{2500}-\x{257F}]+)
        | "(?P<qpath>[A-Za-z]:[\\/][^"<>|*?\r\n]+)"
        | (?P<path>(?:[A-Za-z]:[\\/])?[^\s"'<>|*?:()\[\]{},;`=\x{2500}-\x{257F}]+)
          (?::(?P<line>\d+)(?::(?P<col>\d+))?)?
        "#,
    )
    .unwrap()
});
static EXT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^.\\/]\.[A-Za-z0-9_]{1,10}$").unwrap());
static GITBASH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^/([A-Za-z])(?:/(.*))?$").unwrap());

/// 한 줄에서 링크를 모두 찾는다. 상대경로는 `cwd` 기준으로 해석한다.
pub fn detect(text: &str, cwd: &Path) -> Vec<LinkSpan> {
    let mut out = Vec::new();
    let mut push = |start: usize, end: usize, kind, target: String, line, col| {
        out.push(LinkSpan {
            start: utf16_len(&text[..start]),
            end: utf16_len(&text[..end]),
            kind,
            target,
            line,
            col,
        });
    };

    for caps in TOKEN.captures_iter(text) {
        if let Some(m) = caps.name("url") {
            let s = trim_url(m.as_str());
            push(m.start(), m.start() + s.len(), LinkKind::Url, s.to_string(), None, None);
        } else if let Some(m) = caps.name("furl") {
            let s = trim_url(m.as_str());
            if let Some((kind, p)) = file_url_to_path(s).and_then(|p| stat(&p)) {
                push(m.start(), m.start() + s.len(), kind, path_string(&p), None, None);
            }
        } else if let Some(m) = caps.name("qpath") {
            if let Some((kind, p)) = resolve(m.as_str(), cwd) {
                push(m.start(), m.end(), kind, path_string(&p), None, None);
            }
        } else if let Some(m) = caps.name("path") {
            let raw = m.as_str();
            let whole_end = caps.get(0).unwrap().end();
            // 끝의 마침표는 항상 뗀다(Windows는 이름 끝의 마침표를 무시해서 그대로 두면
            // 문장 끝 마침표까지 경로에 포함된다). 그다음 앞의 '@'를 뗀 모양도 시도한다.
            let trimmed = raw.trim_end_matches('.');
            let mut variants = vec![(0, trimmed.len())];
            if raw.starts_with('@') {
                variants.push((1, trimmed.len()));
            }
            // 한국어 문장에서는 경로 뒤에 조사가 붙는다("src/main.rs를 고쳤습니다"). 끝에 붙은 한글 등(ASCII가 아닌 글자)을
            // 뗀 모양도 시도한다. 있는 파일인지 확인하므로 틀리게 잡힐 일은 없고, 온전한 모양을 먼저 본다.
            let bare = trimmed.trim_end_matches(|c: char| !c.is_ascii()).trim_end_matches('.');
            if bare.len() < trimmed.len() {
                variants.push((0, bare.len()));
                if raw.starts_with('@') {
                    variants.push((1, bare.len()));
                }
            }
            for (from, to) in variants {
                if from >= to {
                    continue;
                }
                let cand = &raw[from..to];
                if !is_candidate(cand) {
                    continue;
                }
                if let Some((kind, p)) = resolve(cand, cwd) {
                    // 줄:칸 접미사는 경로가 원문 끝까지 쓰였을 때만 붙는다.
                    let (end, line, col) = if to == raw.len() {
                        (
                            whole_end,
                            caps.name("line").and_then(|v| v.as_str().parse().ok()),
                            caps.name("col").and_then(|v| v.as_str().parse().ok()),
                        )
                    } else {
                        (m.start() + to, None, None)
                    };
                    push(m.start() + from, end, kind, path_string(&p), line, col);
                    break;
                }
            }
        }
    }
    out
}

/// 클릭된 대상(웹 주소, file:// 주소, 경로)을 다시 검증해 분류한다.
/// 화면에서 넘어온 값도 믿지 않고 여기서 한 번 더 확인한다.
pub fn classify(target: &str, cwd: &Path) -> Option<Target> {
    let t = target.trim();
    if t.chars().any(|c| c.is_control()) {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return (!t.contains(char::is_whitespace)).then(|| Target::Url(t.to_string()));
    }
    let found = if lower.starts_with("file://") {
        file_url_to_path(t).and_then(|p| stat(&p))
    } else {
        resolve(t, cwd)
    };
    found.map(|(kind, p)| match kind {
        LinkKind::Dir => Target::Dir(p),
        _ => Target::File(p),
    })
}

/// 연결 프로그램으로 바로 열면 코드가 실행될 수 있는 파일인지.
/// 이런 파일은 열지 않고 탐색기에서 보여 주기만 한다.
pub fn is_launch_blocked(path: &Path) -> bool {
    const BLOCKED: &[&str] = &[
        "exe", "com", "scr", "pif", "bat", "cmd", "ps1", "psm1", "vbs", "vbe", "js", "jse", "wsf",
        "wsh", "msi", "msp", "lnk", "url", "hta", "cpl", "reg", "jar", "py", "pyw", "appref-ms",
        "msc", "dll", "sh",
    ];
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => BLOCKED.contains(&ext.to_ascii_lowercase().as_str()),
        None => false,
    }
}

/// 편집기 명령 틀을 실제 명령으로 바꾼다.
/// `code -g {file}:{line}:{col}` → `("code", ["-g", "C:\a.rs:42:7"])`
/// 프로그램 경로에 공백이 있으면 큰따옴표로 감싼다. 줄·칸을 모르면 1로 채운다.
pub fn editor_command(
    template: &str,
    file: &Path,
    line: Option<u32>,
    col: Option<u32>,
) -> Option<(String, Vec<String>)> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for ch in template.chars() {
        match ch {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    let fill = |s: &str| {
        s.replace("{file}", &file.to_string_lossy())
            .replace("{line}", &line.unwrap_or(1).to_string())
            .replace("{col}", &col.unwrap_or(1).to_string())
    };
    let mut parts = parts.into_iter();
    let program = parts.next()?; // 프로그램 이름에는 치환을 하지 않는다.
    Some((program, parts.map(|p| fill(&p)).collect()))
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// 문장 끝 구두점과 짝이 맞지 않는 닫는 괄호를 웹 주소 끝에서 떼어 낸다.
fn trim_url(mut s: &str) -> &str {
    loop {
        let before = s.len();
        s = s.trim_end_matches(['.', ',', ';', ':', '!', '?', '\'', '"']);
        s = trim_particle(s);
        for (open, close) in [('(', ')'), ('[', ']')] {
            if s.ends_with(close) && s.matches(open).count() < s.matches(close).count() {
                s = &s[..s.len() - 1];
            }
        }
        if s.len() == before {
            return s;
        }
    }
}

/// 주소 끝에 붙은 조사를 뗀다("https://example.com/docs에서 확인"): 영문·숫자 바로 뒤에 이어진 한글은 주소가 아니라
/// 문장의 일부로 본다. `/`·`=`·`#` 뒤의 한글(`…/wiki/서울`, `?q=서울`)은 주소의 일부이므로 그대로 둔다.
fn trim_particle(s: &str) -> &str {
    let stem = s.trim_end_matches(|c: char| ('\u{AC00}'..='\u{D7A3}').contains(&c));
    if stem.len() < s.len() && stem.ends_with(|c: char| c.is_ascii_alphanumeric()) {
        stem
    } else {
        s
    }
}

/// 존재 확인을 해 볼 만한 모양인지(구분자가 있거나 확장자가 있는 낱말).
fn is_candidate(s: &str) -> bool {
    if s.len() < 2 || is_unc(s) {
        return false;
    }
    s.contains('/') || s.contains('\\') || s == "~" || EXT.is_match(s)
}

fn is_unc(s: &str) -> bool {
    s.starts_with("\\\\") || s.starts_with("//")
}

/// 경로 글자를 실제 경로로 바꾸고 존재를 확인한다.
fn resolve(raw: &str, cwd: &Path) -> Option<(LinkKind, PathBuf)> {
    if raw.is_empty() || is_unc(raw) {
        return None;
    }
    let mut candidates = Vec::new();
    if raw == "~" || raw.starts_with("~/") || raw.starts_with("~\\") {
        let home = PathBuf::from(std::env::var_os("USERPROFILE")?);
        candidates.push(home.join(raw[1..].trim_start_matches(['/', '\\'])));
    } else {
        // Git Bash식 `/c/Users/…`는 드라이브 경로로 먼저 해석해 본다.
        if let Some(c) = GITBASH.captures(raw) {
            let rest = c.get(2).map_or("", |m| m.as_str()).replace('/', "\\");
            candidates.push(PathBuf::from(format!("{}:\\{}", c[1].to_ascii_uppercase(), rest)));
        }
        candidates.push(cwd.join(raw.replace('/', "\\")));
    }
    candidates.into_iter().find_map(|p| stat(&normalize(&p)))
}

fn stat(p: &Path) -> Option<(LinkKind, PathBuf)> {
    if is_unc(&p.to_string_lossy()) {
        return None;
    }
    let meta = std::fs::metadata(p).ok()?;
    let kind = if meta.is_dir() { LinkKind::Dir } else { LinkKind::File };
    Some((kind, p.to_path_buf()))
}

/// `.`과 `..`을 정리한다. (canonicalize는 `\\?\` 접두사를 붙여서 쓰지 않는다.)
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn path_string(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// `file:///C:/a%20b.txt` → `C:\a b.txt`. 다른 컴퓨터를 가리키는 주소는 받지 않는다.
fn file_url_to_path(url: &str) -> Option<PathBuf> {
    let rest = url.get(..8).filter(|p| p.eq_ignore_ascii_case("file:///")).map(|_| &url[8..])?;
    let decoded = percent_decode(rest)?;
    let b = decoded.as_bytes();
    if b.len() < 3 || !b[0].is_ascii_alphabetic() || b[1] != b':' {
        return None;
    }
    Some(normalize(Path::new(&decoded.replace('/', "\\"))))
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// 시험용 폴더: `<임시>/cypher-links-<이름>/src/main.rs`, `메모 장.txt`
    fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("cypher-links-{name}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src").join("main.rs"), "fn main() {}").unwrap();
        fs::write(root.join("메모 장.txt"), "x").unwrap();
        root
    }

    fn slice16(text: &str, span: &LinkSpan) -> String {
        let units: Vec<u16> = text.encode_utf16().collect();
        String::from_utf16(&units[span.start..span.end]).unwrap()
    }

    #[test]
    fn 웹_주소는_끝_구두점을_뗀다() {
        let cwd = fixture("url");
        let text = "문서는 https://example.com/a(b)/c?x=1. 참고 (https://example.org/x).";
        let spans = detect(text, &cwd);
        let urls: Vec<_> = spans.iter().filter(|s| s.kind == LinkKind::Url).collect();
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0].target, "https://example.com/a(b)/c?x=1");
        assert_eq!(urls[1].target, "https://example.org/x");
        // 한글이 앞에 있어도 UTF-16 위치가 맞아야 한다.
        assert_eq!(slice16(text, urls[0]), "https://example.com/a(b)/c?x=1");
    }

    #[test]
    fn 조사가_붙은_주소와_경로() {
        let cwd = fixture("particle");
        // 주소: 영문·숫자 바로 뒤의 한글은 뗀다. 구두점과 섞여 있어도 같다.
        let text = "https://example.com/docs에서 확인하고, (https://example.org/a.html을) 봅니다. https://example.net입니다.";
        let urls: Vec<_> = detect(text, &cwd).into_iter().filter(|s| s.kind == LinkKind::Url).collect();
        assert_eq!(urls.iter().map(|s| s.target.as_str()).collect::<Vec<_>>(), ["https://example.com/docs", "https://example.org/a.html", "https://example.net"]);
        assert_eq!(slice16(text, &urls[0]), "https://example.com/docs");
        // 주소의 일부인 한글은 그대로 둔다.
        for whole in ["https://ko.example.org/wiki/서울", "https://example.com/search?q=서울", "https://example.com/문서#머리말"] {
            let spans = detect(&format!("참고: {whole} 끝"), &cwd);
            assert_eq!(spans[0].target, whole);
        }
        // 경로: 온전한 모양이 없는 파일이면 끝의 한글을 뗀 모양을 본다.
        for text in ["src/main.rs를 고쳤습니다", "고친 파일은 src/main.rs입니다.", "@src/main.rs에 있습니다"] {
            let spans = detect(text, &cwd);
            assert_eq!(spans.len(), 1, "{text}");
            assert_eq!(spans[0].target, path_string(&cwd.join("src").join("main.rs")), "{text}");
            assert_eq!(slice16(text, &spans[0]), "src/main.rs", "{text}");
        }
        // 한글로 끝나는 이름의 파일은 그대로 잡힌다(온전한 모양이 먼저다).
        fs::write(cwd.join("src").join("계획.md"), "x").unwrap();
        let spans = detect("src/계획.md 그리고 src/계획.md를", &cwd);
        assert_eq!(spans.len(), 2);
        assert!(spans.iter().all(|s| s.target == path_string(&cwd.join("src").join("계획.md"))));
    }

    #[test]
    fn 상대경로와_줄_번호() {
        let cwd = fixture("rel");
        let text = "오류: src/main.rs:42:7 에서 발생";
        let spans = detect(text, &cwd);
        assert_eq!(spans.len(), 1);
        let s = &spans[0];
        assert_eq!(s.kind, LinkKind::File);
        assert_eq!(s.target, path_string(&cwd.join("src").join("main.rs")));
        assert_eq!((s.line, s.col), (Some(42), Some(7)));
        assert_eq!(slice16(text, s), "src/main.rs:42:7");
    }

    #[test]
    fn 절대경로_폴더와_문장_끝_마침표() {
        let cwd = fixture("abs");
        let dir = cwd.join("src");
        let text = format!("폴더는 {}.", dir.display());
        let spans = detect(&text, Path::new("C:\\"));
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].kind, LinkKind::Dir);
        assert_eq!(spans[0].target, path_string(&dir));
    }

    #[test]
    fn 공백이_든_경로는_따옴표로_감싼_경우만() {
        let cwd = fixture("quoted");
        let file = cwd.join("메모 장.txt");
        let quoted = format!("열기 \"{}\" 끝", file.display());
        let spans = detect(&quoted, &cwd);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].target, path_string(&file));

        let bare = format!("열기 {} 끝", file.display());
        assert!(detect(&bare, &cwd).iter().all(|s| s.target != path_string(&file)));
    }

    #[test]
    fn 없는_경로와_네트워크_경로는_링크가_아니다() {
        let cwd = fixture("none");
        assert!(detect("없는/파일.rs 그리고 버전 1.99.1", &cwd).is_empty());
        assert!(detect(r"\\server\share\file.txt", &cwd).is_empty());
        assert_eq!(classify(r"\\server\share\file.txt", &cwd), None);
        assert_eq!(classify("file://server/share/file.txt", &cwd), None);
    }

    #[test]
    fn 골뱅이_접두사와_file_주소() {
        let cwd = fixture("at");
        let spans = detect("@src/main.rs 를 보세요", &cwd);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].start, 1);

        let url = format!("file:///{}", cwd.join("src").join("main.rs").display()).replace('\\', "/");
        assert_eq!(
            classify(&url, Path::new("C:\\")),
            Some(Target::File(cwd.join("src").join("main.rs")))
        );
    }

    #[test]
    fn 편집기_명령_틀을_채운다() {
        let file = Path::new(r"C:\ws\a b.rs");
        assert_eq!(
            editor_command("code -g {file}:{line}:{col}", file, Some(42), None),
            Some(("code".into(), vec!["-g".into(), r"C:\ws\a b.rs:42:1".into()]))
        );
        assert_eq!(
            editor_command(r#""C:\Program Files\ed\ed.exe" {file}"#, file, None, None),
            Some((r"C:\Program Files\ed\ed.exe".into(), vec![r"C:\ws\a b.rs".into()]))
        );
        assert_eq!(editor_command("   ", file, None, None), None);
    }

    #[test]
    fn 분류는_허용된_종류만_통과시킨다() {
        let cwd = fixture("classify");
        assert_eq!(
            classify("https://example.com", &cwd),
            Some(Target::Url("https://example.com".into()))
        );
        assert_eq!(classify("javascript:alert(1)", &cwd), None);
        assert_eq!(classify("src", &cwd), Some(Target::Dir(cwd.join("src"))));
        assert!(is_launch_blocked(Path::new("a\\b.PS1")));
        assert!(!is_launch_blocked(Path::new("a\\b.png")));
    }
}
