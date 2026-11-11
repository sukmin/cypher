//! 설정 파일(`%APPDATA%\cypher\config.toml`). 없는 항목은 기본값을 쓴다.

use crate::profile::{self, Profile};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub font: Font,
    pub link: Link,
    pub notify: Notify,
    pub session: Session,
    pub browser: Browser,
    pub window: Window,
    /// 파일에서는 `[[profile]]`로 쓴다. 읽은 뒤에는 기본 프로필과 합쳐진 전체 목록이다.
    #[serde(rename(deserialize = "profile"))]
    pub profiles: Vec<Profile>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Font {
    pub family: String,
    pub size: u16,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Link {
    /// 참이면 Ctrl을 누른 채 클릭해야 링크가 열린다.
    pub require_ctrl: bool,
    /// 파일을 "열기"할 때 쓸 편집기 명령. 비어 있으면 연결 프로그램으로 연다.
    /// 예: `code -g {file}:{line}:{col}`
    pub editor: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notify {
    /// 앱 창 안 알림.
    pub in_app: bool,
    /// Windows 알림(창이 뒤에 있을 때).
    pub system: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    /// 다시 시작할 때 이전 세션과 창 배치를 되살린다.
    pub restore: bool,
    /// 세션마다 기억할 지난 출력 줄 수.
    pub scrollback: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font: Font::default(),
            link: Link::default(),
            notify: Notify::default(),
            session: Session::default(),
            browser: Browser::default(),
            window: Window::default(),
            profiles: profile::builtin(),
        }
    }
}

impl Default for Font {
    fn default() -> Self {
        Self {
            family: r#""Cascadia Mono", "D2Coding", Consolas, "Malgun Gothic", monospace"#.into(),
            size: 14,
        }
    }
}

impl Default for Notify {
    fn default() -> Self {
        Self { in_app: true, system: true }
    }
}

