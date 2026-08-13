//! Handoff bundle export and import.

use crate::db::DashboardDb;
use crate::lan::handoff::HandoffKind;
use crate::observability::session_transcript::session_transcript;
use crate::report::{session_report, ReportOptions};
use crate::schema::{CreateSessionRequest, ProjectDetail, SessionDetail, UpsertProjectRequest};
use anyhow::{bail, Context, Result};
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tar::Builder;
use walkdir::WalkDir;

const MANIFEST_NAME: &str = "manifest.json";
/// Export schema. v2 adds skills + MCP server configs to the bundle; v1
/// bundles remain importable (the new manifest fields serde-default empty).
const SCHEMA_VERSION: &str = "handoff_v2";
const MCP_SERVERS_NAME: &str = "mcp/servers.json";
/// Upper bound on skills bundled per handoff (each counts toward max_bytes).
const MAX_BUNDLE_SKILLS: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleManifest {
    pub schema_version: String,
    pub kind: HandoffKind,
    pub generated_at: String,
    pub source_instance_id: String,
    pub source_device_name: String,
    pub project: BundleProject,
    pub sessions: Vec<BundleSession>,
    #[serde(default)]
    pub memory_files: Vec<String>,
    #[serde(default)]
    pub artifact_paths: Vec<String>,
    /// Metadata only — skill trees ride the tarball under `skills/<id>/`.
    #[serde(default)]
    pub skills: Vec<BundleSkill>,
    /// Metadata only — redacted server entries ride under `mcp/servers.json`.
    /// Kept secret-name-free so the manifest redaction guard still holds.
    #[serde(default)]
    pub mcp_servers: Vec<BundleMcpServerMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleSkill {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    /// Recipient-side hint: the skill ships executable helper scripts and
    /// deserves an explicit look before first use. Import never executes.
    #[serde(default)]
    pub has_run_script: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleMcpServerMeta {
    pub slug: String,
    #[serde(default)]
    pub transport: String,
    #[serde(default)]
    pub secrets_redacted: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleProject {
    pub id: String,
    pub name: String,
    pub root_path: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleSession {
    pub detail: SessionDetail,
    pub transcript_json: String,
    #[serde(default)]
    pub report_json: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BundleExportOptions {
    pub kind: HandoffKind,
    pub project_id: String,
    pub session_id: Option<String>,
    pub source_instance_id: String,
    pub source_device_name: String,
    pub max_bytes: u64,
    /// Bundle the project's enabled skills (trees under `skills/<id>/`).
    pub include_skills: bool,
    /// Bundle MCP server configs from config.json with secrets blanked.
    pub include_mcp: bool,
}

pub async fn export_bundle(
    db: &DashboardDb,
    memory_root: &Path,
    opts: BundleExportOptions,
) -> Result<PathBuf> {
    let project = db
        .get_project(&opts.project_id)
        .await?
        .context("project not found")?;
    let root = PathBuf::from(&project.root_path);
    if !root.is_dir() {
        bail!("project root missing: {}", project.root_path);
    }

    let sessions = match opts.kind {
        HandoffKind::Project => {
            let summaries = db.list_sessions(&opts.project_id, 200).await?;
            let mut out = Vec::new();
            for s in summaries {
                if let Some(d) = db.get_session(&s.id).await? {
                    out.push(d);
                }
            }
            out
        }
        HandoffKind::Session => {
            let sid = opts
                .session_id
                .as_deref()
                .context("session_id required for session handoff")?;
            vec![db.get_session(sid).await?.context("session not found")?]
        }
    };

    let mut bundle_sessions = Vec::new();
    for detail in &sessions {
        let transcript = session_transcript(db, &detail.id).await?;
        let transcript_json = serde_json::to_string_pretty(&transcript)?;
        let report = session_report(db, &detail.id, ReportOptions::default(), false)
            .await
            .ok();
        let report_json = report
            .map(|r| serde_json::to_string_pretty(&r))
            .transpose()?;
        bundle_sessions.push(BundleSession {
            detail: detail.clone(),
            transcript_json,
            report_json,
        });
    }

    let memory_files = collect_memory_files(memory_root, &project)?;
    let artifacts = db
        .list_artifacts(
            Some(&opts.project_id),
            opts.session_id.as_deref(),
            None,
            None,
            None,
            false,
            false,
            false,
            500,
        )
        .await?;
    let artifact_paths: Vec<String> = artifacts
        .into_iter()
        .map(|a| a.path)
        .filter(|p| !p.is_empty())
        .collect();

    let skill_dirs = if opts.include_skills {
        collect_project_skills(db, &opts.project_id).await
    } else {
        Vec::new()
    };
    let bundle_skills: Vec<BundleSkill> = skill_dirs
        .iter()
        .map(|(skill, dir)| {
            let version = anycode_tools::parse_skill_manifest_file(&dir.join("SKILL.md"))
                .and_then(|m| m.version)
                .unwrap_or_default();
            BundleSkill {
                id: skill.id.clone(),
                name: skill.name.clone(),
                version,
                has_run_script: skill_dir_has_run_script(dir),
            }
        })
        .collect();

    let (mcp_meta, mcp_servers_redacted) = if opts.include_mcp {
        collect_mcp_servers()
    } else {
        (Vec::new(), Vec::new())
    };

    let manifest = BundleManifest {
        schema_version: SCHEMA_VERSION.into(),
        kind: opts.kind,
        generated_at: chrono::Utc::now().to_rfc3339(),
        source_instance_id: opts.source_instance_id,
        source_device_name: opts.source_device_name,
        project: BundleProject {
            id: project.id.clone(),
            name: project.name.clone(),
            root_path: project.root_path.clone(),
            description: project.description.clone(),
        },
        sessions: bundle_sessions,
        memory_files: memory_files
            .iter()
            .map(|p| p.display().to_string())
            .collect(),
        artifact_paths: artifact_paths.clone(),
        skills: bundle_skills,
        mcp_servers: mcp_meta,
    };

    redact_secrets_in_manifest(&manifest)?;

    let staging = std::env::temp_dir().join(format!(
        "anycode-handoff-{}.tar.gz",
        uuid::Uuid::new_v4().simple()
    ));
    write_tarball(TarballPayload {
        dest: &staging,
        manifest: &manifest,
        workspace_root: &root,
        include_workspace: opts.kind == HandoffKind::Project,
        memory_files: &memory_files,
        artifact_paths: &artifact_paths,
        project_root: &root,
        skill_dirs: &skill_dirs,
        mcp_servers_redacted: &mcp_servers_redacted,
        max_bytes: opts.max_bytes,
    })?;
    Ok(staging)
}

/// Skills enabled for the project, resolved to on-disk directories. Scan
/// failures and missing dirs degrade to "not bundled" — a stale DB row must
/// never sink the whole handoff.
async fn collect_project_skills(
    db: &DashboardDb,
    project_id: &str,
) -> Vec<(crate::schema::SkillRecord, PathBuf)> {
    let Ok(records) = db.list_skills_for_project(project_id).await else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for record in records {
        if record.enabled != Some(true) {
            continue;
        }
        let dir = PathBuf::from(&record.source_path);
        if !dir.join("SKILL.md").is_file() {
            tracing::debug!(skill = %record.id, "skill source dir missing; skipped from bundle");
            continue;
        }
        out.push((record, dir));
        if out.len() >= MAX_BUNDLE_SKILLS {
            tracing::warn!(
                "bundle skill cap {MAX_BUNDLE_SKILLS} reached; remaining skills dropped"
            );
            break;
        }
    }
    out
}

/// A skill "run script" is any executable-ish helper shipped next to
/// SKILL.md; the wizard and import review flag these for manual inspection.
fn skill_dir_has_run_script(source_path: &Path) -> bool {
    const SCRIPT_EXTS: &[&str] = &["sh", "bash", "py", "js", "mjs", "ts", "rb", "pl"];
    WalkDir::new(source_path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .any(|e| {
            e.path()
                .extension()
                .and_then(|s| s.to_str())
                .map(|ext| SCRIPT_EXTS.contains(&ext.to_ascii_lowercase().as_str()))
                .unwrap_or(false)
        })
}

/// MCP servers from config.json, redacted for transport (secret values
/// blanked — the recipient re-enters them). Returns manifest metadata +
/// the importable redacted entries for `mcp/servers.json`.
fn collect_mcp_servers() -> (Vec<BundleMcpServerMeta>, Vec<serde_json::Value>) {
    let Ok((_, cfg)) = crate::config_patch::read_config_root() else {
        return (Vec::new(), Vec::new());
    };
    let servers = crate::mcp_config::read_mcp_servers(&cfg);
    if servers.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let redacted = crate::mcp_config::handoff_redact_mcp_servers(&servers);
    let meta = servers
        .iter()
        .zip(redacted.iter())
        .map(|(original, red)| BundleMcpServerMeta {
            slug: mcp_server_slug(original),
            transport: mcp_server_transport(original),
            secrets_redacted: crate::mcp_config::count_redacted_secrets(original, red),
        })
        .collect();
    (meta, redacted)
}

fn mcp_server_slug(server: &serde_json::Value) -> String {
    for key in ["slug", "name", "id"] {
        if let Some(s) = server.get(key).and_then(|v| v.as_str()) {
            if !s.trim().is_empty() {
                return s.trim().to_string();
            }
        }
    }
    "unnamed".into()
}

fn mcp_server_transport(server: &serde_json::Value) -> String {
    if let Some(t) = server.get("type").and_then(|v| v.as_str()) {
        return t.to_string();
    }
    if server.get("command").is_some() {
        "stdio".into()
    } else if server.get("url").is_some() {
        "http".into()
    } else {
        "unknown".into()
    }
}

fn redact_secrets_in_manifest(manifest: &BundleManifest) -> Result<()> {
    let json = serde_json::to_string(manifest)?;
    let forbidden = ["api_key", "credentials", "secret", "password", "token_hash"];
    let lower = json.to_lowercase();
    for word in forbidden {
        if lower.contains(word) {
            bail!("bundle manifest contains forbidden secret marker: {word}");
        }
    }
    Ok(())
}

fn collect_memory_files(memory_root: &Path, project: &ProjectDetail) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for sub in ["project", "feedback", "user", "reference"] {
        let dir = memory_root.join(sub);
        if !dir.is_dir() {
            continue;
        }
        for entry in WalkDir::new(&dir).into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            if let Ok(text) = fs::read_to_string(path) {
                if text.contains(&project.id) || text.contains(&project.root_path) {
                    out.push(path.to_path_buf());
                }
            }
        }
    }
    Ok(out)
}

struct TarballPayload<'a> {
    dest: &'a Path,
    manifest: &'a BundleManifest,
    workspace_root: &'a Path,
    include_workspace: bool,
    memory_files: &'a [PathBuf],
    artifact_paths: &'a [String],
    project_root: &'a Path,
    skill_dirs: &'a [(crate::schema::SkillRecord, PathBuf)],
    mcp_servers_redacted: &'a [serde_json::Value],
    max_bytes: u64,
}

fn write_tarball(payload: TarballPayload<'_>) -> Result<()> {
    let TarballPayload {
        dest,
        manifest,
        workspace_root,
        include_workspace,
        memory_files,
        artifact_paths,
        project_root,
        skill_dirs,
        mcp_servers_redacted,
        max_bytes,
    } = payload;
    let file = File::create(dest).context("create bundle file")?;
    let enc = GzEncoder::new(file, Compression::default());
    let mut tar = Builder::new(enc);

    let manifest_bytes = serde_json::to_vec_pretty(manifest)?;
    append_bytes_to_tar(&mut tar, MANIFEST_NAME, &manifest_bytes)?;

    if include_workspace {
        add_dir_to_tar(
            &mut tar,
            workspace_root,
            "workspace",
            project_root,
            max_bytes,
        )?;
    }

    for mem in memory_files {
        if let Ok(rel) = mem.strip_prefix(dirs::home_dir().unwrap_or_default().join(".anycode")) {
            let name = format!("memories/{}", rel.display());
            tar.append_path_with_name(mem, &name)?;
        }
    }

    for rel in artifact_paths {
        let src = project_root.join(rel);
        if src.is_file() {
            let name = format!("artifacts/{}", rel);
            tar.append_path_with_name(&src, &name)?;
        }
    }

    for (skill, dir) in skill_dirs {
        add_dir_to_tar(
            &mut tar,
            dir,
            &format!("skills/{}", skill.id),
            dir,
            max_bytes,
        )?;
    }

    if !mcp_servers_redacted.is_empty() {
        let bytes = serde_json::to_vec_pretty(mcp_servers_redacted)?;
        append_bytes_to_tar(&mut tar, MCP_SERVERS_NAME, &bytes)?;
    }

    for session in &manifest.sessions {
        let tpath = format!("transcripts/{}.json", session.detail.id);
        append_bytes_to_tar(&mut tar, &tpath, session.transcript_json.as_bytes())?;
        if let Some(report) = &session.report_json {
            let rpath = format!("reports/{}.json", session.detail.id);
            append_bytes_to_tar(&mut tar, &rpath, report.as_bytes())?;
        }
    }

    tar.finish()?;
    let enc = tar.into_inner()?;
    let mut file = enc.finish()?;
    file.flush()?;

    let size = fs::metadata(dest)?.len();
    if size > max_bytes {
        let _ = fs::remove_file(dest);
        bail!("bundle size {} exceeds limit {} bytes", size, max_bytes);
    }
    Ok(())
}

/// `tar::Builder::append_data` does NOT set the header size field — without
/// it the entry size reads as garbage and the archive fails to unpack. Set
/// size + sane metadata explicitly before every in-memory append.
fn append_bytes_to_tar<W: Write>(tar: &mut Builder<W>, name: &str, bytes: &[u8]) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar.append_data(&mut header, name, bytes)?;
    Ok(())
}

fn add_dir_to_tar<W: Write>(
    tar: &mut Builder<W>,
    dir: &Path,
    prefix: &str,
    project_root: &Path,
    max_bytes: u64,
) -> Result<()> {
    let mut total: u64 = 0;
    for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            continue;
        }
        let rel = path.strip_prefix(project_root).unwrap_or(path);
        let rel_str = rel.display().to_string();
        if should_skip_path(&rel_str) {
            continue;
        }
        let meta = fs::metadata(path)?;
        total += meta.len();
        if total > max_bytes {
            bail!("workspace exceeds bundle size limit");
        }
        let name = format!("{prefix}/{}", rel.display());
        tar.append_path_with_name(path, &name)?;
    }
    Ok(())
}

