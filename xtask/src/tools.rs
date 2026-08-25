use std::path::Path;
use std::process::Command;

/// The repo's package manager.
///
/// pnpm is the single package manager (see the Node monorepo tooling decision);
/// `pnpm-lock.yaml` is the only lockfile and `pnpm-workspace.yaml` the only
/// workspace declaration.
///
/// On Windows pnpm installs as `pnpm.cmd`, a cmd shim that `CreateProcess` will
/// not resolve through `PATHEXT` when spawned as a bare name.
pub fn package_manager() -> Command {
    if cfg!(windows) {
        Command::new("pnpm.cmd")
    } else {
        Command::new("pnpm")
    }
}

pub fn check_node_version(repo_root: &Path) -> Command {
    let script = repo_root.join("scripts/check-node-version.mjs");
    let mut cmd = Command::new("node");
    cmd.current_dir(repo_root).arg(script);
    cmd
}