impl Default for Session {
    fn default() -> Self {
        Self { restore: true, scrollback: 10_000 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Browser {
    /// Claude 세션에 내장 브라우저 조작 도구를 연결할지.
    pub enabled: bool,
    /// 브라우저를 처음 띄울 때 여는 주소.
    pub home: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Window {
    /// 창을 끌 때 화면 가장자리와 다른 Cypher 창에 달라붙게 할지.
    pub magnet: bool,
    /// 달라붙는 거리(화면 배율 100% 기준 픽셀).
    pub magnet_distance: u32,
}

impl Default for Window {
    fn default() -> Self {
        Self { magnet: true, magnet_distance: 14 }
    }
}

impl Default for Browser {
    fn default() -> Self {
        Self { enabled: true, home: "https://www.google.com".into() }
    }
}

/// 처음 실행할 때 만들어 두는 설정 파일. 모든 항목이 주석이라 기본값 그대로 동작한다.
pub const DEFAULT_TEXT: &str = r#"# Cypher 설정. 고친 뒤 Cypher를 다시 시작하면 적용됩니다.
# 줄 앞의 '#'을 지우면 그 항목이 켜집니다.

[font]
# family = '"Cascadia Mono", "D2Coding", Consolas, "Malgun Gothic", monospace'
# size = 14

[link]
# 참이면 Ctrl을 누른 채 클릭해야 링크가 열립니다(Ctrl+Shift+클릭: 파일 열기).
# require_ctrl = false
# 파일을 열 때 쓸 편집기. 비워 두면 연결 프로그램으로 엽니다.
# editor = "code -g {file}:{line}:{col}"

[notify]
# in_app = true    # 앱 창 안 알림
# system = true    # Windows 알림(창이 뒤에 있을 때)

[session]
# restore = true       # 다시 시작할 때 이전 세션과 배치를 되살림
# scrollback = 10000   # 세션마다 기억할 지난 출력 줄 수

[window]
# magnet = true            # 창을 끌 때 화면 가장자리와 다른 Cypher 창에 달라붙기 (Ctrl을 누른 채 끌면 붙지 않음)
# magnet_distance = 14     # 달라붙는 거리(픽셀). 작을수록 잘 떨어집니다

[browser]
# enabled = true                      # Claude 세션에 내장 브라우저 조작 도구를 연결
# home = "https://www.google.com"     # 브라우저를 처음 띄울 때 여는 주소

# 프로필 추가 예시. 기본 프로필(claude, powershell)과 id가 같으면 바꿔 씁니다.
# [[profile]]
# id = "gitbash"
# label = "Git Bash"
# program = "C:\\Program Files\\Git\\bin\\bash.exe"
# args = ["--login", "-i"]
# claude = false
"#;

impl Config {
    pub fn parse(text: &str) -> Result<Config, String> {
        let mut config: Config = toml::from_str(text).map_err(|e| e.to_string())?;
        // 파일에 [[profile]]이 없으면 기본 목록이 그대로 들어와 있다. 합쳐도 결과는 같다.
        config.profiles = profile::merge(std::mem::take(&mut config.profiles));
        config.font.size = config.font.size.clamp(6, 72);
        Ok(config)
    }

    /// 설정 파일을 읽는다.
    ///
    /// 사용자의 설정 파일은 버전이 바뀌어도 그대로 쓴다. 이 함수는 파일이 **없을 때만** 기본 파일을
    /// 만들고, 이미 있는 파일은 읽지 못하거나 내용이 잘못됐더라도 절대 고치거나 덮어쓰지 않는다
    /// (그때는 기본값으로 동작하고 오류 설명을 돌려준다). 모르는 항목은 무시하고 빠진 항목은
    /// 기본값을 쓰므로, 예전 버전이나 새 버전에서 만든 파일도 그대로 읽힌다.
    ///
    /// 새 버전에서 늘어난 항목을 참고할 수 있도록, 같은 폴더의 `config.example.toml`은
    /// 실행할 때마다 이 버전의 예시로 맞춰 둔다.
    pub fn load(path: &Path) -> (Config, Option<String>) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
            let example = dir.join("config.example.toml");
            if std::fs::read_to_string(&example).ok().as_deref() != Some(DEFAULT_TEXT) {
                let _ = std::fs::write(example, DEFAULT_TEXT);
            }
        }
        if !path.exists() {
            let _ = std::fs::write(path, DEFAULT_TEXT);
            return (Config::default(), None);
        }
        match std::fs::read(path) {
            Ok(bytes) => {
                // 메모장이 붙인 BOM이나 다른 인코딩의 주석이 있어도 읽을 수 있는 만큼 읽는다.
                let text = String::from_utf8_lossy(&bytes);
                match Config::parse(text.strip_prefix('\u{feff}').unwrap_or(&text)) {
                    Ok(config) => (config, None),
                    Err(err) => (Config::default(), Some(err)),
                }
            }
            Err(err) => (Config::default(), Some(format!("설정 파일을 읽지 못했습니다: {err}"))),
        }
    }

    pub fn profile(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 기본_설정_파일은_기본값과_같다() {
        assert_eq!(Config::parse(DEFAULT_TEXT).unwrap(), Config::default());
        assert_eq!(Config::parse("").unwrap(), Config::default());
    }

    #[test]
    fn 일부만_적으면_나머지는_기본값() {
        let config = Config::parse(
            r#"
            [font]
            size = 200
            [link]
            editor = "code -g {file}:{line}:{col}"
            [[profile]]
            id = "powershell"
            label = "파워셸 7"
            program = "pwsh.exe"
            [[profile]]
            id = "gitbash"
            label = "Git Bash"
            program = "bash.exe"
            args = ["-i"]
            "#,
        )
        .unwrap();
        assert_eq!(config.font.size, 72); // 범위를 벗어나면 잘라 낸다.
        assert_eq!(config.font.family, Font::default().family);
        assert!(config.notify.system && config.session.restore);
        assert_eq!(config.profiles.len(), 3);
        assert_eq!(config.profile("powershell").unwrap().program, "pwsh.exe");
        assert_eq!(config.profile("gitbash").unwrap().args, vec!["-i"]);
        assert!(config.profile("claude").unwrap().claude);
    }

    #[test]
    fn 있는_설정_파일은_버전이_바뀌어도_그대로_둔다() {
        let dir = std::env::temp_dir().join("cypher-config-keep");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");

        // 없을 때만 기본 파일을 만든다. 예시 파일도 함께 생긴다.
        let (config, error) = Config::load(&path);
        assert!(error.is_none() && config == Config::default());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_TEXT);
        assert_eq!(std::fs::read_to_string(dir.join("config.example.toml")).unwrap(), DEFAULT_TEXT);

        // 다른 버전이 쓴 파일: 모르는 항목·구역이 있고, 메모장이 BOM을 붙였고, 빠진 항목도 있다.
        let mine = "\u{feff}# 내 설정\n[font]\nsize = 16\nligatures = true\n\n[removed_in_future]\nx = 1\n\n[link]\nrequire_ctrl = true\n";
        std::fs::write(&path, mine).unwrap();
        let (config, error) = Config::load(&path);
        assert!(error.is_none(), "{error:?}");
        assert_eq!((config.font.size, config.link.require_ctrl), (16, true));
        assert!(config.window.magnet); // 파일에 없는 새 항목은 기본값
        assert_eq!(std::fs::read_to_string(&path).unwrap(), mine); // 한 글자도 바뀌지 않는다

        // 내용이 잘못됐거나 글자로 읽을 수 없는 파일도 덮어쓰지 않는다.
        for broken in [&b"[font]\nsize = \"big\"\n"[..], &[0xff, 0xfe, 0x00, 0x41][..]] {
            std::fs::write(&path, broken).unwrap();
            let (config, error) = Config::load(&path);
            assert!(error.is_some() && config == Config::default());
            assert_eq!(std::fs::read(&path).unwrap(), broken);
        }
    }

    #[test]
    fn 잘못된_파일은_오류를_돌려준다() {
        assert!(Config::parse("[font]\nsize = \"큰 글씨\"").is_err());
    }
}
