use std::path::{Path, PathBuf};

use serde::Serialize;

/// Failure modes of a git invocation.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GitError {
    NotInstalled,
    Timeout,
    Failed { stderr: String },
}

/// Abstraction over the `git` executable so callers can be tested with a fake.
pub trait GitRunner: Send + Sync {
    fn available(&self) -> bool;
    fn ls_remote_head(&self, url: &str, git_ref: Option<&str>) -> Result<String, GitError>;
    fn shallow_clone(
        &self,
        url: &str,
        git_ref: Option<&str>,
        dest: &Path,
    ) -> Result<String, GitError>;
    fn remote_url(&self, dir: &Path) -> Option<String>;
    fn head_commit(&self, dir: &Path) -> Option<String>;
    fn repo_root(&self, dir: &Path) -> Option<PathBuf>;
}
