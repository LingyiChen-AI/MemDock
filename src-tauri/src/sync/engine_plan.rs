//! Plan building for the sync engine: which files to compare, blob-id
//! classification, held projects and the batched blob reads.

use super::*;

/// Project key of a `projects/<key>/...` path among the projects present here
/// (longest match, as `expected_local_path`).
pub(super) fn project_of<'p>(repo_path: &str, projects: &'p [(String, PathBuf)]) -> Option<&'p str> {
    let rest = repo_path.strip_prefix("projects/")?;
    projects
        .iter()
        .filter(|(k, _)| rest.strip_prefix(k.as_str()).is_some_and(|t| t.starts_with('/')))
        .max_by_key(|(k, _)| k.len())
        .map(|(k, _)| k.as_str())
}

/// Keys of projects whose base is trusted: synced last time with the same
/// directory as now.
pub(super) fn trusted_keys(ctx: &SyncCtx, synced: &[SyncedProject]) -> HashSet<String> {
    current_synced(ctx)
        .into_iter()
        .filter(|now| synced.iter().any(|s| s == now))
        .map(|p| p.key)
        .collect()
}

pub(super) fn base_valid(repo_path: &str, key: Option<&str>, trusted: &HashSet<String>) -> bool {
    GLOBAL_PATHS.contains(&repo_path) || key.is_some_and(|k| trusted.contains(k))
}

/// `path -> blob id` of the regular files of the base commit; `None` without
/// a usable base (no previous sync, or the commit object is missing here).
pub(super) fn base_tree(
    ctx: &SyncCtx,
    base: Option<&str>,
) -> Result<Option<BTreeMap<String, String>>, SyncError> {
    let Some(b) = base else { return Ok(None) };
    if !ctx.git.has_commit(ctx.repo_dir, b)? {
        return Ok(None);
    }
    let tree = ctx.git.tree_entries(ctx.repo_dir, b)?;
    Ok(Some(
        tree.into_iter()
            .filter(|(_, e)| e.is_regular())
            .map(|(p, e)| (p, e.sha))
            .collect(),
    ))
}

/// One repo path to compare and the local file it maps to.
pub(super) struct Candidate {
    pub(super) repo_path: String,
    pub(super) local_path: PathBuf,
    pub(super) key: Option<String>,
}

