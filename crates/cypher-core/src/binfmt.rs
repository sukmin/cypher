//! 헥사 에디터의 해석: 파일의 앞부분(매직 넘버)으로 형식을 알아보고, 알 수 있는 형식은 구조(머리·덩어리·목록)를 읽는다.
//!
//! 읽은 것은 칸(`Field`)의 나무다. 칸마다 파일에서의 자리(`offset`, `len`)가 있어 화면이 그 바이트를 함께 표시한다.
//! 파일 전체를 읽지 않고 필요한 곳만 읽는다(`Source`): 몇 GB짜리 파일도 머리·목록만 읽는다.
//! 잘렸거나 깨진 파일은 읽을 수 있는 데까지 읽고 `notes`에 까닭을 적는다(패닉하지 않는다).

use serde::Serialize;

/// 읽을 바이트가 있는 곳(파일, 시험에서는 메모리).
pub trait Source {
    fn size(&self) -> u64;
    /// `offset`부터 최대 `len` 바이트(파일 끝이면 짧다).
    fn read(&self, offset: u64, len: usize) -> Vec<u8>;
}

/// 메모리에 든 바이트(시험, 작은 파일).
pub struct Bytes<'a>(pub &'a [u8]);

impl Source for Bytes<'_> {
    fn size(&self) -> u64 {
        self.0.len() as u64
    }
    fn read(&self, offset: u64, len: usize) -> Vec<u8> {
        let start = (offset.min(self.0.len() as u64)) as usize;
        let end = start.saturating_add(len).min(self.0.len());
        self.0[start..end].to_vec()
    }
}

/// 구조의 칸 하나.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub offset: u64,
    pub len: u64,
    pub value: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Field>,
}

impl Field {
    fn new(name: impl Into<String>, offset: u64, len: u64, value: impl Into<String>) -> Self {
        Self { name: name.into(), offset, len, value: value.into(), children: Vec::new() }
    }
    fn with(mut self, children: Vec<Field>) -> Self {
        self.children = children;
        self
    }
}

/// 해석한 결과.
#[derive(Serialize, Debug, Clone, PartialEq, Default)]
pub struct Analysis {
    /// 형식 이름(예: "PNG 그림"). 모르면 "알 수 없는 형식".
    pub format: String,
    /// 이 형식을 알아본 근거(매직 넘버 등).
    pub magic: String,
    /// 한눈에 볼 값들(크기, 버전 …): (이름, 값)
    pub summary: Vec<(String, String)>,
    pub fields: Vec<Field>,
    pub notes: Vec<String>,
}

/// 목록이 너무 길면(ZIP 항목, PNG 덩어리) 앞의 것만 보인다.
const MAX_ITEMS: usize = 1000;

struct Reader<'a> {
    src: &'a dyn Source,
    size: u64,
}

impl<'a> Reader<'a> {
    fn new(src: &'a dyn Source) -> Self {
        Self { src, size: src.size() }
    }
    /// 정확히 `len` 바이트(모자라면 None).
    fn bytes(&self, offset: u64, len: usize) -> Option<Vec<u8>> {
        if offset.checked_add(len as u64)? > self.size {
            return None;
        }
        let got = self.src.read(offset, len);
        (got.len() == len).then_some(got)
    }
    fn u8(&self, at: u64) -> Option<u8> {
        self.bytes(at, 1).map(|b| b[0])
    }
    fn u16(&self, at: u64, le: bool) -> Option<u16> {
        let b: [u8; 2] = self.bytes(at, 2)?.try_into().ok()?;
        Some(if le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    }
    fn u32(&self, at: u64, le: bool) -> Option<u32> {
        let b: [u8; 4] = self.bytes(at, 4)?.try_into().ok()?;
        Some(if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    }
    fn u64(&self, at: u64, le: bool) -> Option<u64> {
        let b: [u8; 8] = self.bytes(at, 8)?.try_into().ok()?;
        Some(if le { u64::from_le_bytes(b) } else { u64::from_be_bytes(b) })
    }
    fn starts(&self, at: u64, magic: &[u8]) -> bool {
        self.bytes(at, magic.len()).is_some_and(|b| b == magic)
    }
}

// ───────────────────────── 글자로 보이기 ─────────────────────────

/// 1234567 → "1,234,567"
pub fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// 바이트 수를 사람이 읽기 좋게: "1,536 바이트 (1.5 KB)"
pub fn size_text(n: u64) -> String {
    const UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    if n < 1024 {
        return format!("{} 바이트", grouped(n));
    }
    let mut value = n as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{} 바이트 ({value:.1} {})", grouped(n), UNITS[unit])
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}

/// 글자로 읽을 수 있는 바이트만 글자로(나머지는 점).
fn ascii(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { '.' }).collect()
}

fn num<T: Into<u64> + Copy>(n: T) -> String {
    grouped(n.into())
}

/// 유닉스 시각(초)을 "2024-05-01 12:30:00 UTC"로.
pub fn unix_time(secs: i64) -> String {
    let days = secs.div_euclid(86400);
    let rest = secs.rem_euclid(86400);
    // 1970-01-01부터의 날 수 → 그레고리력(Howard Hinnant의 civil_from_days)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC", rest / 3600, rest % 3600 / 60, rest % 60)
}

/// DOS 날짜·시각(ZIP)을 글자로.
fn dos_time(date: u16, time: u16) -> String {
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        1980 + (date >> 9),
        (date >> 5) & 0xf,
        date & 0x1f,
        time >> 11,
        (time >> 5) & 0x3f,
        (time & 0x1f) * 2
    )
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

// ───────────────────────── 알아보기 ─────────────────────────

/// 파일을 해석한다.
pub fn analyze(src: &dyn Source) -> Analysis {
    let r = Reader::new(src);
    let mut out = Analysis::default();
    if r.size == 0 {
        out.format = "빈 파일".into();
        return out;
    }
    let head = src.read(0, 64);
    // 구조를 읽는 형식이면 Some(읽은 결과): 읽다가 모자라면 Some(None). 그런 형식이 아니면 None.
    let parsed = Some(if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        png(&r, &mut out)
    } else if head.starts_with(&[0xff, 0xd8, 0xff]) {
        jpeg(&r, &mut out)
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        gif(&r, &mut out)
    } else if head.starts_with(b"BM") && head.len() >= 26 && [12, 40, 52, 56, 64, 108, 124].contains(&u32::from_le_bytes([head[14], head[15], head[16], head[17]])) {
        bmp(&r, &mut out)
    } else if head.starts_with(&[0, 0, 1, 0]) && head.len() >= 6 && head[4] | head[5] != 0 {
        ico(&r, &mut out)
    } else if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        zip(&r, &mut out)
    } else if head.starts_with(&[0x1f, 0x8b]) {
        gzip(&r, &mut out)
    } else if head.starts_with(b"%PDF-") {
        pdf(&r, &mut out)
    } else if head.starts_with(b"MZ") && r.size >= 64 && text_kind(&head).is_none() {
        // 진짜 DOS 머리에는 0바이트가 있다: "MZ…"로 시작하는 글자 파일은 실행 파일로 보지 않는다.
        pe(&r, &mut out)
    } else if head.starts_with(b"\x7fELF") {
        elf(&r, &mut out)
    } else if head.starts_with(&[0xca, 0xfe, 0xba, 0xbe]) {
        cafebabe(&r, &mut out)
    } else if [[0xfe, 0xed, 0xfa, 0xce], [0xfe, 0xed, 0xfa, 0xcf], [0xce, 0xfa, 0xed, 0xfe], [0xcf, 0xfa, 0xed, 0xfe]].iter().any(|m| head.starts_with(m)) {
        macho(&r, &mut out)
    } else if head.starts_with(b"SQLite format 3\0") {
        sqlite(&r, &mut out)
    } else if head.starts_with(b"RIFF") || head.starts_with(b"RIFX") {
        riff(&r, &mut out)
    } else if head.len() >= 12 && &head[4..8] == b"ftyp" {
        isobmff(&r, &mut out)
    } else if head.starts_with(b"fLaC") {
        flac(&r, &mut out)
    } else if head.starts_with(b"ID3") {
        id3(&r, &mut out)
    } else if r.starts(257, b"ustar") {
        tar(&r, &mut out)
    } else {
        return finish(src, &r, out, &head);
    });
    // 머리도 다 읽지 못해 형식을 정하지 못했으면(아주 짧은 파일) 구조 없는 형식처럼 본다.
    if out.format.is_empty() {
        return finish(src, &r, out, &head);
    }
    if parsed == Some(None) && !out.notes.iter().any(|note| note.contains("잘렸") || note.contains("끝나")) {
        out.notes.push("파일이 중간에 끝나 구조를 다 읽지 못했습니다.".into());
    }
    out.summary.insert(0, ("파일 크기".into(), size_text(r.size)));
    out
}

/// 구조를 읽지 않는 형식: 매직 넘버 표, 글자 파일, 모르는 형식(통계).
fn finish(src: &dyn Source, r: &Reader, mut out: Analysis, head: &[u8]) -> Analysis {
    if out.format.is_empty() {
        if let Some((name, magic)) = by_magic(head) {
            out.format = name.into();
            out.magic = magic;
        } else if let Some(name) = text_kind(&src.read(0, 65536)) {
            text(r, &mut out, name);
        }
    }
    if out.format.is_empty() {
        out.format = "알 수 없는 형식".into();
        out.notes.push("알고 있는 매직 넘버가 아닙니다. 아래 통계는 앞의 64KB로 셉니다.".into());
        stats(src, &mut out);
    }
    out.summary.insert(0, ("파일 크기".into(), size_text(r.size)));
    out
}

