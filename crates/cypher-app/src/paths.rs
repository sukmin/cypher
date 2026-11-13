//! 경로를 글자로 견주는 도구. 디스크를 건드리지 않는다(네트워크 경로여도 멈추지 않는다).
//!
//! 대/소문자와 구분자(`\`·`/`), 끝의 구분자는 가리지 않는다. `..`가 든 경로는 견주는 폴더와 실제 폴더가
//! 달라질 수 있으므로 어떤 폴더와도 같지 않고 어떤 폴더 안에도 있지 않은 것으로 친다.

use std::path::{Component, Path};

/// 경로를 견줄 수 있게 소문자 부분들로 나눈다(`C:\A\b` → ["c:", "a", "b"]). `..`가 들어 있으면 None.
pub fn parts(path: &Path) -> Option<Vec<String>> {
    path.components()
        .filter_map(|part| match part {
            Component::Prefix(prefix) => Some(Some(prefix.as_os_str().to_string_lossy().to_lowercase())),
            Component::Normal(name) => Some(Some(name.to_string_lossy().to_lowercase())),
            Component::ParentDir => Some(None),
            Component::RootDir | Component::CurDir => None,
        })
        .collect()
}

/// `path`가 `root` 안에 있으면 `root` 아래의 부분들을 돌려준다(`root` 자신이면 빈 목록). 아니면 None.
pub fn strip_root(path: &Path, root: &Path) -> Option<Vec<String>> {
    let (mut path, root) = (parts(path)?, parts(root)?);
    (!root.is_empty() && path.len() >= root.len() && path[..root.len()] == root[..]).then(|| path.split_off(root.len()))
}

/// `path`가 `root` 안에 있는지(`root` 자신 포함).
pub fn is_within(path: &Path, root: &Path) -> bool {
    strip_root(path, root).is_some()
}

/// 두 경로가 같은 곳인지.
pub fn same_place(a: &Path, b: &Path) -> bool {
    strip_root(a, b).is_some_and(|rest| rest.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 폴더_안인지와_같은_곳인지를_글자로_견준다() {
        assert!(is_within(Path::new(r"C:\Work\Proj\a\b.md"), Path::new(r"c:\work\proj")));
        assert!(is_within(Path::new(r"C:\work\proj"), Path::new(r"C:\work\proj\")));
        assert!(!is_within(Path::new(r"C:\work\project2\a.md"), Path::new(r"C:\work\proj")));
        assert!(!is_within(Path::new(r"D:\work\proj\a.md"), Path::new(r"C:\work\proj")));
        assert!(!is_within(Path::new(r"C:\work"), Path::new(r"C:\work\proj")));
        assert!(!is_within(Path::new(r"C:\work\a.md"), Path::new("")));
        assert!(same_place(Path::new(r"C:\Work\Proj\"), Path::new(r"c:\work\proj")));
        assert!(same_place(Path::new("D:/api/requests/"), Path::new(r"d:\API\requests")));
        assert!(same_place(Path::new(r"\\NAS\team\api"), Path::new(r"\\nas\team\api")));
        assert!(!same_place(Path::new(r"C:\work\proj\a"), Path::new(r"c:\work\proj")));
        assert!(!same_place(Path::new(""), Path::new("")));
        assert_eq!(strip_root(Path::new(r"C:\ws\Proj\docs\a.md"), Path::new(r"c:\ws\proj")), Some(vec!["docs".to_string(), "a.md".to_string()]));
    }

    #[test]
    fn 돌아_나가는_경로는_어디에도_속하지_않는다() {
        assert_eq!(parts(Path::new(r"C:\ws\..\a")), None);
        assert!(!is_within(Path::new(r"C:\ws\proj\..\..\Users\me\a.md"), Path::new(r"C:\ws\proj")));
        assert!(!is_within(Path::new(r"C:\ws\proj\a.md"), Path::new(r"C:\ws\proj\..\proj")));
        assert!(!same_place(Path::new(r"C:\ws\proj\..\proj"), Path::new(r"C:\ws\proj")));
    }
}
