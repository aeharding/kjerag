use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo set manifest dir"));
    let workspace = manifest
        .parent()
        .and_then(Path::parent)
        .expect("spike crate is inside the workspace");

    let head = git(workspace, &["rev-parse", "--verify", "HEAD"]);
    let tree = git(workspace, &["rev-parse", "--verify", "HEAD^{tree}"]);
    let status = git(
        workspace,
        &["status", "--porcelain=v1", "--untracked-files=no"],
    );
    let (head, tree, dirty) = match (head, tree, status) {
        (Some(head), Some(tree), Some(status)) => {
            (head, tree, if status.is_empty() { "false" } else { "true" })
        }
        _ => (
            "unavailable".to_owned(),
            "unavailable".to_owned(),
            "unknown",
        ),
    };
    println!("cargo:rustc-env=KJERAG_BUILD_GIT_HEAD={head}");
    println!("cargo:rustc-env=KJERAG_BUILD_GIT_TREE={tree}");
    println!("cargo:rustc-env=KJERAG_BUILD_GIT_DIRTY={dirty}");

    // Keep cached builds honest across both source edits and commits. This is
    // deliberately generated from Git's tracked set: untracked files do not
    // participate in the evidence claim.
    if let Some(files) = git(workspace, &["ls-files"]) {
        for file in files.lines() {
            println!("cargo:rerun-if-changed={}", workspace.join(file).display());
        }
    }
    let mut git_paths = vec![
        "HEAD".to_owned(),
        "index".to_owned(),
        "packed-refs".to_owned(),
    ];
    if let Some(reference) = git(workspace, &["symbolic-ref", "-q", "HEAD"]) {
        git_paths.push(reference);
    }
    for git_path in git_paths {
        if let Some(path) = git(workspace, &["rev-parse", "--git-path", &git_path]) {
            let path = PathBuf::from(path);
            let path = if path.is_absolute() {
                path
            } else {
                workspace.join(path)
            };
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}

fn git(workspace: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .arg("-C")
        .arg(workspace)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
}