/// 구조는 읽지 않고 이름만 알아보는 형식들: (매직, 자리, 이름)
fn by_magic(head: &[u8]) -> Option<(&'static str, String)> {
    const TABLE: &[(&[u8], usize, &str)] = &[
        (b"7z\xbc\xaf\x27\x1c", 0, "7z 묶음"),
        (b"Rar!\x1a\x07\x01\x00", 0, "RAR 5 묶음"),
        (b"Rar!\x1a\x07\x00", 0, "RAR 묶음"),
        (b"\xfd7zXZ\x00", 0, "xz 압축"),
        (b"BZh", 0, "bzip2 압축"),
        (b"\x28\xb5\x2f\xfd", 0, "Zstandard 압축"),
        (b"\x04\x22\x4d\x18", 0, "LZ4 압축"),
        (b"MSCF", 0, "Windows 캐비닛(CAB)"),
        (b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1", 0, "OLE 복합 문서(옛 Office·MSI)"),
        (b"L\x00\x00\x00\x01\x14\x02\x00", 0, "Windows 바로 가기(LNK)"),
        (b"regf", 0, "Windows 레지스트리 하이브"),
        (b"ElfFile\x00", 0, "Windows 이벤트 기록(EVTX)"),
        (b"\xd4\xc3\xb2\xa1", 0, "pcap 패킷 기록"),
        (b"\xa1\xb2\xc3\xd4", 0, "pcap 패킷 기록"),
        (b"\x0a\x0d\x0d\x0a", 0, "pcapng 패킷 기록"),
        (b"dex\n", 0, "안드로이드 DEX"),
        (b"\x00asm", 0, "WebAssembly 모듈"),
        (b"II*\x00", 0, "TIFF 그림(리틀 엔디언)"),
        (b"MM\x00*", 0, "TIFF 그림(빅 엔디언)"),
        (b"8BPS", 0, "Photoshop 문서(PSD)"),
        (b"wOFF", 0, "WOFF 글꼴"),
        (b"wOF2", 0, "WOFF2 글꼴"),
        (b"OTTO", 0, "OpenType 글꼴(CFF)"),
        (b"\x00\x01\x00\x00\x00", 0, "TrueType 글꼴"),
        (b"OggS", 0, "Ogg 소리·영상"),
        (b"\x1a\x45\xdf\xa3", 0, "Matroska·WebM 영상"),
        (b"\x00\x00\x01\xba", 0, "MPEG 프로그램 스트림"),
        (b"\x00\x00\x01\xb3", 0, "MPEG 영상"),
        (b"FLV", 0, "Flash 영상(FLV)"),
        (b"{\\rtf", 0, "서식 있는 글(RTF)"),
        (b"\x1f\x9d", 0, "compress(.Z) 압축"),
        (b"-----BEGIN ", 0, "PEM 인증서·키"),
        (b"openssh-key-v1", 0, "OpenSSH 개인 키"),
        (b"PuTTY-User-Key-File-", 0, "PuTTY 개인 키(PPK)"),
        (b"\x30\x82", 0, "DER(ASN.1) 인증서·키로 보임"),
    ];
    for (magic, at, name) in TABLE {
        if head.len() >= at + magic.len() && &head[*at..at + magic.len()] == *magic {
            return Some((name, format!("{} 자리에 {}", at, hex(magic))));
        }
    }
    None
}

/// 글자 파일로 보이면 그 종류.
fn text_kind(head: &[u8]) -> Option<&'static str> {
    if head.starts_with(&[0xef, 0xbb, 0xbf]) {
        return Some("글자 파일(UTF-8, BOM 있음)");
    }
    if head.starts_with(&[0xff, 0xfe]) {
        return Some("글자 파일(UTF-16 LE, BOM 있음)");
    }
    if head.starts_with(&[0xfe, 0xff]) {
        return Some("글자 파일(UTF-16 BE, BOM 있음)");
    }
    if head.contains(&0) {
        return None;
    }
    let controls = head.iter().filter(|&&b| b < 0x20 && !matches!(b, b'\t' | b'\n' | b'\r' | 0x0c | 0x1b)).count();
    if controls * 100 > head.len() {
        return None;
    }
    // 잘린 끝 글자는 봐준다.
    let valid = match std::str::from_utf8(head) {
        Ok(_) => true,
        Err(err) => err.error_len().is_none() && err.valid_up_to() + 4 >= head.len(),
    };
    Some(if valid { if head.is_ascii() { "글자 파일(ASCII)" } else { "글자 파일(UTF-8)" } } else { "글자 파일(UTF-8이 아님 · CP949 등으로 보임)" })
}

fn text(r: &Reader, out: &mut Analysis, name: &str) {
    out.format = name.into();
    let sample = r.src.read(0, 1 << 20);
    let lines = sample.iter().filter(|&&b| b == b'\n').count();
    let crlf = sample.windows(2).filter(|w| w == b"\r\n").count();
    let eol = if lines == 0 { "줄 바꿈 없음" } else if crlf * 2 >= lines { "CRLF(Windows)" } else { "LF(유닉스)" };
    out.summary.push(("줄 끝".into(), eol.into()));
    let partial = if r.size > sample.len() as u64 { " (앞의 1MB에서 센 값)" } else { "" };
    out.summary.push(("줄 수".into(), format!("{}{partial}", num(lines as u64 + u64::from(!sample.ends_with(b"\n"))))));
    let bom = if sample.starts_with(&[0xef, 0xbb, 0xbf]) { 3 } else if sample.starts_with(&[0xff, 0xfe]) || sample.starts_with(&[0xfe, 0xff]) { 2 } else { 0 };
    if bom > 0 {
        out.fields.push(Field::new("BOM(바이트 순서 표시)", 0, bom, hex(&sample[..bom as usize])));
    }
}

fn stats(src: &dyn Source, out: &mut Analysis) {
    let sample = src.read(0, 65536);
    if sample.is_empty() {
        return;
    }
    let mut counts = [0u32; 256];
    for &b in &sample {
        counts[b as usize] += 1;
    }
    let total = sample.len() as f64;
    let entropy: f64 = counts.iter().filter(|&&c| c > 0).map(|&c| {
        let p = f64::from(c) / total;
        -p * p.log2()
    }).sum();
    let zeros = f64::from(counts[0]) / total * 100.0;
    let printable = sample.iter().filter(|&&b| (0x20..0x7f).contains(&b) || matches!(b, b'\t' | b'\n' | b'\r')).count() as f64 / total * 100.0;
    out.summary.push(("엔트로피".into(), format!("{entropy:.2} 비트/바이트 (8에 가까우면 압축·암호화된 자료)")));
    out.summary.push(("0 바이트".into(), format!("{zeros:.1}%")));
    out.summary.push(("글자로 읽히는 바이트".into(), format!("{printable:.1}%")));
}

// ───────────────────────── 그림 ─────────────────────────

fn png(r: &Reader, out: &mut Analysis) -> Option<()> {
    out.format = "PNG 그림".into();
    out.magic = "89 50 4E 47 0D 0A 1A 0A".into();
    out.fields.push(Field::new("서명", 0, 8, "89 50 4E 47 0D 0A 1A 0A (PNG)"));
    let mut at = 8u64;
    let mut count = 0;
    let mut ended = false;
    while at + 12 <= r.size {
        if count >= MAX_ITEMS {
            out.notes.push(format!("덩어리가 너무 많아 앞의 {MAX_ITEMS}개만 보입니다."));
            break;
        }
        count += 1;
        let len = u64::from(r.u32(at, false)?);
        let kind = r.bytes(at + 4, 4)?;
        let name = ascii(&kind);
        if at + 12 + len > r.size {
            out.notes.push(format!("{name} 덩어리가 파일 끝을 넘습니다(파일이 잘렸습니다)."));
            out.fields.push(Field::new(format!("덩어리 {name}"), at, r.size - at, format!("길이 {} (잘림)", num(len))));
            at = r.size;
            break;
        }
        let mut children = vec![
            Field::new("길이", at, 4, num(len)),
            Field::new("종류", at + 4, 4, name.clone()),
        ];
        let data = at + 8;
        let mut summary = format!("{} 바이트", num(len));
        match name.as_str() {
            "IHDR" if len >= 13 => {
                let (w, h) = (r.u32(data, false)?, r.u32(data + 4, false)?);
                let (depth, color) = (r.u8(data + 8)?, r.u8(data + 9)?);
                let color_name = match color {
                    0 => "회색조",
                    2 => "RGB",
                    3 => "색상표",
                    4 => "회색조 + 알파",
                    6 => "RGBA",
                    _ => "알 수 없음",
                };
                children.push(Field::new("가로", data, 4, num(w)));
                children.push(Field::new("세로", data + 4, 4, num(h)));
                children.push(Field::new("비트 깊이", data + 8, 1, depth.to_string()));
                children.push(Field::new("색 형식", data + 9, 1, format!("{color} ({color_name})")));
                children.push(Field::new("압축 방식", data + 10, 1, r.u8(data + 10)?.to_string()));
                children.push(Field::new("필터 방식", data + 11, 1, r.u8(data + 11)?.to_string()));
                let interlace = r.u8(data + 12)?;
                children.push(Field::new("격행", data + 12, 1, if interlace == 1 { "Adam7".into() } else { "없음".to_string() }));
                out.summary.push(("그림 크기".into(), format!("{} × {}", num(w), num(h))));
                out.summary.push(("색".into(), format!("{color_name} · {depth}비트")));
                summary = format!("{} × {} · {color_name}", num(w), num(h));
            }
            "tEXt" | "iTXt" | "zTXt" if len > 0 => {
                let body = r.bytes(data, len.min(200) as usize)?;
                let key = body.split(|&b| b == 0).next().map(|k| String::from_utf8_lossy(k).into_owned()).unwrap_or_default();
                if name == "tEXt" {
                    let value = body.splitn(2, |&b| b == 0).nth(1).map(|v| String::from_utf8_lossy(v).into_owned()).unwrap_or_default();
                    summary = format!("{key} = {value}");
                } else {
                    summary = key;
                }
            }
            "pHYs" if len >= 9 => {
                let (x, unit) = (r.u32(data, false)?, r.u8(data + 8)?);
                summary = if unit == 1 { format!("{:.0} DPI", f64::from(x) * 0.0254) } else { format!("{} 픽셀/단위", num(x)) };
            }
            _ => {}
        }
        if len > 0 {
            children.push(Field::new("내용", data, len, if name == "IDAT" { "압축된 그림 자료".into() } else { format!("{} 바이트", num(len)) }));
        }
        let stored = r.u32(data + len, false)?;
        let check = if len <= 4 << 20 {
            let mut bytes = kind.clone();
            bytes.extend(r.bytes(data, len as usize)?);
            if crc32(&bytes) == stored { " (맞음)" } else { " (맞지 않음!)" }
        } else {
            ""
        };
        if check == " (맞지 않음!)" {
            out.notes.push(format!("{name} 덩어리({at:#x})의 CRC가 맞지 않습니다."));
        }
        children.push(Field::new("CRC", data + len, 4, format!("{stored:08X}{check}")));
        out.fields.push(Field::new(format!("덩어리 {name}"), at, 12 + len, summary).with(children));
        at += 12 + len;
        if name == "IEND" {
            ended = true;
            break;
        }
    }
    if !ended && count < MAX_ITEMS {
        if at < r.size {
            out.fields.push(Field::new("잘린 덩어리", at, r.size - at, format!("{} 바이트", num(r.size - at))));
        }
        if !out.notes.iter().any(|note| note.contains("잘렸습니다")) {
            out.notes.push("끝 덩어리(IEND)가 없습니다. 파일이 잘렸습니다.".into());
        }
    } else if ended && at < r.size {
        out.fields.push(Field::new("IEND 뒤의 자료", at, r.size - at, format!("{} 바이트 (그림에 쓰이지 않음)", num(r.size - at))));
        out.notes.push("IEND 뒤에 자료가 더 붙어 있습니다.".into());
    }
    Some(())
}

