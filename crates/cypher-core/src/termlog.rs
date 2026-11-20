//! 세션 출력 기록: 터미널에 찍힌 출력을 사람이 읽는 글자 파일로 바꾼다.
//!
//! 터미널 출력에는 색·커서 이동 같은 제어 글자가 섞여 있다. 기록 파일에는 그것을 빼고 글자만 남긴다.
//! 셸이 한 줄을 고쳐 그리는 방식(캐리지 리턴으로 줄 처음에 돌아가 덮어쓰기, 백스페이스, 줄 끝까지 지우기,
//! 커서를 옆으로 옮기기)은 따라가서 화면에 보이던 줄과 같게 남긴다. 화면 전체를 쓰는 프로그램(vim, top)은
//! 줄 단위로 따라갈 수 없어 글자만 흩어져 남는다(PuTTY의 "출력 기록"과 같다).

/// 한 줄의 칸 한도. 커서 이동은 이 안에서 멈추고(터무니없이 큰 `ESC[99999999C`가 빈칸으로 메모리를 다 쓰지 않게),
/// 줄 바꿈 없이 이어지는 출력은 이만큼마다 끊어 적는다.
const MAX_COL: usize = 16384;

/// 한 칸. 넓은 글자(한글·한자)는 두 칸을 차지하고, 뒤 칸은 `Wide`로 둔다.
#[derive(Clone, Copy, PartialEq)]
enum Cell {
    Char(char),
    Wide,
}

#[derive(Clone, Copy, PartialEq)]
enum Escape {
    /// 제어 문자열 밖.
    None,
    /// ESC 다음.
    Esc,
    /// ESC 뒤의 중간 글자(`ESC ( B`의 `(`) 다음.
    Intermediate,
    /// CSI(`ESC [`) 안.
    Csi,
    /// OSC(`ESC ]`)·DCS(`ESC P`)·APC·PM·SOS 안: BEL이나 ST(`ESC \`)까지 건너뛴다.
    String,
    /// 문자열 안에서 ESC를 만났다(ST의 첫 글자일 수 있다).
    StringEsc,
}

/// 출력을 받아 끝난 줄을 글자로 내놓는다.
pub struct PlainText {
    line: Vec<Cell>,
    col: usize,
    escape: Escape,
    /// CSI의 인자 글자들.
    params: String,
    /// 덜 받은 UTF-8 글자의 앞부분.
    pending: Vec<u8>,
    /// 바로 앞이 캐리지 리턴이었는지(CR LF를 줄 바꿈 하나로 본다).
    after_cr: bool,
}

impl Default for PlainText {
    fn default() -> Self {
        Self::new()
    }
}

/// 터미널에서 두 칸을 차지하는 글자인지(동아시아 넓은 글자와 그림 글자).
pub fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F | 0x2E80..=0x303E | 0x3041..=0x33FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xA000..=0xA4CF
        | 0xA960..=0xA97F | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1F64F | 0x1F900..=0x1F9FF | 0x20000..=0x3FFFD)
}

/// 칸을 차지하지 않는 글자(결합 기호, 너비 없는 글자).
fn is_zero_width(c: char) -> bool {
    matches!(c as u32, 0x0300..=0x036F | 0x200B..=0x200F | 0xFE00..=0xFE0F | 0xFEFF)
}

impl PlainText {
    pub fn new() -> Self {
        Self { line: Vec::new(), col: 0, escape: Escape::None, params: String::new(), pending: Vec::new(), after_cr: false }
    }

    /// 출력 조각을 받는다. 이 조각으로 끝난 줄들을 줄 바꿈(`\r\n`)을 붙여 돌려준다.
    pub fn feed(&mut self, bytes: &[u8]) -> String {
        let mut out = String::new();
        let mut data = std::mem::take(&mut self.pending);
        data.extend_from_slice(bytes);
        let mut rest = &data[..];
        while !rest.is_empty() {
            match std::str::from_utf8(rest) {
                Ok(text) => {
                    for c in text.chars() {
                        self.char(c, &mut out);
                    }
                    rest = &[];
                }
                Err(err) => {
                    let (good, bad) = rest.split_at(err.valid_up_to());
                    // valid_up_to까지는 올바른 UTF-8이다.
                    for c in std::str::from_utf8(good).unwrap_or_default().chars() {
                        self.char(c, &mut out);
                    }
                    match err.error_len() {
                        // 조각 끝에서 잘린 글자: 다음 조각과 이어 읽는다.
                        None => {
                            self.pending = bad.to_vec();
                            rest = &[];
                        }
                        // 깨진 바이트: 대체 글자 하나로 두고 넘어간다.
                        Some(len) => {
                            self.char('\u{fffd}', &mut out);
                            rest = &bad[len..];
                        }
                    }
                }
            }
        }
        out
    }