fn should_skip_path(rel: &str) -> bool {
    const SKIP: &[&str] = &[
        "node_modules/",
        ".git/",
        "target/",
        "dist/",
        "build/",
        ".anycode/credentials",
    ];
    SKIP.iter().any(|s| rel.contains(s))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportOptions {
    pub kind: HandoffKind,
    pub target_root_path: Option<String>,
    pub target_project_id: Option<String>,
}

pub struct ImportResult {
    pub project_id: String,
    pub root_path: String,
    pub sessions_imported: usize,
    pub skills_installed: Vec<String>,
    /// (skill id, reason) — vet rejections and unreadable dirs land here;
    /// a bad skill must never sink the rest of the handoff.
    pub skills_skipped: Vec<(String, String)>,
    pub mcp_imported: usize,
    /// Conflicting server slugs already present in config.json.
    pub mcp_skipped: Vec<String>,
}

pub async fn import_bundle(
    db: &DashboardDb,
    memory_root: &Path,
    bundle_path: &Path,
    opts: ImportOptions,
) -> Result<ImportResult> {
    let skills_root = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".anycode")
        .join("skills");
    import_bundle_with_roots(db, memory_root, bundle_path, opts, &skills_root, None).await
}

/// Import with explicit destination roots — tests point these at tempdirs so
/// the real `~/.anycode/skills` and config.json stay untouched.
async fn import_bundle_with_roots(
    db: &DashboardDb,
    memory_root: &Path,
    bundle_path: &Path,
    opts: ImportOptions,
    skills_root: &Path,
    config_path: Option<&Path>,
) -> Result<ImportResult> {
    let staging = extract_tarball(bundle_path)?;
    let manifest_path = staging.join(MANIFEST_NAME);
    let manifest: BundleManifest =
        serde_json::from_slice(&fs::read(manifest_path).context("read manifest")?)?;

    let root_path = resolve_import_root(db, &manifest, &opts).await?;
    if opts.kind == HandoffKind::Project {
        let ws_src = staging.join("workspace");
        if ws_src.is_dir() {
            copy_tree_merge(&ws_src, Path::new(&root_path))?;
        }
    }

    let project = db
        .upsert_project(UpsertProjectRequest {
            root_path: root_path.clone(),
            name: Some(manifest.project.name.clone()),
            description: Some(manifest.project.description.clone()),
            create_root: Some(true),
            template_id: None,
            app_title: None,
            bundle_org: None,
        })
        .await?;
    let new_project_id = project.id.clone();

    let mut imported = 0usize;
    for session in &manifest.sessions {
        let _new_id = import_session(db, &new_project_id, session).await?;
        imported += 1;
    }

    let mem_src = staging.join("memories");
    if mem_src.is_dir() {
        copy_tree_merge(&mem_src, memory_root)?;
    }

    let art_src = staging.join("artifacts");
    if art_src.is_dir() {
        copy_tree_merge(&art_src, Path::new(&root_path))?;
    }

    // handoff_v2 payloads; v1 manifests serde-default these to empty, so the
    // import loops below are no-ops for legacy bundles.
    let (skills_installed, skills_skipped) = import_bundle_skills(&staging, &manifest, skills_root);
    let (mcp_imported, mcp_skipped) = import_bundle_mcp_servers(&staging, config_path);

    Ok(ImportResult {
        project_id: new_project_id,
        root_path,
        sessions_imported: imported,
        skills_installed,
        skills_skipped,
        mcp_imported,
        mcp_skipped,
    })
}