fn jpeg(r: &Reader, out: &mut Analysis) -> Option<()> {
    out.format = "JPEG 그림".into();
    out.magic = "FF D8 FF".into();
    out.fields.push(Field::new("SOI(그림 시작)", 0, 2, "FF D8"));
    let mut at = 2u64;
    let mut count = 0;
    while at + 4 <= r.size && count < MAX_ITEMS {
        count += 1;
        if r.u8(at)? != 0xff {
            out.notes.push(format!("{at:#x}에 표지(FF)가 없습니다."));
            break;
        }
        let marker = r.u8(at + 1)?;
        if marker == 0xff {
            at += 1; // 채움 바이트
            continue;
        }
        if marker == 0xd9 {
            out.fields.push(Field::new("EOI(그림 끝)", at, 2, "FF D9"));
            at += 2;
            break;
        }
        if (0xd0..=0xd7).contains(&marker) || marker == 0x01 {
            out.fields.push(Field::new(format!("RST{}", marker & 7), at, 2, ""));
            at += 2;
            continue;
        }
        let len = u64::from(r.u16(at + 2, false)?);
        if len < 2 || at + 2 + len > r.size {
            out.notes.push(format!("{at:#x}의 구획 길이가 맞지 않습니다(파일이 잘렸습니다)."));
            break;
        }
        let data = at + 4;
        let (name, value) = match marker {
            0xe0 => {
                let id = r.bytes(data, 5).unwrap_or_default();
                if id == b"JFIF\0" {
                    ("APP0 (JFIF)".to_string(), format!("JFIF {}.{:02}", r.u8(data + 5)?, r.u8(data + 6)?))
                } else {
                    ("APP0".to_string(), ascii(&id))
                }
            }
            0xe1 => {
                let id = r.bytes(data, 6).unwrap_or_default();
                if id.starts_with(b"Exif") { ("APP1 (Exif)".to_string(), "사진 정보(카메라, 시각, 위치 …)".to_string()) } else { ("APP1".to_string(), ascii(&id)) }
            }
            0xe2..=0xef => (format!("APP{}", marker - 0xe0), ascii(&r.bytes(data, (len - 2).min(12) as usize).unwrap_or_default())),
            0xdb => ("DQT (양자화 표)".to_string(), String::new()),
            0xc4 => ("DHT (허프만 표)".to_string(), String::new()),
            0xdd => ("DRI".to_string(), String::new()),
            0xfe => ("COM (주석)".to_string(), String::from_utf8_lossy(&r.bytes(data, (len - 2).min(200) as usize).unwrap_or_default()).into_owned()),
            0xc0..=0xcf if marker != 0xc4 && marker != 0xc8 && marker != 0xcc => {
                let (h, w, parts) = (r.u16(data + 1, false)?, r.u16(data + 3, false)?, r.u8(data + 5)?);
                let kind = match marker {
                    0xc0 => "기본",
                    0xc1 => "확장",
                    0xc2 => "프로그레시브",
                    _ => "기타",
                };
                out.summary.push(("그림 크기".into(), format!("{} × {}", num(w), num(h))));
                out.summary.push(("방식".into(), format!("{kind} · 성분 {parts}개 · {}비트", r.u8(data)?)));
                (format!("SOF{} ({kind})", marker - 0xc0), format!("{} × {} · 성분 {parts}개", num(w), num(h)))
            }
            0xda => ("SOS (자료 시작)".to_string(), String::new()),
            _ => (format!("표지 FF {marker:02X}"), String::new()),
        };
        out.fields.push(Field::new(name, at, 2 + len, value).with(vec![
            Field::new("표지", at, 2, format!("FF {marker:02X}")),
            Field::new("길이", at + 2, 2, num(len)),
        ]));
        at += 2 + len;
        if marker == 0xda {
            // 압축된 자료: 다음 표지(FF 뒤에 00·RST가 아닌 바이트)까지 건너뛴다.
            let start = at;
            let mut pos = at;
            let mut found = None;
            'scan: while pos < r.size {
                let block = r.src.read(pos, 1 << 16);
                if block.is_empty() {
                    break;
                }
                for i in 0..block.len() {
                    if block[i] == 0xff {
                        let next = block.get(i + 1).copied().or_else(|| r.u8(pos + i as u64 + 1));
                        if let Some(n) = next {
                            if n != 0 && !(0xd0..=0xd7).contains(&n) && n != 0xff {
                                found = Some(pos + i as u64);
                                break 'scan;
                            }
                        }
                    }
                }
                pos += block.len() as u64;
            }
            let end = found.unwrap_or(r.size);
            out.fields.push(Field::new("압축된 그림 자료", start, end - start, format!("{} 바이트", num(end - start))));
            at = end;
        }
    }
    if at < r.size {
        out.fields.push(Field::new("그림 뒤의 자료", at, r.size - at, format!("{} 바이트", num(r.size - at))));
    }
    Some(())
}

fn gif(r: &Reader, out: &mut Analysis) -> Option<()> {
    out.format = "GIF 그림".into();
    out.magic = "GIF87a / GIF89a".into();
    let version = ascii(&r.bytes(0, 6)?);
    out.fields.push(Field::new("서명", 0, 6, version.clone()));
    let (w, h, flags) = (r.u16(6, true)?, r.u16(8, true)?, r.u8(10)?);
    let table = if flags & 0x80 != 0 { 3u64 << ((flags & 7) + 1) } else { 0 };
    out.fields.push(Field::new("화면 설명", 6, 7, format!("{} × {}", num(w), num(h))).with(vec![
        Field::new("가로", 6, 2, num(w)),
        Field::new("세로", 8, 2, num(h)),
        Field::new("표시", 10, 1, format!("{flags:08b} (전체 색상표 {})", if table > 0 { format!("{}색", table / 3) } else { "없음".into() })),
        Field::new("배경색 번호", 11, 1, r.u8(11)?.to_string()),
        Field::new("화소 비율", 12, 1, r.u8(12)?.to_string()),
    ]));
    if table > 0 {
        out.fields.push(Field::new("전체 색상표", 13, table, format!("{}색", table / 3)));
    }
    out.summary.push(("그림 크기".into(), format!("{} × {}", num(w), num(h))));
    out.summary.push(("판".into(), version));
    Some(())
}

fn bmp(r: &Reader, out: &mut Analysis) -> Option<()> {
    out.format = "BMP 그림".into();
    out.magic = "BM".into();
    let (size, pixels, dib) = (r.u32(2, true)?, r.u32(10, true)?, r.u32(14, true)?);
    out.fields.push(Field::new("파일 머리", 0, 14, "").with(vec![
        Field::new("서명", 0, 2, "BM"),
        Field::new("파일 크기", 2, 4, num(size)),
        Field::new("화소 자료 자리", 10, 4, format!("{pixels:#x}")),
    ]));
    let (w, h, bpp, fields) = if dib == 12 {
        // OS/2의 BITMAPCOREHEADER: 가로·세로가 2바이트이고 압축 칸이 없다.
        let (w, h, bpp) = (i32::from(r.u16(18, true)?), i32::from(r.u16(20, true)?), r.u16(24, true)?);
        (w, h, bpp, vec![Field::new("가로", 18, 2, w.to_string()), Field::new("세로", 20, 2, h.to_string()), Field::new("비트/화소", 24, 2, bpp.to_string())])
    } else {
        let w = r.u32(18, true)? as i32;
        let h = r.u32(22, true)? as i32;
        let bpp = r.u16(28, true)?;
        let compression = r.u32(30, true).unwrap_or(0);
        (w, h, bpp, vec![
            Field::new("가로", 18, 4, w.to_string()),
            Field::new("세로", 22, 4, format!("{h}{}", if h < 0 { " (위에서 아래로)" } else { "" })),
            Field::new("비트/화소", 28, 2, bpp.to_string()),
            Field::new("압축", 30, 4, compression.to_string()),
        ])
    };
    out.fields.push(Field::new("그림 머리(DIB)", 14, u64::from(dib), format!("{dib} 바이트")).with(fields));
    if u64::from(pixels) < r.size {
        out.fields.push(Field::new("화소 자료", u64::from(pixels), r.size - u64::from(pixels), format!("{} 바이트", num(r.size - u64::from(pixels)))));
    }
    out.summary.push(("그림 크기".into(), format!("{} × {}", w, h.unsigned_abs())));
    out.summary.push(("비트/화소".into(), bpp.to_string()));
    Some(())
}

fn ico(r: &Reader, out: &mut Analysis) -> Option<()> {
    let count = r.u16(4, true)?;
    out.format = "아이콘(ICO)".into();
    out.magic = "00 00 01 00".into();
    out.fields.push(Field::new("머리", 0, 6, format!("그림 {count}개")));
    for i in 0..u64::from(count).min(256) {
        let at = 6 + i * 16;
        let (w, h) = (r.u8(at)?, r.u8(at + 1)?);
        let (bpp, size, offset) = (r.u16(at + 6, true)?, r.u32(at + 8, true)?, r.u32(at + 12, true)?);
        let wh = |v: u8| if v == 0 { 256 } else { u32::from(v) };
        out.fields.push(Field::new(format!("그림 {}", i + 1), at, 16, format!("{} × {} · {bpp}비트", wh(w), wh(h))).with(vec![
            Field::new("자료 크기", at + 8, 4, num(size)),
            Field::new("자료 자리", at + 12, 4, format!("{offset:#x}")),
            Field::new("자료", u64::from(offset), u64::from(size), String::new()),
        ]));
    }
    out.summary.push(("그림 수".into(), count.to_string()));
    Some(())
}

// ───────────────────────── 묶음·압축 ─────────────────────────

