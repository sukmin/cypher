//! 헥사 에디터 항목: 파일의 바이트를 조각씩 읽어 보이고, 고친 바이트를 저장한다.
//!
//! 파일 전체를 메모리에 올리지 않는다. 화면은 보이는 줄의 바이트만 읽고(`hex_read`), 고친 바이트는 화면이 들고 있다가
//! 찾기·해석·저장할 때 함께 넘긴다(`patches`: 파일 위에 덮어 보는 조각들). 오른쪽 절반의 해석은 `cypher_core::binfmt`가 한다.
//! 저장은 덮어쓰기만 한다(파일 크기는 바뀌지 않는다). 작은 파일은 새 파일에 다 쓴 뒤 `ReplaceFileW`로 바꿔치기하고
//! (속성·권한·대체 스트림·만든 시각은 원래 파일의 것을 지킨다), 큰 파일과 링크(하드 링크가 여럿, 심볼릭 링크)는 고친 자리만 쓴다.

use cypher_core::binfmt::{self, Analysis, Source};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// 한 번에 읽는 바이트 수의 한도.
const MAX_READ: usize = 4 << 20;
/// 찾을 글의 길이 한도.
const MAX_PATTERN: usize = 4096;
/// 이보다 작은 파일은 새 파일에 다 쓴 뒤 바꿔치기한다(쓰다가 끊겨도 원래 파일이 남는다). 크면 고친 자리만 쓴다.
const REWRITE_LIMIT: u64 = 256 << 20;

/// 파일 위에 덮어 보는 조각(화면에서 고친 바이트).
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Patch {
    pub offset: u64,
    pub bytes: Vec<u8>,
}

#[derive(Serialize, Debug)]
pub struct HexFile {
    path: String,
    size: u64,
    /// 고친 시각(1970년부터의 밀리초). 밖에서 바뀌었는지 견준다.
    modified: Option<u64>,
    readonly: bool,
}

fn stat(path: &Path) -> Result<HexFile, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("파일을 열지 못했습니다({}): {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("파일이 아닙니다: {}", path.display()));
    }
    Ok(HexFile {
        path: path.to_string_lossy().into_owned(),
        size: meta.len(),
        modified: meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64),
        readonly: meta.permissions().readonly(),
    })
}

/// 조각들을 버퍼(파일의 `at`부터) 위에 덮는다.
fn overlay(buf: &mut [u8], at: u64, patches: &[Patch]) {
    let end = at + buf.len() as u64;
    for patch in patches {
        let (from, to) = (patch.offset, patch.offset + patch.bytes.len() as u64);
        if to <= at || from >= end {
            continue;
        }
        let start = from.max(at);
        let stop = to.min(end);
        let src = &patch.bytes[(start - from) as usize..(stop - from) as usize];
        buf[(start - at) as usize..(stop - at) as usize].copy_from_slice(src);
    }
}

/// 파일(과 그 위에 덮은 조각들)을 해석기·찾기에 주는 원천.
struct FileSource<'a> {
    file: RefCell<File>,
    size: u64,
    patches: &'a [Patch],
}

impl<'a> FileSource<'a> {
    fn open(path: &Path, patches: &'a [Patch]) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("파일을 열지 못했습니다({}): {e}", path.display()))?;
        let size = file.metadata().map_err(|e| e.to_string())?.len();
        Ok(Self { file: RefCell::new(file), size, patches })
    }
}

impl Source for FileSource<'_> {
    fn size(&self) -> u64 {
        self.size
    }
    fn read(&self, offset: u64, len: usize) -> Vec<u8> {
        if offset >= self.size {
            return Vec::new();
        }
        let len = len.min((self.size - offset) as usize);
        let mut buf = vec![0u8; len];
        let mut file = self.file.borrow_mut();
        let mut got = 0;
        if file.seek(SeekFrom::Start(offset)).is_ok() {
            while got < len {
                match file.read(&mut buf[got..]) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => got += n,
                }
            }
        }
        buf.truncate(got);
        overlay(&mut buf, offset, self.patches);
        buf
    }
}

/// 헥사 에디터로 열 파일을 고른다.
#[tauri::command]
pub async fn hex_pick(start: Option<String>) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = rfd::FileDialog::new().set_title("헥사 에디터로 열 파일").add_filter("모든 파일", &["*"]);
        if let Some(dir) = start.filter(|d| Path::new(d).is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        dialog.pick_file().map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .ok()
    .flatten()
}