/// Candidates plus the repo paths that cannot be synced: `(path, reason)`.
pub(super) struct Candidates {
    pub(super) list: Vec<Candidate>,
    pub(super) rejected: Vec<(String, &'static str)>,
}

/// `union(local files, base paths, remote paths)` restricted to files that map
/// to this machine. Repo files of projects absent here are not included.
/// Repo paths differing only by case, or mapping onto a local file another
/// repo path already maps to, are rejected.
pub(super) fn candidates<'s>(
    ctx: &SyncCtx,
    base: impl Iterator<Item = &'s String>,
    remote: impl Iterator<Item = &'s String>,
) -> Candidates {
    let mut all: Vec<Candidate> = Vec::new();
    let mut seen_repo: HashSet<String> = HashSet::new();
    for f in &ctx.files {
        if seen_repo.insert(f.repo_path.clone()) {
            all.push(Candidate {
                repo_path: f.repo_path.clone(),
                local_path: f.local_path.clone(),
                key: f.project_key.clone(),
            });
        }
    }
    for p in base.chain(remote) {
        if seen_repo.contains(p) {
            continue;
        }
        let Some(local) = expected_local_path(p, &ctx.projects, ctx.paths) else {
            continue;
        };
        seen_repo.insert(p.clone());
        all.push(Candidate {
            repo_path: p.clone(),
            local_path: local,
            key: project_of(p, &ctx.projects).map(String::from),
        });
    }
    // Repo paths differing only by case cannot coexist on every file system.
    let mut by_lower: HashMap<String, usize> = HashMap::new();
    for c in &all {
        *by_lower.entry(c.repo_path.to_lowercase()).or_default() += 1;
    }
    let mut out = Candidates {
        list: Vec::new(),
        rejected: Vec::new(),
    };
    // normalize_path is case-insensitive on Windows only
    let mut seen_local: HashSet<String> = HashSet::new();
    for c in all {
        if by_lower[&c.repo_path.to_lowercase()] > 1 {
            out.rejected.push((c.repo_path, "case_collision"));
        } else if !seen_local.insert(normalize_path(&c.local_path.to_string_lossy())) {
            // Two repo paths must never write the same local file.
            out.rejected.push((c.repo_path, "duplicate_local"));
        } else {
            out.list.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------- prepare

/// Fetches and builds the plan. Writes nothing locally or to the repository.
pub fn prepare(ctx: &SyncCtx) -> Result<SyncPlan, SyncError> {
    let remote = ctx.git.fetch(ctx.repo_dir, &ctx.auth)?;
    build_plan(ctx, remote, &SyncBase::of(ctx))
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Take,
    Push,
    Deleted,
    Both,
}

/// A candidate after comparing blob ids, before any blob is read.
struct Pre {
    c: Candidate,
    local_hash: String,
    local: Option<String>,
    l_sha: Option<String>,
    b_sha: Option<String>,
    r_sha: Option<String>,
    kind: Kind,
    held: bool,
}

pub(super) fn build_plan(
    ctx: &SyncCtx,
    remote: Option<String>,
    base: &SyncBase,
) -> Result<SyncPlan, SyncError> {
    let git = ctx.git;
    let repo = ctx.repo_dir;
    let remote_full = match &remote {
        Some(r) => git.tree_entries(repo, r)?,
        None => BTreeMap::new(),
    };
    let remote_tree: BTreeMap<String, String> = remote_full
        .iter()
        .filter(|(_, e)| e.is_regular())
        .map(|(p, e)| (p.clone(), e.sha.clone()))
        .collect();
    let base_tree = base_tree(ctx, base.commit.as_deref())?.unwrap_or_default();
    let trusted = trusted_keys(ctx, &base.synced_projects);
    let unsynced = &base.unsynced_paths;
    // Paths skipped last time have no trusted base: pull or conflict only.
    let base_ok = |path: &str, key: Option<&str>| {
        base_valid(path, key, &trusted) && !unsynced.contains(path)
    };
    let base_paths: Vec<&String> = base_tree
        .keys()
        .filter(|p| base_ok(p, project_of(p, &ctx.projects)))
        .collect();

    let mut plan = SyncPlan {
        id: uuid::Uuid::new_v4().to_string(),
        remote_commit: remote.clone(),
        take_remote: Vec::new(),
        push_local: Vec::new(),
        merged: Vec::new(),
        conflicts: Vec::new(),
        remote_deleted: Vec::new(),
        local_all_deleted: Vec::new(),
        secret_hits: Vec::new(),
        missing_skills: Vec::new(),
        removed_elsewhere: Vec::new(),
        skipped: Vec::new(),
        entries: Vec::new(),
        remote_tree: BTreeMap::new(),
        manifest_json: None,
        unsynced_after: BTreeSet::new(),
        held_after: BTreeSet::new(),
    };
    let skip = |plan: &mut SyncPlan, path: &str, reason: &str| {
        plan.skipped.push(SkippedFile {
            repo_path: path.to_string(),
            reason: reason.to_string(),
        })
    };

    let cands = candidates(ctx, base_paths.into_iter(), remote_full.keys());
    for (p, reason) in &cands.rejected {
        skip(&mut plan, p, reason);
    }
    let mut considered: BTreeSet<String> = BTreeSet::new();

    // Pass 1: compare blob ids.
    let mut pres: Vec<Pre> = Vec::new();
    let mut has_local: HashSet<String> = HashSet::new();
    for c in cands.list {
        considered.insert(c.repo_path.clone());
        // symlinks / gitlinks at a memory path are never touched
        if remote_full.get(&c.repo_path).is_some_and(|e| !e.is_regular()) {
            skip(&mut plan, &c.repo_path, "not_regular_file");
            continue;
        }
        let lbytes = read_opt(&c.local_path)?;
        let local_hash = lbytes.as_deref().map(sha256_hex).unwrap_or_default();
        let local = match &lbytes {
            None => None,
            Some(b) => match decode_text(b) {
                Some(t) => Some(t),
                None => {
                    skip(&mut plan, &c.repo_path, "not_utf8_local");
                    continue;
                }
            },
        };
        if local.is_some() {
            if let Some(k) = &c.key {
                has_local.insert(k.clone());
            }
        }
        let l_sha = local.as_deref().map(|t| blob_id(t.as_bytes()));
        let r_sha = remote_tree.get(&c.repo_path).cloned();
        let b_sha = if base_ok(&c.repo_path, c.key.as_deref()) {
            base_tree.get(&c.repo_path).cloned()
        } else {
            None
        };
        if l_sha == r_sha {
            continue;
        }
        let kind = if l_sha == b_sha {
            if r_sha.is_none() {
                Kind::Deleted
            } else {
                Kind::Take
            }
        } else if r_sha == b_sha {
            Kind::Push
        } else {
            Kind::Both
        };
        pres.push(Pre {
            c,
            local_hash,
            local,
            l_sha,
            b_sha,
            r_sha,
            kind,
            held: false,
        });
    }

    // Held projects: previously held ones, plus synced projects whose memory
    // files are all gone locally while the remote still has them. Remote files
    // missing locally in a held project are never deleted by default.
    let present: HashSet<&str> = ctx.projects.iter().map(|(k, _)| k.as_str()).collect();
    let mut held: HashSet<String> = base
        .held_projects
        .iter()
        .filter(|k| present.contains(k.as_str()))
        .cloned()
        .collect();
    held.extend(
        pres.iter()
            .filter(|p| p.kind == Kind::Push && p.local.is_none())
            .filter_map(|p| p.c.key.clone())
            .filter(|k| trusted.contains(k) && !has_local.contains(k)),
    );
    for p in &mut pres {
        let in_held = p.c.key.as_ref().is_some_and(|k| held.contains(k));
        p.held = in_held && p.local.is_none() && p.r_sha.is_some();
    }

    // Pass 2: read every remote / base blob involved, in one batch, so a
    // non-UTF-8 file is never pushed over, deleted or merged.
    let mut need: Vec<String> = Vec::new();
    for p in &pres {
        need.extend(p.r_sha.clone());
        need.extend(p.b_sha.clone());
    }
    let manifest_sha = remote_tree.get(SKILLS_JSON).cloned();
    if ctx.local_skills.is_some() {
        need.extend(manifest_sha.clone());
    }
    let blobs = git.read_blobs(repo, &need)?;
    let text_of = |sha: &Option<String>| -> Result<Option<String>, ()> {
        match sha {
            None => Ok(None),
            Some(s) => blobs.get(s).and_then(|b| decode_text(b)).map(Some).ok_or(()),
        }
    };

    for p in pres {
        let rp = p.c.repo_path.clone();
        let Ok(r) = text_of(&p.r_sha) else {
            skip(&mut plan, &rp, "not_utf8_remote");
            continue;
        };
        let Ok(b) = text_of(&p.b_sha) else {
            skip(&mut plan, &rp, "not_utf8_base");
            continue;
        };
        let action = if p.held {
            plan.local_all_deleted.push(rp.clone());
            if let Some(k) = &p.c.key {
                plan.held_after.insert(k.clone());
            }
            Action::LocalAllDeleted
        } else {
            match p.kind {
                Kind::Take => {
                    plan.take_remote.push(rp.clone());
                    Action::TakeRemote
                }
                Kind::Deleted => {
                    plan.remote_deleted.push(rp.clone());
                    if let Some(t) = &p.local {
                        scan_into(&rp, t, &mut plan.secret_hits);
                    }
                    Action::RemoteDeleted
                }
                Kind::Push => {
                    plan.push_local.push(rp.clone());
                    if let Some(t) = &p.local {
                        scan_into(&rp, t, &mut plan.secret_hits);
                    }
                    Action::PushLocal
                }
                Kind::Both => {
                    let l = &p.local;
                    let m = git.merge3(
                        b.as_deref().unwrap_or(""),
                        l.as_deref().unwrap_or(""),
                        r.as_deref().unwrap_or(""),
                    )?;
                    if b.is_some() && l.is_some() && r.is_some() && m.conflicts == 0 {
                        plan.merged.push(rp.clone());
                        scan_into(&rp, &m.text, &mut plan.secret_hits);
                        Action::Merged(m.text)
                    } else {
                        plan.conflicts.push(ConflictFile {
                            repo_path: rp.clone(),
                            local_path: p.c.local_path.to_string_lossy().into_owned(),
                            local: l.clone(),
                            remote: r.clone(),
                            base: b.clone(),
                            merged: m.text,
                            conflicts: m.conflicts.max(1),
                        });
                        Action::Conflict
                    }
                }
            }
        };
        plan.entries.push(Entry {
            repo_path: rp,
            local_path: p.c.local_path,
            key: p.c.key,
            local_hash: p.local_hash,
            local: p.local,
            l_sha: p.l_sha,
            r_sha: p.r_sha,
            remote: r,
            action,
        });
    }

    // Held keys of projects not on this machine stay held.
    plan.held_after.extend(
        base.held_projects
            .iter()
            .filter(|k| !present.contains(k.as_str()))
            .cloned(),
    );
    // Skipped paths keep an untrusted base until a sync handles them; paths
    // handled normally leave the set; paths the remote no longer has drop out.
    let skipped: BTreeSet<String> = plan.skipped.iter().map(|s| s.repo_path.clone()).collect();
    plan.unsynced_after = unsynced
        .iter()
        .filter(|p| !considered.contains(*p) && remote_full.contains_key(*p))
        .cloned()
        .chain(skipped)
        .collect();

    if let Some(local) = &ctx.local_skills {
        let remote_m = match &manifest_sha {
            None => Manifest::default(),
            Some(sha) => {
                let text = blobs
                    .get(sha)
                    .and_then(|b| decode_text(b))
                    .ok_or_else(|| SyncError::Manifest("not UTF-8 text".into()))?;
                Manifest::from_json(&text).map_err(SyncError::Manifest)?
            }
        };
        let id = ctx.machine.id.as_str();
        let merged = manifest::merge(&remote_m, local, id);
        plan.missing_skills = manifest::missing_here(&merged, local, id);
        plan.removed_elsewhere = manifest::removed_elsewhere(&merged, local, id);
        let json = merged
            .to_json()
            .map_err(|e| SyncError::Manifest(e.to_string()))?;
        if Some(blob_id(json.as_bytes())) != manifest_sha {
            scan_into(SKILLS_JSON, &json, &mut plan.secret_hits);
        }
        plan.manifest_json = Some(json);
    }
    plan.remote_tree = remote_full;
    Ok(plan)
}