fn zip(r: &Reader, out: &mut Analysis) -> Option<()> {
    out.format = "ZIP 묶음".into();
    out.magic = "50 4B 03 04 (PK)".into();
    // 끝의 중앙 디렉터리 끝(EOCD)을 찾는다: 마지막 64KB+22 안에 있다.
    let tail_len = r.size.min(65_557) as usize;
    let tail_at = r.size - tail_len as u64;
    let tail = r.src.read(tail_at, tail_len);
    let eocd = tail.windows(4).rposition(|w| w == b"PK\x05\x06").map(|i| tail_at + i as u64);
    let Some(eocd) = eocd else {
        out.notes.push("중앙 디렉터리 끝(PK 05 06)을 찾지 못했습니다(잘렸거나 다른 형식이 붙은 묶음).".into());
        return Some(());
    };
    let mut entries = u64::from(r.u16(eocd + 10, true)?);
    let (mut cd_size, mut cd_at) = (u64::from(r.u32(eocd + 12, true)?), u64::from(r.u32(eocd + 16, true)?));
    if entries == 0xffff || cd_size == 0xffff_ffff || cd_at == 0xffff_ffff {
        // ZIP64: EOCD 바로 앞의 위치 표시(PK 06 07)가 ZIP64 끝 레코드(PK 06 06)의 자리를 알려 준다.
        let located = eocd.checked_sub(20).filter(|&at| r.starts(at, b"PK\x06\x07")).and_then(|at| r.u64(at + 8, true));
        match located.filter(|&at| r.starts(at, b"PK\x06\x06")) {
            Some(at) => {
                entries = r.u64(at + 32, true)?;
                cd_size = r.u64(at + 40, true)?;
                cd_at = r.u64(at + 48, true)?;
                out.format = "ZIP64 묶음".into();
                out.fields.push(Field::new("ZIP64 끝 레코드", at, 56, format!("항목 {}개", num(entries))));
            }
            None => {
                out.notes.push("ZIP64 묶음인데 ZIP64 끝 레코드를 찾지 못했습니다(잘린 묶음).".into());
                return Some(());
            }
        }
    }
    let comment_len = r.u16(eocd + 20, true)?;
    let mut names: Vec<String> = Vec::new();
    let mut items: Vec<Field> = Vec::new();
    let mut at = cd_at;
    let mut total = 0u64;
    for _ in 0..entries.min(MAX_ITEMS as u64) {
        if !r.starts(at, b"PK\x01\x02") {
            out.notes.push(format!("{at:#x}에 중앙 디렉터리 항목이 없습니다."));
            break;
        }
        let method = r.u16(at + 10, true)?;
        let (time, date) = (r.u16(at + 12, true)?, r.u16(at + 14, true)?);
        let (packed, size) = (r.u32(at + 20, true)?, r.u32(at + 24, true)?);
        let (name_len, extra_len, note_len) = (r.u16(at + 28, true)?, r.u16(at + 30, true)?, r.u16(at + 32, true)?);
        let local = u64::from(r.u32(at + 42, true)?);
        let name = String::from_utf8_lossy(&r.bytes(at + 46, usize::from(name_len))?).into_owned();
        total += u64::from(size);
        let method_name = match method {
            0 => "저장(압축 안 함)",
            8 => "Deflate",
            9 => "Deflate64",
            12 => "bzip2",
            14 => "LZMA",
            93 => "Zstandard",
            99 => "AES 암호화",
            _ => "기타",
        };
        let mut children = vec![
            Field::new("압축 방식", at + 10, 2, format!("{method} ({method_name})")),
            Field::new("고친 시각", at + 12, 4, dos_time(date, time)),
            Field::new("CRC-32", at + 16, 4, format!("{:08X}", r.u32(at + 16, true)?)),
            Field::new("압축한 크기", at + 20, 4, num(packed)),
            Field::new("원래 크기", at + 24, 4, num(size)),
            Field::new("이름", at + 46, u64::from(name_len), name.clone()),
        ];
        if r.starts(local, b"PK\x03\x04") {
            let (lname, lextra) = (u64::from(r.u16(local + 26, true)?), u64::from(r.u16(local + 28, true)?));
            let data = local + 30 + lname + lextra;
            children.push(Field::new("파일 머리(로컬)", local, 30 + lname + lextra, format!("{local:#x}")));
            children.push(Field::new("압축된 자료", data, u64::from(packed).min(r.size.saturating_sub(data)), format!("{} 바이트", num(packed))));
        }
        let entry_len = 46 + u64::from(name_len) + u64::from(extra_len) + u64::from(note_len);
        items.push(Field::new(name.clone(), at, entry_len, format!("{} → {} · {method_name}", size_text(u64::from(packed)), num(size))).with(children));
        names.push(name);
        at += entry_len;
    }
    if entries > MAX_ITEMS as u64 {
        out.notes.push(format!("항목이 {}개라 앞의 {MAX_ITEMS}개만 보입니다.", num(entries)));
    }
    let kind = if names.iter().any(|n| n == "word/document.xml") {
        "Word 문서(.docx)"
    } else if names.iter().any(|n| n.starts_with("xl/")) {
        "Excel 통합 문서(.xlsx)"
    } else if names.iter().any(|n| n.starts_with("ppt/")) {
        "PowerPoint 프레젠테이션(.pptx)"
    } else if names.iter().any(|n| n == "AndroidManifest.xml") {
        "안드로이드 앱(.apk)"
    } else if names.iter().any(|n| n == "META-INF/MANIFEST.MF") {
        "Java 묶음(.jar)"
    } else if names.first().is_some_and(|n| n == "mimetype") && r.bytes(30, 8).unwrap_or_default().starts_with(b"mimetype") {
        let mime = r.bytes(38, 40).map(|b| ascii(&b)).unwrap_or_default();
        if mime.starts_with("application/epub") { "전자책(.epub)" } else { "OpenDocument 문서" }
    } else {
        ""
    };
    if !kind.is_empty() {
        out.format = format!("ZIP 묶음 · {kind}");
    }
    out.summary.push(("항목 수".into(), num(entries)));
    out.summary.push(("풀면".into(), size_text(total)));
    out.fields.push(Field::new("중앙 디렉터리", cd_at, cd_size, format!("항목 {}개", num(entries))).with(items));
    out.fields.push(Field::new("중앙 디렉터리 끝", eocd, 22 + u64::from(comment_len), format!("항목 {}개", num(entries))).with(vec![
        Field::new("서명", eocd, 4, "PK 05 06"),
        Field::new("항목 수", eocd + 10, 2, num(entries)),
        Field::new("디렉터리 크기", eocd + 12, 4, num(cd_size)),
        Field::new("디렉터리 자리", eocd + 16, 4, format!("{cd_at:#x}")),
        Field::new("주석 길이", eocd + 20, 2, num(comment_len)),
    ]));
    Some(())
}

fn gzip(r: &Reader, out: &mut Analysis) -> Option<()> {
    out.format = "gzip 압축".into();
    out.magic = "1F 8B".into();
    let (method, flags, mtime, os) = (r.u8(2)?, r.u8(3)?, r.u32(4, true)?, r.u8(9)?);
    let os_name = match os {
        0 => "FAT(DOS·Windows)",
        3 => "유닉스",
        7 => "Macintosh",
        11 => "NTFS",
        255 => "모름",
        _ => "기타",
    };
    let mut children = vec![
        Field::new("서명", 0, 2, "1F 8B"),
        Field::new("압축 방식", 2, 1, if method == 8 { "8 (Deflate)".into() } else { method.to_string() }),
        Field::new("표시", 3, 1, format!("{flags:08b}")),
        Field::new("고친 시각", 4, 4, if mtime == 0 { "없음".into() } else { unix_time(i64::from(mtime)) }),
        Field::new("운영체제", 9, 1, format!("{os} ({os_name})")),
    ];
    let mut at = 10u64;
    if flags & 4 != 0 {
        at += 2 + u64::from(r.u16(at, true)?);
    }
    if flags & 8 != 0 {
        let name = r.src.read(at, 1024);
        let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
        children.push(Field::new("원래 파일 이름", at, end as u64 + 1, String::from_utf8_lossy(&name[..end]).into_owned()));
        out.summary.push(("원래 이름".into(), String::from_utf8_lossy(&name[..end]).into_owned()));
        at += end as u64 + 1;
    }
    if flags & 0x10 != 0 {
        let note = r.src.read(at, 4096);
        let end = note.iter().position(|&b| b == 0).unwrap_or(note.len());
        children.push(Field::new("주석", at, end as u64 + 1, String::from_utf8_lossy(&note[..end]).into_owned()));
        at += end as u64 + 1;
    }
    if flags & 0x02 != 0 {
        children.push(Field::new("머리 CRC-16", at, 2, format!("{:04X}", r.u16(at, true).unwrap_or(0))));
        at += 2;
    }
    let at = at.min(r.size);
    out.fields.push(Field::new("머리", 0, at, "").with(children));
    if r.size >= at + 8 {
        out.fields.push(Field::new("압축된 자료", at, r.size - 8 - at, format!("{} 바이트", num(r.size - 8 - at))));
        let original = r.u32(r.size - 4, true)?;
        out.fields.push(Field::new("꼬리", r.size - 8, 8, "").with(vec![
            Field::new("CRC-32", r.size - 8, 4, format!("{:08X}", r.u32(r.size - 8, true)?)),
            Field::new("원래 크기(4GB로 나눈 나머지)", r.size - 4, 4, num(original)),
        ]));
        out.summary.push(("풀면".into(), size_text(u64::from(original))));
    }
    Some(())
}

fn tar(r: &Reader, out: &mut Analysis) -> Option<()> {
    out.format = "tar 묶음".into();
    out.magic = "257 자리에 ustar".into();
    let mut at = 0u64;
    let mut count = 0;
    while at + 512 <= r.size && count < MAX_ITEMS {
        let head = r.bytes(at, 512)?;
        if head.iter().all(|&b| b == 0) {
            break;
        }
        let name = String::from_utf8_lossy(head[..100].split(|&b| b == 0).next().unwrap_or_default()).into_owned();
        let size_text_field = String::from_utf8_lossy(&head[124..136]).trim_matches(|c: char| c == '\0' || c == ' ').to_string();
        let size = u64::from_str_radix(&size_text_field, 8).unwrap_or(0);
        let kind = match head[156] {
            b'5' => "폴더",
            b'2' => "심볼릭 링크",
            b'1' => "하드 링크",
            _ => "파일",
        };
        out.fields.push(Field::new(name, at, 512 + size.div_ceil(512) * 512, format!("{kind} · {}", size_text(size))));
        at += 512 + size.div_ceil(512) * 512;
        count += 1;
    }
    out.summary.push(("항목 수".into(), num(count as u64)));
    Some(())
}