/// 다른 이름으로 저장할 곳을 고른다.
#[tauri::command]
pub async fn hex_save_pick(name: String, start: Option<String>) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = rfd::FileDialog::new().set_title("다른 이름으로 저장").set_file_name(name).add_filter("모든 파일", &["*"]);
        if let Some(dir) = start.filter(|d| Path::new(d).is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        dialog.save_file().map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .ok()
    .flatten()
}

/// 파일의 크기·고친 시각.
#[tauri::command]
pub async fn hex_stat(path: String) -> Result<HexFile, String> {
    tauri::async_runtime::spawn_blocking(move || stat(Path::new(&path))).await.map_err(|e| e.to_string())?
}

/// `offset`부터 최대 `len` 바이트를 읽는다(파일 끝이면 짧다). 화면에는 바이트 그대로(ArrayBuffer) 간다.
#[tauri::command]
pub async fn hex_read(path: String, offset: u64, len: usize) -> Result<tauri::ipc::Response, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let source = FileSource::open(Path::new(&path), &[])?;
        Ok(tauri::ipc::Response::new(source.read(offset, len.min(MAX_READ))))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 파일의 형식을 알아보고 구조를 읽는다(고친 바이트를 덮어 본 내용으로).
#[tauri::command]
pub async fn hex_analyze(path: String, patches: Vec<Patch>) -> Result<Analysis, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let source = FileSource::open(Path::new(&path), &patches)?;
        Ok(binfmt::analyze(&source))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 바이트 열을 찾는다. `from`부터 앞으로(뒤로 찾으면 `from` 앞에서 뒤로) 처음 나온 자리. 없으면 None.
/// `ignore_case`면 영문 대/소문자를 가리지 않는다(글자로 찾을 때).
fn find(source: &dyn Source, pattern: &[u8], from: u64, backward: bool, ignore_case: bool) -> Option<u64> {
    if pattern.is_empty() || pattern.len() as u64 > source.size() {
        return None;
    }
    const BLOCK: u64 = 1 << 20;
    let n = pattern.len() as u64;
    let same = |a: &[u8]| if ignore_case { a.eq_ignore_ascii_case(pattern) } else { a == pattern };
    if !backward {
        let mut at = from;
        while at + n <= source.size() {
            let chunk = source.read(at, (BLOCK + n - 1) as usize);
            if (chunk.len() as u64) < n {
                break;
            }
            if let Some(i) = chunk.windows(pattern.len()).position(same) {
                return Some(at + i as u64);
            }
            at += BLOCK;
        }
        None
    } else {
        // `from` 앞(그 자리는 빼고)에서 시작하는 것 가운데 가장 가까운 것
        let mut end = from.min(source.size() - n + 1);
        while end > 0 {
            let start = end.saturating_sub(BLOCK);
            let chunk = source.read(start, (end - start + n - 1) as usize);
            if let Some(i) = chunk.windows(pattern.len()).rposition(same) {
                if start + (i as u64) < end {
                    return Some(start + i as u64);
                }
            }
            end = start;
        }
        None
    }
}