/// Install bundled skill trees into `~/.anycode/skills/<id>/` through the
/// standard install path — vet scans every dir and critical findings reject
/// the skill. Imported scripts are inert data until a user deliberately
/// invokes the skill; nothing here executes.
fn import_bundle_skills(
    staging: &Path,
    manifest: &BundleManifest,
    skills_root: &Path,
) -> (Vec<String>, Vec<(String, String)>) {
    let mut installed = Vec::new();
    let mut skipped = Vec::new();
    if manifest.skills.is_empty() {
        return (installed, skipped);
    }
    for skill in &manifest.skills {
        if !anycode_tools::SkillCatalog::is_valid_skill_id(&skill.id) {
            skipped.push((skill.id.clone(), "invalid skill id".into()));
            continue;
        }
        let src = staging.join("skills").join(&skill.id);
        if !src.join("SKILL.md").is_file() {
            skipped.push((skill.id.clone(), "skill tree missing from bundle".into()));
            continue;
        }
        match anycode_tools::install_skill(&src.display().to_string(), skills_root) {
            Ok(result) => {
                if skill.has_run_script {
                    tracing::warn!(
                        skill = %skill.id,
                        "imported skill ships run scripts; review before first use"
                    );
                }
                installed.push(result.id);
            }
            Err(e) => {
                tracing::warn!(skill = %skill.id, error = %e, "skill import rejected");
                skipped.push((skill.id.clone(), e.to_string()));
            }
        }
    }
    (installed, skipped)
}