fn pdf(r: &Reader, out: &mut Analysis) -> Option<()> {
    let line = r.src.read(0, 32);
    let end = line.iter().position(|&b| b == b'\r' || b == b'\n').unwrap_or(line.len());
    let version = ascii(&line[5..end.max(5)]);
    out.format = "PDF 문서".into();
    out.magic = "%PDF-".into();
    out.fields.push(Field::new("머리", 0, end as u64, format!("PDF {version}")));
    out.summary.push(("판".into(), version));
    let tail_len = r.size.min(4096) as usize;
    let tail_at = r.size - tail_len as u64;
    let tail = r.src.read(tail_at, tail_len);
    if let Some(i) = tail.windows(9).rposition(|w| w == b"startxref") {
        let rest = String::from_utf8_lossy(&tail[i + 9..]).into_owned();
        let xref = rest.split_whitespace().next().unwrap_or("").to_string();
        out.fields.push(Field::new("startxref", tail_at + i as u64, 9 + xref.len() as u64 + 1, format!("상호 참조 표 자리 {xref}")));
    }
    let eofs = tail.windows(5).filter(|w| w == b"%%EOF").count();
    if let Some(i) = tail.windows(5).rposition(|w| w == b"%%EOF") {
        out.fields.push(Field::new("%%EOF", tail_at + i as u64, 5, "파일 끝 표시"));
    } else {
        out.notes.push("끝 표시(%%EOF)가 없습니다(잘린 파일일 수 있습니다).".into());
    }
    if eofs > 1 {
        out.notes.push("끝 표시가 여러 번 나옵니다(고쳐 저장한 PDF: 뒤에 덧붙인 판이 있습니다).".into());
    }
    let sample = String::from_utf8_lossy(&r.src.read(0, 1 << 20)).into_owned();
    let pages = sample.matches("/Type /Page").count() + sample.matches("/Type/Page").count() - sample.matches("/Type /Pages").count() - sample.matches("/Type/Pages").count();
    if pages > 0 {
        out.summary.push(("쪽(앞 1MB에서 셈)".into(), num(pages as u64)));
    }
    Some(())
}

// ───────────────────────── 실행 파일 ─────────────────────────

fn pe(r: &Reader, out: &mut Analysis) -> Option<()> {
    out.format = "DOS 실행 파일(MZ)".into();
    out.magic = "4D 5A (MZ)".into();
    let lfanew = u64::from(r.u32(0x3c, true)?);
    out.fields.push(Field::new("DOS 머리", 0, 64, "").with(vec![
        Field::new("e_magic", 0, 2, "MZ"),
        Field::new("e_lfanew (PE 머리 자리)", 0x3c, 4, format!("{lfanew:#x}")),
    ]));
    if lfanew > 64 {
        out.fields.push(Field::new("DOS 스텁", 64, lfanew - 64, "\"This program cannot be run in DOS mode\" 같은 작은 프로그램"));
    }
    if !r.starts(lfanew, b"PE\0\0") {
        out.notes.push("PE 서명이 없습니다(옛 DOS 프로그램이거나 다른 형식).".into());
        return Some(());
    }
    let coff = lfanew + 4;
    let machine = r.u16(coff, true)?;
    let sections = r.u16(coff + 2, true)?;
    let stamp = r.u32(coff + 4, true)?;
    let opt_size = r.u16(coff + 16, true)?;
    let chars = r.u16(coff + 18, true)?;
    let machine_name = match machine {
        0x14c => "x86 (32비트)",
        0x8664 => "x64",
        0xaa64 => "ARM64",
        0x1c4 => "ARM (Thumb-2)",
        0x200 => "IA-64",
        _ => "기타",
    };
    let dll = chars & 0x2000 != 0;
    let mut flags = Vec::new();
    if chars & 0x2 != 0 {
        flags.push("실행 가능");
    }
    if dll {
        flags.push("DLL");
    }
    if chars & 0x20 != 0 {
        flags.push("2GB 넘는 주소");
    }
    out.fields.push(Field::new("PE 서명", lfanew, 4, "50 45 00 00 (PE)"));
    out.fields.push(Field::new("COFF 머리", coff, 20, machine_name).with(vec![
        Field::new("기계", coff, 2, format!("{machine:#06x} ({machine_name})")),
        Field::new("구역 수", coff + 2, 2, sections.to_string()),
        Field::new("만든 시각", coff + 4, 4, format!("{} ({stamp:#x})", unix_time(i64::from(stamp)))),
        Field::new("선택 머리 크기", coff + 16, 2, opt_size.to_string()),
        Field::new("특성", coff + 18, 2, format!("{chars:#06x} ({})", flags.join(", "))),
    ]));
    let opt = coff + 20;
    let magic = r.u16(opt, true)?;
    let plus = magic == 0x20b;
    let entry = r.u32(opt + 16, true)?;
    let base = if plus { r.u64(opt + 24, true)? } else { u64::from(r.u32(opt + 28, true)?) };
    let subsystem = r.u16(opt + 68, true)?;
    let subsystem_name = match subsystem {
        1 => "드라이버(native)",
        2 => "창 프로그램(GUI)",
        3 => "콘솔 프로그램",
        9 => "Windows CE",
        10 => "EFI 프로그램",
        _ => "기타",
    };
    let dirs_at = opt + if plus { 112 } else { 96 };
    let dir_count = r.u32(dirs_at - 4, true).unwrap_or(0).min(16);
    let clr = if dir_count > 14 { r.u32(dirs_at + 14 * 8, true).unwrap_or(0) != 0 } else { false };
    const DIRS: [&str; 16] = [
        "내보내기", "가져오기", "리소스", "예외", "인증서", "재배치", "디버그", "아키텍처", "전역 포인터", "TLS", "로드 구성", "바운드 가져오기", "IAT", "지연 가져오기", "CLR(.NET)", "예약",
    ];
    let mut dirs = Vec::new();
    for i in 0..u64::from(dir_count) {
        let (rva, size) = (r.u32(dirs_at + i * 8, true)?, r.u32(dirs_at + i * 8 + 4, true)?);
        if size > 0 {
            dirs.push(Field::new(DIRS[i as usize], dirs_at + i * 8, 8, format!("RVA {rva:#x} · {} 바이트", num(size))));
        }
    }
    out.fields.push(Field::new("선택 머리", opt, u64::from(opt_size), if plus { "PE32+ (64비트)" } else { "PE32 (32비트)" }).with(vec![
        Field::new("Magic", opt, 2, format!("{magic:#x} ({})", if plus { "PE32+" } else { "PE32" })),
        Field::new("링커 판", opt + 2, 2, format!("{}.{}", r.u8(opt + 2)?, r.u8(opt + 3)?)),
        Field::new("시작 주소(RVA)", opt + 16, 4, format!("{entry:#x}")),
        Field::new("이미지 기준 주소", opt + if plus { 24 } else { 28 }, if plus { 8 } else { 4 }, format!("{base:#x}")),
        Field::new("서브시스템", opt + 68, 2, format!("{subsystem} ({subsystem_name})")),
        Field::new("자료 디렉터리", dirs_at, u64::from(dir_count) * 8, format!("{}개 중 {}개 사용", dir_count, dirs.len())).with(dirs),
    ]));
    let table = opt + u64::from(opt_size);
    let mut items = Vec::new();
    for i in 0..u64::from(sections).min(96) {
        let at = table + i * 40;
        let name: String = String::from_utf8_lossy(&r.bytes(at, 8)?).trim_end_matches('\0').chars().filter(|c| !c.is_control()).collect();
        let (vsize, vaddr, raw_size, raw_at, flags) = (r.u32(at + 8, true)?, r.u32(at + 12, true)?, r.u32(at + 16, true)?, r.u32(at + 20, true)?, r.u32(at + 36, true)?);
        let mut kinds = Vec::new();
        if flags & 0x20 != 0 {
            kinds.push("코드");
        }
        if flags & 0x2000_0000 != 0 {
            kinds.push("실행");
        }
        if flags & 0x4000_0000 != 0 {
            kinds.push("읽기");
        }
        if flags & 0x8000_0000 != 0 {
            kinds.push("쓰기");
        }
        let mut children = vec![
            Field::new("이름", at, 8, name.clone()),
            Field::new("메모리 크기", at + 8, 4, num(vsize)),
            Field::new("메모리 자리(RVA)", at + 12, 4, format!("{vaddr:#x}")),
            Field::new("파일 속 크기", at + 16, 4, num(raw_size)),
            Field::new("파일 속 자리", at + 20, 4, format!("{raw_at:#x}")),
            Field::new("특성", at + 36, 4, format!("{flags:#010x} ({})", kinds.join(", "))),
        ];
        if raw_size > 0 {
            children.push(Field::new("구역 자료", u64::from(raw_at), u64::from(raw_size).min(r.size.saturating_sub(u64::from(raw_at))), format!("{} 바이트", num(raw_size))));
        }
        items.push(Field::new(format!("구역 {name}"), at, 40, format!("{} · {}", size_text(u64::from(raw_size)), kinds.join(" "))).with(children));
    }
    out.fields.push(Field::new("구역 표", table, u64::from(sections) * 40, format!("{sections}개")).with(items));
    out.format = format!("Windows {}{} ({machine_name})", if clr { ".NET " } else { "" }, if dll { "DLL" } else { "실행 파일(PE)" });
    out.magic = "4D 5A … 50 45 00 00 (MZ … PE)".into();
    out.summary.push(("기계".into(), machine_name.into()));
    out.summary.push(("서브시스템".into(), subsystem_name.into()));
    out.summary.push(("만든 시각".into(), unix_time(i64::from(stamp))));
    Some(())
}

