//! Reading and writing the editors' JSONC `settings.json` files.
//!
//! Shared by the editor modules that keep sandbox hosts in an editor's user settings. Edits go
//! through `jsonc-parser`'s CST, so comments and formatting stay as the user wrote them.

use jsonc_parser::ParseOptions;
use jsonc_parser::cst::CstObject;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// Environment variable that overrides the editors' config root, so the `vm-tests` can keep the
/// daemon they run away from the developer's own editor settings on every platform.
const CONFIG_ROOT_ENV: &str = "ANVIL_EDITOR_CONFIG_ROOT";

/// Parse options that accept what the editors accept in `settings.json`: JSON with comments and
/// trailing commas. Anything else is left unchanged rather than rewritten.
pub(crate) const PARSE_OPTIONS: ParseOptions = ParseOptions {
    allow_comments: true,
    allow_trailing_commas: true,
    allow_loose_object_property_names: false,
    allow_missing_commas: false,
    allow_single_quoted_strings: false,
    allow_hexadecimal_numbers: false,
    allow_unary_plus_numbers: false,
    allow_bare_decimal_point_numbers: false,
    allow_non_finite_numbers: false,
    allow_extended_string_escapes: false,
};

/// Serializes syncs, so concurrent requests can't interleave their reads and writes of a file.
static SYNC_LOCK: Mutex<()> = Mutex::new(());

/// Takes the lock that serializes the syncs of all editor settings.
pub(crate) fn lock() -> MutexGuard<'static, ()> {
    // A sync that panicked can't leave a half-written file behind, so a poisoned lock is fine.
    SYNC_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Returns `$ANVIL_EDITOR_CONFIG_ROOT` when it's set, or `platform_root` otherwise.
pub(crate) fn config_root_or(platform_root: impl FnOnce() -> Option<PathBuf>) -> Option<PathBuf> {
    std::env::var_os(CONFIG_ROOT_ENV)
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(platform_root)
}

/// Returns `$XDG_CONFIG_HOME`, or `~/.config` when it isn't set.
#[cfg(not(target_os = "macos"))]
pub(crate) fn xdg_config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(home_config)
}

/// Returns `~/.config`.
pub(crate) fn home_config() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"))
}

/// Reads a settings file, treating a missing file as empty. A symlink to a missing file is an
/// error, so the sync doesn't replace the link with a regular file.
pub(crate) fn read(path: &Path) -> io::Result<String> {
    match fs::read_to_string(path) {
        Err(err) if err.kind() == ErrorKind::NotFound && !path.is_symlink() => Ok(String::new()),
        result => result,
    }
}

/// Returns how often `key` appears in the object.
pub(crate) fn key_count(object: &CstObject, key: &str) -> usize {
    object
        .properties()
        .iter()
        .filter(|prop| prop.decoded_name().as_deref() == Some(key))
        .count()
}

/// Writes a temporary file next to the settings file and renames it, so the editor never reads
/// a half-written file. A symlinked settings file (dotfiles) stays a symlink, because the
/// rename replaces the file it points to.
pub(crate) fn write(path: &Path, content: &str) -> io::Result<()> {
    let target = match fs::canonicalize(path) {
        Ok(target) => target,
        Err(err) if err.kind() == ErrorKind::NotFound => path.to_path_buf(),
        Err(err) => return Err(err),
    };
    let file_name = target.file_name().unwrap_or_default().to_string_lossy();
    let temp = target.with_file_name(format!(".{file_name}.anvil-tmp"));

    let result = fs::write(&temp, content)
        .and_then(|()| keep_permissions(&target, &temp))
        .and_then(|()| fs::rename(&temp, &target));

    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }

    result
}

/// Gives `temp` the permissions of `target`, when `target` exists.
fn keep_permissions(target: &Path, temp: &Path) -> io::Result<()> {
    match fs::metadata(target) {
        Ok(metadata) => fs::set_permissions(temp, metadata.permissions()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonc_parser::cst::CstRootNode;
    use tempfile::TempDir;

    #[test]
    fn reads_missing_file_as_empty() {
        let dir = TempDir::new().unwrap();

        assert_eq!(read(&dir.path().join("settings.json")).unwrap(), "");
    }

    #[test]
    fn writes_file_without_leaving_temp_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json");

        write(&path, "{}\n").unwrap();

        assert_eq!(read(&path).unwrap(), "{}\n");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn counts_duplicate_keys() {
        let root = CstRootNode::parse("{\"a\": 1, \"b\": 2, \"a\": 3}", &PARSE_OPTIONS).unwrap();
        let object = root.object_value().unwrap();

        assert_eq!(key_count(&object, "a"), 2);
        assert_eq!(key_count(&object, "c"), 0);
    }
}