/// Merge bundled MCP servers into config.json. Conflicts (same slug) are
/// skipped — never overwrite a local server the user already configured.
fn import_bundle_mcp_servers(staging: &Path, config_path: Option<&Path>) -> (usize, Vec<String>) {
    let path = staging.join(MCP_SERVERS_NAME);
    let Ok(bytes) = fs::read(&path) else {
        return (0, Vec::new());
    };
    let incoming: Vec<serde_json::Value> = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "bundle mcp/servers.json unreadable; skipped");
            return (0, Vec::new());
        }
    };
    if incoming.is_empty() {
        return (0, Vec::new());
    }
    let Ok((cfg_path, mut cfg)) = crate::config_patch::read_config_value(config_path) else {
        return (0, Vec::new());
    };
    let mut existing = crate::mcp_config::read_mcp_servers(&cfg);
    let existing_slugs: std::collections::HashSet<String> =
        existing.iter().map(mcp_server_slug).collect();
    let mut imported = 0usize;
    let mut skipped = Vec::new();
    for server in incoming {
        let slug = mcp_server_slug(&server);
        if existing_slugs.contains(&slug) {
            skipped.push(slug);
            continue;
        }
        existing.push(server);
        imported += 1;
    }
    if imported > 0 {
        crate::mcp_config::set_mcp_servers(&mut cfg, existing);
        if let Err(e) = crate::config_patch::write_config_value(&cfg_path, &cfg) {
            tracing::warn!(error = %e, "failed to persist imported MCP servers");
            return (0, skipped);
        }
    }
    (imported, skipped)
}

