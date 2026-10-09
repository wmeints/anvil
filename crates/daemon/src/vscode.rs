//! VS Code Remote-SSH settings for sandboxes.
//!
//! Remote-SSH asks for the platform of every host it connects to, unless the user settings map
//! the host in `remote.SSH.remotePlatform`. The daemon keeps one `"linux"` entry per sandbox
//! host name in the user settings of every installed VS Code-family editor, and leaves the rest
//! of the file, comments and formatting included, as the user wrote it.

use jsonc_parser::ParseOptions;
use jsonc_parser::cst::{CstObject, CstObjectProp, CstRootNode};
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use thiserror::Error;

/// Errors of syncing one editor's settings file. The other editors are still synced.
#[derive(Error, Debug)]
pub enum SettingsError {
    /// The settings file couldn't be read, or is a symlink to a file that doesn't exist.
    #[error("failed to read {}", path.display())]
    Read {
        /// The settings file.
        path: PathBuf,
        /// The I/O error.
        source: io::Error,
    },
    /// The settings file isn't valid JSONC, or its root isn't an object.
    #[error("failed to parse {}: {message}", path.display())]
    Parse {
        /// The settings file.
        path: PathBuf,
        /// What's wrong with the file.
        message: String,
    },
    /// `remote.SSH.remotePlatform` exists, but isn't an object.
    #[error("{PLATFORM_KEY} in {} isn't an object", path.display())]
    PlatformNotAnObject {
        /// The settings file.
        path: PathBuf,
    },
    /// `remote.SSH.remotePlatform` appears more than once. VS Code reads the last one, so editing
    /// any of them could leave the one it reads unchanged.
    #[error("{PLATFORM_KEY} appears more than once in {}", path.display())]
    DuplicatePlatformKey {
        /// The settings file.
        path: PathBuf,
    },
    /// The settings file couldn't be written.
    #[error("failed to write {}", path.display())]
    Write {
        /// The settings file.
        path: PathBuf,
        /// The I/O error.
        source: io::Error,
    },
}

const PLATFORM_KEY: &str = "remote.SSH.remotePlatform";
const PLATFORM: &str = "linux";
const HOST_SUFFIX: &str = ".anvil";
const SETTINGS_FILE: &str = "settings.json";

/// Configuration directories of the supported editors, relative to the config root.
const EDITORS: [&str; 4] = ["Code", "Code - Insiders", "Cursor", "VSCodium"];

/// Environment variable that overrides the editors' config root, so the `vm-tests` can keep the
/// daemon they run away from the developer's own editor settings on every platform.
const CONFIG_ROOT_ENV: &str = "ANVIL_EDITOR_CONFIG_ROOT";

/// Parse options that accept what VS Code accepts in `settings.json`: JSON with comments and
/// trailing commas. Anything else is left unchanged rather than rewritten.
const PARSE_OPTIONS: ParseOptions = ParseOptions {
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

/// Returns the directory the editors keep their configuration in: `$ANVIL_EDITOR_CONFIG_ROOT`
/// when it's set, or the platform's config directory otherwise.
pub fn config_root() -> Option<PathBuf> {
    std::env::var_os(CONFIG_ROOT_ENV)
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(platform_config_root)
}

/// Returns `$XDG_CONFIG_HOME`, or `~/.config` when it isn't set.
#[cfg(not(target_os = "macos"))]
fn platform_config_root() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
}

/// Returns `~/Library/Application Support`.
#[cfg(target_os = "macos")]
fn platform_config_root() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
}

/// Maps every host name to `"linux"` in the user settings of each editor installed under
/// `config_root`, and removes the stale `*.anvil` hosts. Returns the errors of the editors
/// whose settings couldn't be synced; their files are left unchanged.
pub fn sync_settings(config_root: &Path, hostnames: &[String]) -> Vec<SettingsError> {
    // A sync that panicked can't leave a half-written file behind, so a poisoned lock is fine.
    let _guard = SYNC_LOCK.lock().unwrap_or_else(PoisonError::into_inner);

    EDITORS
        .iter()
        .map(|editor| config_root.join(editor).join("User"))
        .filter(|user_dir| user_dir.is_dir())
        .filter_map(|user_dir| sync_file(&user_dir.join(SETTINGS_FILE), hostnames).err())
        .collect()
}

