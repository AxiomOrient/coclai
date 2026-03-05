use std::path::PathBuf;

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
    absolute.to_string_lossy().into_owned()
}
