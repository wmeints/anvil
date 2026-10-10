use std::path::PathBuf;

/// Returns the path of the daemon's unix socket.
pub fn socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("fbkd.sock")
}

/// Returns the directory the daemon writes its log files to.
pub fn log_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .unwrap_or_else(std::env::temp_dir)
        .join("firebrick")
}

/// Returns the directory holding the SSH keys and config the daemon provisions for sandboxes.
pub fn ssh_dir() -> PathBuf {
    data_dir().join("ssh")
}

/// Returns the file the daemon stores the secrets for sandboxes in.
pub fn secrets_path() -> PathBuf {
    data_dir().join("secrets.yml")
}

/// Returns the directory holding the daemon's persistent data.
fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir)
        .join("firebrick")
}