fn elf(r: &Reader, out: &mut Analysis) -> Option<()> {
    let class = r.u8(4)?;
    let le = r.u8(5)? != 2;
    let wide = class == 2;
    let kind = r.u16(16, le)?;
    let machine = r.u16(18, le)?;
    let kind_name = match kind {
        1 => "재배치 가능(.o)",
        2 => "실행 파일",
        3 => "공유 라이브러리·PIE",
        4 => "코어 덤프",
        _ => "기타",
    };
    let machine_name = match machine {
        3 => "x86",
        40 => "ARM",
        62 => "x86-64",
        183 => "AArch64",
        243 => "RISC-V",
        8 => "MIPS",
        20 => "PowerPC",
        _ => "기타",
    };
    out.format = format!("ELF {kind_name} ({machine_name}, {})", if wide { "64비트" } else { "32비트" });
    out.magic = "7F 45 4C 46 (ELF)".into();
    let (entry, phoff, shoff) = if wide {
        (r.u64(24, le)?, r.u64(32, le)?, r.u64(40, le)?)
    } else {
        (u64::from(r.u32(24, le)?), u64::from(r.u32(28, le)?), u64::from(r.u32(32, le)?))
    };
    let base = if wide { 48 } else { 36 };
    let (phentsize, phnum, shentsize, shnum, shstrndx) = (r.u16(base + 6, le)?, r.u16(base + 8, le)?, r.u16(base + 10, le)?, r.u16(base + 12, le)?, r.u16(base + 14, le)?);
    let w = if wide { 8 } else { 4 };
    out.fields.push(Field::new("ELF 머리", 0, base + 16, "").with(vec![
        Field::new("서명", 0, 4, "7F 45 4C 46"),
        Field::new("종류(클래스)", 4, 1, if wide { "64비트".into() } else { "32비트".to_string() }),
        Field::new("바이트 순서", 5, 1, if le { "리틀 엔디언".into() } else { "빅 엔디언".to_string() }),
        Field::new("OS/ABI", 7, 1, r.u8(7)?.to_string()),
        Field::new("파일 종류", 16, 2, format!("{kind} ({kind_name})")),
        Field::new("기계", 18, 2, format!("{machine} ({machine_name})")),
        Field::new("시작 주소", 24, w, format!("{entry:#x}")),
        Field::new("프로그램 머리 자리", 24 + w, w, format!("{phoff:#x} · {phnum}개")),
        Field::new("구역 머리 자리", 24 + 2 * w, w, format!("{shoff:#x} · {shnum}개")),
    ]));
    if phnum > 0 && phentsize > 0 {
        let mut items = Vec::new();
        for i in 0..u64::from(phnum).min(256) {
            let at = phoff.checked_add(i * u64::from(phentsize))?;
            let ptype = r.u32(at, le)?;
            let (offset, filesz) = if wide { (r.u64(at + 8, le)?, r.u64(at + 32, le)?) } else { (u64::from(r.u32(at + 4, le)?), u64::from(r.u32(at + 16, le)?)) };
            let name = match ptype {
                1 => "LOAD",
                2 => "DYNAMIC",
                3 => "INTERP",
                4 => "NOTE",
                6 => "PHDR",
                7 => "TLS",
                0x6474_e550 => "GNU_EH_FRAME",
                0x6474_e551 => "GNU_STACK",
                0x6474_e552 => "GNU_RELRO",
                _ => "기타",
            };
            items.push(Field::new(name, at, u64::from(phentsize), format!("{offset:#x} · {} 바이트", num(filesz))).with(vec![Field::new("내용", offset, filesz.min(r.size.saturating_sub(offset)), String::new())]));
        }
        out.fields.push(Field::new("프로그램 머리", phoff, u64::from(phnum) * u64::from(phentsize), format!("{phnum}개")).with(items));
    }
    if shnum > 0 && shentsize > 0 && shoff > 0 {
        // 구역 이름 표
        let entry_at = |i: u64| shoff.saturating_add(i * u64::from(shentsize));
        let read_section = |i: u64| -> Option<(u32, u32, u64, u64)> {
            let at = shoff.checked_add(i * u64::from(shentsize))?;
            let (name, kind) = (r.u32(at, le)?, r.u32(at + 4, le)?);
            let (offset, size) = if wide { (r.u64(at + 24, le)?, r.u64(at + 32, le)?) } else { (u64::from(r.u32(at + 16, le)?), u64::from(r.u32(at + 20, le)?)) };
            Some((name, kind, offset, size))
        };
        let names = read_section(u64::from(shstrndx)).and_then(|(_, _, offset, size)| r.bytes(offset, size.min(1 << 20) as usize)).unwrap_or_default();
        let mut items = Vec::new();
        for i in 0..u64::from(shnum).min(512) {
            let Some((name_at, kind, offset, size)) = read_section(i) else { break };
            let name = names.get(name_at as usize..).map(|rest| String::from_utf8_lossy(rest.split(|&b| b == 0).next().unwrap_or_default()).into_owned()).unwrap_or_default();
            let kind_name = match kind {
                0 => "NULL",
                1 => "PROGBITS",
                2 => "SYMTAB",
                3 => "STRTAB",
                4 => "RELA",
                6 => "DYNAMIC",
                7 => "NOTE",
                8 => "NOBITS",
                11 => "DYNSYM",
                _ => "기타",
            };
            let mut field = Field::new(if name.is_empty() { format!("구역 {i}") } else { name }, entry_at(i), u64::from(shentsize), format!("{kind_name} · {} 바이트", num(size)));
            if kind != 8 && size > 0 {
                field = field.with(vec![Field::new("내용", offset, size.min(r.size.saturating_sub(offset)), String::new())]);
            }
            items.push(field);
        }
        out.fields.push(Field::new("구역 머리", shoff, u64::from(shnum) * u64::from(shentsize), format!("{shnum}개")).with(items));
    }
    out.summary.push(("종류".into(), kind_name.into()));
    out.summary.push(("기계".into(), machine_name.into()));
    out.summary.push(("시작 주소".into(), format!("{entry:#x}")));
    Some(())
}

/// CA FE BA BE: Java 클래스 파일이거나 Mach-O 여러 구조 묶음(fat). 둘째 값으로 가린다.
fn cafebabe(r: &Reader, out: &mut Analysis) -> Option<()> {
    let second = r.u32(4, false)?;
    if second < 45 || second > 0x0000_ffff && (second & 0xffff) < 45 {
        // Mach-O fat: 둘째 값이 구조 수(작은 수)
        if second > 0 && second < 32 {
            out.format = "Mach-O 여러 구조 묶음(universal)".into();
            out.magic = "CA FE BA BE".into();
            let mut items = Vec::new();
            for i in 0..u64::from(second) {
                let at = 8 + i * 20;
                let (cpu, offset, size) = (r.u32(at, false)?, r.u32(at + 8, false)?, r.u32(at + 12, false)?);
                items.push(Field::new(format!("구조 {}", i + 1), at, 20, format!("CPU {cpu:#x} · {} 바이트", num(size))).with(vec![Field::new("내용", u64::from(offset), u64::from(size), String::new())]));
            }
            out.fields.push(Field::new("묶음 머리", 0, 8 + u64::from(second) * 20, format!("구조 {second}개")).with(items));
            return Some(());
        }
    }
    let (minor, major) = (r.u16(4, false)?, r.u16(6, false)?);
    if major < 45 {
        out.format = "CA FE BA BE로 시작하는 자료(Java 클래스도 Mach-O 묶음도 아님)".into();
        out.magic = "CA FE BA BE".into();
        return Some(());
    }
    let java = match major {
        45 => "1.1".to_string(),
        46 => "1.2".into(),
        47 => "1.3".into(),
        48 => "1.4".into(),
        n => (u32::from(n) - 44).to_string(),
    };
    out.format = format!("Java 클래스 파일 (Java {java})");
    out.magic = "CA FE BA BE".into();
    out.fields.push(Field::new("서명", 0, 4, "CA FE BA BE"));
    out.fields.push(Field::new("판", 4, 4, format!("{major}.{minor} (Java {java})")));
    out.fields.push(Field::new("상수 풀 크기", 8, 2, r.u16(8, false)?.to_string()));
    out.summary.push(("Java 판".into(), java));
    Some(())
}

fn macho(r: &Reader, out: &mut Analysis) -> Option<()> {
    let magic = r.u32(0, false)?;
    let le = matches!(magic, 0xcefa_edfe | 0xcffa_edfe);
    let wide = matches!(magic, 0xfeed_facf | 0xcffa_edfe);
    let (cpu, kind, ncmds) = (r.u32(4, le)?, r.u32(12, le)?, r.u32(16, le)?);
    let cpu_name = match cpu & 0xff_ffff {
        7 => "x86",
        12 => "ARM",
        _ => "기타",
    };
    let cpu_name = if cpu & 0x0100_0000 != 0 { if cpu_name == "x86" { "x86-64" } else if cpu_name == "ARM" { "ARM64" } else { cpu_name } } else { cpu_name };
    let kind_name = match kind {
        1 => "목적 파일",
        2 => "실행 파일",
        6 => "동적 라이브러리",
        8 => "번들",
        _ => "기타",
    };
    out.format = format!("Mach-O {kind_name} ({cpu_name}, {})", if wide { "64비트" } else { "32비트" });
    out.magic = hex(&r.bytes(0, 4)?);
    out.fields.push(Field::new("Mach-O 머리", 0, if wide { 32 } else { 28 }, "").with(vec![
        Field::new("CPU", 4, 4, format!("{cpu:#x} ({cpu_name})")),
        Field::new("파일 종류", 12, 4, format!("{kind} ({kind_name})")),
        Field::new("명령 수", 16, 4, ncmds.to_string()),
    ]));
    Some(())
}

// ───────────────────────── 자료·소리·영상 ─────────────────────────

fn sqlite(r: &Reader, out: &mut Analysis) -> Option<()> {
    out.format = "SQLite 데이터베이스".into();
    out.magic = "SQLite format 3 + 00".into();
    let page = match r.u16(16, false)? {
        1 => 65536,
        n => u32::from(n),
    };
    let pages = r.u32(28, false)?;
    let encoding = match r.u32(56, false)? {
        1 => "UTF-8",
        2 => "UTF-16 LE",
        3 => "UTF-16 BE",
        _ => "알 수 없음",
    };
    out.fields.push(Field::new("데이터베이스 머리", 0, 100, "").with(vec![
        Field::new("서명", 0, 16, "SQLite format 3"),
        Field::new("쪽 크기", 16, 2, num(page)),
        Field::new("쓰기 판 / 읽기 판", 18, 2, format!("{} / {} (2면 WAL)", r.u8(18)?, r.u8(19)?)),
        Field::new("바뀐 횟수", 24, 4, num(r.u32(24, false)?)),
        Field::new("쪽 수", 28, 4, num(pages)),
        Field::new("글자 인코딩", 56, 4, encoding),
        Field::new("사용자 판", 60, 4, r.u32(60, false)?.to_string()),
        Field::new("앱 번호", 68, 4, format!("{:#x}", r.u32(68, false)?)),
    ]));
    out.summary.push(("쪽".into(), format!("{} × {}", num(pages), size_text(u64::from(page)))));
    out.summary.push(("인코딩".into(), encoding.into()));
    Some(())
}

