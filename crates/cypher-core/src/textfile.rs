//! 글자 파일 읽기·쓰기: 인코딩과 줄 끝 형식을 알아내고, 저장할 때 그대로 되돌린다.
//!
//! 화면(편집기)에서는 줄 끝을 `\n` 하나로만 다룬다. 파일의 원래 인코딩과 줄 끝 형식은
//! 이름으로 기억해 두었다가 저장할 때 다시 적용한다.

use serde::Serialize;

pub const UTF8: &str = "UTF-8";
pub const UTF8_BOM: &str = "UTF-8 BOM";
pub const UTF16_LE: &str = "UTF-16 LE";
pub const UTF16_BE: &str = "UTF-16 BE";
pub const CP949: &str = "ANSI (CP949)";

pub const CRLF: &str = "CRLF";
pub const LF: &str = "LF";
pub const CR: &str = "CR";

#[derive(Debug, PartialEq, Serialize)]
pub struct Decoded {
    /// 줄 끝을 `\n`으로 통일한 글.
    pub text: String,
    pub encoding: &'static str,
    pub eol: &'static str,
    /// 읽지 못해 `�`로 바뀐 글자가 있는지.
    pub lossy: bool,
}

fn utf16(bytes: &[u8], big_endian: bool) -> (String, bool) {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| if big_endian { u16::from_be_bytes(*pair) } else { u16::from_le_bytes(*pair) })
        .collect();
    let mut lossy = bytes.len() % 2 == 1;
    let text = char::decode_utf16(units)
        .map(|unit| unit.unwrap_or_else(|_| { lossy = true; '\u{fffd}' }))
        .collect();
    (text, lossy)
}

/// 가장 많이 쓰인 줄 끝 형식. 줄 바꿈이 없으면 Windows 기본(CRLF).
fn detect_eol(text: &str) -> &'static str {
    let bytes = text.as_bytes();
    let (mut crlf, mut lf, mut cr) = (0usize, 0usize, 0usize);
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' if bytes.get(i + 1) == Some(&b'\n') => { crlf += 1; i += 1; }
            b'\r' => cr += 1,
            b'\n' => lf += 1,
            _ => {}
        }
        i += 1;
    }
    if lf > crlf && lf >= cr { LF } else if cr > crlf && cr > lf { CR } else { CRLF }
}

/// 파일 내용을 글로 읽는다. BOM → UTF-8 → CP949 순으로 알아본다.
/// 글자 파일로 볼 수 없으면(NUL이 든 이진 파일) 오류를 돌려준다.
pub fn decode(bytes: &[u8]) -> Result<Decoded, String> {
    let (raw, encoding, lossy) = if let Some(rest) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        (String::from_utf8_lossy(rest).into_owned(), UTF8_BOM, std::str::from_utf8(rest).is_err())
    } else if let Some(rest) = bytes.strip_prefix(&[0xff, 0xfe]) {
        let (text, lossy) = utf16(rest, false);
        (text, UTF16_LE, lossy)
    } else if let Some(rest) = bytes.strip_prefix(&[0xfe, 0xff]) {
        let (text, lossy) = utf16(rest, true);
        (text, UTF16_BE, lossy)
    } else if bytes.contains(&0) {
        return Err("글자 파일이 아닙니다(이진 파일로 보입니다).".into());
    } else if let Ok(text) = std::str::from_utf8(bytes) {
        (text.to_string(), UTF8, false)
    } else {
        // UTF-8로는 읽히지 않는다. 몇 군데만 깨진 UTF-8(끝이 잘린 파일, 쓰는 중인 로그 등)인지,
        // 아예 다른 인코딩(CP949)인지 가린다. CP949 글을 UTF-8로 읽으면 대부분이 깨진 자리가 되므로,
        // 제대로 읽힌 여러 바이트 글자가 깨진 자리보다 훨씬 많으면 UTF-8로 본다.
        let as_utf8 = String::from_utf8_lossy(bytes);
        let broken = as_utf8.matches('\u{fffd}').count();
        let multibyte = as_utf8.chars().filter(|c| !c.is_ascii() && *c != '\u{fffd}').count();
        if broken * 4 <= multibyte {
            (as_utf8.into_owned(), UTF8, true)
        } else {
            let (text, _, lossy) = encoding_rs::EUC_KR.decode(bytes);
            (text.into_owned(), CP949, lossy)
        }
    };
    let eol = detect_eol(&raw);
    let text = raw.replace("\r\n", "\n").replace('\r', "\n");
    Ok(Decoded { text, encoding, eol, lossy })
}