async fn resolve_import_root(
    db: &DashboardDb,
    manifest: &BundleManifest,
    opts: &ImportOptions,
) -> Result<String> {
    if let Some(path) = opts
        .target_root_path
        .as_ref()
        .filter(|p| !p.trim().is_empty())
    {
        return Ok(path.clone());
    }
    if let Some(pid) = opts
        .target_project_id
        .as_ref()
        .filter(|p| !p.trim().is_empty())
    {
        let project = db
            .get_project(pid)
            .await?
            .context("target project not found")?;
        return Ok(project.root_path);
    }
    if opts.kind == HandoffKind::Project {
        let base = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("anycode-handoffs")
            .join(sanitize_dir_name(&manifest.project.name));
        fs::create_dir_all(&base)?;
        return Ok(base.display().to_string());
    }
    bail!("target_root_path or target_project_id required for session handoff");
}

async fn import_session(
    db: &DashboardDb,
    project_id: &str,
    session: &BundleSession,
) -> Result<String> {
    let req = CreateSessionRequest {
        project_id: project_id.into(),
        kind: session.detail.kind.clone(),
        task_id: None,
        title: session.detail.title.clone(),
        prompt_preview: Some(session.detail.prompt_preview.clone()),
        agent_type: Some(session.detail.agent_type.clone()),
        model: Some(session.detail.model.clone()),
        metadata_json: Some(session.detail.metadata_json.clone()),
    };
    let created = db.create_session(req).await?;
    if !session.detail.summary.is_empty() {
        let _ = db
            .finish_session(&created.id, "completed", Some(&session.detail.summary))
            .await;
    }
    Ok(created.id)
}

