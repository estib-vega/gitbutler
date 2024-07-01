use std::path::Path;

use anyhow::{Context, Result};
use bstr::BString;
use itertools::Itertools;
use serde::Serialize;

use super::{get_remote_commit_file_paths, target, Author, CommitMetrics, VirtualBranchesHandle};
use crate::{
    git::{self, CommitExt, RepositoryExt},
    project_repository::{self, LogUntil},
};

// this struct is a mapping to the view `RemoteBranch` type in Typescript
// found in src-tauri/src/routes/repo/[project_id]/types.ts
//
// it holds data calculated for presentation purposes of one Git branch
// with comparison data to the Target commit, determining if it is mergeable,
// and how far ahead or behind the Target it is.
// an array of them can be requested from the frontend to show in the sidebar
// Tray and should only contain branches that have not been converted into
// virtual branches yet (ie, we have no `Branch` struct persisted in our data.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBranch {
    #[serde(with = "crate::serde::oid")]
    pub sha: git2::Oid,
    pub name: git::Refname,
    pub upstream: Option<git::RemoteRefname>,
    pub last_commit_timestamp_ms: Option<u128>,
    pub last_commit_author: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBranchData {
    #[serde(with = "crate::serde::oid")]
    pub sha: git2::Oid,
    pub name: git::Refname,
    pub upstream: Option<git::RemoteRefname>,
    pub behind: u32,
    pub commits: Vec<RemoteCommit>,
    #[serde(with = "crate::serde::oid_opt", default)]
    pub fork_point: Option<git2::Oid>,
    pub recent_authors: Vec<CommitMetrics>,
    pub recent_files: Vec<CommitMetrics>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteCommit {
    pub id: String,
    #[serde(serialize_with = "crate::serde::as_string_lossy")]
    pub description: BString,
    pub created_at: u128,
    pub author: Author,
    pub change_id: Option<String>,
    #[serde(with = "crate::serde::oid_vec")]
    pub parent_ids: Vec<git2::Oid>,
    pub file_paths: Vec<String>,
}

pub fn get_recent_commits_metric(
    recent_commits: &Vec<RemoteCommit>,
) -> (Vec<CommitMetrics>, Vec<CommitMetrics>) {
    let mut recent_authors =
        std::collections::HashMap::<String, CommitMetrics>::with_capacity(recent_commits.len());
    let mut recent_files = std::collections::HashMap::<String, CommitMetrics>::new();
    for commit in recent_commits {
        let commit_id = &commit.id;
        let author_name = &commit.author.name;
        let author_metric =
            recent_authors
                .entry(author_name.to_string())
                .or_insert(CommitMetrics {
                    name: author_name.to_string(),
                    value: 0,
                    commit_ids: Vec::new(),
                });
        author_metric.value += 1;
        author_metric.commit_ids.push(commit_id.to_string());
        for file_path in &commit.file_paths {
            let file_metric = recent_files
                .entry(file_path.to_string())
                .or_insert(CommitMetrics {
                    name: file_path.to_string(),
                    value: 0,
                    commit_ids: Vec::new(),
                });
            file_metric.value += 1;
            file_metric.commit_ids.push(commit_id.to_string());
        }
    }

    let recent_authors = recent_authors
        .into_values()
        .sorted_by_key(|a| a.value)
        .rev()
        .collect();

    let recent_files = recent_files
        .into_values()
        .sorted_by_key(|f| f.value)
        .rev()
        .collect();

    (recent_authors, recent_files)
}

// for legacy purposes, this is still named "remote" branches, but it's actually
// a list of all the normal (non-gitbutler) git branches.
pub fn list_remote_branches(
    project_repository: &project_repository::Repository,
) -> Result<Vec<RemoteBranch>> {
    let default_target = default_target(&project_repository.project().gb_dir())?;

    let mut remote_branches = vec![];
    for (branch, _) in project_repository
        .repo()
        .branches(None)
        .context("failed to list remote branches")?
        .flatten()
    {
        let branch = branch_to_remote_branch(&branch)?;

        if let Some(branch) = branch {
            let branch_is_trunk = branch.name.branch() == Some(default_target.branch.branch())
                && branch.name.remote() == Some(default_target.branch.remote());

            if !branch_is_trunk
                && branch.name.branch() != Some("gitbutler/integration")
                && branch.name.branch() != Some("gitbutler/target")
            {
                remote_branches.push(branch);
            }
        }
    }
    Ok(remote_branches)
}

pub fn get_branch_data(
    project_repository: &project_repository::Repository,
    refname: &git::Refname,
) -> Result<RemoteBranchData> {
    let default_target = default_target(&project_repository.project().gb_dir())?;

    let branch = project_repository
        .repo()
        .find_branch_by_refname(refname)?
        .ok_or(anyhow::anyhow!("failed to find branch {}", refname))?;

    branch_to_remote_branch_data(project_repository, &branch, default_target.sha)?
        .context("failed to get branch data")
}

pub fn branch_to_remote_branch(branch: &git2::Branch) -> Result<Option<RemoteBranch>> {
    let commit = match branch.get().peel_to_commit() {
        Ok(c) => c,
        Err(err) => {
            tracing::warn!(
                ?err,
                "ignoring branch {:?} as peeling failed",
                branch.name()
            );
            return Ok(None);
        }
    };
    let name = git::Refname::try_from(branch).context("could not get branch name");
    match name {
        Ok(name) => branch
            .get()
            .target()
            .map(|sha| {
                Ok(RemoteBranch {
                    sha,
                    upstream: if let git::Refname::Local(local_name) = &name {
                        local_name.remote().cloned()
                    } else {
                        None
                    },
                    name,
                    last_commit_timestamp_ms: commit
                        .time()
                        .seconds()
                        .try_into()
                        .map(|t: u128| t * 1000)
                        .ok(),
                    last_commit_author: commit
                        .author()
                        .name()
                        .map(std::string::ToString::to_string),
                })
            })
            .transpose(),
        Err(_) => Ok(None),
    }
}

pub fn branch_to_remote_branch_data(
    project_repository: &project_repository::Repository,
    branch: &git2::Branch,
    base: git2::Oid,
) -> Result<Option<RemoteBranchData>> {
    branch
        .get()
        .target()
        .map(|sha| {
            let ahead = project_repository
                .log(sha, LogUntil::Commit(base))
                .context("failed to get ahead commits")?;

            let name = git::Refname::try_from(branch).context("could not get branch name")?;

            let count_behind = project_repository
                .distance(base, sha)
                .context("failed to get behind count")?;

            let fork_point = ahead.last().and_then(|c| c.parent(0).ok()).map(|c| c.id());

            let commits = ahead
                .iter()
                .map(|commit| commit_to_remote_commit(project_repository, commit))
                .collect::<Vec<_>>();

            let (recent_authors, recent_files) = super::get_recent_commits_metric(&commits);

            Ok(RemoteBranchData {
                sha,
                upstream: if let git::Refname::Local(local_name) = &name {
                    local_name.remote().cloned()
                } else {
                    None
                },
                name,
                behind: count_behind,
                commits,
                fork_point,
                recent_authors,
                recent_files,
            })
        })
        .transpose()
}

pub fn commit_to_remote_commit(
    project_repository: &project_repository::Repository,
    commit: &git2::Commit,
) -> RemoteCommit {
    let parent_ids: Vec<git2::Oid> = commit.parents().map(|c| c.id()).collect::<Vec<_>>();
    let file_paths = get_remote_commit_file_paths(project_repository.repo(), commit)
        .ok()
        .filter(|files| !files.is_empty())
        .unwrap_or_default();

    RemoteCommit {
        id: commit.id().to_string(),
        description: commit.message_bstr().to_owned(),
        created_at: commit.time().seconds().try_into().unwrap(),
        author: commit.author().into(),
        change_id: commit.change_id(),
        parent_ids,
        file_paths,
    }
}

fn default_target(base_path: &Path) -> Result<target::Target> {
    VirtualBranchesHandle::new(base_path).get_default_target()
}
