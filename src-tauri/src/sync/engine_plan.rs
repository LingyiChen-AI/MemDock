//! Plan building for the sync engine: which files to compare, blob-id
//! classification, the local-all-deleted guard and the batched blob reads.

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

/// `path -> blob id` of the base commit; `None` without a usable base (no
/// previous sync, or the commit object is missing from this clone).
pub(super) fn base_tree(
    ctx: &SyncCtx,
    base: Option<&str>,
) -> Result<Option<BTreeMap<String, String>>, SyncError> {
    let Some(b) = base else { return Ok(None) };
    if !ctx.git.has_commit(ctx.repo_dir, b)? {
        return Ok(None);
    }
    Ok(Some(ctx.git.tree_entries(ctx.repo_dir, b)?))
}

/// One repo path to compare and the local file it maps to.
pub(super) struct Candidate {
    pub(super) repo_path: String,
    pub(super) local_path: PathBuf,
    pub(super) key: Option<String>,
}

/// `union(local files, base paths, remote paths)` restricted to files that map
/// to this machine. Repo files of projects absent here are not included.
/// Repo paths that differ only by case are returned separately and left out.
pub(super) fn candidates<'s>(
    ctx: &SyncCtx,
    base: impl Iterator<Item = &'s String>,
    remote: impl Iterator<Item = &'s String>,
) -> (Vec<Candidate>, Vec<String>) {
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
    let mut out = Vec::new();
    let mut collided = Vec::new();
    // normalize_path is case-insensitive on Windows only
    let mut seen_local: HashSet<String> = HashSet::new();
    for c in all {
        if by_lower[&c.repo_path.to_lowercase()] > 1 {
            collided.push(c.repo_path);
            continue;
        }
        // Two repo paths must never write the same local file.
        if seen_local.insert(normalize_path(&c.local_path.to_string_lossy())) {
            out.push(c);
        }
    }
    (out, collided)
}

// ---------------------------------------------------------------- prepare

/// Fetches and builds the plan. Writes nothing locally or to the repository.
pub fn prepare(ctx: &SyncCtx) -> Result<SyncPlan, SyncError> {
    let remote = ctx.git.fetch(ctx.repo_dir, &ctx.auth)?;
    build_plan(ctx, remote, ctx.base_commit.as_deref(), &ctx.synced_projects)
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
    all_deleted: bool,
}

pub(super) fn build_plan(
    ctx: &SyncCtx,
    remote: Option<String>,
    base: Option<&str>,
    synced: &[SyncedProject],
) -> Result<SyncPlan, SyncError> {
    let git = ctx.git;
    let repo = ctx.repo_dir;
    let remote_tree = match &remote {
        Some(r) => git.tree_entries(repo, r)?,
        None => BTreeMap::new(),
    };
    let base_tree = base_tree(ctx, base)?.unwrap_or_default();
    let trusted = trusted_keys(ctx, synced);
    let base_paths: Vec<&String> = base_tree
        .keys()
        .filter(|p| base_valid(p, project_of(p, &ctx.projects), &trusted))
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
    };
    let skip = |plan: &mut SyncPlan, path: &str, reason: &str| {
        plan.skipped.push(SkippedFile {
            repo_path: path.to_string(),
            reason: reason.to_string(),
        })
    };

    let (cands, collided) = candidates(ctx, base_paths.into_iter(), remote_tree.keys());
    for p in &collided {
        skip(&mut plan, p, "case_collision");
    }

    // Pass 1: compare blob ids.
    let mut pres: Vec<Pre> = Vec::new();
    let mut has_local: HashSet<String> = HashSet::new();
    for c in cands {
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
        let b_sha = if base_valid(&c.repo_path, c.key.as_deref(), &trusted) {
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
            all_deleted: false,
        });
    }

    // Guard: a synced project whose memory files are all gone locally is not
    // turned into a mass deletion of the remote copy.
    let gone: HashSet<String> = pres
        .iter()
        .filter(|p| p.kind == Kind::Push && p.local.is_none())
        .filter_map(|p| p.c.key.clone())
        .filter(|k| trusted.contains(k) && !has_local.contains(k))
        .collect();
    for p in &mut pres {
        if p.c.key.as_ref().is_some_and(|k| gone.contains(k)) {
            p.all_deleted = true;
        }
    }

    // Pass 2: read the blobs that are needed, in one batch.
    let mut need: Vec<String> = Vec::new();
    for p in &pres {
        match (p.kind, p.all_deleted) {
            (_, true) | (Kind::Take, _) => need.extend(p.r_sha.clone()),
            (Kind::Both, _) => {
                need.extend(p.r_sha.clone());
                need.extend(p.b_sha.clone());
            }
            _ => {}
        }
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
        let mut remote_text: Option<String> = None;
        let action = if p.all_deleted {
            let Ok(r) = text_of(&p.r_sha) else {
                skip(&mut plan, &rp, "not_utf8_remote");
                continue;
            };
            remote_text = r;
            plan.local_all_deleted.push(rp.clone());
            Action::LocalAllDeleted
        } else {
            match p.kind {
                Kind::Take => {
                    let Ok(r) = text_of(&p.r_sha) else {
                        skip(&mut plan, &rp, "not_utf8_remote");
                        continue;
                    };
                    remote_text = r;
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
                    let Ok(r) = text_of(&p.r_sha) else {
                        skip(&mut plan, &rp, "not_utf8_remote");
                        continue;
                    };
                    let Ok(b) = text_of(&p.b_sha) else {
                        skip(&mut plan, &rp, "not_utf8_base");
                        continue;
                    };
                    let l = &p.local;
                    let m = git.merge3(
                        b.as_deref().unwrap_or(""),
                        l.as_deref().unwrap_or(""),
                        r.as_deref().unwrap_or(""),
                    )?;
                    remote_text = r.clone();
                    if b.is_some() && l.is_some() && r.is_some() && m.conflicts == 0 {
                        plan.merged.push(rp.clone());
                        scan_into(&rp, &m.text, &mut plan.secret_hits);
                        Action::Merged(m.text)
                    } else {
                        plan.conflicts.push(ConflictFile {
                            repo_path: rp.clone(),
                            local_path: p.c.local_path.to_string_lossy().into_owned(),
                            local: l.clone(),
                            remote: r,
                            base: b,
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
            local_hash: p.local_hash,
            local: p.local,
            l_sha: p.l_sha,
            r_sha: p.r_sha,
            remote: remote_text,
            action,
        });
    }

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
    plan.remote_tree = remote_tree;
    Ok(plan)
}
