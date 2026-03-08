use std::path::{Path, PathBuf};

pub(crate) fn absolutize_cwd_without_fs_checks(cwd: &str) -> String {
    let path = PathBuf::from(cwd);
    let absolute = if path.is_absolute() {
        path
    } else {
        match std::env::current_dir() {
            Ok(current) => current.join(path),
            Err(err) => {
                tracing::warn!(
                    "failed to resolve current_dir while normalizing cwd, using relative path as-is: {err}"
                );
                path
            }
        }
    };

    path_to_utf8_string(&absolute).unwrap_or_else(|| {
        // Preserve caller-provided value when absolute path is not UTF-8.
        // This avoids silent lossy conversion of path bytes.
        tracing::warn!(
            "normalized cwd is non-utf8; preserving caller-provided cwd string without lossy conversion"
        );
        cwd.to_owned()
    })
}

fn path_to_utf8_string(path: &Path) -> Option<String> {
    path.to_str().map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolutize_keeps_relative_and_absolute_utf8_paths_lossless() {
        let relative = "ergonomic_relative";
        let normalized_relative = absolutize_cwd_without_fs_checks(relative);
        let expected = std::env::current_dir().expect("cwd").join(relative);
        assert_eq!(PathBuf::from(normalized_relative), expected);

        let absolute = std::env::temp_dir().join("ergonomic_absolute");
        let absolute_utf8 = absolute
            .to_str()
            .expect("temp dir path must be utf-8 in this test");
        assert_eq!(
            absolutize_cwd_without_fs_checks(absolute_utf8),
            absolute_utf8
        );
    }

    #[test]
    #[cfg(unix)]
    fn path_to_utf8_string_rejects_non_utf8_paths() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let non_utf8 = PathBuf::from(OsString::from_vec(vec![0x66, 0x6f, 0x80, 0x6f]));
        assert!(path_to_utf8_string(&non_utf8).is_none());
    }
}
