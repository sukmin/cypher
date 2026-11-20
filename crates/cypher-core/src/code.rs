//! 코드 보기: 파일에서 정의(함수·클래스·필드 …)를 찾고, 이름이 쓰인 곳을 찾는다.
//!
//! 언어 서버 없이 글자만 보고 찾는다. 주석과 문자열을 가린 뒤(`mask`) 줄마다 언어별 모양(정규식)과 중괄호 깊이로
//! 정의를 가려낸다. 그래서 자료형을 따져야 알 수 있는 것(같은 이름의 메서드 가운데 어느 것인지)은 가리지 못하고,
//! 이름이 같은 정의를 모두 내놓는다. 화면이 같은 파일·같은 폴더·감싼 이름으로 가까운 것을 앞에 세운다.

use regex::Regex;
use serde::Serialize;
use std::sync::LazyLock;

/// 정의를 찾을 줄 아는 언어. 그 밖의 글자 파일은 `Text`(이름이 쓰인 곳만 찾는다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    Java,
    Kotlin,
    Js,
    Vue,
    Html,
    Css,
    Python,
    Go,
    C,
    Markdown,
    Config,
    /// XML: id 속성과 MyBatis 매퍼의 구문(select·insert·update·delete)을 정의로 본다.
    Xml,
    Text,
}

/// 파일 이름으로 언어를 정한다. 코드·글자 파일로 보지 않는 것(그림, 실행 파일, 묶음 …)은 None.
pub fn lang_of(file_name: &str) -> Option<Lang> {
    let name = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name).to_lowercase();
    // 점으로 시작하는 이름(`.gitignore`)은 이름 전체가 확장자로 읽히므로, 이름으로 정하는 것을 먼저 본다.
    if matches!(name.as_str(), ".gitignore" | ".gitattributes" | ".editorconfig" | ".dockerignore" | ".npmrc") {
        return Some(Lang::Text);
    }
    let ext = name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");
    Some(match ext {
        "rs" => Lang::Rust,
        "java" | "groovy" | "gradle" | "cs" => Lang::Java,
        "kt" | "kts" => Lang::Kotlin,
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" | "mts" | "cts" => Lang::Js,
        "vue" | "svelte" => Lang::Vue,
        "html" | "htm" | "xhtml" | "jsp" => Lang::Html,
        "css" | "scss" | "sass" | "less" => Lang::Css,
        "py" | "pyi" => Lang::Python,
        "go" => Lang::Go,
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "m" | "mm" => Lang::C,
        "md" | "markdown" | "mdx" => Lang::Markdown,
        "toml" | "ini" | "cfg" | "conf" | "properties" => Lang::Config,
        "xml" => Lang::Xml,
        "json" | "jsonc" | "json5" | "yaml" | "yml" | "svg" | "sql" | "sh" | "bash" | "zsh" | "ps1" | "psm1" | "bat" | "cmd" | "txt" | "lock"
        | "http" | "env" | "graphql" | "proto" | "tf" | "rb" | "php" | "swift" | "dart" | "lua" | "pl" | "r" | "scala" | "ex" | "exs" | "erl" | "hs"
        | "clj" | "fxml" | "pom" | "xsd" | "csv" | "tsv" | "log" => Lang::Text,
        "" if matches!(name.as_str(), "dockerfile" | "makefile" | "license" | "readme") => Lang::Text,
        _ => return None,
    })
}

// ───────────────────────── 주석·문자열 가리기 ─────────────────────────

pub const CODE: u8 = 0;
pub const COMMENT: u8 = 1;
pub const STRING: u8 = 2;

/// 바이트마다의 종류(코드·주석·문자열). 길이는 글의 바이트 수와 같다.
pub fn mask(lang: Lang, text: &str) -> Vec<u8> {
    let mut kinds = vec![CODE; text.len()];
    match lang {
        Lang::Rust | Lang::Java | Lang::Kotlin | Lang::Js | Lang::Go | Lang::C | Lang::Css => mask_braces(lang, text.as_bytes(), 0, text.len(), &mut kinds),
        Lang::Python => mask_python(text.as_bytes(), &mut kinds),
        Lang::Vue | Lang::Html => {
            let bytes = text.as_bytes();
            mask_html_comments(bytes, &mut kinds);
            for (from, to, inner) in embedded(text) {
                // 안쪽 구역은 그 언어대로 다시 가린다(HTML 주석 표시가 코드 안에 있어도 주석이 아니다).
                kinds[from..to].fill(CODE);
                mask_braces(inner, bytes, from, to, &mut kinds);
            }
        }
        Lang::Xml => mask_html_comments(text.as_bytes(), &mut kinds),
        Lang::Markdown | Lang::Config | Lang::Text => {}
    }
    kinds
}

/// 가린 글: 주석과 문자열의 글자를 빈칸으로 바꾼 것(줄바꿈은 그대로). 바이트 자리가 원래 글과 같다.
pub fn code_only(text: &str, kinds: &[u8]) -> String {
    let mut out = Vec::with_capacity(text.len());
    for (index, &byte) in text.as_bytes().iter().enumerate() {
        out.push(if kinds[index] == CODE || byte == b'\n' { byte } else { b' ' });
    }
    // 가린 자리는 바이트마다 빈칸이라, 여러 바이트 글자도 통째로 빈칸이 된다(깨진 글자가 남지 않는다).
    String::from_utf8(out).unwrap_or_default()
}

fn starts(bytes: &[u8], at: usize, end: usize, mark: &[u8]) -> bool {
    at + mark.len() <= end && &bytes[at..at + mark.len()] == mark
}

/// 중괄호 언어(와 CSS)의 주석·문자열을 가린다. `from..to` 구간만 본다(Vue·HTML의 안쪽 구역에 쓴다).
fn mask_braces(lang: Lang, bytes: &[u8], from: usize, to: usize, kinds: &mut [u8]) {
    let nested = matches!(lang, Lang::Rust | Lang::Kotlin);
    let mut i = from;
    while i < to {
        let byte = bytes[i];
        // 줄 주석. CSS에서는 `//`가 주소(`url(http://…)`)에도 나오므로 앞이 빈칸·줄 처음일 때만 주석으로 본다.
        if starts(bytes, i, to, b"//") && (lang != Lang::Css || i == from || bytes[i - 1].is_ascii_whitespace()) {
            let end = bytes[i..to].iter().position(|&b| b == b'\n').map_or(to, |n| i + n);
            kinds[i..end].fill(COMMENT);
            i = end;
            continue;
        }
        if starts(bytes, i, to, b"/*") {
            let mut depth = 1;
            let mut j = i + 2;
            while j < to && depth > 0 {
                if starts(bytes, j, to, b"*/") {
                    depth -= 1;
                    j += 2;
                } else if nested && starts(bytes, j, to, b"/*") {
                    depth += 1;
                    j += 2;
                } else {
                    j += 1;
                }
            }
            kinds[i..j].fill(COMMENT);
            i = j;
            continue;
        }
        // Rust의 날 문자열: r"…", r#"…"#, br#"…"#
        if lang == Lang::Rust && (byte == b'r' || byte == b'b') && (i == from || !is_word(bytes[i - 1])) {
            let mut j = i + if starts(bytes, i, to, b"br") { 2 } else { 1 };
            if byte == b'r' || starts(bytes, i, to, b"br") {
                let hashes = bytes[j..to].iter().take_while(|&&b| b == b'#').count();
                j += hashes;
                if j < to && bytes[j] == b'"' {
                    let mut close = vec![b'"'];
                    close.extend(std::iter::repeat_n(b'#', hashes));
                    let end = find(bytes, j + 1, to, &close).map_or(to, |at| at + close.len());
                    kinds[i..end].fill(STRING);
                    i = end;
                    continue;
                }
            }
        }
        // 여러 줄 문자열: Java·Kotlin의 """…""".
        if matches!(lang, Lang::Java | Lang::Kotlin) && starts(bytes, i, to, b"\"\"\"") {
            let end = find(bytes, i + 3, to, b"\"\"\"").map_or(to, |at| at + 3);
            kinds[i..end].fill(STRING);
            i = end;
            continue;
        }
        // JavaScript의 정규식 리터럴(`/\//g`, `/["']/`): 안의 `//`·따옴표·중괄호가 주석·문자열·몸통으로 읽히지 않게 문자열로 가린다.
        if lang == Lang::Js && byte == b'/' && regex_may_start(bytes, from, i) {
            if let Some(end) = js_regex_end(bytes, i, to) {
                kinds[i..end].fill(STRING);
                i = end;
                continue;
            }
        }
        if byte == b'"' || byte == b'`' && matches!(lang, Lang::Js | Lang::Go) {
            // 여러 줄에 걸칠 수 있는 것: Rust의 문자열, JavaScript의 `…`, Go의 `…`. 그 밖의 것은 줄 끝에서 끊는다.
            let multi = lang == Lang::Rust || byte == b'`';
            let raw = lang == Lang::Go && byte == b'`';
            let end = string_end(bytes, i, to, byte, multi, raw);
            kinds[i..end].fill(STRING);
            i = end;
            continue;
        }
        if byte == b'\'' {
            if lang == Lang::Rust {
                // 'a'·'\n'·'\u{1F600}'는 글자, 그 밖의 '이름은 수명이다(코드로 둔다).
                if let Some(end) = rust_char_end(bytes, i, to) {
                    kinds[i..end].fill(STRING);
                    i = end;
                    continue;
                }
            } else {
                let end = string_end(bytes, i, to, b'\'', false, false);
                kinds[i..end].fill(STRING);
                i = end;
                continue;
            }
        }
        i += 1;
    }
}

/// `at`의 `/`가 값이 올 자리에 있는지(나눗셈이 아니라 정규식 리터럴이 시작될 수 있는지): 그 줄에서 앞의 뜻 있는 글자가 없거나,
/// 연산자·여는 괄호·구분 기호이거나, `return`·`typeof` 같은 낱말이다.
fn regex_may_start(bytes: &[u8], from: usize, at: usize) -> bool {
    let mut j = at;
    while j > from && matches!(bytes[j - 1], b' ' | b'\t') {
        j -= 1;
    }
    if j == from || bytes[j - 1] == b'\n' {
        return true;
    }
    let before = bytes[j - 1];
    let earlier = if j >= from + 2 { bytes[j - 2] } else { b' ' };
    // `a++ / 2`의 `/`는 나눗셈이다. `>` 뒤는 화살표(`x => /re/`)일 때만 값이 올 자리다(JSX의 `</b>`와 가린다).
    if (before == b'+' && earlier == b'+') || (before == b'-' && earlier == b'-') {
        return false;
    }
    if before == b'>' {
        return earlier == b'=';
    }
    // (`}` 뒤는 값이 올 자리로 보지 않는다: JSX의 `item={i} />`에서 `/`부터 다음 `/`까지가 정규식으로 읽히면 그 사이의 중괄호가 가려진다.)
    if b"=(,:;!&|?{[+-*%~^".contains(&before) {
        return true;
    }
    if !is_word(before) {
        return false;
    }
    let mut start = j;
    while start > from && is_word(bytes[start - 1]) {
        start -= 1;
    }
    matches!(&bytes[start..j], b"return" | b"typeof" | b"case" | b"in" | b"of" | b"delete" | b"void" | b"throw" | b"new" | b"else" | b"do")
}

/// `open`의 `/`에서 시작한 정규식 리터럴이 끝나는 자리(닫는 `/`와 그 뒤의 깃발 글자 다음). 그 줄에서 닫히지 않으면 None.
fn js_regex_end(bytes: &[u8], open: usize, to: usize) -> Option<usize> {
    let mut j = open + 1;
    let mut class = false; // [ … ] 안(그 안의 /는 닫는 것이 아니다)
    if j >= to || bytes[j] == b'/' || bytes[j] == b'*' {
        return None;
    }
    while j < to {
        match bytes[j] {
            b'\n' => return None,
            b'\\' => j += 1,
            b'[' => class = true,
            b']' => class = false,
            b'/' if !class => {
                j += 1;
                while j < to && bytes[j].is_ascii_alphabetic() {
                    j += 1;
                }
                return Some(j);
            }
            _ => {}
        }
        j += 1;
    }
    None
}

fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

fn find(bytes: &[u8], from: usize, to: usize, needle: &[u8]) -> Option<usize> {
    if from >= to || needle.is_empty() || to - from < needle.len() {
        return None;
    }
    bytes[from..to].windows(needle.len()).position(|window| window == needle).map(|at| from + at)
}

/// 따옴표로 연 문자열이 끝나는 자리(닫는 따옴표 다음). 닫히지 않으면 줄 끝(여러 줄 문자열이면 구간 끝).
fn string_end(bytes: &[u8], open: usize, to: usize, quote: u8, multi: bool, raw: bool) -> usize {
    let mut j = open + 1;
    while j < to {
        match bytes[j] {
            b'\\' if !raw => j += 2,
            b'\n' if !multi => return j,
            byte if byte == quote => return j + 1,
            _ => j += 1,
        }
    }
    to
}

/// Rust의 글자 리터럴(`'a'`, `'\n'`, `'\u{1F600}'`, `'한'`)이면 그 끝 자리.
fn rust_char_end(bytes: &[u8], open: usize, to: usize) -> Option<usize> {
    let mut j = open + 1;
    if j >= to {
        return None;
    }
    if bytes[j] == b'\\' {
        j += 2;
        // \x41, \u{…}
        while j < to && j < open + 14 && bytes[j] != b'\'' && bytes[j] != b'\n' {
            j += 1;
        }
    } else {
        // 글자 하나(여러 바이트일 수 있다).
        j += 1;
        while j < to && (bytes[j] & 0xc0) == 0x80 {
            j += 1;
        }
    }
    (j < to && bytes[j] == b'\'').then_some(j + 1)
}

fn mask_python(bytes: &[u8], kinds: &mut [u8]) {
    let to = bytes.len();
    let mut i = 0;
    while i < to {
        let byte = bytes[i];
        if byte == b'#' {
            let end = bytes[i..].iter().position(|&b| b == b'\n').map_or(to, |n| i + n);
            kinds[i..end].fill(COMMENT);
            i = end;
        } else if byte == b'"' || byte == b'\'' {
            let triple = [byte; 3];
            let end = if starts(bytes, i, to, &triple) {
                find(bytes, i + 3, to, &triple).map_or(to, |at| at + 3)
            } else {
                string_end(bytes, i, to, byte, false, false)
            };
            kinds[i..end].fill(STRING);
            i = end;
        } else {
            i += 1;
        }
    }
}

fn mask_html_comments(bytes: &[u8], kinds: &mut [u8]) {
    let to = bytes.len();
    let mut i = 0;
    while let Some(open) = find(bytes, i, to, b"<!--") {
        let end = find(bytes, open + 4, to, b"-->").map_or(to, |at| at + 3);
        kinds[open..end].fill(COMMENT);
        i = end;
    }
}