/// Syncs the host names into one settings file, writing it only when it changes.
fn sync_file(path: &Path, hostnames: &[String]) -> Result<(), SettingsError> {
    let text = read_settings(path)?;

    match update_platforms(path, &text, hostnames)? {
        Some(updated) => write_settings(path, &updated),
        None => Ok(()),
    }
}

/// Reads a settings file, treating a missing file as empty. A symlink to a missing file is an
/// error, so the sync doesn't replace the link with a regular file.
fn read_settings(path: &Path) -> Result<String, SettingsError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(err) if err.kind() == ErrorKind::NotFound && !path.is_symlink() => Ok(String::new()),
        Err(source) => Err(SettingsError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Returns the settings text with the host names mapped to `"linux"` and the stale anvil hosts
/// removed, or `None` when nothing changes.
fn update_platforms(
    path: &Path,
    text: &str,
    hostnames: &[String],
) -> Result<Option<String>, SettingsError> {
    let root = CstRootNode::parse(text, &PARSE_OPTIONS).map_err(|err| SettingsError::Parse {
        path: path.to_path_buf(),
        message: err.to_string(),
    })?;

    let Some(platforms) = platforms_object(&root, path, hostnames)? else {
        return Ok(None);
    };

    remove_stale_hosts(&platforms, hostnames);
    add_hosts(&platforms, hostnames);

    let updated = root.to_string();
    Ok((updated != text).then_some(updated))
}

/// Returns the `remote.SSH.remotePlatform` object, creating it when there are hosts to add, or
/// `None` when it doesn't exist and there's nothing to add.
fn platforms_object(
    root: &CstRootNode,
    path: &Path,
    hostnames: &[String],
) -> Result<Option<CstObject>, SettingsError> {
    let settings = match root.value() {
        None if hostnames.is_empty() => return Ok(None),
        None => root.object_value_or_set(),
        Some(value) => value.as_object().ok_or_else(|| SettingsError::Parse {
            path: path.to_path_buf(),
            message: "the settings aren't an object".to_string(),
        })?,
    };

    if platform_key_count(&settings) > 1 {
        return Err(SettingsError::DuplicatePlatformKey {
            path: path.to_path_buf(),
        });
    }

    match settings.get(PLATFORM_KEY) {
        None if hostnames.is_empty() => Ok(None),
        None => Ok(Some(settings.object_value_or_set(PLATFORM_KEY))),
        Some(prop) => {
            prop.object_value()
                .map(Some)
                .ok_or_else(|| SettingsError::PlatformNotAnObject {
                    path: path.to_path_buf(),
                })
        }
    }
}

/// Returns how often `remote.SSH.remotePlatform` appears in the settings.
fn platform_key_count(settings: &CstObject) -> usize {
    settings
        .properties()
        .iter()
        .filter(|prop| prop.decoded_name().as_deref() == Some(PLATFORM_KEY))
        .count()
}

/// Removes the anvil hosts that aren't in `hostnames`, keeping every other host.
fn remove_stale_hosts(platforms: &CstObject, hostnames: &[String]) {
    for prop in platforms.properties() {
        if prop
            .decoded_name()
            .is_some_and(|name| name.ends_with(HOST_SUFFIX) && !hostnames.contains(&name))
        {
            prop.remove();
        }
    }
}

/// Maps every host name to `"linux"`, appending the missing ones in sorted order.
fn add_hosts(platforms: &CstObject, hostnames: &[String]) {
    let mut hostnames: Vec<&String> = hostnames.iter().collect();
    hostnames.sort();

    for hostname in hostnames {
        match platforms.get(hostname) {
            Some(prop) if maps_to_linux(&prop) => {}
            Some(prop) => prop.set_value(PLATFORM.into()),
            None => {
                platforms.append(hostname, PLATFORM.into());
            }
        }
    }
}

/// Returns whether the property's value is the string `"linux"`.
fn maps_to_linux(prop: &CstObjectProp) -> bool {
    prop.value()
        .and_then(|value| value.as_string_lit())
        .and_then(|value| value.decoded_value().ok())
        .is_some_and(|value| value == PLATFORM)
}

/// Writes a temporary file next to the settings file and renames it, so the editor never reads
/// a half-written file. A symlinked settings file (dotfiles) stays a symlink, because the
/// rename replaces the file it points to.
fn write_settings(path: &Path, content: &str) -> Result<(), SettingsError> {
    let write_error = |source| SettingsError::Write {
        path: path.to_path_buf(),
        source,
    };

    let target = match fs::canonicalize(path) {
        Ok(target) => target,
        Err(err) if err.kind() == ErrorKind::NotFound => path.to_path_buf(),
        Err(err) => return Err(write_error(err)),
    };
    let temp = target.with_file_name(format!(".{SETTINGS_FILE}.anvil-tmp"));

    let result = fs::write(&temp, content)
        .and_then(|()| keep_permissions(&target, &temp))
        .and_then(|()| fs::rename(&temp, &target));

    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }

    result.map_err(write_error)
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
    use tempfile::TempDir;

    fn hosts(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    /// Returns a config root with a `User` directory for VS Code, and the path of its settings.
    fn vscode_root() -> (TempDir, PathBuf) {
        let root = TempDir::new().unwrap();
        let user = root.path().join("Code/User");
        fs::create_dir_all(&user).unwrap();

        (root, user.join("settings.json"))
    }

    #[test]
    fn adds_hosts_and_keeps_comments_and_other_keys() {
        let (root, settings) = vscode_root();
        fs::write(
            &settings,
            "{\n  // my own comment stays\n  \"editor.fontSize\": 14,\n  \
             \"remote.SSH.remotePlatform\": {\n    \"myserver\": \"linux\",\n  },\n}\n",
        )
        .unwrap();

        let errors = sync_settings(root.path(), &hosts(&["project.anvil"]));

        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            fs::read_to_string(&settings).unwrap(),
            "{\n  // my own comment stays\n  \"editor.fontSize\": 14,\n  \
             \"remote.SSH.remotePlatform\": {\n    \"myserver\": \"linux\",\n    \
             \"project.anvil\": \"linux\",\n  },\n}\n"
        );
    }

    #[test]
    fn removes_stale_hosts_only() {
        let (root, settings) = vscode_root();
        fs::write(
            &settings,
            "{\n  \"remote.SSH.remotePlatform\": {\n    \"old.anvil\": \"linux\",\n    \
             \"myserver\": \"linux\",\n    \"web.anvil\": \"linux\"\n  }\n}\n",
        )
        .unwrap();

        sync_settings(root.path(), &hosts(&["web.anvil"]));

        assert_eq!(
            fs::read_to_string(&settings).unwrap(),
            "{\n  \"remote.SSH.remotePlatform\": {\n    \"myserver\": \"linux\",\n    \
             \"web.anvil\": \"linux\"\n  }\n}\n"
        );
    }

    #[test]
    fn creates_missing_settings_file() {
        let (root, settings) = vscode_root();

        sync_settings(root.path(), &hosts(&["project.anvil"]));

        let content = fs::read_to_string(&settings).unwrap();
        let value: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
        assert_eq!(value[PLATFORM_KEY]["project.anvil"], "linux", "{content}");
    }

    #[test]
    fn leaves_missing_settings_file_alone_without_hosts() {
        let (root, settings) = vscode_root();

        sync_settings(root.path(), &[]);

        assert!(!settings.exists());
    }

    #[test]
    fn skips_editors_that_are_not_installed() {
        let root = TempDir::new().unwrap();
        fs::create_dir_all(root.path().join("Cursor")).unwrap();

        let errors = sync_settings(root.path(), &hosts(&["project.anvil"]));

        assert!(errors.is_empty(), "{errors:?}");
        assert!(!root.path().join("Cursor/User").exists());
        assert!(!root.path().join("Code").exists());
    }

    #[test]
    fn syncs_every_installed_editor() {
        let root = TempDir::new().unwrap();
        for editor in ["Code - Insiders", "Cursor", "VSCodium"] {
            fs::create_dir_all(root.path().join(editor).join("User")).unwrap();
        }

        sync_settings(root.path(), &hosts(&["project.anvil"]));

        for editor in ["Code - Insiders", "Cursor", "VSCodium"] {
            let settings = root.path().join(editor).join("User/settings.json");
            assert!(
                fs::read_to_string(settings)
                    .unwrap()
                    .contains("\"project.anvil\": \"linux\""),
                "{editor}"
            );
        }
    }

    #[test]
    fn leaves_invalid_file_unchanged_and_syncs_the_others() {
        let (root, settings) = vscode_root();
        let invalid = "{ \"editor.fontSize\": 14, oops";
        fs::write(&settings, invalid).unwrap();
        fs::create_dir_all(root.path().join("Cursor/User")).unwrap();

        let errors = sync_settings(root.path(), &hosts(&["project.anvil"]));

        assert!(
            matches!(errors.as_slice(), [SettingsError::Parse { path, .. }] if *path == settings),
            "{errors:?}"
        );
        assert_eq!(fs::read_to_string(&settings).unwrap(), invalid);
        assert!(root.path().join("Cursor/User/settings.json").exists());
    }

    #[test]
    fn leaves_file_with_non_object_platform_unchanged() {
        let (root, settings) = vscode_root();
        let content = "{ \"remote.SSH.remotePlatform\": \"linux\" }\n";
        fs::write(&settings, content).unwrap();

        let errors = sync_settings(root.path(), &hosts(&["project.anvil"]));

        assert!(
            matches!(
                errors.as_slice(),
                [SettingsError::PlatformNotAnObject { .. }]
            ),
            "{errors:?}"
        );
        assert_eq!(fs::read_to_string(&settings).unwrap(), content);
    }

    #[test]
    fn keeps_a_symlinked_settings_file_a_symlink() {
        let (root, settings) = vscode_root();
        let target = root.path().join("dotfiles-settings.json");
        fs::write(&target, "{}\n").unwrap();
        std::os::unix::fs::symlink(&target, &settings).unwrap();

        sync_settings(root.path(), &hosts(&["project.anvil"]));

        assert!(fs::symlink_metadata(&settings).unwrap().is_symlink());
        assert!(
            fs::read_to_string(&target)
                .unwrap()
                .contains("\"project.anvil\": \"linux\"")
        );
    }

    #[test]
    fn leaves_file_vs_code_rejects_unchanged() {
        let (root, settings) = vscode_root();
        for invalid in [
            "{ \"a\": 1 \"b\": 2 }\n",
            "{ 'a': 1 }\n",
            "{ a: 1 }\n",
            "{ \"a\": 0xFF }\n",
        ] {
            fs::write(&settings, invalid).unwrap();

            let errors = sync_settings(root.path(), &hosts(&["project.anvil"]));

            assert!(
                matches!(errors.as_slice(), [SettingsError::Parse { .. }]),
                "{invalid}: {errors:?}"
            );
            assert_eq!(fs::read_to_string(&settings).unwrap(), invalid);
        }
    }

    #[test]
    fn leaves_file_with_duplicate_platform_key_unchanged() {
        let (root, settings) = vscode_root();
        let content = "{\n  \"remote.SSH.remotePlatform\": {},\n  \
                       \"remote.SSH.remotePlatform\": {}\n}\n";
        fs::write(&settings, content).unwrap();

        let errors = sync_settings(root.path(), &hosts(&["project.anvil"]));

        assert!(
            matches!(
                errors.as_slice(),
                [SettingsError::DuplicatePlatformKey { .. }]
            ),
            "{errors:?}"
        );
        assert_eq!(fs::read_to_string(&settings).unwrap(), content);
    }

    #[test]
    fn leaves_dangling_settings_symlink_alone() {
        let (root, settings) = vscode_root();
        let target = root.path().join("dotfiles/settings.json");
        std::os::unix::fs::symlink(&target, &settings).unwrap();

        let errors = sync_settings(root.path(), &hosts(&["project.anvil"]));

        assert!(
            matches!(errors.as_slice(), [SettingsError::Read { .. }]),
            "{errors:?}"
        );
        assert!(fs::symlink_metadata(&settings).unwrap().is_symlink());
        assert!(!target.exists());
    }

    #[test]
    fn leaves_up_to_date_file_untouched() {
        let (root, settings) = vscode_root();
        let content = "{\"remote.SSH.remotePlatform\": {\"project.anvil\": \"linux\"}}";
        fs::write(&settings, content).unwrap();

        sync_settings(root.path(), &hosts(&["project.anvil"]));

        assert_eq!(fs::read_to_string(&settings).unwrap(), content);
    }
}
