use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use uuid::Uuid;

use crate::core::storage;
use crate::core::ui as core_ui;

pub struct RemoteDeployTarget {
    pub host: String,
    pub remote_dir: String,
    pub remote_file: String,
}

impl RemoteDeployTarget {
    pub fn display_target(&self) -> String {
        format!("{}:{}", self.host, self.remote_file)
    }

    pub fn scp_destination(&self) -> String {
        format!("{}:{}", self.host, shell_single_quote(&self.remote_file))
    }
}

pub struct SshMasterConnection {
    ssh_bin: PathBuf,
    host: String,
    control_path: PathBuf,
}

impl SshMasterConnection {
    pub fn without_control(&self) -> Self {
        Self {
            ssh_bin: self.ssh_bin.clone(),
            host: self.host.clone(),
            control_path: PathBuf::new(),
        }
    }

    pub fn base_args(&self) -> Vec<std::ffi::OsString> {
        if self.control_path.as_os_str().is_empty() {
            return Vec::new();
        }

        vec![
            "-o".into(),
            "ControlMaster=auto".into(),
            "-o".into(),
            format!("ControlPath={}", self.control_path.display()).into(),
            "-o".into(),
            "ControlPersist=60".into(),
        ]
    }

    pub fn close(&self, identity_file: Option<&Path>) -> Result<()> {
        if self.control_path.as_os_str().is_empty() || !self.control_path.exists() {
            return Ok(());
        }

        let _ = Command::new(&self.ssh_bin)
            .args(self.base_args())
            .args(identity_arg(identity_file))
            .arg("-O")
            .arg("exit")
            .arg(&self.host)
            .status();
        Ok(())
    }
}

pub fn deploy_file(
    source: &Path,
    target: &str,
    default_filename: &str,
    identity_file: Option<&Path>,
) -> Result<()> {
    let ui = core_ui::messages();
    if !source.exists() {
        bail!("{}", ui.deploy_missing_auth(source));
    }

    let Some(ssh_bin) = resolve_ssh_bin() else {
        bail!("{}", ui.deploy_missing_ssh());
    };
    let Some(scp_bin) = resolve_scp_bin() else {
        bail!("{}", ui.deploy_missing_scp());
    };

    let remote = parse_remote_deploy_target(target, default_filename)?;
    if let Some(identity_file) = identity_file {
        storage::ensure_exists(identity_file, "SSH identity file")
            .map_err(|_| anyhow::anyhow!(ui.deploy_identity_not_found(identity_file)))?;
    }

    println!("{}", ui.deploy_start(&remote.display_target()));
    with_ssh_master_connection(&ssh_bin, identity_file, &remote.host, |master| {
        let ssh_status = Command::new(&ssh_bin)
            .args(master.base_args())
            .args(identity_arg(identity_file))
            .arg(&remote.host)
            .arg(format!(
                "mkdir -p {}",
                shell_single_quote(&remote.remote_dir)
            ))
            .status()
            .with_context(|| format!("failed to execute {}", ssh_bin.display()))?;
        if !ssh_status.success() {
            bail!(
                "{}",
                ui.deploy_prepare_remote_dir_failed(ssh_status.code().unwrap_or(1))
            );
        }

        let scp_status = Command::new(&scp_bin)
            .args(master.base_args())
            .args(identity_arg(identity_file))
            .arg(source)
            .arg(remote.scp_destination())
            .status()
            .with_context(|| format!("failed to execute {}", scp_bin.display()))?;
        if !scp_status.success() {
            bail!("{}", ui.deploy_copy_failed(scp_status.code().unwrap_or(1)));
        }

        Ok(())
    })?;

    println!("{}", ui.deploy_completed(&remote.display_target()));
    Ok(())
}

fn resolve_ssh_bin() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["ssh.exe", "ssh"]
    } else {
        &["ssh"]
    };
    for name in names {
        if let Some(path) = crate::core::storage::find_in_path(name) {
            return Some(path);
        }
    }
    None
}

fn resolve_scp_bin() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["scp.exe", "scp"]
    } else {
        &["scp"]
    };
    for name in names {
        if let Some(path) = crate::core::storage::find_in_path(name) {
            return Some(path);
        }
    }
    None
}