fn riff(r: &Reader, out: &mut Analysis) -> Option<()> {
    let le = r.starts(0, b"RIFF");
    let size = u64::from(r.u32(4, le)?);
    let form = ascii(&r.bytes(8, 4)?);
    out.format = match form.as_str() {
        "WAVE" => "WAV 소리".into(),
        "AVI " => "AVI 영상".into(),
        "WEBP" => "WebP 그림".into(),
        other => format!("RIFF ({other})"),
    };
    out.magic = format!("RIFF … {form}");
    out.fields.push(Field::new("RIFF 머리", 0, 12, format!("{form} · {} 바이트", num(size))));
    let mut at = 12u64;
    let mut count = 0;
    let mut wave_rate = 0u32;
    let mut wave_bytes = 0u32;
    while at + 8 <= r.size && count < MAX_ITEMS {
        count += 1;
        let id = ascii(&r.bytes(at, 4)?);
        let len = u64::from(r.u32(at + 4, le)?);
        let mut value = format!("{} 바이트", num(len));
        let mut children = Vec::new();
        if id == "fmt " && len >= 16 {
            let (tag, channels, rate, bytes_per_sec, bits) = (r.u16(at + 8, le)?, r.u16(at + 10, le)?, r.u32(at + 12, le)?, r.u32(at + 16, le)?, r.u16(at + 22, le)?);
            wave_rate = bytes_per_sec;
            let tag_name = match tag {
                1 => "PCM",
                3 => "IEEE float",
                0xfffe => "확장",
                _ => "기타",
            };
            value = format!("{tag_name} · {channels}채널 · {} Hz · {bits}비트", num(rate));
            children = vec![
                Field::new("형식", at + 8, 2, format!("{tag} ({tag_name})")),
                Field::new("채널", at + 10, 2, channels.to_string()),
                Field::new("표본 빈도", at + 12, 4, format!("{} Hz", num(rate))),
                Field::new("초당 바이트", at + 16, 4, num(bytes_per_sec)),
                Field::new("비트/표본", at + 22, 2, bits.to_string()),
            ];
            out.summary.push(("소리".into(), value.clone()));
        } else if id == "data" {
            wave_bytes = len as u32;
        } else if id == "VP8X" && len >= 10 {
            let w = u32::from(r.u8(at + 12)?) | u32::from(r.u8(at + 13)?) << 8 | u32::from(r.u8(at + 14)?) << 16;
            let h = u32::from(r.u8(at + 15)?) | u32::from(r.u8(at + 16)?) << 8 | u32::from(r.u8(at + 17)?) << 16;
            value = format!("{} × {}", num(w + 1), num(h + 1));
            out.summary.push(("그림 크기".into(), value.clone()));
        }
        let body = len.min(r.size.saturating_sub(at + 8));
        children.insert(0, Field::new("내용", at + 8, body, String::new()));
        out.fields.push(Field::new(format!("덩어리 {id}"), at, 8 + body, value).with(children));
        at += 8 + len + (len & 1);
    }
    if wave_rate > 0 && wave_bytes > 0 {
        out.summary.push(("길이".into(), format!("{:.1}초", f64::from(wave_bytes) / f64::from(wave_rate))));
    }
    Some(())
}

/// MP4·MOV·HEIC·AVIF(ISO 기본 미디어 형식): 맨 위 상자들.
fn isobmff(r: &Reader, out: &mut Analysis) -> Option<()> {
    let brand = ascii(&r.bytes(8, 4)?);
    out.format = match brand.trim() {
        "qt" => "QuickTime 영상(MOV)".to_string(),
        "heic" | "heix" | "mif1" | "msf1" => "HEIF 그림(HEIC)".into(),
        "avif" | "avis" => "AVIF 그림".into(),
        "M4A" => "MPEG-4 소리(M4A)".into(),
        "3gp4" | "3gp5" | "3g2a" => "3GPP 영상".into(),
        _ => "MPEG-4 영상(MP4)".into(),
    };
    out.magic = format!("4 자리에 ftyp ({brand})");
    let mut at = 0u64;
    let mut count = 0;
    while at + 8 <= r.size && count < MAX_ITEMS {
        count += 1;
        let small = u64::from(r.u32(at, false)?);
        let kind = ascii(&r.bytes(at + 4, 4)?);
        let (len, head) = match small {
            1 => (r.u64(at + 8, false)?, 16),
            0 => (r.size - at, 8),
            n => (n, 8),
        };
        if len < head {
            out.notes.push(format!("{at:#x}의 상자 크기가 맞지 않습니다."));
            break;
        }
        let shown = len.min(r.size - at);
        if len > r.size - at {
            out.notes.push(format!("{at:#x}의 상자({kind})가 파일 끝을 넘습니다(파일이 잘렸습니다)."));
        }
        let value = match kind.as_str() {
            "ftyp" => {
                let brands: Vec<String> = (16..len.min(64)).step_by(4).filter_map(|o| r.bytes(at + o, 4).map(|b| ascii(&b))).collect();
                format!("주 상표 {brand} · 호환 {}", brands.join(", "))
            }
            "moov" => "영상 정보(트랙·길이)".into(),
            "mdat" => format!("영상·소리 자료 {}", size_text(len)),
            "free" | "skip" => "빈 자리".into(),
            "meta" => "메타데이터".into(),
            _ => format!("{} 바이트", num(len)),
        };
        out.fields.push(Field::new(format!("상자 {kind}"), at, shown, value));
        at += shown;
    }
    Some(())
}

fn flac(r: &Reader, out: &mut Analysis) -> Option<()> {
    out.format = "FLAC 소리".into();
    out.magic = "fLaC".into();
    out.fields.push(Field::new("서명", 0, 4, "fLaC"));
    // STREAMINFO: 4 자리부터 머리(4) + 34바이트
    let info = r.bytes(8, 18)?;
    let rate = (u32::from(info[10]) << 12) | (u32::from(info[11]) << 4) | (u32::from(info[12]) >> 4);
    let channels = ((info[12] >> 1) & 7) + 1;
    let bits = (((info[12] & 1) << 4) | (info[13] >> 4)) + 1;
    let samples = (u64::from(info[13] & 0xf) << 32) | u64::from(u32::from_be_bytes([info[14], info[15], info[16], info[17]]));
    out.fields.push(Field::new("STREAMINFO", 4, 38, format!("{} Hz · {channels}채널 · {bits}비트", num(rate))));
    out.summary.push(("소리".into(), format!("{} Hz · {channels}채널 · {bits}비트", num(rate))));
    if rate > 0 {
        out.summary.push(("길이".into(), format!("{:.1}초", samples as f64 / f64::from(rate))));
    }
    Some(())
}

