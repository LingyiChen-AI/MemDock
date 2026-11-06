import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  BackupInfo,
  DeletedSkill,
  InspectResult,
  InstallFrom,
  InstallTarget,
  ManifestCompare,
  ManifestSkill,
  SkillManifest,
  SkillEntry,
  SkillScope,
  SkillFileDto,
  SkillFileList,
  SkillsChangedPayload,
  UpdateStatus,
} from "./types";

// Every function rejects with a `CommandError` ({ code, message, dir?, current_hash? }).

export function listSkills(): Promise<SkillEntry[]> {
  return invoke<SkillEntry[]>("list_skills");
}

/** This machine's skill install manifest. */
export function skillManifestLocal(): Promise<SkillManifest> {
  return invoke<SkillManifest>("skill_manifest_local");
}

/** Compares this machine with `remote`, or with the synced `skills.json` when `remote` is null. */
export function skillManifestCompare(remote: SkillManifest | null): Promise<ManifestCompare> {
  return invoke<ManifestCompare>("skill_manifest_compare", { remote });
}

/** Writes this machine's manifest to `path`; resolves with the number of skills. */
export function skillManifestExport(path: string): Promise<number> {
  return invoke<number>("skill_manifest_export", { path });
}

/** Reads and validates a manifest file (installs nothing). */
export function skillManifestImport(path: string): Promise<SkillManifest> {
  return invoke<SkillManifest>("skill_manifest_import", { path });
}

/** Installs one manifest entry from its git source into `scope`. */
export function installFromManifest(m: ManifestSkill, scope: SkillScope): Promise<SkillEntry[]> {
  return installSkill(
    { type: "git", url: m.url, clone_url: m.url, git_ref: m.git_ref, skill_path: m.skill_path, commit: m.commit },
    [{ tool: m.tool, scope }],
    false,
  );
}

export function gitAvailable(): Promise<boolean> {
  return invoke<boolean>("git_available");
}

/** Shallow-clones (or reuses the cache) and lists the skills inside the repository. */
export function inspectGitSource(input: string): Promise<InspectResult> {
  return invoke<InspectResult>("inspect_git_source", { input });
}

export function installSkill(
  from: InstallFrom,
  targets: InstallTarget[],
  overwrite: boolean,
): Promise<SkillEntry[]> {
  return invoke<SkillEntry[]>("install_skill", { from, targets, overwrite });
}

export function checkSkillUpdates(): Promise<UpdateStatus[]> {
  return invoke<UpdateStatus[]>("check_skill_updates");
}

export function updateSkill(skillId: string, acceptLocalChanges: boolean): Promise<SkillEntry> {
  return invoke<SkillEntry>("update_skill", { skillId, acceptLocalChanges });
}

export function rollbackSkill(skillId: string, backupId: string): Promise<SkillEntry> {
  return invoke<SkillEntry>("rollback_skill", { skillId, backupId });
}

export function listSkillBackups(skillId: string): Promise<BackupInfo[]> {
  return invoke<BackupInfo[]>("list_skill_backups", { skillId });
}

/** Resolves with the backup taken first; `id === ""` means no backup was made. */
export function deleteSkill(skillId: string): Promise<BackupInfo> {
  return invoke<BackupInfo>("delete_skill", { skillId });
}

/** Skills whose folder is gone but whose backups remain. */
export function listDeletedSkillBackups(): Promise<DeletedSkill[]> {
  return invoke<DeletedSkill[]>("list_deleted_skill_backups");
}

/** Restores a deleted skill to its original folder; resolves with that folder. */
export function restoreDeletedSkill(bucket: string, backupId: string): Promise<string> {
  return invoke<string>("restore_deleted_skill", { bucket, backupId });
}

/** Absolute path of a backup folder (there is no opener plugin; the UI shows it with a copy button). */
export function revealBackupFolder(bucket: string): Promise<string> {
  return invoke<string>("reveal_backup_folder", { bucket });
}

export function syncSkill(
  skillId: string,
  target: InstallTarget,
  overwrite: boolean,
): Promise<SkillEntry> {
  return invoke<SkillEntry>("sync_skill", { skillId, target, overwrite });
}

export function registerSkillSource(
  skillId: string,
  url: string,
  skillPath: string,
): Promise<SkillEntry> {
  return invoke<SkillEntry>("register_skill_source", { skillId, url, skillPath });
}

export function listSkillFiles(skillId: string): Promise<SkillFileList> {
  return invoke<SkillFileList>("list_skill_files", { skillId });
}

export function readSkillFile(skillId: string, relPath: string): Promise<SkillFileDto> {
  return invoke<SkillFileDto>("read_skill_file", { skillId, relPath });
}

/** Reads a text file of a backup (default SKILL.md). */
export function readSkillBackupFile(skillId: string, backupId: string, relPath?: string): Promise<SkillFileDto> {
  return invoke<SkillFileDto>("read_skill_backup_file", { skillId, backupId, relPath: relPath ?? null });
}

/** Rejects with code `disk_changed` (carrying `current_hash`) on conflict, `read_only` for read-only skills. */
export function writeSkillFile(
  skillId: string,
  relPath: string,
  text: string,
  expectedHash: string,
): Promise<SkillFileDto> {
  return invoke<SkillFileDto>("write_skill_file", { skillId, relPath, text, expectedHash });
}

export function onSkillsChanged(
  cb: (payload: SkillsChangedPayload) => void,
): Promise<UnlistenFn> {
  return listen<SkillsChangedPayload>("skills-changed", (e) => cb(e.payload));
}