    /// 아직 끝나지 않은 줄(프롬프트 등)을 내놓는다. 기록을 멈출 때 부른다.
    pub fn finish(&mut self) -> String {
        let mut out = String::new();
        if !self.line.is_empty() {
            self.emit(&mut out);
        }
        out
    }

    fn emit(&mut self, out: &mut String) {
        let text: String = self.line.iter().filter_map(|cell| if let Cell::Char(c) = cell { Some(*c) } else { None }).collect();
        out.push_str(text.trim_end());
        out.push_str("\r\n");
        self.line.clear();
        self.col = 0;
    }

    fn char(&mut self, c: char, out: &mut String) {
        match self.escape {
            Escape::None => {}
            Escape::Esc => {
                self.escape = match c {
                    '[' => {
                        self.params.clear();
                        Escape::Csi
                    }
                    ']' | 'P' | 'X' | '^' | '_' => Escape::String,
                    // ESC 두 번(tmux가 감싼 명령 등): 뒤의 ESC로 새로 시작한다.
                    '\u{1b}' => Escape::Esc,
                    ' '..='/' => Escape::Intermediate,
                    _ => Escape::None, // ESC 한 글자 명령(ESC 7, ESC M …)
                };
                return;
            }
            Escape::Intermediate => {
                if !(' '..='/').contains(&c) {
                    self.escape = Escape::None;
                }
                return;
            }
            Escape::Csi => {
                if ('@'..='~').contains(&c) {
                    self.escape = Escape::None;
                    self.csi(c);
                } else if self.params.len() < 64 {
                    self.params.push(c);
                }
                return;
            }
            Escape::String => {
                match c {
                    // BEL, ST(U+009C), CAN·SUB는 문자열을 끝낸다.
                    '\u{7}' | '\u{9c}' | '\u{18}' | '\u{1a}' => self.escape = Escape::None,
                    '\u{1b}' => self.escape = Escape::StringEsc,
                    _ => {}
                }
                return;
            }
            Escape::StringEsc => {
                // ESC \(ST)이면 끝. 다른 글자면 문자열은 거기서 끝나고 새 ESC 명령이 시작된 것이다(터미널과 같게).
                if c == '\\' {
                    self.escape = Escape::None;
                } else {
                    self.escape = Escape::Esc;
                    self.char(c, out);
                }
                return;
            }
        }
        let after_cr = std::mem::replace(&mut self.after_cr, false);
        match c {
            '\u{1b}' => self.escape = Escape::Esc,
            '\u{9b}' => {
                self.params.clear();
                self.escape = Escape::Csi;
            }
            '\n' => self.emit(out),
            '\r' => {
                self.col = 0;
                self.after_cr = true;
            }
            // 한 칸 왼쪽으로(넓은 글자는 셸이 두 번 보낸다).
            '\u{8}' => self.col = self.col.saturating_sub(1),
            '\t' => {
                let next = ((self.col / 8 + 1) * 8).min(MAX_COL);
                while self.col < next {
                    self.put(' ', 1);
                }
            }
            c if (c as u32) < 0x20 || (0x7f..0xa0).contains(&(c as u32)) => {}
            c if is_zero_width(c) => {}
            c => {
                let _ = after_cr;
                if self.col >= MAX_COL {
                    self.emit(out);
                }
                self.put(c, if is_wide(c) { 2 } else { 1 });
            }
        }
    }

    /// 지금 칸에 글자를 쓴다(덮어쓰기). 넓은 글자를 반쯤 덮으면 남은 반쪽은 빈칸이 된다.
    fn put(&mut self, c: char, width: usize) {
        while self.line.len() < self.col {
            self.line.push(Cell::Char(' '));
        }
        for at in self.col..self.col + width {
            match self.line.get(at).copied() {
                Some(Cell::Wide) if at > 0 => self.line[at - 1] = Cell::Char(' '),
                Some(Cell::Char(old)) if is_wide(old) && at + 1 < self.line.len() && self.line[at + 1] == Cell::Wide => {
                    self.line[at + 1] = Cell::Char(' ');
                }
                _ => {}
            }
        }
        let cells = if width == 2 { vec![Cell::Char(c), Cell::Wide] } else { vec![Cell::Char(c)] };
        for (offset, cell) in cells.into_iter().enumerate() {
            let at = self.col + offset;
            if at < self.line.len() {
                self.line[at] = cell;
            } else {
                self.line.push(cell);
            }
        }
        self.col += width;
    }

