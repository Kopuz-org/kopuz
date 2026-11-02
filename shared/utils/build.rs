use std::path::{Path, PathBuf};
use std::process::Command;

fn command_output(repo_root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn safe_revision(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'+')))
    .then_some(value)
}

fn repo_path(repo_root: &Path, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        repo_root.join(path)
    }
}

fn watch_git_revision(repo_root: &Path) {
    if let Some(head) = command_output(repo_root, &["rev-parse", "--git-path", "HEAD"]) {
        println!(
            "cargo:rerun-if-changed={}",
            repo_path(repo_root, &head).display()
        );
    }

    if let Some(reference) = command_output(repo_root, &["symbolic-ref", "-q", "HEAD"])
        && let Some(path) =
            command_output(repo_root, &["rev-parse", "--git-path", reference.as_str()])
    {
        println!(
            "cargo:rerun-if-changed={}",
            repo_path(repo_root, &path).display()
        );
    }
}

fn main() {
    println!("cargo:rerun-if-env-changed=KOPUZ_GIT_COMMIT");

    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    watch_git_revision(&repo_root);

    let revision = std::env::var("KOPUZ_GIT_COMMIT")
        .ok()
        .and_then(|value| safe_revision(&value).map(str::to_owned))
        .or_else(|| command_output(&repo_root, &["rev-parse", "--verify", "HEAD"]))
        .and_then(|value| safe_revision(&value).map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=KOPUZ_GIT_COMMIT={revision}");
}