fn identity_arg(identity_file: Option<&Path>) -> Vec<&std::ffi::OsStr> {
    identity_file
        .map(|path| vec![std::ffi::OsStr::new("-i"), path.as_os_str()])
        .unwrap_or_default()
}

pub fn parse_remote_deploy_target(
    target: &str,
    default_filename: &str,
) -> Result<RemoteDeployTarget> {
    let ui = core_ui::messages();
    let Some((host, raw_path)) = target.split_once(':') else {
        bail!("{}", ui.deploy_invalid_target(target));
    };
    let host = host.trim();
    let raw_path = raw_path.trim();
    if host.is_empty() || raw_path.is_empty() {
        bail!("{}", ui.deploy_invalid_target(target));
    }

    let remote_file = normalize_remote_file(raw_path, default_filename);
    let remote_dir = remote_parent_dir(&remote_file);

    Ok(RemoteDeployTarget {
        host: host.to_string(),
        remote_dir,
        remote_file,
    })
}

fn normalize_remote_file(path: &str, default_filename: &str) -> String {
    let trimmed = path.trim();
    if trimmed.ends_with(default_filename) || trimmed == default_filename {
        return trimmed.to_string();
    }
    let base = trimmed.trim_end_matches('/');
    if base.is_empty() {
        default_filename.into()
    } else {
        format!("{base}/{default_filename}")
    }
}

fn remote_parent_dir(path: &str) -> String {
    let trimmed = path.trim();
    if let Some((parent, _)) = trimmed.rsplit_once('/') {
        if parent.is_empty() {
            "/".into()
        } else {
            parent.into()
        }
    } else {
        ".".into()
    }
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub fn with_ssh_master_connection<F>(
    ssh_bin: &Path,
    identity_file: Option<&Path>,
    host: &str,
    f: F,
) -> Result<()>
where
    F: FnOnce(&SshMasterConnection) -> Result<()>,
{
    let temp_root = env::temp_dir().join(format!("scodex-ssh-{}", Uuid::new_v4()));
    fs::create_dir_all(&temp_root)
        .with_context(|| format!("failed to create {}", temp_root.display()))?;
    let control_path = temp_root.join("mux");
    let master = SshMasterConnection {
        ssh_bin: ssh_bin.to_path_buf(),
        host: host.to_string(),
        control_path,
    };

    let establish = Command::new(ssh_bin)
        .args(master.base_args())
        .args(identity_arg(identity_file))
        .arg("-Nf")
        .arg(host)
        .status()
        .with_context(|| format!("failed to execute {}", ssh_bin.display()));

    let result = match establish {
        Ok(status) if status.success() => f(&master),
        Ok(_) | Err(_) => f(&master.without_control()),
    };

    let _ = master.close(identity_file);
    let _ = fs::remove_dir_all(&temp_root);
    result
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::{parse_remote_deploy_target, remote_parent_dir};

    #[test]
    fn deploy_target_directory_appends_default_filename() -> Result<()> {
        let target = parse_remote_deploy_target("user@example.com:/srv/codex", "auth.json")?;
        assert_eq!(target.host, "user@example.com");
        assert_eq!(target.remote_dir, "/srv/codex");
        assert_eq!(target.remote_file, "/srv/codex/auth.json");
        Ok(())
    }

    #[test]
    fn deploy_target_exact_file_is_preserved() -> Result<()> {
        let target =
            parse_remote_deploy_target("root@host:/opt/custom/auth.json", "auth.json")?;
        assert_eq!(target.host, "root@host");
        assert_eq!(target.remote_dir, "/opt/custom");
        assert_eq!(target.remote_file, "/opt/custom/auth.json");
        Ok(())
    }

    #[test]
    fn deploy_target_helpers_handle_relative_paths() {
        assert_eq!(remote_parent_dir("auth.json"), ".");
        assert_eq!(remote_parent_dir("configs/auth.json"), "configs");
        assert_eq!(remote_parent_dir("/auth.json"), "/");
    }
}