fn extract_tarball(path: &Path) -> Result<PathBuf> {
    let staging = std::env::temp_dir().join(format!(
        "anycode-handoff-import-{}",
        uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir_all(&staging)?;
    let file = File::open(path)?;
    let dec = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(dec);
    archive.unpack(&staging)?;
    Ok(staging)
}

fn copy_tree_merge(src: &Path, dst: &Path) -> Result<()> {
    for entry in WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            continue;
        }
        let rel = path.strip_prefix(src)?;
        let target = dst.join(rel);
        if target.exists() {
            let stem = target
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("file");
            let ext = target.extension().and_then(|s| s.to_str()).unwrap_or("");
            let parent = target.parent().unwrap_or(dst);
            let alt = if ext.is_empty() {
                parent.join(format!("{stem}-imported"))
            } else {
                parent.join(format!("{stem}-imported.{ext}"))
            };
            if let Some(parent) = alt.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(path, &alt)?;
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(path, &target)?;
        }
    }
    Ok(())
}

fn sanitize_dir_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Schema marker for standalone skill packages (`skill_package_v1`).
pub const SKILL_PACKAGE_SCHEMA: &str = "skill_package_v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillPackageManifest {
    pub schema_version: String,
    pub generated_at: String,
    pub skill: BundleSkill,
}