#[tauri::command]
pub async fn hex_find(path: String, patches: Vec<Patch>, pattern: Vec<u8>, from: u64, backward: bool, ignore_case: bool) -> Result<Option<u64>, String> {
    if pattern.len() > MAX_PATTERN {
        return Err(format!("찾을 것이 너무 깁니다({MAX_PATTERN}바이트까지)."));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let source = FileSource::open(Path::new(&path), &patches)?;
        Ok(find(&source, &pattern, from, backward, ignore_case))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 바꿔치기해도 되는 파일인지. 심볼릭 링크나 하드 링크가 여럿인 파일을 바꿔치기하면 링크가 끊겨(다른 이름 쪽에는 옛 내용이 남는다)
/// 제자리에 써야 한다.
fn replaceable(path: &Path) -> bool {
    if std::fs::symlink_metadata(path).map(|m| m.file_type().is_symlink()).unwrap_or(true) {
        return false;
    }
    File::open(path).ok().and_then(|file| links(&file)).is_some_and(|n| n <= 1)
}

/// 파일의 하드 링크 수.
fn links(file: &File) -> Option<u32> {
    use std::os::windows::io::AsRawHandle;
    #[repr(C)]
    #[derive(Default)]
    struct Info {
        attributes: u32,
        created: [u32; 2],
        accessed: [u32; 2],
        written: [u32; 2],
        volume: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileInformationByHandle(file: *mut std::ffi::c_void, info: *mut Info) -> i32;
    }
    let mut info = Info::default();
    // SAFETY: 열린 파일의 핸들과 유효한 구조체 하나를 넘긴다.
    (unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } != 0).then_some(info.links)
}

/// `dest`를 `temp`의 내용으로 바꾼다. 원래 파일의 속성(숨김 등)·권한·대체 스트림(인터넷에서 받은 파일 표시 등)·만든 시각은 지킨다.
fn replace_file(dest: &Path, temp: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    const REPLACEFILE_IGNORE_MERGE_ERRORS: u32 = 0x2;
    #[link(name = "kernel32")]
    extern "system" {
        fn ReplaceFileW(replaced: *const u16, replacement: *const u16, backup: *const u16, flags: u32, exclude: *mut std::ffi::c_void, reserved: *mut std::ffi::c_void) -> i32;
    }
    let wide = |p: &Path| p.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let (replaced, replacement) = (wide(dest), wide(temp));
    // SAFETY: 0으로 끝나는 UTF-16 경로 둘을 넘기고, 나머지는 쓰지 않는다(null).
    let ok = unsafe { ReplaceFileW(replaced.as_ptr(), replacement.as_ptr(), std::ptr::null(), REPLACEFILE_IGNORE_MERGE_ERRORS, std::ptr::null_mut(), std::ptr::null_mut()) };
    if ok != 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

/// 고친 바이트를 파일에 쓴다. `target`이 있으면 원래 파일은 두고 그 이름으로 저장한다(다른 이름으로 저장).
/// 열었을 때와 크기·고친 시각이 다르면(밖에서 바뀌었으면) `force`가 아닐 때 "changed-on-disk"로 거절한다.
fn save(path: &Path, patches: &[Patch], expected: (u64, Option<u64>), force: bool, target: Option<&Path>) -> Result<HexFile, String> {
    let now = stat(path)?;
    if !force && (now.size != expected.0 || (expected.1.is_some() && now.modified != expected.1)) {
        return Err("changed-on-disk".into());
    }
    if let Some(bad) = patches.iter().find(|p| p.offset + p.bytes.len() as u64 > now.size) {
        return Err(format!("파일 끝을 넘는 자리({:#x})를 고쳤습니다. 파일이 줄었으면 다시 여세요.", bad.offset));
    }
    match target {
        Some(target) if target != path => {
            let existed = target.exists();
            std::fs::copy(path, target).map_err(|e| format!("{}에 복사하지 못했습니다: {e}", target.display()))?;
            // 복사는 읽기 전용 속성까지 옮긴다: 새 파일은 고칠 수 있게 푼다.
            let result = std::fs::metadata(target)
                .and_then(|meta| {
                    let mut permissions = meta.permissions();
                    #[allow(clippy::permissions_set_readonly_false)]
                    permissions.set_readonly(false);
                    std::fs::set_permissions(target, permissions)
                })
                .map_err(|e| format!("{}의 읽기 전용을 풀지 못했습니다: {e}", target.display()))
                .and_then(|_| write(target, now.size, patches));
            if result.is_err() && !existed {
                let _ = std::fs::remove_file(target);
            }
            result
        }
        _ => write(path, now.size, patches),
    }
}

/// 고친 조각을 `dest`에 쓴다.
fn write(dest: &Path, size: u64, patches: &[Patch]) -> Result<HexFile, String> {
    if std::fs::metadata(dest).map(|m| m.permissions().readonly()).unwrap_or(false) {
        return Err(format!("읽기 전용 파일이라 쓸 수 없습니다: {}", dest.display()));
    }
    if size <= REWRITE_LIMIT && replaceable(dest) {
        // 다 읽어 고친 뒤 옆에 새 파일로 쓰고 바꿔치기한다(쓰다가 끊겨도 원래 파일이 남는다).
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let mut bytes = std::fs::read(dest).map_err(|e| format!("파일을 읽지 못했습니다: {e}"))?;
        overlay(&mut bytes, 0, patches);
        let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let temp = dest.with_file_name(format!(".{name}.{}-{}.cypher-saving", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let written = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp).and_then(|mut file| {
            file.write_all(&bytes)?;
            file.sync_all()
        });
        if let Err(err) = written {
            let _ = std::fs::remove_file(&temp);
            return Err(format!("저장하지 못했습니다(원래 파일은 그대로입니다): {err}"));
        }
        if let Err(err) = replace_file(dest, &temp) {
            // 백업 이름 없이 부른 ReplaceFileW의 실패는 어디서 멈췄는지에 따라 남은 것이 다르다(MS 문서).
            const UNABLE_TO_MOVE_REPLACEMENT: i32 = 1176; // 원래 파일은 지워졌고, 새 내용은 임시 파일에만 있다
            const UNABLE_TO_MOVE_REPLACEMENT_2: i32 = 1177; // 원래 파일은 다른 이름으로 남았고, 새 내용은 임시 파일에만 있다
            return match err.raw_os_error() {
                Some(UNABLE_TO_MOVE_REPLACEMENT) => match std::fs::rename(&temp, dest) {
                    // 임시 파일을 원래 이름으로 옮긴다(이때는 원래 파일의 속성을 지키지 못한다).
                    Ok(()) => stat(dest),
                    Err(again) => Err(format!("저장하다 멈췄습니다. 새 내용은 {}에 남아 있습니다. 그 파일의 이름을 {}(으)로 바꾸세요: {err} / {again}", temp.display(), dest.display())),
                },
                Some(UNABLE_TO_MOVE_REPLACEMENT_2) => {
                    Err(format!("저장하다 멈췄습니다. 새 내용은 {}에 남아 있고, 원래 파일은 같은 폴더에 다른 이름으로 남았을 수 있습니다: {err}", temp.display()))
                }
                _ => {
                    let _ = std::fs::remove_file(&temp);
                    Err(format!("저장하지 못했습니다(다른 프로그램이 파일을 쓰고 있을 수 있습니다. 원래 파일은 그대로입니다): {err}"))
                }
            };
        }
    } else {
        let mut file = std::fs::OpenOptions::new().write(true).open(dest).map_err(|e| format!("파일을 쓰려고 열지 못했습니다: {e}"))?;
        for patch in patches {
            file.seek(SeekFrom::Start(patch.offset)).and_then(|_| file.write_all(&patch.bytes)).map_err(|e| format!("{:#x}에 쓰지 못했습니다: {e}", patch.offset))?;
        }
        file.sync_all().map_err(|e| e.to_string())?;
    }
    stat(dest)
}

#[tauri::command]
pub async fn hex_save(
    path: String,
    patches: Vec<Patch>,
    expected_size: u64,
    expected_modified: Option<u64>,
    force: bool,
    target: Option<String>,
) -> Result<HexFile, String> {
    tauri::async_runtime::spawn_blocking(move || save(Path::new(&path), &patches, (expected_size, expected_modified), force, target.as_deref().map(Path::new)))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::current_exe().unwrap().parent().unwrap().join("test-files").join(format!("hex-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn 고친_바이트를_덮어_읽고_찾는다() {
        let dir = dir("find");
        let path = dir.join("자료.bin.txt");
        let mut data: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        data[2_500_000..2_500_005].copy_from_slice(b"Hello");
        std::fs::write(&path, &data).unwrap();
        let patches = vec![Patch { offset: 10, bytes: vec![0xaa, 0xbb] }, Patch { offset: 1_048_575, bytes: b"HELLO".to_vec() }];
        let source = FileSource::open(&path, &patches).unwrap();
        assert_eq!(source.read(9, 4), [9, 0xaa, 0xbb, 12]);
        assert_eq!(source.read(2_999_998, 10).len(), 2);
        // 블록 경계에 걸친 것, 대/소문자, 뒤로 찾기
        assert_eq!(find(&source, b"HELLO", 0, false, false), Some(1_048_575));
        assert_eq!(find(&source, b"hello", 1_048_576, false, true), Some(2_500_000));
        assert_eq!(find(&source, b"Hello", 2_500_001, false, false), None);
        assert_eq!(find(&source, b"hello", 3_000_000, true, true), Some(2_500_000));
        assert_eq!(find(&source, b"hello", 2_500_000, true, true), Some(1_048_575));
        assert_eq!(find(&source, b"HELLO", 1_048_575, true, false), None);
        assert_eq!(find(&source, &[0xaa, 0xbb], 0, false, false), Some(10));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 고친_바이트를_저장한다() {
        let dir = dir("save");
        let path = dir.join("그림.dat.txt");
        std::fs::write(&path, b"0123456789").unwrap();
        let opened = stat(&path).unwrap();
        let patches = vec![Patch { offset: 2, bytes: b"AB".to_vec() }, Patch { offset: 9, bytes: b"Z".to_vec() }];
        // 다른 이름으로: 원래 파일은 그대로
        let copy = dir.join("사본.txt");
        save(&path, &patches, (opened.size, opened.modified), false, Some(&copy)).unwrap();
        assert_eq!(std::fs::read(&copy).unwrap(), b"01AB45678Z");
        assert_eq!(std::fs::read(&path).unwrap(), b"0123456789");
        // 제자리에
        let saved = save(&path, &patches, (opened.size, opened.modified), false, None).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"01AB45678Z");
        let leftovers = || std::fs::read_dir(&dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".cypher-saving")).count();
        assert_eq!(leftovers(), 0);
        // 밖에서 바뀌었으면 묻는다(힘으로 쓰면 쓴다). 파일 끝을 넘는 조각은 받지 않는다.
        std::fs::write(&path, b"changed!!!!").unwrap();
        assert_eq!(save(&path, &patches, (saved.size, saved.modified), false, None).unwrap_err(), "changed-on-disk");
        save(&path, &patches, (0, None), true, None).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"chABged!!Z!");
        assert!(save(&path, &[Patch { offset: 10, bytes: b"xy".to_vec() }], (0, None), true, None).unwrap_err().contains("끝을 넘는"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 저장해도_파일의_다른_정보를_지킨다() {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        const HIDDEN: u32 = 0x2;
        let dir = dir("keep");
        let patch = [Patch { offset: 0, bytes: b"X".to_vec() }];
        // 숨김 속성과 대체 스트림(인터넷에서 받은 파일 표시 같은 것)이 남는다.
        let path = dir.join("숨김.dat.txt");
        std::fs::OpenOptions::new().write(true).create(true).attributes(HIDDEN).open(&path).unwrap().write_all(b"abc").unwrap();
        std::fs::write(dir.join("숨김.dat.txt:Zone.Identifier"), b"[ZoneTransfer]\r\nZoneId=3\r\n").unwrap();
        save(&path, &patch, (0, None), true, None).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"Xbc");
        assert_ne!(std::fs::metadata(&path).unwrap().file_attributes() & HIDDEN, 0);
        assert_eq!(std::fs::read(dir.join("숨김.dat.txt:Zone.Identifier")).unwrap(), b"[ZoneTransfer]\r\nZoneId=3\r\n");
        // 하드 링크는 끊지 않는다(다른 이름 쪽에도 고친 내용이 보인다).
        let one = dir.join("하나.dat.txt");
        std::fs::write(&one, b"abc").unwrap();
        let other = dir.join("둘.dat.txt");
        std::fs::hard_link(&one, &other).unwrap();
        save(&one, &patch, (0, None), true, None).unwrap();
        assert_eq!(std::fs::read(&other).unwrap(), b"Xbc");
        // 읽기 전용 파일도 다른 이름으로는 저장되고, 새 파일은 고칠 수 있다.
        let locked = dir.join("잠김.dat.txt");
        std::fs::write(&locked, b"abc").unwrap();
        let mut permissions = std::fs::metadata(&locked).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&locked, permissions).unwrap();
        assert!(save(&locked, &patch, (0, None), true, None).unwrap_err().contains("읽기 전용"));
        let copy = dir.join("잠김-사본.dat.txt");
        save(&locked, &patch, (0, None), true, Some(&copy)).unwrap();
        assert_eq!(std::fs::read(&copy).unwrap(), b"Xbc");
        assert!(!std::fs::metadata(&copy).unwrap().permissions().readonly());
        let mut permissions = std::fs::metadata(&locked).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(&locked, permissions).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