fn id3(r: &Reader, out: &mut Analysis) -> Option<()> {
    out.format = "MP3 소리(ID3 태그)".into();
    out.magic = "ID3".into();
    let size_bytes = r.bytes(6, 4)?;
    let size = size_bytes.iter().fold(0u64, |acc, &b| (acc << 7) | u64::from(b & 0x7f));
    out.fields.push(Field::new("ID3 태그", 0, 10 + size, format!("ID3v2.{}.{} · {} 바이트", r.u8(3)?, r.u8(4)?, num(size))).with(vec![
        Field::new("판", 3, 2, format!("2.{}.{}", r.u8(3)?, r.u8(4)?)),
        Field::new("크기(7비트씩)", 6, 4, num(size)),
    ]));
    if 10 + size < r.size {
        out.fields.push(Field::new("소리 자료", 10 + size, r.size - 10 - size, format!("{} 바이트", num(r.size - 10 - size))));
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(bytes: &[u8]) -> Analysis {
        analyze(&Bytes(bytes))
    }

    fn png_chunk(kind: &[u8], data: &[u8]) -> Vec<u8> {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let mut crc_input = kind.to_vec();
        crc_input.extend_from_slice(data);
        out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
        out
    }

    fn names(fields: &[Field]) -> Vec<&str> {
        fields.iter().map(|f| f.name.as_str()).collect()
    }

    #[test]
    fn png를_덩어리로_읽는다() {
        let mut file = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = 640u32.to_be_bytes().to_vec();
        ihdr.extend(480u32.to_be_bytes());
        ihdr.extend([8, 6, 0, 0, 0]);
        file.extend(png_chunk(b"IHDR", &ihdr));
        file.extend(png_chunk(b"tEXt", b"Software\0Cypher"));
        let mut bad = png_chunk(b"IDAT", b"xyz");
        let n = bad.len();
        bad[n - 1] ^= 1; // CRC를 깨뜨린다.
        file.extend(bad);
        file.extend(png_chunk(b"IEND", b""));
        file.extend(b"tail");
        let result = run(file.as_slice());
        assert_eq!(result.format, "PNG 그림");
        assert_eq!(names(&result.fields), ["서명", "덩어리 IHDR", "덩어리 tEXt", "덩어리 IDAT", "덩어리 IEND", "IEND 뒤의 자료"]);
        assert!(result.summary.contains(&("그림 크기".into(), "640 × 480".into())));
        assert_eq!(result.fields[2].value, "Software = Cypher");
        let ihdr = &result.fields[1];
        assert_eq!((ihdr.offset, ihdr.len), (8, 25));
        assert!(ihdr.children.iter().any(|f| f.name == "CRC" && f.value.ends_with("(맞음)")));
        assert!(result.notes.iter().any(|n| n.contains("IDAT") && n.contains("CRC")));
        // 잘린 파일도 읽을 수 있는 데까지 읽는다.
        let cut = run(&file[..40]);
        assert_eq!(cut.format, "PNG 그림");
        assert!(cut.notes.iter().any(|n| n.contains("잘렸습니다")), "{:?}", cut.notes);
    }

    #[test]
    fn 실행_파일의_머리와_구역을_읽는다() {
        // 작은 PE32+ 파일: DOS 머리 → PE 서명 → COFF → 선택 머리(240) → 구역 표 1개
        let mut file = vec![0u8; 0x400];
        file[0..2].copy_from_slice(b"MZ");
        file[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        file[0x80..0x84].copy_from_slice(b"PE\0\0");
        let coff = 0x84;
        file[coff..coff + 2].copy_from_slice(&0x8664u16.to_le_bytes());
        file[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
        file[coff + 4..coff + 8].copy_from_slice(&1_700_000_000u32.to_le_bytes());
        file[coff + 16..coff + 18].copy_from_slice(&240u16.to_le_bytes());
        file[coff + 18..coff + 20].copy_from_slice(&0x22u16.to_le_bytes());
        let opt = coff + 20;
        file[opt..opt + 2].copy_from_slice(&0x20bu16.to_le_bytes());
        file[opt + 16..opt + 20].copy_from_slice(&0x1234u32.to_le_bytes());
        file[opt + 24..opt + 32].copy_from_slice(&0x1_4000_0000u64.to_le_bytes());
        file[opt + 68..opt + 70].copy_from_slice(&3u16.to_le_bytes());
        file[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes());
        let section = opt + 240;
        file[section..section + 5].copy_from_slice(b".text");
        file[section + 16..section + 20].copy_from_slice(&0x100u32.to_le_bytes());
        file[section + 20..section + 24].copy_from_slice(&0x200u32.to_le_bytes());
        file[section + 36..section + 40].copy_from_slice(&0x6000_0020u32.to_le_bytes());
        let result = run(file.as_slice());
        assert_eq!(result.format, "Windows 실행 파일(PE) (x64)");
        assert!(result.summary.contains(&("서브시스템".into(), "콘솔 프로그램".into())));
        assert!(result.summary.contains(&("만든 시각".into(), "2023-11-14 22:13:20 UTC".into())));
        let table = result.fields.iter().find(|f| f.name == "구역 표").unwrap();
        let text = &table.children[0];
        assert_eq!(text.name, "구역 .text");
        assert!(text.children.iter().any(|f| f.name == "구역 자료" && f.offset == 0x200 && f.len == 0x100));
        // PE 서명이 없으면 DOS 프로그램으로 둔다.
        let mut dos = vec![0u8; 128];
        dos[0..2].copy_from_slice(b"MZ");
        assert!(run(dos.as_slice()).notes.iter().any(|n| n.contains("PE 서명")));
    }

    #[test]
    fn zip과_office_문서를_알아본다() {
        // 항목 하나(word/document.xml, 저장 방식)
        let name = b"word/document.xml";
        let data = b"<w/>";
        let mut file = Vec::new();
        file.extend(b"PK\x03\x04");
        file.extend([20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        file.extend(crc32(data).to_le_bytes());
        file.extend((data.len() as u32).to_le_bytes());
        file.extend((data.len() as u32).to_le_bytes());
        file.extend((name.len() as u16).to_le_bytes());
        file.extend([0, 0]);
        file.extend(name);
        file.extend(data);
        let cd = file.len() as u32;
        file.extend(b"PK\x01\x02");
        file.extend([20, 0, 20, 0, 0, 0, 0, 0, 0x21, 0x58, 0x51, 0x57]);
        file.extend(crc32(data).to_le_bytes());
        file.extend((data.len() as u32).to_le_bytes());
        file.extend((data.len() as u32).to_le_bytes());
        file.extend((name.len() as u16).to_le_bytes());
        file.extend([0u8; 12]);
        file.extend(0u32.to_le_bytes());
        file.extend(name);
        let cd_size = file.len() as u32 - cd;
        file.extend(b"PK\x05\x06");
        file.extend([0, 0, 0, 0, 1, 0, 1, 0]);
        file.extend(cd_size.to_le_bytes());
        file.extend(cd.to_le_bytes());
        file.extend([0, 0]);
        let result = run(file.as_slice());
        assert_eq!(result.format, "ZIP 묶음 · Word 문서(.docx)");
        let directory = result.fields.iter().find(|f| f.name == "중앙 디렉터리").unwrap();
        let entry = &directory.children[0];
        assert_eq!(entry.name, "word/document.xml");
        assert!(entry.children.iter().any(|f| f.name == "압축된 자료" && f.offset == 30 + name.len() as u64 && f.len == 4));
        assert!(entry.children.iter().any(|f| f.name == "고친 시각" && f.value == "2023-10-17 11:01:02"));
        // 중앙 디렉터리가 잘린 묶음
        assert!(run(&file[..40]).notes.iter().any(|n| n.contains("PK 05 06")));
    }

    #[test]
    fn 그_밖의_형식과_글자_파일을_알아본다() {
        let gif = b"GIF89a\x40\x01\xf0\x00\x00\x00\x00";
        assert!(run(gif.as_slice()).summary.contains(&("그림 크기".into(), "320 × 240".into())));
        let mut elf = vec![0u8; 64];
        elf[..4].copy_from_slice(b"\x7fELF");
        elf[4] = 2;
        elf[5] = 1;
        elf[16..18].copy_from_slice(&3u16.to_le_bytes());
        elf[18..20].copy_from_slice(&62u16.to_le_bytes());
        assert_eq!(run(elf.as_slice()).format, "ELF 공유 라이브러리·PIE (x86-64, 64비트)");
        assert_eq!(run(b"\xca\xfe\xba\xbe\x00\x00\x00\x41\x00\x10".as_slice()).format, "Java 클래스 파일 (Java 21)");
        assert_eq!(run(b"\xca\xfe\xba\xbe\x00\x00\x00\x02".as_slice()).format, "Mach-O 여러 구조 묶음(universal)");
        let mut sqlite = b"SQLite format 3\0".to_vec();
        sqlite.resize(100, 0);
        sqlite[16..18].copy_from_slice(&4096u16.to_be_bytes());
        sqlite[56..60].copy_from_slice(&1u32.to_be_bytes());
        assert!(run(sqlite.as_slice()).summary.contains(&("인코딩".into(), "UTF-8".into())));
        let mut wav = b"RIFF\0\0\0\0WAVEfmt \x10\0\0\0\x01\0\x02\0\x44\xac\0\0\x10\xb1\x02\0\x04\0\x10\0data".to_vec();
        wav.extend(176_400u32.to_le_bytes());
        wav.extend(vec![0u8; 16]);
        let wav = run(wav.as_slice());
        assert_eq!(wav.format, "WAV 소리");
        assert!(wav.summary.contains(&("길이".into(), "1.0초".into())), "{:?}", wav.summary);
        assert_eq!(run(b"7z\xbc\xaf\x27\x1c\0\x04".as_slice()).format, "7z 묶음");
        assert_eq!(run("한글 글\r\n둘째 줄\r\n".as_bytes()).format, "글자 파일(UTF-8)");
        let utf8 = run("한글 글\r\n둘째 줄\r\n".as_bytes());
        assert!(utf8.summary.contains(&("줄 끝".into(), "CRLF(Windows)".into())) && utf8.summary.contains(&("줄 수".into(), "2".into())));
        assert_eq!(run(b"\xff\xfeh\0i\0".as_slice()).format, "글자 파일(UTF-16 LE, BOM 있음)");
        let unknown = run([0x13u8, 0x37, 0, 1, 2, 3].as_slice());
        assert_eq!(unknown.format, "알 수 없는 형식");
        assert!(unknown.summary.iter().any(|(k, _)| k == "엔트로피"));
        assert_eq!(run([].as_slice()).format, "빈 파일");
        assert_eq!(size_text(1536), "1,536 바이트 (1.5 KB)");
        assert_eq!(unix_time(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(unix_time(951_782_400), "2000-02-29 00:00:00 UTC");
    }

    #[test]
    fn 아무_바이트를_넣어도_멈추지_않는다() {
        // 앞부분을 알려진 매직으로 시작하게 한 뒤 나머지를 흔든다.
        let magics: [&[u8]; 14] = [
            b"\x89PNG\r\n\x1a\n", b"\xff\xd8\xff", b"GIF89a", b"BM", b"PK\x03\x04", b"\x1f\x8b", b"%PDF-1.7", b"MZ", b"\x7fELF", b"\xca\xfe\xba\xbe",
            b"SQLite format 3\0", b"RIFF", b"\0\0\0\x18ftypmp42", b"fLaC",
        ];
        let mut seed = 0x1234_5678u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        for magic in magics {
            for round in 0..300 {
                let len = (next() % 600) as usize + round % 3;
                let mut file = magic.to_vec();
                file.extend((0..len).map(|_| (next() & 0xff) as u8));
                // 자리·길이 칸에 큰 값이 자주 오게 한다.
                if round % 4 == 0 && file.len() > 70 {
                    file[0x3c..0x40].copy_from_slice(&(next() % 200).to_le_bytes());
                }
                let result = run(file.as_slice());
                fn check(fields: &[Field], depth: u32) {
                    assert!(depth < 8);
                    for field in fields {
                        check(&field.children, depth + 1);
                    }
                }
                check(&result.fields, 0);
                assert!(!result.format.is_empty());
            }
        }
    }

    #[test]
    fn 검토에서_나온_바이트들() {
        // Java 판이 45보다 작으면 Java 클래스가 아니다(뺄셈이 넘치지 않는다).
        for head in [b"\xca\xfe\xba\xbe\x00\x01\x00\x03\x00\x10".as_slice(), b"\xca\xfe\xba\xbe\x00\x00\x00\x28\x00\x10"] {
            assert!(!run(head).format.contains("Java 클래스 파일"), "{}", run(head).format);
        }
        assert!(run(b"\xca\xfe\xba\xbe\x00\x00\x00\x41\x00\x10").format.contains("Java 21"));
        // MP4: 64비트 상자 크기가 u64::MAX여도 넘치지 않고 멈춘다.
        let mut mp4 = b"\0\0\0\x10ftypisom\0\0\0\0".to_vec();
        mp4.extend(1u32.to_be_bytes());
        mp4.extend(b"mdat");
        mp4.extend(u64::MAX.to_be_bytes());
        mp4.extend([0u8; 32]);
        let result = run(&mp4);
        assert_eq!(names(&result.fields), ["상자 ftyp", "상자 mdat"]);
        assert!(result.fields.iter().all(|f| f.offset + f.len <= mp4.len() as u64));
        // ELF64: 구역 머리 자리가 끝 가까이여도 넘치지 않는다.
        let mut elf = vec![0u8; 64];
        elf[..4].copy_from_slice(b"\x7fELF");
        elf[4] = 2;
        elf[5] = 1;
        elf[40..48].copy_from_slice(&(u64::MAX - 10).to_le_bytes());
        elf[58..60].copy_from_slice(&64u16.to_le_bytes());
        elf[60..62].copy_from_slice(&3u16.to_le_bytes());
        elf[62..64].copy_from_slice(&2u16.to_le_bytes());
        assert!(run(&elf).format.starts_with("ELF"));
        // "BM"·"MZ"로 시작하는 글자 파일은 그림·실행 파일이 아니다.
        assert!(!run(b"BMW and Benz are car makers.\r\nThis is plain text.\r\n").format.contains("BMP"));
        assert!(!run(b"MZ is a text file\r\n").format.contains("DOS"));
        assert!(!run("MZ로 시작하는 꽤 긴 글자 파일입니다. 예순네 바이트를 넘게 적어 DOS 머리 크기보다 길게 만듭니다.\r\n".as_bytes()).format.contains("DOS"));
        // OS/2 BMP(머리 12바이트): 가로·세로가 2바이트다.
        let mut core = b"BM".to_vec();
        core.extend(26u32.to_le_bytes());
        core.extend([0u8; 4]);
        core.extend(26u32.to_le_bytes());
        core.extend(12u32.to_le_bytes());
        core.extend(100u16.to_le_bytes());
        core.extend(50u16.to_le_bytes());
        core.extend(1u16.to_le_bytes());
        core.extend(24u16.to_le_bytes());
        let result = run(&core);
        assert!(result.summary.iter().any(|(k, v)| k == "그림 크기" && v == "100 × 50"), "{:?}", result.summary);
        assert!(result.summary.iter().any(|(k, v)| k == "비트/화소" && v == "24"));
        // 잘린 PNG 덩어리는 한 번만 보인다.
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend(100u32.to_be_bytes());
        png.extend(b"IDAT");
        png.extend([1u8; 20]);
        assert_eq!(run(&png).fields.iter().filter(|f| f.offset == 8).count(), 1);
        // gzip: 주석(FCOMMENT)을 건너뛴 자리에서 압축된 자료가 시작한다.
        let mut gz = vec![0x1f, 0x8b, 8, 0x18, 0, 0, 0, 0, 0, 3];
        gz.extend(b"name.txt\0");
        gz.extend(b"a comment\0");
        gz.extend([0u8; 20]);
        let result = run(&gz);
        let head = result.fields.iter().find(|f| f.name == "머리").unwrap();
        assert_eq!(head.len, 29);
        assert!(head.children.iter().any(|f| f.name == "주석" && f.value == "a comment"));
    }
}