static EMBED_OPEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<(script|style)\b[^>]*>").unwrap());

/// HTML·Vue 파일 안의 `<script>`·`<style>` 구역: (내용의 시작, 끝, 그 안의 언어).
fn embedded(text: &str) -> Vec<(usize, usize, Lang)> {
    let mut found = Vec::new();
    let mut at = 0;
    let lower = text.to_ascii_lowercase(); // 바이트 자리가 원래 글과 같다
    while let Some(open) = EMBED_OPEN.captures_at(text, at) {
        let whole = open.get(0).unwrap();
        let script = open[1].eq_ignore_ascii_case("script");
        let close = if script { "</script" } else { "</style" };
        let from = whole.end();
        let to = lower[from..].find(close).map_or(text.len(), |n| from + n);
        // 닫는 태그가 바로 붙은 것(`<script src=…></script>`)은 내용이 없다.
        if to > from {
            found.push((from, to, if script { Lang::Js } else { Lang::Css }));
        }
        at = to.max(whole.end());
        if at >= text.len() {
            break;
        }
    }
    found
}

// ───────────────────────── 정의 찾기 ─────────────────────────

/// 파일 안의 정의 하나.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Def {
    pub name: String,
    /// function, method, constructor, class, interface, enum, struct, trait, type, module, constant, variable, field, variant,
    /// macro, id, selector, heading, section
    pub kind: &'static str,
    /// 줄 번호(1부터).
    pub line: u32,
    /// 이름이 시작하는 칸(0부터, UTF-16 단위 — 화면의 글자 자리와 같다).
    pub col: u32,
    /// 감싸고 있는 것의 이름(클래스, impl의 자료형 …). 없으면 빈 글.
    pub container: String,
    /// 얼마나 안쪽에 있는지(개요를 들여 그리는 데 쓴다).
    pub depth: u32,
    /// 몸통이 끝나는 줄(몸통이 없으면 정의한 줄). 어느 정의 안에 있는 줄인지 가리는 데 쓴다.
    pub end: u32,
}

/// 감싸고 있는 것(클래스, 함수 …). 그 몸통이 끝나면 스택에서 뺀다.
struct Scope {
    name: String,
    kind: &'static str,
    /// 선언한 줄이 시작할 때의 중괄호 깊이. 몸통은 이보다 깊다.
    depth: i32,
    /// 몸통에 들어간 적이 있는지(선언과 `{`가 다른 줄에 있을 수 있다).
    entered: bool,
    /// 선언한 줄.
    line: u32,
    /// 멤버(필드·메서드)를 가질 수 있는 것인지. 함수의 몸통 안에서는 멤버를 찾지 않는다.
    typeish: bool,
    /// 이것을 선언한 정의의 순번(끝 줄을 적어 줄 곳). 정의 없이 감싸기만 하는 것(impl)은 None.
    def: Option<usize>,
}

struct Finder<'a> {
    defs: Vec<Def>,
    scopes: Vec<Scope>,
    /// 지금 줄의 원래 글(칸을 세는 데 쓴다)과 줄 번호.
    original: &'a str,
    line: u32,
    /// 이 줄이 시작할 때 열려 있는 소괄호 수(여러 줄에 걸친 인자 목록 안인지).
    parens: i32,
    /// Go의 묶음 선언(`const (` … `)`) 안이면 그 종류.
    group: &'static str,
    /// 묶음 선언 안에서 열려 있는 괄호 수(여러 줄에 걸친 호출의 인자 안인지).
    group_parens: i32,
}

impl Finder<'_> {
    fn container(&self) -> String {
        self.scopes.iter().rev().find(|scope| scope.typeish).map(|scope| scope.name.clone()).unwrap_or_default()
    }

    /// 정의를 적는다. `at`은 그 줄에서 이름이 시작하는 바이트 자리.
    fn add(&mut self, name: &str, kind: &'static str, at: usize) {
        let col = self.original.get(..at).map_or(0, |before| before.encode_utf16().count()) as u32;
        let container = self.container();
        let depth = self.scopes.len() as u32;
        self.defs.push(Def { name: name.to_string(), kind, line: self.line, col, container, depth, end: self.line });
    }

    /// 방금 적은 정의를 감싸는 것으로 올린다(그 몸통 안의 정의가 이것을 감싼 이름으로 갖는다).
    fn open(&mut self, name: &str, kind: &'static str, depth: i32, typeish: bool) {
        let def = self.defs.len().checked_sub(1).filter(|&last| self.defs[last].line == self.line && self.defs[last].name == name);
        self.scopes.push(Scope { name: name.to_string(), kind, depth, entered: false, line: self.line, typeish, def });
    }

    /// 가장 안쪽의 감싼 것을 닫는다. 그 몸통이 `line`번째 줄에서 끝났다.
    fn close(&mut self, line: u32) {
        if let Some(scope) = self.scopes.pop() {
            // 몸통에 들어간 적이 없는 것(한 줄짜리 정의, 몸통 없는 선언)은 정의한 줄에서 끝난다.
            if let Some(def) = scope.def.filter(|_| scope.entered) {
                self.defs[def].end = line.max(self.defs[def].line);
            }
        }
    }

    /// 남은 것을 모두 닫는다(파일이 끝났다).
    fn close_all(&mut self, line: u32) {
        while !self.scopes.is_empty() {
            self.close(line);
        }
    }

    /// 지금 줄이 멤버를 찾을 자리인지: 가장 안쪽의 감싼 것이 멤버를 가질 수 있고, 그 몸통의 바로 안쪽 깊이다.
    fn member_of(&self, depth: i32) -> Option<&Scope> {
        // 여러 줄에 걸친 괄호 안(튜플 값의 자료형, 호출의 인자)은 멤버가 아니다.
        self.scopes.last().filter(|scope| self.parens == 0 && scope.typeish && depth == scope.depth + 1)
    }
}

/// 그 파일의 정의들(나오는 차례대로).
pub fn definitions(lang: Lang, text: &str) -> Vec<Def> {
    match lang {
        Lang::Vue | Lang::Html => {
            let mut defs = line_defs(Lang::Html, text, text);
            let starts = line_starts(text);
            for (from, to, inner) in embedded(text) {
                // 구역이 든 줄들만 읽는다(구역마다 파일 전체를 다시 훑지 않게). 첫 줄에서 구역 앞의 글자는 빈칸으로 바꿔 칸을 지킨다.
                let first = starts.partition_point(|&start| start <= from) - 1;
                let line_start = starts[first];
                let original = &text[line_start..to];
                let mut only = " ".repeat(from - line_start);
                only.push_str(&text[from..to]);
                let kinds = mask(inner, &only);
                defs.extend(brace_defs(inner, original, &code_only(&only, &kinds), first as u32));
            }
            defs.sort_by_key(|def| (def.line, def.col));
            defs
        }
        Lang::Python => python_defs(text, &code_only(text, &mask(lang, text))),
        Lang::Markdown | Lang::Config => line_defs(lang, text, text),
        Lang::Xml => line_defs(lang, text, &code_only(text, &mask(lang, text))),
        Lang::Text => Vec::new(),
        _ => brace_defs(lang, text, &code_only(text, &mask(lang, text)), 0),
    }
}

macro_rules! re {
    ($name:ident, $pattern:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pattern).unwrap());
    };
}

