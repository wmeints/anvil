use anyhow::{Result, bail};
use std::path::Path;

use crate::manage::SPEC_FILE_NAME;

/// Validates the spec file in the working directory and reports any problem with its
/// line and column. Returns whether the spec is valid.
pub fn validate_spec(working_dir: &Path) -> Result<bool> {
    let spec_path = working_dir.join(SPEC_FILE_NAME);

    if !spec_path.is_file() {
        bail!("can't find {SPEC_FILE_NAME} in {}", working_dir.display());
    }

    match check_spec(&spec_path) {
        Ok(()) => {
            println!("{SPEC_FILE_NAME} is valid");
            Ok(true)
        }
        Err(report) => {
            eprintln!("{report}");
            Ok(false)
        }
    }
}

/// Loads the spec, formatting a parse problem as `file:line:column: error: message`.
fn check_spec(spec_path: &Path) -> Result<(), String> {
    let Err(err) = anvil_spec::from_file(spec_path) else {
        return Ok(());
    };

    match err.diagnostic() {
        Some(diagnostic) => Err(format!(
            "{SPEC_FILE_NAME}:{}:{}: error: {}",
            diagnostic.line, diagnostic.column, diagnostic.message
        )),
        None => Err(format!("{SPEC_FILE_NAME}: error: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_spec(content: &str) -> TempDir {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(SPEC_FILE_NAME), content).unwrap();
        dir
    }

    #[test]
    fn valid_spec_passes() {
        let dir = write_spec("name: dev\nimage: ubuntu:24.04\n");

        assert!(validate_spec(dir.path()).unwrap());
    }

    #[test]
    fn invalid_spec_fails() {
        let dir = write_spec("image: ubuntu:24.04\n");

        assert!(!validate_spec(dir.path()).unwrap());
    }

    #[test]
    fn missing_spec_returns_error() {
        let dir = TempDir::new().unwrap();

        assert!(validate_spec(dir.path()).is_err());
    }

    #[test]
    fn report_includes_line_and_column() {
        let dir = write_spec("name: dev\nresources:\n  cpu: 2\n  memroy: 4Gi\n");

        let report = check_spec(&dir.path().join(SPEC_FILE_NAME)).unwrap_err();

        assert_eq!(
            report,
            ".anvil.yml:4:3: error: resources: unknown field `memroy`, expected `cpu` or `memory`"
        );
    }
}
