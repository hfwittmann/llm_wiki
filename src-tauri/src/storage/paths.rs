use std::path::{Component, Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum PathError {
    #[error("path is absolute")]
    Absolute,
    #[error("path contains `..` segment")]
    Traversal,
    #[error("path is empty")]
    Empty,
    #[error("path not found")]
    NotFound,
    #[error("canonical path escapes root")]
    Escape,
    #[error("invalid path: {0}")]
    Invalid(String),
}

pub fn resolve_under(root: &Path, requested: &str) -> Result<PathBuf, PathError> {
    let trimmed = requested.trim();
    if trimmed.is_empty() {
        return Err(PathError::Empty);
    }

    let req_path = Path::new(trimmed);

    // Reject pre-canonicalize: absolute paths, drive letters, traversal segments.
    if req_path.is_absolute() {
        return Err(PathError::Absolute);
    }
    for component in req_path.components() {
        match component {
            Component::ParentDir => return Err(PathError::Traversal),
            Component::Prefix(_) | Component::RootDir => return Err(PathError::Absolute),
            Component::CurDir | Component::Normal(_) => {}
        }
    }

    // Canonicalize root once for the prefix comparison. We accept the
    // overhead per-call for v1; later we can cache the canonical root.
    let root_canon = root
        .canonicalize()
        .map_err(|e| PathError::Invalid(format!("root not canonicalizable: {e}")))?;

    let joined = root_canon.join(req_path);
    let canon = joined.canonicalize().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => PathError::NotFound,
        _ => PathError::Invalid(e.to_string()),
    })?;

    if !canon.starts_with(&root_canon) {
        return Err(PathError::Escape);
    }
    Ok(canon)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup() -> TempDir {
        let dir = TempDir::new().expect("create tempdir");
        fs::create_dir_all(dir.path().join("sub/nested")).unwrap();
        fs::write(dir.path().join("sub/nested/file.txt"), b"hi").unwrap();
        dir
    }

    #[test]
    fn resolves_a_valid_relative_path() {
        let dir = setup();
        let resolved = resolve_under(dir.path(), "sub/nested/file.txt").unwrap();
        let expected = dir.path().join("sub/nested/file.txt").canonicalize().unwrap();
        assert_eq!(resolved, expected);
    }

    #[test]
    fn rejects_empty_path() {
        let dir = setup();
        assert!(matches!(resolve_under(dir.path(), ""), Err(PathError::Empty)));
        assert!(matches!(resolve_under(dir.path(), "   "), Err(PathError::Empty)));
    }

    #[test]
    fn rejects_absolute_unix_path() {
        let dir = setup();
        assert!(matches!(resolve_under(dir.path(), "/etc/passwd"), Err(PathError::Absolute)));
    }

    #[test]
    fn rejects_parent_traversal_segment() {
        let dir = setup();
        assert!(matches!(resolve_under(dir.path(), ".."), Err(PathError::Traversal)));
        assert!(matches!(resolve_under(dir.path(), "sub/../.."), Err(PathError::Traversal)));
        assert!(matches!(resolve_under(dir.path(), "sub/../../etc"), Err(PathError::Traversal)));
    }

    #[test]
    fn accepts_curdir_segments() {
        let dir = setup();
        let resolved = resolve_under(dir.path(), "./sub/./nested/file.txt").unwrap();
        let root_canon = dir.path().canonicalize().unwrap();
        assert!(resolved.starts_with(&root_canon));
        assert!(resolved.ends_with("file.txt"));
    }

    #[test]
    fn returns_not_found_for_missing() {
        let dir = setup();
        assert!(matches!(
            resolve_under(dir.path(), "does/not/exist"),
            Err(PathError::NotFound)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_pointing_outside_root() {
        use std::os::unix::fs::symlink;
        let dir = setup();
        let outside = TempDir::new().unwrap();
        fs::write(outside.path().join("secret.txt"), b"hush").unwrap();
        symlink(outside.path().join("secret.txt"), dir.path().join("link_out")).unwrap();
        assert!(matches!(
            resolve_under(dir.path(), "link_out"),
            Err(PathError::Escape)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn accepts_symlink_pointing_inside_root() {
        use std::os::unix::fs::symlink;
        let dir = setup();
        symlink(
            dir.path().join("sub/nested/file.txt"),
            dir.path().join("link_in"),
        )
        .unwrap();
        let resolved = resolve_under(dir.path(), "link_in").unwrap();
        let root_canon = dir.path().canonicalize().unwrap();
        assert!(resolved.starts_with(&root_canon));
        assert!(resolved.ends_with("file.txt"));
    }

    #[test]
    fn idempotent_on_canonical_paths() {
        let dir = setup();
        let first = resolve_under(dir.path(), "sub/nested/file.txt").unwrap();
        let rel = first
            .strip_prefix(dir.path().canonicalize().unwrap())
            .unwrap()
            .to_string_lossy()
            .to_string();
        let second = resolve_under(dir.path(), &rel).unwrap();
        assert_eq!(first, second);
    }

    #[cfg(unix)]
    #[test]
    fn invalid_when_root_does_not_exist() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("does_not_exist");
        // root path is gone before we resolve_under it.
        let result = resolve_under(&root, "anything");
        assert!(matches!(result, Err(PathError::Invalid(_))));
    }

    #[cfg(unix)]
    #[test]
    fn invalid_when_joined_path_is_a_symlink_loop() {
        use std::os::unix::fs::symlink;
        let dir = setup();
        // self-referential symlink: loop -> loop
        symlink(dir.path().join("loop"), dir.path().join("loop")).unwrap();
        let result = resolve_under(dir.path(), "loop");
        // canonicalize on a symlink loop returns ELOOP, which our code maps
        // to PathError::Invalid (not NotFound).
        assert!(
            matches!(result, Err(PathError::Invalid(_))),
            "expected Invalid for symlink loop, got {:?}",
            result
        );
    }
}