// Rust
re!(RS_FN, r"^\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\s*\([^)]*\))?\s+)?(?:(?:default|const|async|unsafe|extern)\s+)*fn\s+([\p{L}_]\w*)");
re!(RS_TYPE, r"^\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\s*\([^)]*\))?\s+)?(?:unsafe\s+)?(struct|enum|trait|union|mod|type)\s+([\p{L}_]\w*)");
re!(RS_CONST, r"^\s*(?:pub(?:\s*\([^)]*\))?\s+)?(?:const|static)\s+(?:(?:mut|ref)\s+)?([\p{L}_]\w*)\s*:");
re!(RS_MACRO, r"^\s*macro_rules!\s*([\p{L}_]\w*)");
re!(RS_IMPL, r"^\s*(?:unsafe\s+)?impl\b(.*)$");
re!(RS_FIELD, r"^\s*(?:pub(?:\s*\([^)]*\))?\s+)?([\p{Ll}\p{Lo}_]\w*)\s*:");
re!(RS_VARIANT, r"^\s*([\p{Lu}\p{Lo}]\w*)\s*(?:[({,=]|$)");
// Java·C#·C 계열
re!(JV_TYPE, r"(?:^|[^.\w])(class|interface|enum(?:\s+(?:class|struct))?|record|@interface|struct|namespace)\s+([\p{L}_$][\w$]*)");
re!(
    CS_PROPERTY,
    r"^\s*(?:\[[^\]]*\]\s*)*(?:(?:public|protected|private|internal|static|virtual|override|abstract|sealed|required|new|readonly)\s+)+([\w.<>\[\],? ]+?)\s+([\p{L}_][\w]*)\s*\{\s*(?:get|set|init)\b"
);
re!(
    JV_METHOD,
    r"^\s*(?:@[\w.]+(?:\([^)]*\))?\s+)*((?:(?:public|protected|private|internal|static|final|abstract|synchronized|native|default|strictfp|virtual|override|async|inline|extern|unsafe|sealed|partial|readonly|constexpr|explicit|friend)\s+|@[\w.]+(?:\([^)]*\))?\s+)*)(?:<(?:[^<>]|<[^<>]*>)*>\s*)?(?:@[\w.]+(?:\([^)]*\))?\s+)*([\w$.<>\[\],?&*: @]*?)[\s*&]*\b([\p{L}_$~][\w$]*)\s*\("
);
re!(
    JV_FIELD,
    r"^\s*(?:@[\w.]+(?:\([^)]*\))?\s+)*(?:(?:public|protected|private|internal|static|final|volatile|transient|readonly|const|mutable)\s+|@[\w.]+(?:\([^)]*\))?\s+)*([\w$.<>\[\],?@ ]+?)\s+([\p{L}_$][\w$]*)\s*(?:=|;|,)"
);
re!(JV_ENUM_CONST, r"^\s*([A-Z_][A-Z0-9_]*)\s*(?:[({,;=]|$)");
re!(JV_ENUM_LIST, r"^\s*[A-Z_][A-Z0-9_]*(?:\s*,\s*[A-Z_][A-Z0-9_]*)+\s*[,;]?\s*$");
re!(C_DEFINE, r"^\s*#\s*define\s+([\p{L}_]\w*)");
// Kotlin
re!(KT_TYPE, r"(?:^|[^.\w])(class|interface|object)\s+([\p{L}_]\w*)");
re!(KT_FUN, r"\bfun\s+(?:<[^>]*>\s*)?(?:[\w.<>?, ]+\.)?([\p{L}_]\w*)\s*\(");
re!(KT_PROP, r"^\s*(?:(?:private|protected|public|internal|override|lateinit|const|open|final|abstract)\s+)*(?:val|var)\s+(?:[\w<>?]+\.)?([\p{L}_]\w*)");
re!(KT_ALIAS, r"\btypealias\s+([\p{L}_]\w*)");
re!(KT_COMPANION, r"\bcompanion\s+object\s*(?:\{|$)");
// JavaScript·TypeScript
re!(JS_FUNC, r"\bfunction\s*\*?\s*([\p{L}_$][\w$]*)\s*[(<]");
re!(JS_CLASS, r"(?:^|[^.\w$])class\s+([\p{L}_$][\w$]*)");
re!(TS_TYPE, r"^\s*(?:export\s+)?(?:declare\s+)?(?:default\s+)?(?:abstract\s+)?(interface|type|enum|namespace|module)\s+([\p{L}_$][\w$]*)\s*(.?)");
// 자료형 표기 안의 `=>`(`const f: (a: number) => void = …`)는 값의 시작이 아니다.
re!(JS_VAR, r"^\s*(?:export\s+)?(?:declare\s+)?(?:default\s+)?(const|let|var)\s+([\p{L}_$][\w$]*)\s*(?::(?:[^=]|=>)*)?(=\s*(.*))?");
re!(
    JS_METHOD,
    r"^\s*(?:(?:public|private|protected|static|async|readonly|override|abstract|get|set)\s+)*\*?\s*(#?[\p{L}_$][\w$]*)\s*(?:<[^>()]*>)?\s*\([^()]*\)\s*(?::\s*[^={]+?)?\s*\{"
);
re!(
    JS_PROP_FN,
    r"^\s*(?:(?:public|private|protected|static|readonly|override)\s+)*(#?[\p{L}_$][\w$]*)\s*[:=]\s*(?:async\s+)?(?:function\b|(?:\([^()]*\)|[\p{L}_$][\w$]*)\s*(?::\s*[^=]+?)?\s*=>)"
);
re!(JS_EXPORTS, r"\bexports\.([\p{L}_$][\w$]*)\s*=");
re!(JS_ARROW, r"^(?:async\s+)?(?:function\b|\([^()]*\)\s*(?::\s*[^=]+?)?\s*=>|[\p{L}_$][\w$]*\s*=>)");
// Go
re!(GO_FUNC, r"^func\s+(?:\(\s*(?:\w+\s+)?\*?\s*(\w+)\s*(?:\[[^\]]*\])?\s*\)\s*)?([\p{L}_]\w*)\s*[(\[]");
re!(GO_GROUP, r"^(const|var|type)\s*\(\s*$");
re!(GO_GROUPED, r"^\s+([\p{L}_]\w*)\b");
re!(GO_TYPE, r"^\s*type\s+([\p{L}_]\w*)");
re!(GO_VAR, r"^(?:var|const)\s+([\p{L}_]\w*)");
// CSS
re!(CSS_CLASS, r"\.(-?[\p{L}_][\w-]*)");
re!(CSS_ID, r"#([\p{L}_][\w-]*)");
re!(CSS_VAR, r"(?:^|[{;\s])(--[\w-]+|\$[\w-]+)\s*:");
re!(CSS_AT, r"@(mixin|keyframes|function)\s+([\w-]+)");
// HTML
re!(HTML_ID, r#"(?:^|[\s"'])(id|ref)\s*=\s*["']([^"'\s{}]+)["']"#);
// XML
re!(XML_ID, r#"<([\w:.-]+)\b[^<>]*?\sid\s*=\s*["']([\p{L}_][^"'\s{}$#]*)["']"#);
re!(XML_NAMESPACE, r#"<mapper\b[^<>]*?\snamespace\s*=\s*["']([^"']+)["']"#);
// Python
re!(PY_DEF, r"^(\s*)(?:async\s+)?(def|class)\s+([\p{L}_]\w*)");
re!(PY_VAR, r"^([\p{L}_]\w*)\s*(?::[^=]+)?=[^=]");
// Markdown, 설정 파일
// 닫는 `#`들은 앞에 빈칸이 있을 때만 뗀다(`# C#`의 제목은 "C#"이다).
re!(MD_HEAD, r"^(#{1,6})\s+(.+?)(?:\s+#+)?\s*$");
re!(CONFIG_SECTION, r"^\s*\[+\s*([^\]]+?)\s*\]+\s*(?:[#;].*)?$");

const JV_NOT_NAMES: &[&str] = &[
    "if", "for", "while", "switch", "catch", "synchronized", "return", "new", "throw", "super", "this", "sizeof", "using", "lock", "foreach", "when", "else", "do",
    "try", "assert", "typeof", "await", "delete", "defined", "alignof", "decltype", "static_assert",
];
const JV_NOT_TYPES: &[&str] = &[
    "return", "new", "throw", "else", "case", "assert", "yield", "await", "delete", "goto", "package", "import", "break", "continue", "using", "namespace", "typedef",
    "do", "in", "is", "as", "instanceof", "throws", "extends", "implements",
];
const JS_NOT_NAMES: &[&str] = &[
    "if", "for", "while", "switch", "catch", "function", "return", "with", "do", "else", "try", "typeof", "await", "new", "delete", "void", "yield", "super", "import", "export",
];

/// 중괄호 언어의 정의. `original`은 원래 글, `code`는 주석·문자열을 가린 글(바이트 자리가 같다).
/// `line_offset`은 이 글의 첫 줄이 파일에서 몇 번째 줄인지(0부터. 파일의 일부만 넘겨받았을 때).
fn brace_defs(lang: Lang, original: &str, code: &str, line_offset: u32) -> Vec<Def> {
    let mut finder = Finder { defs: Vec::new(), scopes: Vec::new(), original: "", line: 0, parens: 0, group: "", group_parens: 0 };
    let mut depth: i32 = 0;
    let mut parens: i32 = 0; // 열린 소괄호 수(여러 줄에 걸친 인자 목록 안인지)
    let mut where_clause = false; // Rust의 `where` 절 안(선언과 `{` 사이의 여러 줄)
    let mut after_comma = false; // 앞의 뜻 있는 줄이 쉼표로 끝났는지(상위 자료형 목록이 다음 줄로 이어진다)
    for (index, (line, original_line)) in code.split('\n').zip(original.split('\n')).enumerate() {
        finder.line = line_offset + index as u32 + 1;
        finder.original = original_line;
        finder.parens = parens;
        let tail = line.trim_end();
        if !line.trim().is_empty() {
            // 몸통 없이 끝난 선언(`fun f() = 값`, `fn f();` 뒤의 줄)이 감싸는 것으로 남아 있으면 뺀다: 앞줄에서 선언했는데 아직
            // `{`를 만나지 못했고, 인자 목록도 끝났고, 이 줄이 선언을 잇는 줄(`{`, `where`, `throws` …)도 아니다.
            let lead = line.trim_start();
            if lead.starts_with("where") || tail.ends_with(" where") {
                where_clause = true;
            }
            // 머리가 여러 줄인 선언: rustfmt는 긴 impl 머리를 `for` 앞에서 나누고(`impl<T> Trait<T>` / `    for Type<T>`),
            // Kotlin·C++은 상위 자료형 목록을 쉼표 뒤에서 나눈다(`) : Base(),` / `    Iface {`).
            let continues = where_clause
                || after_comma
                || (lang == Lang::Rust && lead.starts_with("for "))
                || lead.starts_with(['{', '.', ')', ',', ':', '=', '-', '|', '&', '?', '+'])
                || ["throws", "extends", "implements"].iter().any(|word| lead.starts_with(word));
            // 쉼표로 이어지는 머리는 객체 글자가 없는 언어에서만 본다(JS의 `get() { … },` 다음 줄은 새 멤버다).
            after_comma = matches!(lang, Lang::Kotlin | Lang::C | Lang::Java) && tail.ends_with(',') && !line.contains(['{', '}']);
            if lang == Lang::Rust && lead.starts_with("for ") {
                // 나뉜 impl 머리의 둘째 줄: 감싸는 이름은 트레이트가 아니라 `for` 뒤의 자료형이다.
                let joined = format!("_ {lead}");
                let target = impl_target(&joined);
                if !target.is_empty() && target.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    if let Some(scope) = finder.scopes.last_mut().filter(|scope| scope.kind == "impl" && !scope.entered) {
                        scope.name = target.to_string();
                    }
                }
            }
            if parens == 0 && !continues {
                while finder.scopes.last().is_some_and(|scope| !scope.entered && scope.line < finder.line) {
                    finder.scopes.pop();
                }
            }
            let before = finder.scopes.len();
            match lang {
                Lang::Rust => rust_line(&mut finder, line, depth),
                Lang::Java | Lang::C => java_line(&mut finder, line, depth, lang == Lang::C),
                Lang::Kotlin => kotlin_line(&mut finder, line, depth),
                Lang::Js => js_line(&mut finder, line, depth),
                Lang::Go => go_line(&mut finder, line, depth),
                Lang::Css => css_line(&mut finder, line, depth),
                _ => {}
            }
            // 이 줄에서 새로 올린 "감싸는 것"이 정말 몸통을 갖는지: 선언이 `;`로 끝나면(몸통 없는 선언) 뺀다.
            if finder.scopes.len() > before && !line.contains('{') && tail.ends_with(';') {
                finder.scopes.truncate(before);
            }
        }
        // 이 줄에서 짝이 맞는 중괄호의 여는 자리들. 이 줄에서 선언한 것은 그런 중괄호로는 몸통에 들어간 것으로 치지 않는다:
        // `function f(a, opts = {}) {`, `const Row = ({ item }) => {`의 앞쪽 `{…}`는 몸통이 아니다. 줄에 남은 짝 없는 `{`만 몸통이다.
        let mut paired: Vec<usize> = Vec::new();
        let mut open: Vec<usize> = Vec::new();
        for (at, byte) in line.bytes().enumerate() {
            match byte {
                b'{' => open.push(at),
                b'}' => {
                    if let Some(start) = open.pop() {
                        paired.push(start);
                    }
                }
                _ => {}
            }
        }
        let this_line = finder.line;
        // 중괄호 깊이를 따라가며 끝난 몸통을 스택에서 뺀다.
        for (at, byte) in line.bytes().enumerate() {
            match byte {
                b'{' => {
                    depth += 1;
                    where_clause = false;
                    let inline = paired.contains(&at);
                    for scope in finder.scopes.iter_mut().filter(|scope| depth > scope.depth && !(inline && scope.line == this_line)) {
                        scope.entered = true;
                    }
                }
                b'}' => {
                    depth = (depth - 1).max(0);
                    while finder.scopes.last().is_some_and(|scope| scope.entered && depth <= scope.depth) {
                        let line = finder.line;
                        finder.close(line);
                    }
                }
                b'(' => parens += 1,
                b')' => parens = (parens - 1).max(0),
                _ => {}
            }
        }
        // 몸통 없이 `;`로 끝난 여러 줄 선언(추상 메서드 등)이 스택에 남지 않게 한다.
        if tail.ends_with(';') {
            where_clause = false;
            while finder.scopes.last().is_some_and(|scope| !scope.entered && depth <= scope.depth) {
                finder.scopes.pop();
            }
        }
        // 문장이 끝난 줄에서는 괄호 수를 0으로 되돌린다. 가리지 못한 글자(정규식·글자 리터럴 안의 괄호) 때문에 한 번 어긋난 수가
        // 파일 끝까지 이어지지 않게 한다. (`foo(() => {`처럼 괄호가 열린 채 `{`로 끝난 줄은 뒤의 `)`가 0 아래로 내려가지 않는다.)
        if tail.ends_with([';', '{', '}']) {
            parens = 0;
        }
    }
    let last = line_offset + code.split('\n').count() as u32;
    finder.close_all(last);
    finder.defs
}

/// `impl` 뒤의 글에서 감싸는 자료형의 이름을 고른다: `<T: A<B>> Trait<X> for path::Type<T> where …` → `Type`.
fn impl_target(rest: &str) -> &str {
    // 맨 앞의 `<…>`(제네릭 인자)를 짝을 맞춰 건너뛴다.
    let mut rest = rest.trim_start();
    if rest.starts_with('<') {
        let mut depth = 0;
        let mut end = rest.len();
        let mut before = ' ';
        for (at, ch) in rest.char_indices() {
            let arrow = ch == '>' && before == '-'; // Fn() -> u32
            before = ch;
            match ch {
                '<' => depth += 1,
                '>' if arrow => {}
                '>' => {
                    depth -= 1;
                    if depth == 0 {
                        end = at + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        rest = &rest[end..];
    }
    let rest = rest.split('{').next().unwrap_or("");
    let rest = rest.split(" where ").next().unwrap_or("").trim().trim_end_matches("where").trim();
    let target = rest.rsplit(" for ").next().unwrap_or("").trim();
    // 앞에 붙는 것(`&`, `&'a `, `mut `, `dyn `, `impl `)을 뗀다.
    let mut target = target.trim_start_matches('&').trim_start();
    loop {
        let next = ["mut ", "dyn ", "impl "].iter().find_map(|word| target.strip_prefix(word)).or_else(|| {
            target.strip_prefix('\'').and_then(|life| life.find(' ').map(|space| &life[space + 1..]))
        });
        match next {
            Some(next) => target = next.trim_start().trim_start_matches('&').trim_start(),
            None => break,
        }
    }
    let path = target.split(['<', ' ', '(']).next().unwrap_or("");
    path.rsplit("::").next().unwrap_or("")
}

fn rust_line(finder: &mut Finder, line: &str, depth: i32) {
    if let Some(found) = RS_FN.captures(line) {
        let name = found.get(1).unwrap();
        let kind = if finder.scopes.last().is_some_and(|scope| scope.typeish && scope.kind != "module") { "method" } else { "function" };
        finder.add(name.as_str(), kind, name.start());
        finder.open(name.as_str(), "function", depth, false);
    } else if let Some(found) = RS_TYPE.captures(line) {
        let name = found.get(2).unwrap();
        let kind = match &found[1] {
            "struct" | "union" => "struct",
            "enum" => "enum",
            "trait" => "trait",
            "mod" => "module",
            _ => "type",
        };
        finder.add(name.as_str(), kind, name.start());
        if kind != "type" {
            finder.open(name.as_str(), kind, depth, true);
        }
    } else if let Some(found) = RS_CONST.captures(line) {
        let name = found.get(1).unwrap();
        finder.add(name.as_str(), "constant", name.start());
    } else if let Some(found) = RS_MACRO.captures(line) {
        let name = found.get(1).unwrap();
        finder.add(name.as_str(), "macro", name.start());
    } else if let Some(found) = RS_IMPL.captures(line) {
        // `impl Trait for Type` → Type, `impl<T> Type<T>` → Type. 감싸는 이름으로만 쓴다(정의로 적지는 않는다).
        let name = impl_target(found.get(1).map_or("", |rest| rest.as_str()));
        if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            finder.open(name, "impl", depth, true);
        }
    } else if let Some(scope) = finder.member_of(depth) {
        match scope.kind {
            "struct" => {
                if let Some(name) = RS_FIELD.captures(line).and_then(|found| found.get(1)) {
                    finder.add(name.as_str(), "field", name.start());
                }
            }
            "enum" => {
                if let Some(name) = RS_VARIANT.captures(line).and_then(|found| found.get(1)) {
                    finder.add(name.as_str(), "variant", name.start());
                }
            }
            _ => {}
        }
    }
}

fn java_line(finder: &mut Finder, line: &str, depth: i32, top_level_functions: bool) {
    if top_level_functions {
        if let Some(name) = C_DEFINE.captures(line).and_then(|found| found.get(1)) {
            finder.add(name.as_str(), "macro", name.start());
            return;
        }
    }
    if let Some(found) = JV_TYPE.captures(line) {
        let name = found.get(2).unwrap();
        // C·C++에서는 `struct 이름`·`class T`·`namespace 이름`이 쓰는 곳에도 나온다(`struct point *p`, `template <class T>`,
        // `using namespace std;`). 이름 바로 뒤가 몸통(`{`)이나 상속(`:`)이거나 줄의 끝일 때만 선언으로 본다.
        let rest = line[name.end()..].trim_start();
        let declares = !top_level_functions || rest.is_empty() || rest.starts_with(['{', ':']) || rest.starts_with("final");
        if declares {
            let word = &found[1];
            let kind = if word.starts_with("enum") {
                "enum"
            } else {
                match word {
                    "interface" | "@interface" => "interface",
                    "struct" => "struct",
                    "namespace" => "module",
                    _ => "class",
                }
            };
            finder.add(name.as_str(), kind, name.start());
            finder.open(name.as_str(), kind, depth, true);
            if word == "record" {
                record_fields(finder, line, name.end());
            }
            return;
        }
    }
    let member = finder.member_of(depth).map(|scope| (scope.name.clone(), scope.kind));
    if member.is_none() && !(top_level_functions && depth == 0) {
        return;
    }
    if let Some((_, "enum")) = member {
        // 한 줄에 여럿 적은 상수(`RED, GREEN, BLUE;`)는 하나씩 적는다.
        if JV_ENUM_LIST.is_match(line) {
            let mut at = 0;
            for part in line.split(',') {
                let name = part.trim().trim_end_matches(';').trim();
                if !name.is_empty() {
                    finder.add(name, "constant", at + part.find(name).unwrap_or(0));
                }
                at += part.len() + 1;
            }
            return;
        }
        if let Some(name) = JV_ENUM_CONST.captures(line).and_then(|found| found.get(1)) {
            finder.add(name.as_str(), "constant", name.start());
            return;
        }
    }
    if let Some(found) = JV_METHOD.captures(line) {
        let (modifiers, returns, name) = (found[1].trim(), found[2].trim(), found.get(3).unwrap());
        let first = returns.split([' ', '<', '.']).next().unwrap_or("");
        let constructor = member.as_ref().is_some_and(|(owner, _)| owner == name.as_str().trim_start_matches('~'));
        // (`delete`는 C++에서만 예약어다. Java·C#에서는 흔한 메서드 이름이다.)
        let ok = (!JV_NOT_NAMES.contains(&name.as_str()) || (!top_level_functions && name.as_str() == "delete"))
            && !JV_NOT_TYPES.contains(&first)
            // 인자가 붙은 애노테이션 줄(`@Value("…")`)은 선언이 아니다.
            && !returns.starts_with('@')
            // 앞줄에서 이어진 호출(`Props.getFlag(…)`, `.and(…)`)도 선언이 아니다.
            && !returns.ends_with('.')
            && !returns.starts_with('.')
            // 반환형이 없으면 생성자여야 한다(그렇지 않으면 `foo(x);` 같은 호출이다). 한정어만 붙은 것도 마찬가지다.
            && (!returns.is_empty() || constructor)
            && !(returns.is_empty() && modifiers.is_empty() && !constructor);
        if ok {
            let kind = if constructor { "constructor" } else if member.is_some() { "method" } else { "function" };
            finder.add(name.as_str(), kind, name.start());
            finder.open(name.as_str(), "function", depth, false);
            return;
        }
    }
    if member.is_some() {
        // C#의 속성: `public string Name { get; set; }`
        if let Some(name) = CS_PROPERTY.captures(line).and_then(|found| found.get(2)) {
            finder.add(name.as_str(), "field", name.start());
            return;
        }
        if let Some(found) = JV_FIELD.captures(line) {
            let (kind, name) = (found[1].trim(), found.get(2).unwrap());
            let first = kind.split([' ', '<', '.']).next().unwrap_or("");
            if !JV_NOT_TYPES.contains(&first) && !JV_NOT_NAMES.contains(&first) && !kind.is_empty() && !kind.starts_with('@') {
                let constant = name.as_str().chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
                finder.add(name.as_str(), if constant { "constant" } else { "field" }, name.start());
            }
        }
    }
}

/// `record Point(int x, List<String> names)`의 구성 요소를 필드로 적는다(같은 줄에 적힌 것만).
fn record_fields(finder: &mut Finder, line: &str, from: usize) {
    let Some(open) = line[from..].find('(').map(|at| from + at) else { return };
    let mut depth = 0;
    let mut start = open + 1;
    let mut parts: Vec<(usize, usize)> = Vec::new();
    for (at, ch) in line[open..].char_indices().map(|(at, ch)| (open + at, ch)) {
        match ch {
            '(' | '<' | '[' => depth += 1,
            ')' | '>' | ']' => {
                depth -= 1;
                if depth == 0 {
                    parts.push((start, at));
                    break;
                }
            }
            ',' if depth == 1 => {
                parts.push((start, at));
                start = at + 1;
            }
            _ => {}
        }
    }
    for (from, to) in parts {
        let part = line[from..to].trim_end();
        let name_at = part.char_indices().rev().find(|(_, ch)| !(ch.is_alphanumeric() || *ch == '_' || *ch == '$')).map_or(0, |(at, ch)| at + ch.len_utf8());
        let name = &part[name_at..];
        if !name.is_empty() && name_at > 0 && !name.starts_with(|ch: char| ch.is_ascii_digit()) {
            finder.add(name, "field", from + name_at);
            if let Some(def) = finder.defs.last_mut() {
                def.depth += 1;
            }
        }
    }
}

fn kotlin_line(finder: &mut Finder, line: &str, depth: i32) {
    if let Some(found) = KT_TYPE.captures(line) {
        let name = found.get(2).unwrap();
        let kind = if &found[1] == "interface" { "interface" } else if line.contains("enum class") { "enum" } else { "class" };
        finder.add(name.as_str(), kind, name.start());
        finder.open(name.as_str(), kind, depth, true);
    } else if let Some(name) = KT_FUN.captures(line).and_then(|found| found.get(1)) {
        let kind = if finder.scopes.last().is_some_and(|scope| scope.typeish) { "method" } else { "function" };
        finder.add(name.as_str(), kind, name.start());
        finder.open(name.as_str(), "function", depth, false);
    } else if let Some(name) = KT_ALIAS.captures(line).and_then(|found| found.get(1)) {
        finder.add(name.as_str(), "type", name.start());
    } else if KT_COMPANION.is_match(line) {
        // 이름 없는 `companion object { … }`: 그 안의 상수·함수는 감싼 클래스의 것이다.
        let owner = finder.container();
        finder.open(&owner, "object", depth, true);
    } else if finder.scopes.is_empty() && depth == 0 || finder.member_of(depth).is_some() {
        if let Some(name) = KT_PROP.captures(line).and_then(|found| found.get(1)) {
            let kind = if line.contains("const val") { "constant" } else if finder.scopes.is_empty() { "variable" } else { "field" };
            finder.add(name.as_str(), kind, name.start());
        } else if finder.scopes.last().is_some_and(|scope| scope.kind == "enum") {
            if let Some(name) = JV_ENUM_CONST.captures(line).and_then(|found| found.get(1)) {
                finder.add(name.as_str(), "constant", name.start());
            }
        }
    }
}

fn js_line(finder: &mut Finder, line: &str, depth: i32) {
    if let Some(name) = JS_FUNC.captures(line).and_then(|found| found.get(1)) {
        finder.add(name.as_str(), "function", name.start());
        finder.open(name.as_str(), "function", depth, false);
        return;
    }
    if let Some(name) = JS_CLASS.captures(line).and_then(|found| found.get(1)) {
        finder.add(name.as_str(), "class", name.start());
        finder.open(name.as_str(), "class", depth, true);
        return;
    }
    if let Some(found) = TS_TYPE.captures(line) {
        let (word, name, next) = (&found[1], found.get(2).unwrap(), &found[3]);
        // `type`은 예약어가 아니라서 변수 이름으로도 쓰인다. `type 이름 =`·`type 이름<`일 때만 자료형 선언이다.
        if word != "type" || next == "=" || next == "<" {
            let kind = match word {
                "interface" => "interface",
                "enum" => "enum",
                "type" => "type",
                _ => "module",
            };
            finder.add(name.as_str(), kind, name.start());
            if kind != "type" {
                finder.open(name.as_str(), kind, depth, true);
            }
            return;
        }
    }
    // 맨 바깥(또는 클래스·네임스페이스·객체의 바로 안쪽)의 const·let·var만 정의로 친다. 함수 안의 지역 변수는 화면이 그 파일 안에서 찾는다.
    let outer = finder.scopes.is_empty() && depth == 0 || finder.member_of(depth).is_some();
    if outer {
        if let Some(found) = JS_VAR.captures(line) {
            let name = found.get(2).unwrap();
            let value = found.get(4).map_or("", |value| value.as_str()).trim();
            let function = JS_ARROW.is_match(value);
            let kind = if function { "function" } else if &found[1] == "const" { "constant" } else { "variable" };
            finder.add(name.as_str(), kind, name.start());
            if function {
                finder.open(name.as_str(), "function", depth, false);
            } else if value.ends_with('{') && !["=>", ")"].iter().any(|end| value.trim_end_matches('{').trim_end().ends_with(end)) {
                // 객체를 담는 이름: 그 안의 메서드가 이 이름을 감싼 이름으로 갖는다.
                // 콜백을 여는 줄(`const out = items.map((item) => {`, `… = run(function () {`)은 객체가 아니다:
                // 그 안의 지역 변수가 `out.name` 같은 정의로 올라가면 안 된다.
                finder.open(name.as_str(), "object", depth, true);
            }
            return;
        }
    }
    if let Some(name) = JS_PROP_FN.captures(line).and_then(|found| found.get(1)) {
        if !JS_NOT_NAMES.contains(&name.as_str()) {
            let kind = if finder.scopes.last().is_some_and(|scope| scope.typeish) { "method" } else { "function" };
            finder.add(name.as_str(), kind, name.start());
            finder.open(name.as_str(), "function", depth, false);
            return;
        }
    }
    if let Some(name) = JS_METHOD.captures(line).and_then(|found| found.get(1)) {
        if !JS_NOT_NAMES.contains(&name.as_str()) {
            let kind = if name.as_str() == "constructor" { "constructor" } else { "method" };
            finder.add(name.as_str(), kind, name.start());
            finder.open(name.as_str(), "function", depth, false);
            return;
        }
    }
    if let Some(name) = JS_EXPORTS.captures(line).and_then(|found| found.get(1)) {
        finder.add(name.as_str(), "variable", name.start());
    }
}

fn go_line(finder: &mut Finder, line: &str, depth: i32) {
    // 묶음 선언: `const (` … `)`, `var (` … `)`, `type (` … `)`. 그 안의 줄마다 맨 앞 이름이 정의다.
    if let Some(found) = GO_GROUP.captures(line) {
        finder.group = match &found[1] {
            "const" => "constant",
            "var" => "variable",
            _ => "type",
        };
        finder.group_parens = 0;
        return;
    }
    if !finder.group.is_empty() {
        let inside = finder.group_parens > 0; // 앞줄에서 연 호출의 인자 안
        let opened = line.matches(['(', '[']).count() as i32 - line.matches([')', ']']).count() as i32;
        if !inside && line.starts_with(')') {
            finder.group = "";
            return;
        }
        finder.group_parens = (finder.group_parens + opened).max(0);
        if inside {
            return;
        }
        if depth == 0 {
            if let Some(name) = GO_GROUPED.captures(line).and_then(|found| found.get(1)) {
                let kind = match finder.group {
                    "type" if line.contains("interface") => "interface",
                    "type" if line.contains("struct") => "struct",
                    other => other,
                };
                finder.add(name.as_str(), kind, name.start());
            }
        }
        return;
    }
    if let Some(found) = GO_FUNC.captures(line) {
        let name = found.get(2).unwrap();
        let receiver = found.get(1).map(|receiver| receiver.as_str().to_string());
        let col_at = name.start();
        match receiver {
            Some(receiver) => {
                // 받는 자료형이 감싼 이름이다.
                finder.open(&receiver, "struct", depth - 1, true);
                finder.add(name.as_str(), "method", col_at);
                finder.scopes.pop();
            }
            None => finder.add(name.as_str(), "function", col_at),
        }
        finder.open(name.as_str(), "function", depth, false);
    } else if depth <= 1 && !finder.scopes.iter().any(|scope| !scope.typeish) {
        if let Some(name) = GO_TYPE.captures(line).and_then(|found| found.get(1)) {
            let kind = if line.contains("interface") { "interface" } else if line.contains("struct") { "struct" } else { "type" };
            finder.add(name.as_str(), kind, name.start());
        } else if let Some(name) = GO_VAR.captures(line).and_then(|found| found.get(1)) {
            finder.add(name.as_str(), if line.starts_with("const") { "constant" } else { "variable" }, name.start());
        }
    }
}

fn css_line(finder: &mut Finder, line: &str, depth: i32) {
    let first = finder.defs.len();
    // 한 줄에 선택자와 선언이 함께 있을 수 있다(`:root { --gap: 4px; }`). 찾은 것은 나온 차례대로 적는다.
    for found in CSS_VAR.captures_iter(line) {
        let name = found.get(1).unwrap();
        finder.add(name.as_str(), "variable", name.start());
    }
    css_selectors(finder, line, depth);
    finder.defs[first..].sort_by_key(|def| def.col);
}

fn css_selectors(finder: &mut Finder, line: &str, depth: i32) {
    if let Some(found) = CSS_AT.captures(line) {
        let name = found.get(2).unwrap();
        finder.add(name.as_str(), "selector", name.start());
        return;
    }
    // 선택자가 있는 자리: `{` 앞, 또는 쉼표로 끝나 다음 줄로 이어지는 선택자 줄(값의 쉼표와 가리려고 `이름: 값` 꼴은 뺀다).
    let selector = match line.find('{') {
        Some(brace) => &line[..brace],
        // 중괄호 안에서는 여러 줄에 걸친 값(`box-shadow:` 아래의 `0 0 4px #fafafa,`)도 쉼표로 끝난다. 선택자답게 시작하는 줄만 본다.
        None if line.trim_end().ends_with(',') && !line.contains(": ") && !line.contains('(') && (depth == 0 || line.trim_start().starts_with(['.', '&', '*', '[', ':', '>', '~', '+'])) => line,
        None => return,
    };
    if selector.trim_start().starts_with('@') {
        return;
    }
    for found in CSS_CLASS.captures_iter(selector) {
        let name = found.get(1).unwrap();
        finder.add(name.as_str(), "selector", name.start());
    }
    for found in CSS_ID.captures_iter(selector) {
        let name = found.get(1).unwrap();
        finder.add(name.as_str(), "id", name.start());
    }
}

/// Python: 들여쓰기로 감싼 것을 가린다.
fn python_defs(original: &str, code: &str) -> Vec<Def> {
    let mut finder = Finder { defs: Vec::new(), scopes: Vec::new(), original: "", line: 0, parens: 0, group: "", group_parens: 0 };
    let mut written = 0; // 마지막으로 글이 있던 줄
    let mut open: i32 = 0; // 열려 있는 괄호 수(여러 줄에 걸친 서명·호출 안인지)
    for (index, (line, original_line)) in code.split('\n').zip(original.split('\n')).enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        finder.line = index as u32 + 1;
        finder.original = original_line;
        let indent = (line.len() - line.trim_start().len()) as i32;
        // 괄호 안의 줄과 닫는 괄호로 시작하는 줄(`) -> User:`)은 들여쓰기가 얕아도 앞의 정의를 닫지 않는다.
        let continued = open > 0 || line.trim_start().starts_with([')', ']', '}']);
        open = (open + line.matches(['(', '[', '{']).count() as i32 - line.matches([')', ']', '}']).count() as i32).max(0);
        if continued {
            written = finder.line;
            continue;
        }
        while finder.scopes.last().is_some_and(|scope| indent <= scope.depth) {
            finder.close(written);
        }
        written = finder.line;
        if let Some(found) = PY_DEF.captures(line) {
            let name = found.get(3).unwrap();
            let class = &found[2] == "class";
            let kind = if class { "class" } else if finder.scopes.last().is_some_and(|scope| scope.typeish) { "method" } else { "function" };
            finder.add(name.as_str(), kind, name.start());
            finder.open(name.as_str(), if class { "class" } else { "function" }, indent, class);
            // 들여쓰기로 몸통을 가리는 언어라 여는 표시가 따로 없다. 선언한 것으로 몸통이 시작된 것으로 친다.
            if let Some(scope) = finder.scopes.last_mut() {
                scope.entered = true;
            }
        } else if let Some(name) = PY_VAR.captures(line).and_then(|found| found.get(1)) {
            let constant = name.as_str().chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
            finder.add(name.as_str(), if constant { "constant" } else { "variable" }, name.start());
        }
    }
    finder.close_all(written);
    finder.defs
}

/// 줄마다 따로 보는 것: HTML의 id, Markdown의 제목, 설정 파일의 구역.
fn line_defs(lang: Lang, original: &str, code: &str) -> Vec<Def> {
    let mut finder = Finder { defs: Vec::new(), scopes: Vec::new(), original: "", line: 0, parens: 0, group: "", group_parens: 0 };
    let mut fence = String::new(); // 열려 있는 코드 울타리(```·~~~). 같은 글자로 그만큼 이상 길어야 닫힌다.
    for (index, (line, original_line)) in code.split('\n').zip(original.split('\n')).enumerate() {
        finder.line = index as u32 + 1;
        finder.original = original_line;
        match lang {
            Lang::Html => {
                for found in HTML_ID.captures_iter(line) {
                    let name = found.get(2).unwrap();
                    finder.add(name.as_str(), "id", name.start());
                }
            }
            Lang::Markdown => {
                let lead = line.trim_start();
                let mark: String = lead.chars().take_while(|&c| c == lead.chars().next().unwrap_or(' ') && (c == '`' || c == '~')).collect();
                if mark.len() >= 3 {
                    if fence.is_empty() {
                        fence = mark;
                    } else if mark.starts_with(fence.as_str()) && lead[mark.len()..].trim().is_empty() {
                        fence.clear();
                    }
                } else if fence.is_empty() {
                    if let Some(found) = MD_HEAD.captures(line) {
                        let name = found.get(2).unwrap();
                        let level = found[1].len() as u32;
                        finder.add(name.as_str(), "heading", name.start());
                        if let Some(def) = finder.defs.last_mut() {
                            def.depth = level - 1;
                        }
                    }
                }
            }
            Lang::Xml => {
                // MyBatis 매퍼: `<mapper namespace="a.b.OrderMapper">` 안의 구문은 그 인터페이스(OrderMapper)의 메서드와 짝이다.
                if let Some(space) = XML_NAMESPACE.captures(line).and_then(|found| found.get(1)) {
                    let owner = space.as_str().rsplit('.').next().unwrap_or("");
                    finder.scopes.clear();
                    finder.scopes.push(Scope { name: owner.to_string(), kind: "interface", depth: 0, entered: true, line: finder.line, typeish: true, def: None });
                }
                for found in XML_ID.captures_iter(line) {
                    let name = found.get(2).unwrap();
                    let kind = if matches!(&found[1], "select" | "insert" | "update" | "delete") { "method" } else { "id" };
                    finder.add(name.as_str(), kind, name.start());
                    if let Some(def) = finder.defs.last_mut() {
                        def.depth = 0;
                    }
                }
            }
            Lang::Config => {
                // 여러 줄 배열의 한 줄(`["a", "b"],`)은 구역이 아니다.
                if let Some(name) = CONFIG_SECTION.captures(line).and_then(|found| found.get(1)).filter(|name| !name.as_str().contains([',', '"', '\''])) {
                    finder.add(name.as_str(), "section", name.start());
                }
            }
            _ => {}
        }
    }
    finder.defs
}

// ───────────────────────── 서명, 상위 자료형, 구현 ─────────────────────────

const MAX_SIGNATURE: usize = 600;

/// 파일 하나에서 정의들의 서명을 읽어 주는 것(주석·문자열을 한 번만 가린다).
pub struct Signer {
    code: String,
    starts: Vec<usize>,
    python: bool,
    /// 선언이 줄 끝에서 끝날 수 있는 언어인지(코드 언어). XML·HTML의 여러 줄 태그는 예전처럼 끝까지 읽는다.
    line_ends: bool,
}

impl Signer {
    pub fn new(lang: Lang, text: &str) -> Self {
        let line_ends = matches!(lang, Lang::Rust | Lang::Java | Lang::Kotlin | Lang::Js | Lang::Vue | Lang::Python | Lang::Go | Lang::C);
        Self { code: code_only(text, &mask(lang, text)), starts: line_starts(text), python: lang == Lang::Python, line_ends }
    }

    /// 정의의 서명: 정의한 줄의 처음부터 몸통이 시작하는 여는 중괄호나 문장이 끝나는 ';' 앞까지. 여러 줄이면 한 줄로 잇는다.
    /// (반환형·인자 수·상위 자료형을 읽는 데 쓴다. 주석과 문자열은 빈칸이다.)
    pub fn of(&self, def: &Def) -> String {
        let Some(&from) = self.starts.get(def.line.saturating_sub(1) as usize) else { return String::new() };
        let rest = &self.code[from..];
        let one_line = !matches!(def.kind, "function" | "method" | "constructor" | "class" | "interface" | "enum" | "struct" | "trait" | "type");
        let mut out = String::new();
        let mut parens = 0i32;
        let mut space = true;
        let mut where_clause = false;
        let mut open_tail = false;
        let ahead: Vec<&str> = rest.split('\n').take(24).collect();
        for (lines, line) in ahead.iter().copied().enumerate() {
            if lines >= 16 || (one_line && lines >= 1) {
                break;
            }
            // 세미콜론을 쓰지 않는 언어의 몸통 없는 선언(Kotlin의 `data class User(…)`, 인터페이스의 `fun find(): User`,
            // Go 인터페이스의 메서드, `def f(): ...`)은 줄 끝에서 끝난다. 괄호가 닫힌 채 줄이 끝났으면, 이 줄이나 다음 줄이
            // 선언을 잇는 꼴일 때만 더 읽는다(그러지 않으면 아래 선언이 서명에 붙어 반환형·상위 자료형을 잘못 읽는다).
            let next = ahead[lines + 1..].iter().find(|next| !next.trim().is_empty()).map_or("", |next| next.trim_start());
            let tail = line.trim_end();
            where_clause = where_clause || tail.ends_with(" where") || tail.trim_start() == "where" || next.starts_with("where");
            // 줄 끝이 잇는 꼴인지는 빈 줄(주석만 있던 줄)을 건너 이어진다: `class Msg extends` / `// 주석` / `Base {`.
            if !tail.trim().is_empty() {
                open_tail = tail.ends_with([',', ':', '=', '(', '[', '+', '|', '&'])
                    || ["->", "=>", " extends", " implements", " throws"].iter().any(|end| tail.ends_with(end));
            }
            let goes_on = where_clause
                || open_tail
                || next.starts_with(['{', '.', ')', ',', ':', '=', '-', '|', '&', '?', '+', '<'])
                || ["throws", "extends", "implements", "by "].iter().any(|word| next.starts_with(word));
            let mut done = false;
            for ch in line.chars() {
                match ch {
                    '(' | '[' => parens += 1,
                    ')' | ']' => parens = (parens - 1).max(0),
                    '{' | ';' if parens == 0 => {
                        done = true;
                        break;
                    }
                    _ => {}
                }
                if ch.is_whitespace() {
                    if !space {
                        out.push(' ');
                    }
                    space = true;
                } else {
                    out.push(ch);
                    space = false;
                }
            }
            if done || out.len() >= MAX_SIGNATURE || (self.python && parens == 0 && line.trim_end().ends_with(':')) {
                break;
            }
            if self.line_ends && parens == 0 && !goes_on {
                break;
            }
            if !space {
                out.push(' ');
                space = true;
            }
        }
        let mut cut = out.trim_end().len().min(MAX_SIGNATURE);
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
        out
    }
}

/// 짝이 맞는 <…>(과 skip_parens이면 (…)) 안의 글을 뺀다.
fn without_groups(text: &str, skip_parens: bool) -> String {
    let mut out = String::new();
    let (mut angle, mut paren) = (0i32, 0i32);
    let mut before = ' ';
    for ch in text.chars() {
        let arrow = ch == '>' && (before == '-' || before == '=');
        before = ch;
        match ch {
            '<' => angle += 1,
            '>' if !arrow && angle > 0 => angle -= 1,
            '(' if skip_parens => paren += 1,
            ')' if skip_parens && paren > 0 => paren -= 1,
            _ if angle == 0 && paren == 0 => out.push(ch),
            _ => {}
        }
    }
    out
}

fn last_segment(path: &str) -> &str {
    path.trim().rsplit(['.', ':']).next().unwrap_or("").trim()
}

fn is_type_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '$') && !name.starts_with(|ch: char| ch.is_ascii_digit())
}

/// 자료형 정의의 서명에서 상위 자료형(물려받는 클래스, 구현하는 인터페이스, 트레이트의 상위 트레이트)의 이름을 읽는다.
pub fn supertypes(lang: Lang, sig: &str, name: &str) -> Vec<String> {
    // 이름 바로 뒤부터 본다.
    let Some(at) = sig.match_indices(name).map(|(at, _)| at).find(|&at| {
        let before = sig[..at].chars().next_back();
        let after = sig[at + name.len()..].chars().next();
        !before.is_some_and(|ch| ch.is_alphanumeric() || ch == '_') && !after.is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
    }) else {
        return Vec::new();
    };
    let rest = &sig[at + name.len()..];
    let mut found: Vec<String> = Vec::new();
    let mut add = |part: &str| {
        let part = part.trim();
        let word = part.rsplit(' ').next().unwrap_or(""); // public Base, virtual Base
        let word = last_segment(word);
        if is_type_name(word) && !matches!(word, "public" | "private" | "protected" | "virtual" | "object" | "Object") && !found.iter().any(|each| each == word) {
            found.push(word.to_string());
        }
    };
    match lang {
        Lang::Python => {
            // class A(B, C, metaclass=M):
            let rest = rest.trim_start();
            if let Some(inner) = rest.strip_prefix('(').and_then(|inner| inner.split(')').next()) {
                for part in without_groups(inner, false).split(',').filter(|part| !part.contains('=')) {
                    add(part);
                }
            }
        }
        Lang::Rust => {
            // trait A: B + C<T> where …
            let rest = without_groups(rest, true);
            let rest = rest.split(" where ").next().unwrap_or("");
            if let Some(bounds) = rest.trim_start().strip_prefix(':') {
                for part in bounds.split('+').filter(|part| !part.trim_start().starts_with(['\'', '?'])) {
                    add(part);
                }
            }
        }
        Lang::Java | Lang::Kotlin | Lang::Js | Lang::Vue | Lang::C => {
            // class A<T> extends B<T> implements C, D / class A(…) : B(), C / class A : public B
            let rest = without_groups(rest, true);
            let rest = rest.split(" permits ").next().unwrap_or("");
            let rest = rest.split(" where ").next().unwrap_or("");
            let start = [" extends ", " implements ", ":"].iter().filter_map(|mark| rest.find(mark).map(|at| at + mark.len())).min();
            if let Some(start) = start {
                for part in rest[start..].replace(" extends ", ",").replace(" implements ", ",").split(',') {
                    // Kotlin의 위임(`: Base by impl`)에서 `by` 뒤는 자료형이 아니다.
                    add(if lang == Lang::Kotlin { part.split(" by ").next().unwrap_or(part) } else { part });
                }
            }
        }
        _ => {}
    }
    found
}

/// impl 뒤의 글에서 구현하는 트레이트의 이름을 고른다: <T> fmt::Display for Type<T> → Display. 트레이트 구현이 아니면 빈 글.
fn impl_trait(rest: &str) -> &str {
    let rest = rest.trim_start();
    // 맨 앞의 <…>를 건너뛴다.
    let rest = if rest.starts_with('<') {
        let mut depth = 0;
        let mut end = rest.len();
        let mut before = ' ';
        for (at, ch) in rest.char_indices() {
            let arrow = ch == '>' && before == '-'; // Fn() -> u32
            before = ch;
            match ch {
                '<' => depth += 1,
                '>' if arrow => {}
                '>' => {
                    depth -= 1;
                    if depth == 0 {
                        end = at + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        &rest[end..]
    } else {
        rest
    };
    let Some(at) = rest.find(" for ") else { return "" };
    let head = rest[..at].trim().trim_start_matches('!');
    let path = head.split(['<', ' ', '(']).next().unwrap_or("");
    path.rsplit("::").next().unwrap_or("")
}

/// 이 파일에서 name을 물려받거나 구현하는 것들: 자료형 정의(클래스·인터페이스·트레이트)와, Rust의 impl name for 자료형.
/// impl은 종류가 "impl"이고 이름이 구현하는 자료형, 감싼 이름이 트레이트다. 함께 내놓는 글은 그 서명이다.
pub fn subtypes(lang: Lang, text: &str, name: &str) -> Vec<(Def, String)> {
    if name.is_empty() || !text.contains(name) {
        return Vec::new();
    }
    let signer = Signer::new(lang, text);
    let mut found: Vec<(Def, String)> = definitions(lang, text)
        .into_iter()
        .filter(|def| matches!(def.kind, "class" | "interface" | "enum" | "struct" | "trait"))
        .filter_map(|def| {
            let sig = signer.of(&def);
            supertypes(lang, &sig, &def.name).iter().any(|each| each == name).then_some((def, sig))
        })
        .collect();
    if lang == Lang::Rust {
        let lines: Vec<&str> = signer.code.split('\n').collect();
        let originals: Vec<&str> = text.split('\n').collect();
        for (index, line) in lines.iter().copied().enumerate() {
            let Some(rest) = RS_IMPL.captures(line).and_then(|found| found.get(1)) else { continue };
            // 머리가 여러 줄이면(rustfmt는 긴 머리를 `for` 앞에서 나눈다) 몸통이 시작하는 `{` 전까지 이어 붙여 읽는다.
            let mut head = rest.as_str().to_string();
            let mut last = index;
            while !lines[last].contains(['{', ';']) && last + 1 < lines.len() && last - index < 8 {
                last += 1;
                head.push(' ');
                head.push_str(lines[last].trim());
            }
            if impl_trait(&head) != name {
                continue;
            }
            let target = impl_target(&head);
            if target.is_empty() {
                continue;
            }
            // 구현하는 자료형이 적힌 자리: `for` 뒤에서 처음 나온 곳(where 절에 같은 이름이 또 나올 수 있다).
            let spot = (index..=last).find_map(|row| {
                let from = if row == index { rest.start() } else { 0 };
                let hay = &lines[row][from..];
                let indent = hay.len() - hay.trim_start().len();
                let after = hay.find(" for ").map(|at| at + 5).or_else(|| hay.trim_start().starts_with("for ").then_some(indent + 4))?;
                Some((row, from + after + hay[after..].find(target)?))
            });
            let (row, at) = spot.unwrap_or((index, rest.start() + rest.as_str().find(target).unwrap_or(0)));
            // 칸은 원래 글에서 센다(가린 글은 주석·문자열의 여러 바이트 글자가 바이트 수만큼의 빈칸이라 칸이 밀린다).
            let before = originals.get(row).and_then(|original| original.get(..at)).or_else(|| lines[row].get(..at));
            let col = before.map_or(0, |before| before.encode_utf16().count()) as u32;
            let line_number = row as u32 + 1;
            let def = Def { name: target.to_string(), kind: "impl", line: line_number, col, container: name.to_string(), depth: 0, end: line_number };
            let sig = lines[index..=last].iter().map(|each| each.trim()).collect::<Vec<_>>().join(" ");
            found.push((def, sig.chars().take(MAX_SIGNATURE).collect()));
        }
        found.sort_by_key(|(def, _)| (def.line, def.col));
    }
    found
}

// ───────────────────────── 쓰인 곳 찾기 ─────────────────────────

/// 이름이 나온 자리 하나.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hit {
    /// 줄 번호(1부터)와 칸·길이(UTF-16 단위).
    pub line: u32,
    pub col: u32,
    pub len: u32,
    /// def(정의), use(쓰는 곳), comment(주석 안), string(문자열 안), text(글 찾기의 결과)
    pub role: &'static str,
    /// 그 줄의 글(길면 앞뒤를 잘라 낸다).
    pub text: String,
    /// `text`에서 이름이 시작하는 칸(UTF-16 단위).
    pub at: u32,
    /// 그 자리를 감싼 정의(`클래스.메서드`). 쓰는 곳 찾기에서만 채운다. 없으면 빈 글.
    pub within: String,
}

const MAX_LINE_SHOWN: usize = 200;
/// 파일 하나에서 찾아 주는 자리 수(아주 흔한 이름이 든 큰 파일에서 결과가 끝없이 불어나지 않게).
const MAX_FILE_HITS: usize = 3000;

/// 줄의 글을 보여 줄 만큼만 남긴다: 앞의 빈칸을 떼고, 길면 찾은 자리 둘레만 남긴다. (글, 그 글에서 찾은 자리의 칸)
fn shown_line(line: &str, start: usize, len: usize) -> (String, u32) {
    let trimmed = line.trim_start();
    let cut = line.len() - trimmed.len();
    let mut from = cut.min(start);
    if start - from > 80 {
        from = start - 60;
        while !line.is_char_boundary(from) {
            from -= 1;
        }
    }
    let mut to = line.len().min((start + len).max(from + MAX_LINE_SHOWN));
    while !line.is_char_boundary(to) {
        to -= 1;
    }
    let to = to.max(start + len).min(line.len());
    let at = line[from..start].encode_utf16().count() as u32;
    (line[from..to].trim_end().to_string(), at)
}

/// 이름을 이루는 글자인지. `dash`이면 `-`도 이름의 일부다(CSS의 class 이름, HTML의 속성 이름).
/// 찾는 이름이 영문이면 영문·숫자·`_`·`$`만 이름의 글자로 본다: 한국어 주석에서 이름 뒤에 조사가 붙은 것("load를")도 찾게 한다.
/// 이름에 그 밖의 글자(한글 등)가 들어 있으면(`wide`) 모든 글자·숫자를 이름의 글자로 본다.
fn is_name_char(ch: char, dash: bool, wide: bool) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' || (dash && ch == '-') || (wide && !ch.is_ascii() && ch.is_alphanumeric())
}

/// 글 안의 줄 시작 자리들.
fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0).chain(text.match_indices('\n').map(|(at, _)| at + 1)).collect()
}

/// 그 언어의 파일에서 이름을 견줄 때 `-`를 이름의 일부로 볼지.
pub fn dash_names(lang: Lang) -> bool {
    matches!(lang, Lang::Css | Lang::Html | Lang::Vue)
}

/// 이름이 낱말로 나온 자리를 모두 찾는다(다른 이름의 일부로 들어 있는 것은 뺀다).
pub fn occurrences(lang: Lang, text: &str, name: &str) -> Vec<Hit> {
    if name.is_empty() || !text.contains(name) {
        return Vec::new();
    }
    let dash = dash_names(lang) || name.contains('-');
    let wide = !name.is_ascii();
    let starts = line_starts(text);
    let kinds = mask(lang, text);
    let mut defs: Option<Vec<Def>> = None;
    let mut hits = Vec::new();
    for (at, _) in text.match_indices(name) {
        let before = text[..at].chars().next_back();
        let after = text[at + name.len()..].chars().next();
        if before.is_some_and(|ch| is_name_char(ch, dash, wide)) || after.is_some_and(|ch| is_name_char(ch, dash, wide)) {
            continue;
        }
        let line_index = starts.partition_point(|&start| start <= at) - 1;
        let line_start = starts[line_index];
        let line_end = starts.get(line_index + 1).map_or(text.len(), |next| next - 1);
        let line = &text[line_start..line_end];
        let col = text[line_start..at].encode_utf16().count() as u32;
        let line_number = line_index as u32 + 1;
        let role = match kinds[at] {
            COMMENT => "comment",
            STRING => "string",
            _ => {
                let defs = defs.get_or_insert_with(|| definitions(lang, text));
                if defs.iter().any(|def| def.line == line_number && def.col == col && def.name == name) {
                    "def"
                } else {
                    "use"
                }
            }
        };
        let (shown, shown_at) = shown_line(line, at - line_start, name.len());
        hits.push(Hit { line: line_number, col, len: name.encode_utf16().count() as u32, role, text: shown, at: shown_at, within: String::new() });
        if hits.len() >= MAX_FILE_HITS {
            break;
        }
    }
    // 찾은 자리마다 어느 정의 안인지 적는다(누가 부르는지 한눈에 보인다).
    if !hits.is_empty() && lang != Lang::Text {
        let defs = defs.unwrap_or_else(|| definitions(lang, text));
        for hit in &mut hits {
            hit.within = within(&defs, hit.line, hit.col);
        }
    }
    hits
}

/// 그 자리를 감싼 가장 안쪽의 정의 이름(`감싼 것.이름`). 그 자리가 바로 그 정의의 이름이면 그것을 감싼 것을 내놓는다.
fn within(defs: &[Def], line: u32, col: u32) -> String {
    defs.iter()
        .filter(|def| matches!(def.kind, "function" | "method" | "constructor" | "class" | "interface" | "enum" | "struct" | "trait" | "module"))
        .filter(|def| def.line <= line && def.end >= line && !(def.line == line && def.col == col))
        .max_by_key(|def| (def.line, def.depth))
        .map(|def| if def.container.is_empty() || def.container == def.name { def.name.clone() } else { format!("{}.{}", def.container, def.name) })
        // 감싼 정의가 없는 정의 줄(impl 안의 메서드 — impl은 정의가 아니다)은 그 감싼 이름을 적는다.
        .or_else(|| defs.iter().find(|def| def.line == line && def.col == col).map(|def| def.container.clone()))
        .unwrap_or_default()
}

/// 글 찾기: 정규식(`pattern`)에 맞는 자리를 모두 찾는다. 빈 글에 맞는 것은 뺀다.
pub fn search(text: &str, pattern: &Regex, limit: usize) -> Vec<Hit> {
    let starts = line_starts(text);
    let mut hits = Vec::new();
    for found in pattern.find_iter(text) {
        if found.is_empty() {
            continue;
        }
        let at = found.start();
        let line_index = starts.partition_point(|&start| start <= at) - 1;
        let line_start = starts[line_index];
        let line_end = starts.get(line_index + 1).map_or(text.len(), |next| next - 1);
        // 여러 줄에 걸친 것은 첫 줄의 것만 보여 준다.
        let end = found.end().min(line_end);
        let line = &text[line_start..line_end];
        let (shown, shown_at) = shown_line(line, at - line_start, end - at);
        hits.push(Hit {
            line: line_index as u32 + 1,
            col: text[line_start..at].encode_utf16().count() as u32,
            len: text[at..end].encode_utf16().count() as u32,
            role: "text",
            text: shown,
            at: shown_at,
            within: String::new(),
        });
        if hits.len() >= limit {
            break;
        }
    }
    hits
}

/// 글 찾기에 쓸 정규식을 만든다. `regex`가 아니면 적은 글자 그대로 찾는다.
pub fn search_pattern(query: &str, case: bool, word: bool, regex: bool) -> Result<Regex, String> {
    let body = if regex { query.to_string() } else { regex::escape(query) };
    let body = if word { format!(r"\b(?:{body})\b") } else { body };
    regex::RegexBuilder::new(&body).case_insensitive(!case).multi_line(true).size_limit(1 << 20).build().map_err(|e| format!("정규식을 읽지 못했습니다: {e}"))
}

/// 기호 찾기: 적은 글(`query`)이 이름과 얼마나 잘 맞는지. 작을수록 잘 맞는다. 맞지 않으면 None.
/// 0 똑같음 · 1 대/소문자만 다름 · 2 앞부분이 같음 · 3 낱말 머리글자가 차례로 맞음(`gF` → `getFoo`) · 4 안에 들어 있음 · 5 글자가 차례로 들어 있음
pub fn rank(query: &str, name: &str) -> Option<u32> {
    if query.is_empty() {
        return None;
    }
    if query == name {
        return Some(0);
    }
    let (q, n) = (query.to_lowercase(), name.to_lowercase());
    if q == n {
        return Some(1);
    }
    if n.starts_with(&q) {
        return Some(2);
    }
    // 낱말 머리글자: 대문자, `_`·`-` 뒤의 글자, 맨 앞 글자.
    let chars: Vec<char> = name.chars().collect();
    let heads: String = chars
        .iter()
        .enumerate()
        .filter(|(index, ch)| *index == 0 || ch.is_uppercase() || matches!(chars[index - 1], '_' | '-' | '.'))
        .map(|(_, ch)| ch.to_lowercase().next().unwrap_or(*ch))
        .collect();
    if heads.starts_with(&q) {
        return Some(3);
    }
    if n.contains(&q) {
        return Some(4);
    }
    let mut wanted = q.chars().peekable();
    for ch in n.chars() {
        if wanted.peek() == Some(&ch) {
            wanted.next();
        }
    }
    wanted.peek().is_none().then_some(5)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(lang: Lang, text: &str) -> Vec<String> {
        definitions(lang, text).into_iter().map(|def| format!("{}:{}{}", def.kind, if def.container.is_empty() { String::new() } else { format!("{}.", def.container) }, def.name)).collect()
    }

    #[test]
    fn 파일_이름으로_언어를_가린다() {
        assert_eq!(lang_of("src/main.rs"), Some(Lang::Rust));
        assert_eq!(lang_of(r"C:\a\B.JAVA"), Some(Lang::Java));
        assert_eq!(lang_of("App.vue"), Some(Lang::Vue));
        assert_eq!(lang_of("a.d.ts"), Some(Lang::Js));
        assert_eq!(lang_of("style.scss"), Some(Lang::Css));
        assert_eq!(lang_of("Dockerfile"), Some(Lang::Text));
        assert_eq!(lang_of("Cargo.toml"), Some(Lang::Config));
        for other in ["a.png", "a.exe", "a", "archive.zip", "x.class"] {
            assert_eq!(lang_of(other), None, "{other}");
        }
    }

    #[test]
    fn 주석과_문자열을_가린다() {
        let text = "let a = \"x // y\"; // 주석\nlet b = 'c'; /* 여러\n줄 */ let c: &'static str = r#\"날 \"글\" \"#;";
        let code = code_only(text, &mask(Lang::Rust, text));
        assert_eq!(code.len(), text.len());
        assert!(code.starts_with(&format!("let a = {};", " ".repeat(8))));
        assert!(!code.contains("주석") && !code.contains("여러") && !code.contains("날"));
        assert!(code.contains("&'static str")); // 수명은 문자열이 아니다
        assert!(code.contains("let c:"));
        // JavaScript: 여러 줄에 걸친 `…`, 닫히지 않은 따옴표는 줄 끝에서 끊는다.
        let js = "const a = `한\n줄 ${x}`; const b = 'it\nconst c = 1; // 끝";
        let code = code_only(js, &mask(Lang::Js, js));
        assert!(code.contains("const c = 1;") && !code.contains("끝") && !code.contains("한"));
        // CSS: 주소 안의 //는 주석이 아니다.
        let css = "a { background: url(http://x/y.png); } /* c */";
        assert!(code_only(css, &mask(Lang::Css, css)).contains("url(http://x/y.png)"));
        // Vue: <script> 안은 JavaScript로, 그 밖의 <!-- -->는 주석으로.
        let vue = "<template><!-- 주석 --><div id=\"a\"/></template>\n<script>\nconst s = 'x'; // c\n</script>";
        let code = code_only(vue, &mask(Lang::Vue, vue));
        assert!(code.contains("id=\"a\"") && !code.contains("주석") && code.contains("const s =") && !code.contains("// c"));
    }

    #[test]
    fn rust의_정의를_찾는다() {
        let text = r#"
pub struct Session { pub id: u32, }
pub struct Config {
    /// 문서
    pub name: String,
    retries: u8,
}
enum Launch {
    Exe(PathBuf),
    AsGiven,
}
impl Session {
    pub fn new(id: u32) -> Self { Self { id } }
    pub(crate) async fn run(&self) -> Result<()> {
        let name = 1;
        fn inner() {}
        Ok(())
    }
}
impl std::fmt::Display for Session {
    fn fmt(&self) {}
}
pub trait Shape {
    fn area(&self) -> f64;
}
const MAX: usize = 3;
static mut COUNT: u32 = 0;
type Alias = Vec<u8>;
mod tests {
    fn helper() {}
}
macro_rules! re { () => {}; }
fn main() { let x = "fn fake() {}"; } // fn commented() {}
"#;
        assert_eq!(
            names(Lang::Rust, text),
            [
                "struct:Session", "struct:Config", "field:Config.name", "field:Config.retries", "enum:Launch", "variant:Launch.Exe", "variant:Launch.AsGiven",
                "method:Session.new", "method:Session.run", "function:Session.inner", "method:Session.fmt", "trait:Shape", "method:Shape.area",
                "constant:MAX", "constant:COUNT", "type:Alias", "module:tests", "function:tests.helper", "macro:re", "function:main",
            ]
        );
        let defs = definitions(Lang::Rust, text);
        let run = defs.iter().find(|def| def.name == "run").unwrap();
        assert_eq!((run.line, run.col), (14, 24));
        // 몸통 없는 선언(트레이트의 메서드) 뒤에도 감싼 이름이 어긋나지 않는다.
        assert_eq!(defs.iter().find(|def| def.name == "MAX").unwrap().container, "");
    }

    #[test]
    fn java의_정의를_찾는다() {
        let text = r#"
package a.b;
import java.util.List;

@Service
public class OrderService extends Base implements Runnable {
    private static final int MAX_RETRY = 3;
    private final OrderRepository repository;
    private List<String> names = new ArrayList<>();

    public OrderService(OrderRepository repository) {
        this.repository = repository;
    }

    @Override
    public void run() {
        int count = 0;
        if (count > 0) { helper(count); }
        return;
    }

    protected <T> List<T> findAll(Class<T> type,
                                  int limit) throws IOException {
        return repository.find(type, limit);
    }

    abstract void later();

    enum Status { ACTIVE, CLOSED; }

    interface Listener {
        void onDone(String id);
    }
}
"#;
        assert_eq!(
            names(Lang::Java, text),
            [
                "class:OrderService", "constant:OrderService.MAX_RETRY", "field:OrderService.repository", "field:OrderService.names",
                "constructor:OrderService.OrderService", "method:OrderService.run", "method:OrderService.findAll", "method:OrderService.later",
                "enum:OrderService.Status", "interface:OrderService.Listener", "method:Listener.onDone",
            ]
        );
    }

    #[test]
    fn javascript와_typescript의_정의를_찾는다() {
        let text = r#"
import { x } from './x.js';
export const MAX = 3;
let count = 0;
export function load(a, b) {
  const local = 1;
  return inner();
}
const add = (a, b) => a + b;
export const api = {
  get(url) { return 1; },
  post: async (url) => {},
};
export default class Store extends Base {
  static #count = 0;
  constructor() { super(); }
  async save(item) {
    if (item) { go(item); }
  }
  remove = (id) => {};
}
interface Options { name: string }
type Id = string | number;
enum Color { Red }
window.addEventListener('x', () => { const hidden = 1; });
exports.legacy = 1;
"#;
        assert_eq!(
            names(Lang::Js, text),
            [
                "constant:MAX", "variable:count", "function:load", "function:add", "constant:api", "method:api.get", "method:api.post", "class:Store",
                "constructor:Store.constructor", "method:Store.save", "method:Store.remove", "interface:Options", "type:Id", "enum:Color", "variable:legacy",
            ]
        );
    }

    #[test]
    fn vue_파일은_구역마다_그_언어로_읽는다() {
        let text = "<template>\n  <div id=\"app\" ref=\"box\" class=\"card\"></div>\n</template>\n<script setup lang=\"ts\">\nconst count = ref(0)\nfunction go() {}\n</script>\n<style scoped>\n.card, .card-title { color: red; }\n#app { --gap: 4px; }\n</style>\n";
        assert_eq!(
            names(Lang::Vue, text),
            ["id:app", "id:box", "constant:count", "function:go", "selector:card", "selector:card-title", "id:app", "variable:--gap"]
        );
        let card = definitions(Lang::Vue, text).into_iter().find(|def| def.name == "card").unwrap();
        assert_eq!((card.line, card.col), (9, 1));
    }

    #[test]
    fn 그_밖의_언어의_정의를_찾는다() {
        let css = "@media (max-width: 10px) {\n  .a:hover,\n  #main > .b { color: #fff; margin: 0 .5em; }\n}\n:root { --gap: 4px; }\n@keyframes spin { from {} }\n";
        assert_eq!(names(Lang::Css, css), ["selector:a", "id:main", "selector:b", "variable:--gap", "selector:spin"]);
        let py = "MAX = 3\nclass Repo(Base):\n    def find(self, id):\n        x = 1\n        return x\n\nasync def main():\n    pass\n";
        assert_eq!(names(Lang::Python, py), ["constant:MAX", "class:Repo", "method:Repo.find", "function:main"]);
        let go = "package a\n\ntype Server struct {\n\tname string\n}\n\nfunc (s *Server) Run(port int) error {\n\treturn nil\n}\n\nfunc main() {}\nconst Max = 3\n";
        assert_eq!(names(Lang::Go, go), ["struct:Server", "method:Server.Run", "function:main", "constant:Max"]);
        let kt = "data class User(val name: String) {\n    val age: Int = 0\n    fun greet(): String {\n        val local = 1\n        return name\n    }\n}\nfun String.shout() = uppercase()\nconst val LIMIT = 3\n";
        assert_eq!(names(Lang::Kotlin, kt), ["class:User", "field:User.age", "method:User.greet", "function:shout", "constant:LIMIT"]);
        let c = "#define MAX 3\nstruct point {\n  int x;\n};\nstatic int add(int a, int b) {\n  return a + b;\n}\nint main(void) { return add(1, 2); }\n";
        assert_eq!(names(Lang::C, c), ["macro:MAX", "struct:point", "field:point.x", "function:add", "function:main"]);
        let md = "# 제목\n```\n# 코드 안\n```\n## 둘째 ##\n";
        assert_eq!(names(Lang::Markdown, md), ["heading:제목", "heading:둘째"]);
        assert_eq!(names(Lang::Config, "[package]\nname = \"x\"\n[[bin]] # 주석\n"), ["section:package", "section:bin"]);
    }

    #[test]
    fn 검토에서_나온_경우들() {
        // JavaScript의 정규식 리터럴 안의 //·따옴표·중괄호가 뒤의 정의를 가리지 않는다.
        let js = "const slash = (p) => p.replace(/\\//g, '/');\nconst quote = /[\"'`]/;\nconst brace = /\\{(\\w+)\\}/g;\nfunction after(x) {\n  if (/^\\//.test(x)) {\n    return x / 2 / 3;\n  }\n  const local = 2;\n}\nexport const last = 1;\n";
        assert_eq!(names(Lang::Js, js), ["function:slash", "constant:quote", "constant:brace", "function:after", "constant:last"]);
        let after = definitions(Lang::Js, js).into_iter().find(|def| def.name == "after").unwrap();
        assert_eq!((after.line, after.end), (4, 9));
        // 나눗셈과 JSX의 닫는 태그는 정규식이 아니다: 뒤의 정의가 가려지지 않는다.
        let jsx = "const half = (a) => a++ / 2 + a / 3;\nconst view = () => <b>굵게</b>;\nconst test = (x) => /re/.test(x);\nconst last = 1;\n";
        assert_eq!(names(Lang::Js, jsx), ["function:half", "function:view", "function:test", "constant:last"]);
        // C: 쓰는 곳의 `struct 이름`은 정의가 아니다.
        let c = "struct point;\nstatic int area(struct point *p) {\n  int w = p->x;\n  struct point q = {0};\n  return w;\n}\nenum class Color {\n  RED = 1,\n  BLUE,\n};\ntemplate <class T>\nusing namespace std;\n";
        assert_eq!(names(Lang::C, c), ["function:area", "enum:Color", "constant:Color.RED", "constant:Color.BLUE"]);
        // Rust: 제네릭이 든 impl, 여러 줄 where, 여러 줄 튜플 값.
        let rs = "impl<T: AsRef<str>> Wrapper<T>\nwhere\n    T: Clone,\n{\n    fn get(&self) {}\n}\nimpl<'a> From<&'a str> for &'a Name {\n    fn from(s: &str) {}\n}\nimpl dyn Shape {\n    fn area(&self) {}\n}\nenum Kind {\n    Gamma(\n        String,\n    ),\n    Delta,\n}\nstruct Pair<T>\nwhere\n    T: Copy,\n{\n    left: T,\n}\n";
        assert_eq!(
            names(Lang::Rust, rs),
            ["method:Wrapper.get", "method:Name.from", "method:Shape.area", "enum:Kind", "variant:Kind.Gamma", "variant:Kind.Delta", "struct:Pair", "field:Pair.left"]
        );
        // Kotlin의 companion object, Go의 묶음 선언과 이름 없는 받는 쪽, C#의 속성.
        let kt = "class Repo {\n    companion object {\n        const val TAG = \"x\"\n        fun create() = Repo()\n    }\n    fun load() {}\n}\n";
        assert_eq!(names(Lang::Kotlin, kt), ["class:Repo", "constant:Repo.TAG", "method:Repo.create", "method:Repo.load"]);
        let go = "const (\n\tMaxConn = 10\n\tMinConn = 1\n)\n\ntype (\n\tServer struct {\n\t\tname string\n\t}\n)\n\nfunc (Server) Name() string { return \"\" }\nfunc (s *Server) Run() {}\n";
        assert_eq!(names(Lang::Go, go), ["constant:MaxConn", "constant:MinConn", "struct:Server", "method:Server.Name", "method:Server.Run"]);
        let cs = "public class User {\n    public string Name { get; set; }\n    private int age;\n    public void Save() {\n        var x = 1;\n    }\n}\n";
        assert_eq!(names(Lang::Java, cs), ["class:User", "field:User.Name", "field:User.age", "method:User.Save"]);
        // HTML: 다른 속성의 끝에 붙은 id는 정의가 아니다. Markdown: 긴 울타리 안의 짧은 울타리. 설정: 배열의 줄.
        assert_eq!(names(Lang::Html, "<div id=\"a\" data-id=\"b\" :id=\"c\"><p ref='d'></p></div>"), ["id:a", "id:d"]);
        assert_eq!(names(Lang::Markdown, "# 가\n````md\n```\n# 안\n```\n````\n# 나\n"), ["heading:가", "heading:나"]);
        assert_eq!(names(Lang::Config, "[a]\nlist = [\n  [\"x\", \"y\"],\n]\n[b.c]\n"), ["section:a", "section:b.c"]);
        // 끝 줄: 몸통이 끝나는 줄. Python은 들여쓰기가 돌아오기 전의 마지막 줄.
        let ends = |lang, text: &str| definitions(lang, text).into_iter().map(|def| (def.name, def.line, def.end)).collect::<Vec<_>>();
        assert_eq!(
            ends(Lang::Rust, "struct A {\n    x: u8,\n}\nfn f() {\n    g();\n}\nfn g() {}\n"),
            [("A".to_string(), 1, 3), ("x".to_string(), 2, 2), ("f".to_string(), 4, 6), ("g".to_string(), 7, 7)]
        );
        assert_eq!(
            ends(Lang::Python, "class A:\n    def f(self):\n        return 1\n\n    def g(self):\n        pass\n\nX = 1\n"),
            [("A".to_string(), 1, 6), ("f".to_string(), 2, 3), ("g".to_string(), 5, 6), ("X".to_string(), 8, 8)]
        );
        // 선언한 줄 안의 짝 맞는 중괄호는 몸통이 아니다: 함수의 끝 줄이 맞게 잡힌다.
        let js = "function plain(a, opts = {}) {\n  const items = 1;\n  return items;\n}\nconst Row = ({ item }) => {\n  return item;\n};\nclass K {\n  render({ a }) {\n    return a;\n  }\n}\nfunction one() { return 1; }\nconst after = 2;\n";
        assert_eq!(
            ends(Lang::Js, js),
            [("plain".to_string(), 1, 4), ("Row".to_string(), 5, 7), ("K".to_string(), 8, 12), ("render".to_string(), 9, 11), ("one".to_string(), 13, 13), ("after".to_string(), 14, 14)]
        );
        // JSX: `={…} />` 뒤에 `/`가 또 있어도 깊이가 어긋나지 않는다.
        let jsx = "export function List(props) {\n  return <ul>{props.items.map((i) => <Item key={i.id} item={i} />)}</ul>;\n}\nexport const after = 1;\nclass K { m() {} }\n";
        assert_eq!(ends(Lang::Js, jsx)[..3], [("List".to_string(), 1, 3), ("after".to_string(), 4, 4), ("K".to_string(), 5, 5)]);
        // Python: 여러 줄에 걸친 서명과 클래스 머리.
        let py = "class Repo(\n    Base,\n):\n    def find(\n        self,\n        id: int,\n    ) -> \"User\":\n        row = 1\n        return row\n\n    def all(self):\n        pass\n";
        assert_eq!(names(Lang::Python, py), ["class:Repo", "method:Repo.find", "method:Repo.all"]);
        assert_eq!(ends(Lang::Python, py), [("Repo".to_string(), 1, 12), ("find".to_string(), 4, 9), ("all".to_string(), 11, 12)]);
        // 한글 이름, Rust의 `->`가 든 제네릭, Go 묶음 안의 여러 줄 호출, CSS의 여러 줄 값.
        assert_eq!(names(Lang::Rust, "fn 정의를_찾는다() {}\nstruct 주문 {\n    값: u32,\n}\nimpl<F: Fn() -> u32> Runner<F> {\n    fn run(&self) {}\n}\n"), ["function:정의를_찾는다", "struct:주문", "field:주문.값", "method:Runner.run"]);
        assert_eq!(names(Lang::Go, "var (\n\tclient = newClient(\n\t\ttimeout,\n\t\tretries,\n\t)\n\tname = 1\n)\nfunc main() {}\n"), ["variable:client", "variable:name", "function:main"]);
        assert_eq!(names(Lang::Css, ".a,\n.b {\n  box-shadow:\n    0 0 4px #fafafa,\n    inset 0 0 1px red;\n}\n"), ["selector:a", "selector:b"]);
        // 구역이 많은 HTML도 줄 번호와 칸이 맞는다.
        let html = "<p id=\"top\"></p>\n<script>function one() {}</script> <script>\n  const two = 2;\n</script>\n<style>\n.box { }\n</style>\n";
        let found: Vec<(String, u32, u32)> = definitions(Lang::Html, html).into_iter().map(|def| (def.name, def.line, def.col)).collect();
        assert_eq!(found, [("top".to_string(), 1, 7), ("one".to_string(), 2, 17), ("two".to_string(), 3, 8), ("box".to_string(), 6, 1)]);
    }

    #[test]
    fn 이름이_쓰인_곳을_찾는다() {
        let text = "fn load(x: u32) {}\n// load를 부른다\nfn main() { load(1); reload(2); let s = \"load\"; self.load_all(); }\n";
        let hits = occurrences(Lang::Rust, text, "load");
        let roles: Vec<(u32, u32, &str)> = hits.iter().map(|hit| (hit.line, hit.col, hit.role)).collect();
        assert_eq!(roles, [(1, 3, "def"), (2, 3, "comment"), (3, 12, "use"), (3, 41, "string")]);
        assert_eq!(hits[2].text, "fn main() { load(1); reload(2); let s = \"load\"; self.load_all(); }");
        assert_eq!(hits[2].at, 12);
        // CSS·HTML에서는 `-`가 이름의 일부다.
        let css = ".card { }\n.card-title { }\n";
        assert_eq!(occurrences(Lang::Css, css, "card").len(), 1);
        assert_eq!(occurrences(Lang::Css, css, "card-title").len(), 1);
        // 한글 이름과 칸 수(UTF-16)
        let hits = occurrences(Lang::Text, "가나 이름 = 1; 이름표; 이름", "이름");
        assert_eq!(hits.iter().map(|hit| hit.col).collect::<Vec<_>>(), [3, 16]);
        assert!(occurrences(Lang::Rust, text, "").is_empty());
        // 긴 줄은 찾은 자리 둘레만 보여 준다.
        let long = format!("{}needle{}", "a ".repeat(200), " b".repeat(200));
        let hit = &occurrences(Lang::Text, &long, "needle")[0];
        assert!(hit.text.len() <= MAX_LINE_SHOWN + 8 && &hit.text[hit.at as usize..hit.at as usize + 6] == "needle");
    }

    #[test]
    fn 글_찾기와_기호_순위() {
        let text = "Alpha beta\nalphabet ALPHA\n";
        let find = |query: &str, case: bool, word: bool, regex: bool| {
            search(text, &search_pattern(query, case, word, regex).unwrap(), 100).iter().map(|hit| (hit.line, hit.col, hit.len)).collect::<Vec<_>>()
        };
        assert_eq!(find("alpha", false, false, false), [(1, 0, 5), (2, 0, 5), (2, 9, 5)]);
        assert_eq!(find("alpha", true, false, false), [(2, 0, 5)]);
        assert_eq!(find("alpha", false, true, false), [(1, 0, 5), (2, 9, 5)]);
        assert_eq!(find(r"a\w+t\b", false, false, true), [(2, 0, 8)]);
        assert_eq!(find("a.c", false, false, false), []); // 정규식이 아니면 글자 그대로
        assert!(search_pattern("(", false, false, true).is_err());
        assert_eq!(search(text, &search_pattern("a", false, false, false).unwrap(), 2).len(), 2);

        assert_eq!(rank("getFoo", "getFoo"), Some(0));
        assert_eq!(rank("getfoo", "getFoo"), Some(1));
        assert_eq!(rank("get", "getFoo"), Some(2));
        assert_eq!(rank("gf", "getFoo"), Some(3));
        assert_eq!(rank("sc", "snake_case"), Some(3));
        assert_eq!(rank("foo", "getFoo"), Some(4));
        assert_eq!(rank("gto", "getFoo"), Some(5));
        assert_eq!(rank("xyz", "getFoo"), None);
        assert_eq!(rank("", "getFoo"), None);
    }

    #[test]
    fn java의_요즘_꼴_선언을_찾는다() {
        // 한정어 사이·자료형 안의 애노테이션, 겹친 제네릭 인자, delete라는 이름, record의 구성 요소
        let text = "public class Tag {\n\t@Value(\"${redis.key}\")\n\t@Scheduled(cron = \"0 0 * * * ?\")\n\tprivate @Nullable UrlType type;\n\tprivate String @Nullable [] names;\n\tpublic final @Nullable Object getSource() {\n\t\treturn null;\n\t}\n\tpublic <T extends Temporal & Comparable<? super T>> @Nullable T next(T temporal) {\n\t\treturn null;\n\t}\n\tpublic CommonResult delete(@PathVariable(\"id\") Long id) {\n\t\treturn null;\n\t}\n}\npublic record Point(int x, List<String> names) implements Shape {\n}\n";
        assert_eq!(
            names(Lang::Java, text),
            [
                "class:Tag",
                "field:Tag.type",
                "field:Tag.names",
                "method:Tag.getSource",
                "method:Tag.next",
                "method:Tag.delete",
                "class:Point",
                "field:Point.x",
                "field:Point.names",
            ]
        );
        // 한 줄에 여럿 적은 열거 상수, 앞줄에서 이어진 호출
        let text = "enum Color {\n    RED, GREEN, BLUE;\n}\nclass A {\n    private static final boolean FLAG =\n            Props.getFlag(\"x\");\n    private final Spec spec = Spec.where(a)\n            .and(b);\n}\n";
        assert_eq!(names(Lang::Java, text), ["enum:Color", "constant:Color.RED", "constant:Color.GREEN", "constant:Color.BLUE", "class:A", "constant:A.FLAG", "field:A.spec"]);
        // C++에서는 delete가 예약어다(메서드 이름으로 보지 않는다).
        assert!(!names(Lang::C, "class A {\n  void run() {\n  }\n  delete(x);\n};\n").iter().any(|name| name.ends_with("delete")));
    }

    #[test]
    fn rust의_같은_줄_속성과_static_ref를_찾는다() {
        let text = "#[inline] pub fn fast() {}\n#[cfg(test)] mod tests {\n}\nlazy_static! {\n    static ref TABLE: Vec<u8> = Vec::new();\n}\n";
        assert_eq!(names(Lang::Rust, text), ["function:fast", "module:tests", "constant:TABLE"]);
    }

    #[test]
    fn xml의_id와_매퍼_구문을_찾는다() {
        let text = "<?xml version=\"1.0\"?>\n<mapper namespace=\"com.shop.mapper.OrderMapper\">\n  <resultMap id=\"BaseResultMap\" type=\"com.shop.Order\">\n  </resultMap>\n  <!-- <select id=\"old\"> -->\n  <sql id=\"Base_Column_List\">id, name</sql>\n  <select id=\"selectByPrimaryKey\" resultMap=\"BaseResultMap\">\n    select <include refid=\"Base_Column_List\" /> from t where id = #{id}\n  </select>\n  <insert id=\"insert\" parameterType=\"com.shop.Order\">\n  </insert>\n</mapper>\n";
        assert_eq!(
            names(Lang::Xml, text),
            ["id:OrderMapper.BaseResultMap", "id:OrderMapper.Base_Column_List", "method:OrderMapper.selectByPrimaryKey", "method:OrderMapper.insert"]
        );
        assert_eq!(lang_of("a/OrderMapper.xml"), Some(Lang::Xml));
        assert_eq!(names(Lang::Xml, "<beans>\n  <bean id=\"dataSource\" class=\"x.Y\"/>\n</beans>\n"), ["id:dataSource"]);
    }

    #[test]
    fn 서명과_상위_자료형을_읽는다() {
        let java = "@Service\npublic class OrderServiceImpl<T extends Base<T>>\n        extends AbstractService<Order>\n        implements OrderService, java.io.Serializable {\n    @Override\n    public List<Order> list(Long memberId,\n            List<Long> ids) throws IOException {\n        return null;\n    }\n    private final OrderMapper orderMapper;\n}\ninterface OrderService extends BaseService<Order>, Closeable {\n    List<Order> list(Long memberId, List<Long> ids);\n}\n";
        let signer = Signer::new(Lang::Java, java);
        let defs = definitions(Lang::Java, java);
        let of = |name: &str| signer.of(defs.iter().find(|def| def.name == name).unwrap());
        assert_eq!(of("OrderServiceImpl"), "public class OrderServiceImpl<T extends Base<T>> extends AbstractService<Order> implements OrderService, java.io.Serializable");
        assert_eq!(supertypes(Lang::Java, &of("OrderServiceImpl"), "OrderServiceImpl"), ["AbstractService", "OrderService", "Serializable"]);
        assert_eq!(supertypes(Lang::Java, &of("OrderService"), "OrderService"), ["BaseService", "Closeable"]);
        // 여러 줄에 걸친 서명은 한 줄로 잇고, 필드는 그 줄만 읽는다.
        assert_eq!(signer.of(defs.iter().find(|def| def.name == "list" && def.container == "OrderServiceImpl").unwrap()), "public List<Order> list(Long memberId, List<Long> ids) throws IOException");
        assert_eq!(of("orderMapper"), "private final OrderMapper orderMapper");

        let rust = "pub trait Sink: Send + Sync + 'static {\n    fn matched(&self) -> bool;\n}\npub(crate) fn search_path<M: Matcher, W: WriteColor>(\n    matcher: M, // 주석 { ;\n    path: &Path,\n) -> io::Result<SearchResult>\nwhere\n    M: Matcher,\n{\n    todo!()\n}\nstruct Config {\n    globs: Override,\n}\n";
        let signer = Signer::new(Lang::Rust, rust);
        let defs = definitions(Lang::Rust, rust);
        let of = |name: &str| signer.of(defs.iter().find(|def| def.name == name).unwrap());
        assert_eq!(supertypes(Lang::Rust, &of("Sink"), "Sink"), ["Send", "Sync"]);
        assert_eq!(of("search_path"), "pub(crate) fn search_path<M: Matcher, W: WriteColor>( matcher: M, path: &Path, ) -> io::Result<SearchResult> where M: Matcher,");
        assert_eq!(of("globs"), "globs: Override,");
        // 그 밖의 언어
        assert_eq!(supertypes(Lang::Python, "class A(Base, mixins.Other, metaclass=Meta)", "A"), ["Base", "Other"]);
        assert_eq!(supertypes(Lang::Kotlin, "class A(val x: Int) : Base(x), Runnable", "A"), ["Base", "Runnable"]);
        assert_eq!(supertypes(Lang::Js, "export class A<T> extends Base<T> implements One, Two", "A"), ["Base", "One", "Two"]);
        assert_eq!(supertypes(Lang::C, "class A : public Base, private detail::Other", "A"), ["Base", "Other"]);
        assert!(supertypes(Lang::Java, "public class A", "A").is_empty());
        // Kotlin의 위임: `by` 뒤는 자료형이 아니다.
        assert_eq!(supertypes(Lang::Kotlin, "class A(b: B) : B by b, Runnable", "A"), ["B", "Runnable"]);
    }

    #[test]
    fn 몸통_없는_선언의_서명은_그_줄에서_끝난다() {
        // 세미콜론을 쓰지 않는 언어: 아래 선언이 서명에 붙으면 상위 자료형·반환형을 잘못 읽는다.
        let kotlin = "data class User(val name: String)\n\nclass Admin(name: String) : Person(name), Auditable {\n}\ninterface Repo {\n    fun find(id: Long): User\n    fun count(): Int\n}\nclass Long(\n    val a: Int,\n) : Base(),\n    Iface {\n    fun inner() {}\n}\n";
        let signer = Signer::new(Lang::Kotlin, kotlin);
        let defs = definitions(Lang::Kotlin, kotlin);
        let of = |name: &str| signer.of(defs.iter().find(|def| def.name == name).unwrap());
        assert_eq!(of("User"), "data class User(val name: String)");
        assert!(supertypes(Lang::Kotlin, &of("User"), "User").is_empty());
        assert_eq!(of("find"), "fun find(id: Long): User");
        // 상위 자료형 목록이 쉼표 뒤에서 다음 줄로 이어져도 끝까지 읽고, 그 클래스의 멤버는 그 클래스 것이다.
        assert_eq!(supertypes(Lang::Kotlin, &of("Long"), "Long"), ["Base", "Iface"]);
        assert!(names(Lang::Kotlin, kotlin).contains(&"method:Long.inner".to_string()), "{:?}", names(Lang::Kotlin, kotlin));
        // Python의 한 줄 정의도 다음 줄을 끌어오지 않는다.
        let python = "def one(): return 1\ndef two(a,\n        b) -> int:\n    return 2\n";
        let signer = Signer::new(Lang::Python, python);
        let defs = definitions(Lang::Python, python);
        assert_eq!(signer.of(&defs[0]), "def one(): return 1");
        assert_eq!(signer.of(&defs[1]), "def two(a, b) -> int:");
    }

    #[test]
    fn 여러_줄에_걸친_impl_머리를_읽는다() {
        let rust = "impl<'a, T> LongTrait<'a, T>\n    for LongType<'a, T>\nwhere\n    T: Clone,\n{\n    fn run(&self) {}\n}\n/* 한글 주석 */ impl Shape for T where T: Clone {\n}\n";
        assert!(names(Lang::Rust, rust).contains(&"method:LongType.run".to_string()), "{:?}", names(Lang::Rust, rust));
        let found = subtypes(Lang::Rust, rust, "LongTrait");
        assert_eq!(found.iter().map(|(def, _)| (def.name.as_str(), def.line, def.col)).collect::<Vec<_>>(), [("LongType", 2, 8)]);
        // 칸은 원래 글에서 세고(주석의 한글이 칸을 밀지 않는다), where 절이 아니라 `for` 뒤의 이름을 가리킨다.
        let found = subtypes(Lang::Rust, rust, "Shape");
        assert_eq!(found.iter().map(|(def, _)| (def.name.as_str(), def.line, def.col)).collect::<Vec<_>>(), [("T", 8, 27)]);
    }

    #[test]
    fn 콜백을_여는_줄은_객체가_아니다() {
        let js = "const result = items.map((item) => {\n  const name = item.name;\n  return name;\n});\nconst api = {\n  load() {\n  },\n};\nconst typed: (a: number) => void = (a) => {\n  const inner = 1;\n};\n";
        assert_eq!(names(Lang::Js, js), ["constant:result", "constant:api", "method:api.load", "function:typed"]);
        // 점으로 시작하는 설정 파일, 닫는 `#` 앞에 빈칸이 없는 제목.
        assert_eq!((lang_of(".gitignore"), lang_of("a/.editorconfig"), lang_of(".env")), (Some(Lang::Text), Some(Lang::Text), Some(Lang::Text)));
        assert_eq!(names(Lang::Markdown, "# C#\n## 둘째 ##\n"), ["heading:C#", "heading:둘째"]);
    }

    #[test]
    fn 물려받거나_구현한_것을_찾는다() {
        let java = "class A implements Shape {\n}\nclass B extends A {\n}\ninterface Solid extends Shape, Other {\n}\nclass C {\n    Shape shape;\n}\n";
        let found: Vec<String> = subtypes(Lang::Java, java, "Shape").into_iter().map(|(def, _)| format!("{}:{}", def.kind, def.name)).collect();
        assert_eq!(found, ["class:A", "interface:Solid"]);
        let rust = "struct Circle;\nimpl Shape for Circle {\n    fn area(&self) -> f64 { 1.0 }\n}\nimpl<'a, T: Shape> Shape for &'a T {\n}\nimpl fmt::Display for Circle {\n}\nimpl Circle {\n}\ntrait Solid: Shape {\n}\n";
        let found: Vec<String> = subtypes(Lang::Rust, rust, "Shape").into_iter().map(|(def, _)| format!("{}:{}@{}", def.kind, def.name, def.line)).collect();
        assert_eq!(found, ["impl:Circle@2", "impl:T@5", "trait:Solid@11"]);
        assert!(subtypes(Lang::Rust, rust, "Display").iter().any(|(def, _)| def.name == "Circle" && def.container == "Display"));
        // impl 머리의 <…> 안에 화살표가 있어도 트레이트를 읽는다.
        assert_eq!(subtypes(Lang::Rust, "impl<F: Fn() -> u32> Shape for Lazy<F> {\n}\n", "Shape")[0].0.name, "Lazy");
        // 숫자로 시작하는 XML의 id는 정의가 아니다.
        assert!(names(Lang::Xml, "<rows>\n  <row id=\"123\"/>\n</rows>\n").is_empty());
    }

    #[test]
    fn 쓰인_곳이_어느_정의_안인지_적는다() {
        let java = "class OrderService {\n    void place() {\n        save(1);\n    }\n    void save(int n) {\n    }\n}\n";
        let hits = occurrences(Lang::Java, java, "save");
        let seen: Vec<(u32, &str, &str)> = hits.iter().map(|hit| (hit.line, hit.role, hit.within.as_str())).collect();
        assert_eq!(seen, [(3, "use", "OrderService.place"), (5, "def", "OrderService")]);
        let rust = "impl Worker {\n    fn run(&self) {\n        helper();\n    }\n}\nfn helper() {}\n";
        let hits = occurrences(Lang::Rust, rust, "helper");
        assert_eq!(hits.iter().map(|hit| hit.within.as_str()).collect::<Vec<_>>(), ["Worker.run", ""]);
        // impl 안의 메서드를 정의한 줄은 그 자료형을 적는다.
        assert_eq!(occurrences(Lang::Rust, rust, "run")[0].within, "Worker");
    }
}