/// Package a single skill directory as a `.tar.gz` (manifest + `skill/`
/// tree) reusing the handoff bundle primitives — the "封装" half of wish 4.
/// The archive unpacks into the same `skills/<id>/` layout the handoff
/// importer installs, so a package doubles as a one-skill handoff payload.
pub fn package_skill_dir(skill_dir: &Path, dest_dir: &Path, max_bytes: u64) -> Result<PathBuf> {
    let skill_md = skill_dir.join("SKILL.md");
    if !skill_md.is_file() {
        bail!("skill dir missing SKILL.md: {}", skill_dir.display());
    }
    let manifest_parsed = anycode_tools::parse_skill_manifest_file(&skill_md)
        .ok_or_else(|| anyhow::anyhow!("parse SKILL.md frontmatter failed"))?;
    let id = skill_dir
        .file_name()
        .and_then(|s| s.to_str())
        .context("skill dir name")?
        .to_string();
    if !anycode_tools::SkillCatalog::is_valid_skill_id(&id) || manifest_parsed.name != id {
        bail!("skill id/frontmatter name mismatch: {id}");
    }
    let package_manifest = SkillPackageManifest {
        schema_version: SKILL_PACKAGE_SCHEMA.into(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        skill: BundleSkill {
            id: id.clone(),
            name: manifest_parsed.name.clone(),
            version: manifest_parsed.version.clone().unwrap_or_default(),
            has_run_script: skill_dir_has_run_script(skill_dir),
        },
    };
    fs::create_dir_all(dest_dir)?;
    let dest = dest_dir.join(format!("{id}.skill.tar.gz"));
    let file = File::create(&dest).context("create skill package")?;
    let enc = GzEncoder::new(file, Compression::default());
    let mut tar = Builder::new(enc);
    let manifest_bytes = serde_json::to_vec_pretty(&package_manifest)?;
    append_bytes_to_tar(&mut tar, MANIFEST_NAME, &manifest_bytes)?;
    add_dir_to_tar(
        &mut tar,
        skill_dir,
        &format!("skills/{id}"),
        skill_dir,
        max_bytes,
    )?;
    tar.finish()?;
    let enc = tar.into_inner()?;
    let mut file = enc.finish()?;
    file.flush()?;
    let size = fs::metadata(&dest)?.len();
    if size > max_bytes {
        let _ = fs::remove_file(&dest);
        bail!("skill package size {size} exceeds limit {max_bytes} bytes");
    }
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_root::project_id_for_root;

    fn test_manifest(description: &str) -> BundleManifest {
        BundleManifest {
            schema_version: SCHEMA_VERSION.into(),
            kind: HandoffKind::Project,
            generated_at: chrono::Utc::now().to_rfc3339(),
            source_instance_id: "x".into(),
            source_device_name: "x".into(),
            project: BundleProject {
                id: "p".into(),
                name: "n".into(),
                root_path: "/tmp".into(),
                description: description.into(),
            },
            sessions: vec![],
            memory_files: vec![],
            artifact_paths: vec![],
            skills: vec![],
            mcp_servers: vec![],
        }
    }

    #[test]
    fn manifest_redaction_blocks_api_key() {
        let manifest = test_manifest("d with api_key leak");
        assert!(redact_secrets_in_manifest(&manifest).is_err());
    }

    #[test]
    fn v1_manifest_without_v2_fields_still_parses() {
        // A handoff_v1 manifest has no skills/mcp_servers keys at all.
        let v1 = r#"{
            "schema_version": "handoff_v1",
            "kind": "project",
            "generated_at": "2026-01-01T00:00:00Z",
            "source_instance_id": "a",
            "source_device_name": "b",
            "project": { "id": "p", "name": "n", "root_path": "/tmp", "description": "" },
            "sessions": [],
            "memory_files": [],
            "artifact_paths": []
        }"#;
        let manifest: BundleManifest = serde_json::from_str(v1).unwrap();
        assert_eq!(manifest.schema_version, "handoff_v1");
        assert!(manifest.skills.is_empty());
        assert!(manifest.mcp_servers.is_empty());
    }

    #[test]
    fn mcp_meta_detects_slug_and_transport() {
        let stdio = serde_json::json!({ "name": "fs", "command": "npx", "args": [] });
        assert_eq!(mcp_server_slug(&stdio), "fs");
        assert_eq!(mcp_server_transport(&stdio), "stdio");
        let http = serde_json::json!({ "slug": "remote", "url": "https://x/sse" });
        assert_eq!(mcp_server_slug(&http), "remote");
        assert_eq!(mcp_server_transport(&http), "http");
        let typed = serde_json::json!({ "type": "sse", "url": "https://x" });
        assert_eq!(mcp_server_transport(&typed), "sse");
        let bare = serde_json::json!({});
        assert_eq!(mcp_server_slug(&bare), "unnamed");
        assert_eq!(mcp_server_transport(&bare), "unknown");
    }

    #[test]
    fn run_script_detection_flags_executable_helpers() {
        let tmp = tempfile::tempdir().unwrap();
        let plain = tmp.path().join("plain");
        fs::create_dir_all(&plain).unwrap();
        fs::write(plain.join("SKILL.md"), "---\nname: plain\n---\n").unwrap();
        assert!(!skill_dir_has_run_script(&plain));
        let scripted = tmp.path().join("scripted");
        fs::create_dir_all(scripted.join("scripts")).unwrap();
        fs::write(scripted.join("scripts/run.py"), "print('hi')").unwrap();
        assert!(skill_dir_has_run_script(&scripted));
    }

    #[test]
    fn package_skill_dir_produces_installable_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let skill = tmp.path().join("demo-skill");
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            "---\nname: demo-skill\ndescription: demo\nversion: 1.2.3\n---\nbody\n",
        )
        .unwrap();
        fs::write(skill.join("template.txt"), "asset").unwrap();
        let out = tmp.path().join("out");
        let pkg = package_skill_dir(&skill, &out, 10 * 1024 * 1024).unwrap();
        assert!(pkg.is_file());

        let staging = extract_tarball(&pkg).unwrap();
        let manifest: SkillPackageManifest =
            serde_json::from_slice(&fs::read(staging.join(MANIFEST_NAME)).unwrap()).unwrap();
        assert_eq!(manifest.schema_version, SKILL_PACKAGE_SCHEMA);
        assert_eq!(manifest.skill.id, "demo-skill");
        assert_eq!(manifest.skill.version, "1.2.3");
        assert!(!manifest.skill.has_run_script);
        assert!(staging.join("skills/demo-skill/SKILL.md").is_file());
        assert!(staging.join("skills/demo-skill/template.txt").is_file());
    }

    #[test]
    fn package_skill_dir_rejects_missing_skill_md() {
        let tmp = tempfile::tempdir().unwrap();
        let not_a_skill = tmp.path().join("nope");
        fs::create_dir_all(&not_a_skill).unwrap();
        assert!(package_skill_dir(&not_a_skill, tmp.path(), 1024).is_err());
    }

    #[test]
    fn project_id_remapped_on_new_root() {
        let a = project_id_for_root("/tmp/project-a");
        let b = project_id_for_root("/tmp/project-b");
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn v2_export_import_roundtrip_installs_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let db = crate::db::DashboardDb::open(tmp.path().join("t.db"))
            .await
            .unwrap();
        let project_root = tmp.path().join("proj");
        fs::create_dir_all(&project_root).unwrap();
        let project = db
            .upsert_project(crate::schema::UpsertProjectRequest {
                root_path: project_root.display().to_string(),
                name: Some("Demo".into()),
                description: Some("".into()),
                create_root: Some(true),
                template_id: None,
                app_title: None,
                bundle_org: None,
            })
            .await
            .unwrap();
        let session = db
            .create_session(crate::schema::CreateSessionRequest {
                project_id: project.id.clone(),
                kind: "chat".into(),
                task_id: None,
                title: "s".into(),
                prompt_preview: None,
                agent_type: None,
                model: None,
                metadata_json: None,
            })
            .await
            .unwrap();

        // A skill on disk, registered + enabled for the project.
        let skill_dir = tmp.path().join("skills-src").join("demo-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: demo-skill\ndescription: demo roundtrip\nversion: 0.9.0\n---\nbody\n",
        )
        .unwrap();
        db.upsert_skill(
            "demo-skill",
            "demo-skill",
            "demo roundtrip",
            None,
            None,
            "0.9.0",
            &skill_dir.display().to_string(),
            None,
            &serde_json::json!({}),
        )
        .await
        .unwrap();
        db.link_project_skill(&project.id, "demo-skill", true)
            .await
            .unwrap();

        let memory_root = tmp.path().join("memory");
        let bundle = export_bundle(
            &db,
            &memory_root,
            BundleExportOptions {
                kind: HandoffKind::Project,
                project_id: project.id.clone(),
                session_id: Some(session.id.clone()),
                source_instance_id: "inst-a".into(),
                source_device_name: "dev".into(),
                max_bytes: 64 * 1024 * 1024,
                include_skills: true,
                include_mcp: false,
            },
        )
        .await
        .unwrap();

        // Import into a fresh DB + temp destination roots.
        let db2 = crate::db::DashboardDb::open(tmp.path().join("t2.db"))
            .await
            .unwrap();
        let dest_skills = tmp.path().join("dest-skills");
        let target_root = tmp.path().join("imported-proj");
        let result = import_bundle_with_roots(
            &db2,
            &memory_root,
            &bundle,
            ImportOptions {
                kind: HandoffKind::Project,
                target_root_path: Some(target_root.display().to_string()),
                target_project_id: None,
            },
            &dest_skills,
            None,
        )
        .await
        .unwrap();

        assert_eq!(result.sessions_imported, 1);
        assert_eq!(result.skills_installed, vec!["demo-skill".to_string()]);
        assert!(result.skills_skipped.is_empty());
        assert!(dest_skills.join("demo-skill/SKILL.md").is_file());
        assert_eq!(result.mcp_imported, 0);
        let _ = fs::remove_file(&bundle);
    }

    #[tokio::test]
    async fn v2_import_merges_mcp_servers_skipping_slug_conflicts() {
        let tmp = tempfile::tempdir().unwrap();
        let staging = tmp.path().join("staging");
        fs::create_dir_all(staging.join("mcp")).unwrap();
        fs::write(
            staging.join(MCP_SERVERS_NAME),
            serde_json::to_string(&serde_json::json!([
                { "slug": "existing", "command": "npx", "env": { "MCP_API_KEY": "" } },
                { "slug": "fresh", "type": "http", "url": "https://mcp.example/sse" }
            ]))
            .unwrap(),
        )
        .unwrap();
        let cfg_path = tmp.path().join("config.json");
        fs::write(
            &cfg_path,
            serde_json::to_string(&serde_json::json!({
                "mcp": { "servers": [{ "slug": "existing", "command": "local-real-command" }] }
            }))
            .unwrap(),
        )
        .unwrap();

        let (imported, skipped) = import_bundle_mcp_servers(&staging, Some(&cfg_path));
        assert_eq!(imported, 1);
        assert_eq!(skipped, vec!["existing".to_string()]);

        let (_, cfg) = crate::config_patch::read_config_value(Some(&cfg_path)).unwrap();
        let servers = crate::mcp_config::read_mcp_servers(&cfg);
        assert_eq!(servers.len(), 2);
        // The local entry wins — never overwritten by a bundle.
        let existing = servers.iter().find(|s| s["slug"] == "existing").unwrap();
        assert_eq!(existing["command"], "local-real-command");
        assert!(servers.iter().any(|s| s["slug"] == "fresh"));
    }
}