/// 글을 그 인코딩과 줄 끝 형식의 파일 내용으로 바꾼다. 모르는 이름은 UTF-8·CRLF로 본다.
/// 그 인코딩으로 쓸 수 없는 글자가 있으면 오류를 돌려준다(파일을 망가뜨리지 않게).
pub fn encode(text: &str, encoding: &str, eol: &str) -> Result<Vec<u8>, String> {
    let unified = text.replace("\r\n", "\n").replace('\r', "\n");
    let text = match eol {
        LF => unified,
        CR => unified.replace('\n', "\r"),
        _ => unified.replace('\n', "\r\n"),
    };
    Ok(match encoding {
        UTF8_BOM => [&[0xef, 0xbb, 0xbf][..], text.as_bytes()].concat(),
        UTF16_LE => [0xff, 0xfe].into_iter().chain(text.encode_utf16().flat_map(u16::to_le_bytes)).collect(),
        UTF16_BE => [0xfe, 0xff].into_iter().chain(text.encode_utf16().flat_map(u16::to_be_bytes)).collect(),
        CP949 => {
            let (bytes, _, unmappable) = encoding_rs::EUC_KR.encode(&text);
            if unmappable {
                return Err("ANSI(CP949)로 쓸 수 없는 글자가 있습니다. 상태 표시줄에서 인코딩을 UTF-8로 바꾼 뒤 저장하세요.".into());
            }
            bytes.into_owned()
        }
        _ => text.into_bytes(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 인코딩과_줄_끝을_알아낸다() {
        let d = decode("가나\r\n다라\r\n".as_bytes()).unwrap();
        assert_eq!((d.text.as_str(), d.encoding, d.eol, d.lossy), ("가나\n다라\n", UTF8, CRLF, false));

        let d = decode(b"\xef\xbb\xbfa\nb\n").unwrap();
        assert_eq!((d.text.as_str(), d.encoding, d.eol), ("a\nb\n", UTF8_BOM, LF));

        // "한글" (CP949)
        let d = decode(b"\xc7\xd1\xb1\xdb\rx").unwrap();
        assert_eq!((d.text.as_str(), d.encoding, d.eol), ("한글\nx", CP949, CR));

        let le: Vec<u8> = [0xff, 0xfe].into_iter().chain("한 줄".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        let d = decode(&le).unwrap();
        assert_eq!((d.text.as_str(), d.encoding, d.eol), ("한 줄", UTF16_LE, CRLF));

        assert!(decode(b"MZ\x00\x01\x02").is_err());
        assert_eq!(decode(b"").unwrap().text, "");
    }

    #[test]
    fn 조금_깨진_utf8은_cp949로_읽지_않는다() {
        // 긴 UTF-8 글의 끝에 잘못된 바이트 하나, 또는 마지막 글자가 잘린 경우.
        let text = "가나다라마바사아자차카타파하 각각각각 한글 문서입니다.\n".repeat(3);
        for tail in [&[0xffu8][..], &"각".as_bytes()[..2]] {
            let d = decode(&[text.as_bytes(), tail].concat()).unwrap();
            assert_eq!((d.encoding, d.lossy), (UTF8, true));
            assert!(d.text.starts_with("가나다라") && d.text.ends_with('\u{fffd}'));
        }
        // 진짜 CP949 글은 그대로 CP949로 읽는다("한글 문서입니다").
        let (cp949, _, _) = encoding_rs::EUC_KR.encode("한글 문서입니다. 가각간갇갈 똠방각하");
        let d = decode(&cp949).unwrap();
        assert_eq!((d.text.as_str(), d.encoding, d.lossy), ("한글 문서입니다. 가각간갇갈 똠방각하", CP949, false));
        // BOM이 있는 파일에 원래 들어 있던 U+FFFD는 "읽지 못한 글자"가 아니다.
        assert!(!decode("\u{feff}a\u{fffd}b".as_bytes()).unwrap().lossy);
    }

    #[test]
    fn 읽은_그대로_다시_쓴다() {
        for original in [
            "가나\r\n다라\r\n".as_bytes().to_vec(),
            b"\xef\xbb\xbfa\nb\n".to_vec(),
            b"\xc7\xd1\xb1\xdb\r\nx".to_vec(),
            [0xfe, 0xff].into_iter().chain("위\n아래".encode_utf16().flat_map(u16::to_be_bytes)).collect(),
        ] {
            let d = decode(&original).unwrap();
            assert_eq!(encode(&d.text, d.encoding, d.eol).unwrap(), original, "{}", d.encoding);
        }
    }

    #[test]
    fn 쓸_수_없는_글자는_오류() {
        assert!(encode("😀", CP949, CRLF).is_err());
        assert_eq!(encode("a\nb", "모르는 이름", "?").unwrap(), b"a\r\nb");
    }
}