    fn csi(&mut self, command: char) {
        // 사적 인자(`?25h` 등)는 줄 내용과 무관하다.
        if self.params.starts_with(['?', '>', '<', '=']) {
            return;
        }
        // 인자는 칸 한도로 자른다(넘치는 수는 한도로 본다).
        let numbers: Vec<usize> = self.params.split(';').map(|part| part.parse::<usize>().unwrap_or(if part.is_empty() { 0 } else { MAX_COL }).min(MAX_COL)).collect();
        let last = MAX_COL - 1;
        let first = numbers.first().copied().unwrap_or(0);
        let count = first.max(1);
        match command {
            // 줄 지우기: 0(커서부터 끝), 1(처음부터 커서), 2(줄 전체)
            'K' => match first {
                0 => self.line.truncate(self.col),
                1 => {
                    for at in 0..=self.col.min(self.line.len().saturating_sub(1)) {
                        if at < self.line.len() {
                            self.line[at] = Cell::Char(' ');
                        }
                    }
                }
                _ => self.line.clear(),
            },
            // 커서 옆으로
            'C' => self.col = (self.col + count).min(last),
            'D' => self.col = self.col.saturating_sub(count),
            // 줄 안의 칸으로(1부터)
            'G' | '`' => self.col = (count - 1).min(last),
            // 글자 지우기(ECH): 커서부터 n칸을 빈칸으로
            'X' => {
                for at in self.col..(self.col + count).min(self.line.len()) {
                    self.line[at] = Cell::Char(' ');
                }
            }
            // 글자 빼기(DCH): 커서의 n칸을 빼고 뒤를 당긴다
            'P' => {
                let end = (self.col + count).min(self.line.len());
                if self.col < end {
                    self.line.drain(self.col..end);
                }
            }
            // 빈칸 끼우기(ICH)
            '@' => {
                // 한 번에 끼우고(칸마다 줄을 옮기지 않게), 한도를 넘는 끝은 버린다.
                let at = self.col.min(self.line.len());
                let room = MAX_COL.saturating_sub(at);
                self.line.truncate(MAX_COL.saturating_sub(count.min(room)).max(at));
                self.line.splice(at..at, std::iter::repeat_n(Cell::Char(' '), count.min(room)));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(chunks: &[&[u8]]) -> String {
        let mut text = PlainText::new();
        let mut out = String::new();
        for chunk in chunks {
            out.push_str(&text.feed(chunk));
        }
        out.push_str(&text.finish());
        out
    }

    #[test]
    fn 색과_제어_글자를_뺀다() {
        assert_eq!(plain(&[b"\x1b[1;32muser@host\x1b[0m:~$ ls\r\n"]), "user@host:~$ ls\r\n");
        // 창 제목(OSC)은 BEL이나 ST로 끝난다.
        assert_eq!(plain(&[b"\x1b]0;title\x07a\x1b]2;t\x1b\\b\n"]), "ab\r\n");
        // ESC 한 글자 명령, 문자 집합 고르기, 사적 인자
        assert_eq!(plain(&[b"\x1b(Bx\x1b7y\x1b[?25lz\x1b[?2004h\n"]), "xyz\r\n");
        // 줄 끝의 빈칸은 뗀다. 마지막 줄은 끝낼 때 내놓는다.
        assert_eq!(plain(&[b"a   \nprompt$ "]), "a\r\nprompt$\r\n");
    }

    #[test]
    fn 줄을_고쳐_그린_것을_따라간다() {
        // 진행률처럼 CR로 처음에 돌아가 덮어쓴다.
        assert_eq!(plain(&[b"10%\r20%\r100%\n"]), "100%\r\n");
        // 백스페이스로 지우고 다시 쓰기(셸의 줄 편집)
        assert_eq!(plain(&[b"lss\x08 \x08\n"]), "ls\r\n");
        assert_eq!(plain(&[b"abc\x08\x08\x1b[K\n"]), "a\r\n");
        // 커서를 옮겨 가운데 끼워 넣고 줄 끝까지 다시 그리기
        assert_eq!(plain(&[b"echo world\x1b[5Dhello \x1b[Kworld\n"]), "echo hello world\r\n");
        // 줄 전체 지우기, 칸으로 가기, 글자 빼기
        assert_eq!(plain(&[b"junk\x1b[2K\rok\n"]), "ok\r\n");
        assert_eq!(plain(&[b"abcdef\x1b[3GX\n"]), "abXdef\r\n");
        assert_eq!(plain(&[b"abcdef\x1b[3D\x1b[2P\n"]), "abcf\r\n");
        // CR LF와 LF는 같은 줄 바꿈이다.
        assert_eq!(plain(&[b"a\r\nb\nc\r\n"]), "a\r\nb\r\nc\r\n");
        // 탭은 다음 8칸 자리까지 빈칸
        assert_eq!(plain(&[b"a\tb\n"]), "a       b\r\n");
    }

    #[test]
    fn 한글과_잘린_글자를_다룬다() {
        // 조각 사이에서 잘린 UTF-8 글자, 조각 사이에서 잘린 제어 문자열
        let text = "안녕 세상\n".as_bytes();
        assert_eq!(plain(&[&text[..2], &text[2..7], &text[7..]]), "안녕 세상\r\n");
        assert_eq!(plain(&[b"x\x1b[3", b"1my\x1b[0m\n"]), "xy\r\n");
        // 넓은 글자는 두 칸: 백스페이스 두 번으로 한 글자를 지운다(셸이 보내는 꼴).
        assert_eq!(plain(&["가나".as_bytes(), b"\x08\x08  \x08\x08\n"]), "가\r\n");
        // 넓은 글자의 반쪽을 덮으면 나머지 반쪽은 빈칸이 된다.
        assert_eq!(plain(&["가나".as_bytes(), b"\r\x1b[1Cx\n"]), " x나\r\n");
        // 깨진 바이트는 대체 글자 하나
        assert_eq!(plain(&[b"a\xffb\n"]), "a\u{fffd}b\r\n");
        assert!(is_wide('한') && is_wide('漢') && !is_wide('a'));
    }

    #[test]
    fn 터무니없는_인자와_끝나지_않은_문자열에도_버틴다() {
        // 아주 큰 커서 이동은 칸 한도에서 멈춘다(메모리를 다 쓰지 않는다).
        let out = plain(&[b"\x1b[99999999999Cx\x1b[50000000Gy\n"]);
        assert!(out.len() < MAX_COL + 10 && out.trim_end().ends_with('y'), "{}", out.len());
        // 줄 바꿈 없이 끝없이 이어지는 출력은 한도마다 끊는다.
        let long = vec![b'a'; MAX_COL * 2 + 5];
        let mut text = PlainText::new();
        let lines = text.feed(&long);
        assert_eq!(lines.matches("\r\n").count(), 2);
        // ESC로 끝나는(ST 없는) 문자열 뒤의 출력은 남는다. CAN·SUB·U+009C로도 끝난다.
        assert_eq!(plain(&[b"\x1b]0;t\x1b[0mhello\nworld\n"]), "hello\r\nworld\r\n");
        assert_eq!(plain(&[b"\x1bPjunk\x1b\\$ ls\r\n"]), "$ ls\r\n");
        assert_eq!(plain(&["\x1b]0;title\u{9c}visible\n".as_bytes()]), "visible\r\n");
        assert_eq!(plain(&[b"\x1b]2;x\x18ok\n"]), "ok\r\n");
        // ESC 두 번 뒤의 명령도 빠진다(tmux가 감싼 명령).
        assert_eq!(plain(&[b"a\x1b\x1b[31mb\n"]), "ab\r\n");
        assert_eq!(plain(&[b"\x1bPtmux;\x1b\x1b]0;title\x07\x1b\\ok\n"]), "ok\r\n");
        // 빈칸 끼우기를 거듭해도 줄은 한도를 넘지 않고, 오래 걸리지 않는다.
        let mut text = PlainText::new();
        let started = std::time::Instant::now();
        text.feed(&vec![b'a'; MAX_COL - 100]);
        text.feed(b"\r");
        for _ in 0..2000 {
            text.feed(b"\x1b[512@");
        }
        assert!(text.line.len() <= MAX_COL && started.elapsed() < std::time::Duration::from_secs(2), "{:?}", started.elapsed());
    }
}
