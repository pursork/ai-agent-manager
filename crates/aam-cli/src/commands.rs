use crate::cli::{
    Command, DeviceAction, ProfileAction, ProjectAction, ProviderAction, SessionAction, SkillsAction,
    SyncAction,
};
use aam_memory::ProjectIndex;
use aam_switcher::{
    build_provider, claude_backend, codex_backend, provider_secret_store, ApplyCodexProvider,
    Profile, ProfileRegistry, Provider, ProviderRecord, ProviderRegistry, Tool,
};
use std::error::Error;
use std::io::{self, Write};
use std::path::PathBuf;

pub fn run(command: Command) -> Result<(), Box<dyn Error>> {
    match command {
        Command::Profile { action } => run_profile(action),
        Command::Provider { action } => run_provider(action),
        Command::Claude { label, extra } => run_launch(Tool::Claude, &label, &extra),
        Command::Codex { label, extra } => run_launch(Tool::Codex, &label, &extra),
        Command::Skills { action } => run_skills(action),
        Command::Device { action } => run_device(action),
        Command::Sync { action } => run_sync(action),
        Command::Project { action } => run_project(action),
        Command::Session { action } => run_session(action),
        Command::Whoami { tool, config_dir } => run_whoami(tool.into(), config_dir),
    }
}

fn profile_registry() -> ProfileRegistry {
    ProfileRegistry::open_default()
}

fn provider_registry() -> ProviderRegistry {
    ProviderRegistry::open_default()
}

fn project_index() -> ProjectIndex {
    ProjectIndex::open_default()
}

fn run_profile(action: ProfileAction) -> Result<(), Box<dyn Error>> {
    match action {
        ProfileAction::List { tool } => {
            let registry = profile_registry();
            let profiles = match tool {
                Some(t) => registry.list_for_tool(t.into())?,
                None => registry.list()?,
            };
            if profiles.is_empty() {
                println!("(no profiles yet -- use `aam profile add --tool <claude|codex> <label>`)");
            }
            for p in profiles {
                let provider_note = p
                    .provider
                    .map(|id| format!("  [provider: {id}]"))
                    .unwrap_or_default();
                println!(
                    "{:<8} {:<20} {}{}",
                    p.tool.as_str(),
                    p.label,
                    p.config_dir.display(),
                    provider_note
                );
            }
            Ok(())
        }

        ProfileAction::Add { tool, label } => {
            let registry = profile_registry();
            let profile = match tool.into() {
                Tool::Claude => claude_backend::create_profile(&registry, &label)?,
                Tool::Codex => codex_backend::create_profile(&registry, &label)?,
            };
            println!(
                "created {} profile '{}' at {}",
                profile.tool, profile.label, profile.config_dir.display()
            );
            println!(
                "next: run `aam {} {}` to log in interactively, then `aam profile verify --tool {} {}`",
                profile.tool.as_str(),
                profile.label,
                profile.tool.as_str(),
                profile.label
            );
            Ok(())
        }

        ProfileAction::Verify { tool, label } => {
            let registry = profile_registry();
            let tool: Tool = tool.into();
            let profile = get_profile(&registry, tool, &label)?;
            let logged_in = match tool {
                Tool::Claude => claude_backend::verify_login(&profile)?,
                Tool::Codex => codex_backend::verify_login(&profile)?,
            };
            if logged_in {
                println!("{tool} profile '{label}': logged in");
                Ok(())
            } else {
                Err(format!("{tool} profile '{label}': NOT logged in").into())
            }
        }

        ProfileAction::UseProvider { tool, label, provider } => {
            let registry = profile_registry();
            let tool: Tool = tool.into();
            let profile = get_profile(&registry, tool, &label)?;
            let record = provider_registry()
                .get(&provider)?
                .ok_or_else(|| format!("no provider named '{provider}' (run `aam provider add` first)"))?;
            let api_key = provider_secret_store()?
                .load(&record.id)?
                .ok_or_else(|| format!("no API key saved for provider '{}'", record.id))?;
            let provider_obj = build_provider(&record, api_key);

            match tool {
                Tool::Claude => {
                    claude_backend::apply_provider(&registry, &profile, provider_obj.as_ref())?;
                }
                Tool::Codex => {
                    let mut op = ApplyCodexProvider::new(profile.config_dir.clone(), provider_obj.as_ref());
                    aam_core::execute(&mut op).map_err(|e| format!("{e}"))?;
                    registry.set_provider(tool, &label, Some(record.id.clone()))?;
                }
            }
            println!("profile '{label}' ({tool}) now uses provider '{}'", record.id);
            Ok(())
        }
    }
}

fn get_profile(registry: &ProfileRegistry, tool: Tool, label: &str) -> Result<Profile, Box<dyn Error>> {
    registry
        .get(tool, label)?
        .ok_or_else(|| format!("no {tool} profile named '{label}' (run `aam profile add --tool {} {label}` first)", tool.as_str()).into())
}

fn run_provider(action: ProviderAction) -> Result<(), Box<dyn Error>> {
    match action {
        ProviderAction::Add {
            kind,
            id,
            base_url,
            model,
            api_key,
            supports_websockets,
            reasoning_effort,
            plan_reasoning_effort,
        } => {
            let kind: aam_switcher::ProviderKind = kind.into();
            let id = id.unwrap_or_else(|| kind.to_string());
            let store = provider_secret_store()?;

            let key = match api_key {
                Some(k) if !k.is_empty() => k,
                _ => {
                    print!("API key for provider '{id}' (leave blank to reuse a previously saved key): ");
                    io::stdout().flush().ok();
                    let mut input = String::new();
                    io::stdin().read_line(&mut input)?;
                    let input = input.trim().to_string();
                    if input.is_empty() {
                        store
                            .load(&id)?
                            .ok_or_else(|| format!("no API key provided and none saved yet for '{id}'"))?
                    } else {
                        input
                    }
                }
            };
            store.save(&id, &key)?;

            let model = match kind {
                aam_switcher::ProviderKind::DeepseekV4Flash => "deepseek-v4-flash".to_string(),
                aam_switcher::ProviderKind::Cpa => model.ok_or("--model is required for --kind cpa")?,
            };

            let record = ProviderRecord {
                id: id.clone(),
                kind,
                base_url,
                model,
                reasoning_effort,
                plan_reasoning_effort,
                supports_websockets,
            };
            provider_registry().upsert(record)?;
            println!(
                "provider '{id}' saved (materialized into a Profile via `aam profile use-provider --tool <claude|codex> <label> --provider {id}`)"
            );
            Ok(())
        }

        ProviderAction::List => {
            for record in provider_registry().list()? {
                println!(
                    "{:<20} {:<18} {}  model={}",
                    record.id, record.kind, record.base_url, record.model
                );
            }
            Ok(())
        }
    }
}

fn run_launch(tool: Tool, label: &str, extra: &[String]) -> Result<(), Box<dyn Error>> {
    let registry = profile_registry();
    let profile = get_profile(&registry, tool, label)?;

    let env: Vec<(String, String)> = match tool {
        Tool::Claude => {
            let provider_obj = resolve_provider(&profile)?;
            claude_backend::launch_env(&profile, provider_obj.as_deref())
        }
        Tool::Codex => codex_backend::launch_env(&profile),
    };

    let binary = tool.as_str();
    let mut cmd = std::process::Command::new(binary);
    cmd.args(extra);
    for (k, v) in env {
        cmd.env(k, v);
    }

    let status = cmd
        .status()
        .map_err(|e| format!("failed to launch '{binary}' (is it on PATH?): {e}"))?;
    std::process::exit(status.code().unwrap_or(1));
}

fn resolve_provider(profile: &Profile) -> Result<Option<Box<dyn Provider>>, Box<dyn Error>> {
    let Some(id) = &profile.provider else {
        return Ok(None);
    };
    let record = provider_registry()
        .get(id)?
        .ok_or_else(|| format!("profile references provider '{id}' but it's no longer registered"))?;
    let api_key = provider_secret_store()?
        .load(id)?
        .ok_or_else(|| format!("no API key saved for provider '{id}'"))?;
    Ok(Some(build_provider(&record, api_key)))
}

fn run_skills(action: SkillsAction) -> Result<(), Box<dyn Error>> {
    match action {
        SkillsAction::List => {
            let skills = aam_skills::list_managed_skills()?;
            if skills.is_empty() {
                println!(
                    "(no skills found under {})",
                    aam_skills::claude_personal_skills_dir().display()
                );
            }
            for s in skills {
                println!(
                    "{:<24} codex-linked={:<5} git={}",
                    s.name, s.linked_to_codex, s.is_git_repo
                );
            }
            Ok(())
        }

        SkillsAction::Status => {
            let root = aam_skills::claude_personal_skills_dir();
            println!("canonical store: {}", root.display());
            let skills = aam_skills::list_managed_skills()?;
            if skills.iter().any(|s| s.is_git_repo) {
                println!(
                    "this looks like a git repository -- use `git push`/`git pull` to sync it \
                     across devices (docs/09-skills-management.md §9.2); aam does not sync skill \
                     content itself."
                );
            }
            println!(
                "{} skill(s), {} linked into Codex's $HOME/.agents/skills",
                skills.len(),
                skills.iter().filter(|s| s.linked_to_codex).count()
            );
            Ok(())
        }

        SkillsAction::Adopt { name, share_with } => {
            for target in share_with.split(',').map(str::trim) {
                match target {
                    "codex" => {
                        let extra_keys = aam_skills::share_skill_with_codex(&name)?;
                        if extra_keys.is_empty() {
                            println!("linked '{name}' into Codex's $HOME/.agents/skills");
                        } else {
                            println!(
                                "linked '{name}' into Codex's $HOME/.agents/skills (warning: uses \
                                 non-standard frontmatter fields [{}], Codex may not understand them \
                                 -- docs/09-skills-management.md §9.1)",
                                extra_keys.join(", ")
                            );
                        }
                    }
                    other => {
                        return Err(format!(
                            "unsupported --share-with target '{other}' (Phase 1 only supports \
                             'codex'; per-Profile Claude sharing lands in Phase 3)"
                        )
                        .into());
                    }
                }
            }
            Ok(())
        }

        SkillsAction::InstallBundled { name, force } => {
            let names: Vec<&str> = match &name {
                Some(n) => vec![n.as_str()],
                None => aam_skills::BUNDLED_SKILLS.iter().map(|s| s.name).collect(),
            };
            for n in names {
                let outcome = aam_skills::install_bundled_skill(n, force)?;
                match outcome {
                    aam_skills::InstallOutcome::Installed => {
                        println!(
                            "installed '{n}' into {}",
                            aam_skills::claude_personal_skills_dir().join(n).display()
                        );
                        println!("see its SKILL.md for the (manual) hook-registration step");
                    }
                    aam_skills::InstallOutcome::AlreadyUpToDate => {
                        println!("'{n}' already up to date");
                    }
                    aam_skills::InstallOutcome::Overwritten => {
                        println!("'{n}' overwritten with the bundled version");
                    }
                }
            }
            Ok(())
        }
    }
}

/// Local per-machine state for `aam-sync` (this device's age identity) --
/// distinct from `aam_core::aam_home()`'s other subdirectories, which hold
/// Profile/Provider registries, not sync state.
fn sync_state_dir() -> PathBuf {
    aam_core::aam_home().join("sync")
}

fn prompt_hidden(prompt: &str) -> Result<String, Box<dyn Error>> {
    Ok(rpassword::prompt_password(prompt)?)
}

fn webdav_backend(url: String, user: String, password: String) -> aam_sync::WebDavBackend {
    aam_sync::WebDavBackend::new(url, user, password)
}

fn require_local_identity() -> Result<aam_sync::LocalIdentity, Box<dyn Error>> {
    aam_sync::local_identity(&sync_state_dir())?.ok_or_else(|| {
        "no local device identity yet -- run `aam sync init` (new vault) or `aam device join` \
         (existing vault) first"
            .into()
    })
}

fn run_device(action: DeviceAction) -> Result<(), Box<dyn Error>> {
    match action {
        DeviceAction::Join { webdav_url, webdav_user, label } => {
            let password = prompt_hidden("WebDAV password: ")?;
            let passphrase = prompt_hidden("Vault master passphrase: ")?;
            let backend = webdav_backend(webdav_url, webdav_user, password);
            let entry = aam_sync::join_device_to_vault(&backend, &sync_state_dir(), &passphrase, &label)?;
            println!("joined vault as device '{}' ({})", entry.label, entry.device_id);
            println!(
                "note: this device is listed but cannot decrypt existing blobs yet -- ask an \
                 already-authorized device to run `aam sync reencrypt`"
            );
            Ok(())
        }

        DeviceAction::List { webdav_url, webdav_user } => {
            let password = prompt_hidden("WebDAV password: ")?;
            let passphrase = prompt_hidden("Vault master passphrase: ")?;
            let backend = webdav_backend(webdav_url, webdav_user, password);
            let manifest = aam_sync::list_devices(&backend, &passphrase)?;
            for d in manifest.devices {
                println!(
                    "{:<36} {:<20} revoked={:<5} added={}",
                    d.device_id, d.label, d.revoked, d.added_at
                );
            }
            Ok(())
        }

        DeviceAction::Revoke { webdav_url, webdav_user, device_id } => {
            let password = prompt_hidden("WebDAV password: ")?;
            let passphrase = prompt_hidden("Vault master passphrase: ")?;
            let backend = webdav_backend(webdav_url, webdav_user, password);
            aam_sync::revoke_device_in_vault(&backend, &passphrase, &device_id)?;
            println!(
                "device '{device_id}' revoked -- run `aam sync reencrypt` so future pushes exclude it \
                 (already-synced blobs stay readable to it until then, per docs/04 §4.4's documented \
                 limitation)"
            );
            Ok(())
        }
    }
}

fn run_sync(action: SyncAction) -> Result<(), Box<dyn Error>> {
    match action {
        SyncAction::Init { webdav_url, webdav_user, label } => {
            let password = prompt_hidden("WebDAV password: ")?;
            let passphrase = prompt_hidden("Set a new vault master passphrase: ")?;
            let confirm = prompt_hidden("Confirm master passphrase: ")?;
            if passphrase != confirm {
                return Err("passphrases did not match".into());
            }
            let backend = webdav_backend(webdav_url, webdav_user, password);
            let entry = aam_sync::init_vault(&backend, &sync_state_dir(), &passphrase, &label)?;
            println!(
                "vault initialized; this device registered as '{}' ({})",
                entry.label, entry.device_id
            );
            Ok(())
        }

        SyncAction::Reencrypt { webdav_url, webdav_user } => {
            let password = prompt_hidden("WebDAV password: ")?;
            let passphrase = prompt_hidden("Vault master passphrase: ")?;
            let backend = webdav_backend(webdav_url, webdav_user, password);
            let identity = require_local_identity()?;
            let manifest = aam_sync::list_devices(&backend, &passphrase)?;
            let recipients = manifest.active_recipients();

            let registry = provider_registry();
            let results = aam_switcher::reencrypt_all_known_providers(
                &backend,
                &registry,
                &identity.private_key,
                &recipients,
                &identity.device_id,
            )?;
            for (id, meta) in results {
                match meta {
                    Some(m) => println!("re-encrypted provider '{id}' (now version {})", m.version),
                    None => println!("provider '{id}': no blob to re-encrypt yet (never pushed)"),
                }
            }
            Ok(())
        }

        SyncAction::Push { webdav_url, webdav_user, provider } => {
            let password = prompt_hidden("WebDAV password: ")?;
            let passphrase = prompt_hidden("Vault master passphrase: ")?;
            let backend = webdav_backend(webdav_url, webdav_user, password);
            let identity = require_local_identity()?;
            let manifest = aam_sync::list_devices(&backend, &passphrase)?;
            let recipients = manifest.active_recipients();

            let registry = provider_registry();
            let meta = aam_switcher::push_provider(
                &backend,
                &registry,
                &provider,
                &recipients,
                &identity.device_id,
            )?;
            println!("pushed provider '{provider}' (version {})", meta.version);
            Ok(())
        }

        SyncAction::Pull { webdav_url, webdav_user, provider } => {
            let password = prompt_hidden("WebDAV password: ")?;
            let backend = webdav_backend(webdav_url, webdav_user, password);
            let identity = require_local_identity()?;

            let registry = provider_registry();
            match aam_switcher::pull_provider(&backend, &registry, &provider, &identity.private_key)? {
                Some(meta) => println!("pulled provider '{provider}' (version {})", meta.version),
                None => println!("no blob found for provider '{provider}' at this vault"),
            }
            Ok(())
        }

        SyncAction::PushAccount { webdav_url, webdav_user, tool, label } => {
            let password = prompt_hidden("WebDAV password: ")?;
            let passphrase = prompt_hidden("Vault master passphrase: ")?;
            let backend = webdav_backend(webdav_url, webdav_user, password);
            let identity = require_local_identity()?;
            let manifest = aam_sync::list_devices(&backend, &passphrase)?;
            let recipients = manifest.active_recipients();

            let tool: Tool = tool.into();
            let registry = profile_registry();
            let profile = get_profile(&registry, tool, &label)?;
            let meta = aam_switcher::push_account(
                &backend,
                &profile,
                &recipients,
                &identity.device_id,
                &passphrase,
            )?;
            println!(
                "pushed {tool} account credential for '{label}' (version {})",
                meta.version
            );
            Ok(())
        }

        SyncAction::ListAccounts { webdav_url, webdav_user } => {
            let password = prompt_hidden("WebDAV password: ")?;
            let passphrase = prompt_hidden("Vault master passphrase: ")?;
            let backend = webdav_backend(webdav_url, webdav_user, password);
            let accounts = aam_switcher::list_accounts(&backend, &passphrase)?;
            if accounts.is_empty() {
                println!("(no accounts pushed to this vault yet)");
            }
            for a in accounts {
                println!(
                    "{:<8} {:<24} label={:<16} email={}",
                    a.tool,
                    a.key,
                    a.label_hint,
                    a.email_hint.as_deref().unwrap_or("-")
                );
            }
            Ok(())
        }

        SyncAction::PullAccount { webdav_url, webdav_user, tool, key, as_label } => {
            let password = prompt_hidden("WebDAV password: ")?;
            let backend = webdav_backend(webdav_url, webdav_user, password);
            let identity = require_local_identity()?;

            let tool: Tool = tool.into();
            let registry = profile_registry();
            let profile = aam_switcher::pull_account(
                &backend,
                &registry,
                tool,
                &key,
                &as_label,
                &identity.private_key,
            )?;
            println!(
                "pulled {tool} account credential '{key}' into local profile '{}' -- \
                 run `aam {tool} {}` to use it",
                profile.label, profile.label
            );
            Ok(())
        }
    }
}

/// A session `scan` found, tagged with which local Profile it came from
/// (needed for `ProjectRecord::profile_label` on adopt, and useful in
/// `scan`'s own printed report).
struct TaggedDiscovery {
    session: aam_memory::DiscoveredSession,
    profile_label: String,
}

/// `05.7`: scans every registered Profile's config directory, not just
/// one -- this is what makes plain `aam session scan`/`adopt` (no
/// `--tool`/`--profile` needed) actually cover "every account on this
/// machine" per the design doc.
fn scan_all_profiles() -> Result<Vec<TaggedDiscovery>, Box<dyn Error>> {
    let index = project_index();
    let known_ids: Vec<String> = index.list()?.into_iter().map(|r| r.last_session_id).collect();

    let mut out = Vec::new();
    for profile in profile_registry().list()? {
        let discovered = match profile.tool {
            Tool::Claude => aam_memory::scan_claude_sessions(
                &profile.config_dir,
                std::slice::from_ref(&profile.config_dir),
                &known_ids,
            ),
            Tool::Codex => aam_memory::scan_codex_sessions(&profile.config_dir, &known_ids),
        };
        out.extend(discovered.into_iter().map(|session| TaggedDiscovery {
            session,
            profile_label: profile.label.clone(),
        }));
    }
    Ok(out)
}

fn run_session(action: SessionAction) -> Result<(), Box<dyn Error>> {
    match action {
        SessionAction::Scan => {
            let found = scan_all_profiles()?;
            if found.is_empty() {
                println!("(no undiscovered sessions found across registered profiles)");
            }
            for item in &found {
                println!(
                    "{:<8} {:<16} {:<50} {}",
                    item.session.tool_kind,
                    item.profile_label,
                    item.session.path,
                    item.session.auto_status.as_deref().unwrap_or("-")
                );
            }
            if !found.is_empty() {
                println!("run `aam session adopt` to write these into the local index (syncApproved=false)");
            }
            Ok(())
        }

        SessionAction::Adopt => {
            let found = scan_all_profiles()?;
            let index = project_index();
            // Reuse this machine's sync identity if a vault has already
            // been set up; an empty string is a legitimate default
            // otherwise (ProjectRecord's own doc comment covers this --
            // Memory-Bank tracking doesn't require `aam sync init` first).
            let device_id = aam_sync::local_identity(&aam_core::aam_home().join("sync"))
                .ok()
                .flatten()
                .map(|i| i.device_id)
                .unwrap_or_default();

            let mut adopted = 0;
            for item in &found {
                aam_memory::adopt_session(&index, &item.session, &device_id, &item.profile_label, None)?;
                adopted += 1;
            }
            println!("adopted {adopted} session(s) (discoverySource=scan, syncApproved=false)");
            if adopted > 0 {
                println!(
                    "run `aam session approve-sync <path...>` (or --all-scanned) before syncing them \
                     anywhere"
                );
            }
            Ok(())
        }

        SessionAction::ApproveSync { names, all_scanned } => {
            let index = project_index();
            let approved = if all_scanned {
                aam_memory::approve_all_scanned(&index)?
            } else {
                if names.is_empty() {
                    return Err("no project paths given -- pass some, or use --all-scanned".into());
                }
                aam_memory::approve_sync(&index, &names)?
            };
            println!("approved {approved} record(s) for sync");
            Ok(())
        }

        SessionAction::Sync { webdav_url, webdav_user } => {
            let password = prompt_hidden("WebDAV password: ")?;
            let passphrase = prompt_hidden("Vault master passphrase: ")?;
            let backend = webdav_backend(webdav_url, webdav_user, password);
            let identity = require_local_identity()?;
            let manifest = aam_sync::list_devices(&backend, &passphrase)?;
            let recipients = manifest.active_recipients();

            let local = project_index();
            let mirror = aam_memory::remote_mirror_index();
            let meta = aam_memory::sync_index(
                &backend,
                &local,
                &mirror,
                &recipients,
                &identity.device_id,
                &identity.private_key,
            )?;
            println!("synced Memory-Bank index (version {})", meta.version);
            println!(
                "other devices' records now visible in `aam project list` (mirrored separately -- \
                 project-index.json itself is untouched)"
            );
            Ok(())
        }
    }
}

/// Concatenates the local (real `project-index.json`, `project-tracker`
/// still writes it) and mirrored (other devices', from the last `aam
/// session sync`) record sets for display. Deliberately not deduplicated
/// by name/path here -- that needs the real cross-device identity
/// (`projectId`, `docs/08-open-questions-risks.md` #8) this round only
/// adds a field placeholder for, not matching logic.
fn local_and_mirrored_projects() -> Result<Vec<aam_memory::ProjectRecord>, Box<dyn Error>> {
    let mut all = project_index().list()?;
    all.extend(aam_memory::remote_mirror_index().list()?);
    Ok(all)
}

fn run_project(action: ProjectAction) -> Result<(), Box<dyn Error>> {
    match action {
        ProjectAction::List => {
            let mut projects = local_and_mirrored_projects()?;
            projects.sort_by(|a, b| b.last_active.cmp(&a.last_active));
            if projects.is_empty() {
                println!("(no tracked projects yet -- run `aam session scan`/`adopt` to find some)");
            }
            for p in &projects {
                println!(
                    "{:<24} {:<8} {:<16} {:<50} {}",
                    p.name,
                    p.tool_kind,
                    p.profile_label.as_deref().unwrap_or("-"),
                    p.path,
                    p.display_status().unwrap_or("(尚无记录)")
                );
            }
            Ok(())
        }

        ProjectAction::Show { name } => {
            let query = name.to_lowercase();
            let matches: Vec<_> = local_and_mirrored_projects()?
                .into_iter()
                .filter(|p| p.name.to_lowercase().contains(&query) || p.path.to_lowercase().contains(&query))
                .collect();
            if matches.is_empty() {
                return Err(format!("no project matching '{name}' found").into());
            }
            for p in &matches {
                println!("path:            {}", p.path);
                println!("name:            {}", p.name);
                println!("tool:            {}", p.tool_kind);
                println!("profile:         {}", p.profile_label.as_deref().unwrap_or("-"));
                println!(
                    "device:          {}",
                    if p.device_id.is_empty() { "-" } else { &p.device_id }
                );
                println!("last active:     {}", p.last_active);
                println!("created:         {}", p.created);
                println!("status:          {}", p.display_status().unwrap_or("(尚无记录)"));
                println!("discovery:       {}", p.discovery_source);
                println!("sync approved:   {}", p.sync_approved);
                println!("last session id: {}", p.last_session_id);
                println!("project id:      {}", p.project_id.as_deref().unwrap_or("-"));
                println!();
            }
            Ok(())
        }

        ProjectAction::Resume { name } => {
            let query = name.to_lowercase();
            let matches: Vec<_> = local_and_mirrored_projects()?
                .into_iter()
                .filter(|p| p.name.to_lowercase().contains(&query) || p.path.to_lowercase().contains(&query))
                .collect();
            let record = match matches.as_slice() {
                [] => return Err(format!("no project matching '{name}' found").into()),
                [one] => one,
                many => {
                    println!("multiple projects match '{name}':");
                    for m in many {
                        println!("  {} ({})", m.name, m.path);
                    }
                    return Err("ambiguous match -- be more specific".into());
                }
            };

            // Profile mismatch check: does the recorded Profile still
            // exist locally? aam has no "currently active default
            // Profile" concept the way project-tracker's env-var-sniffed
            // "current shell backend" does -- each `aam claude/codex
            // <label>` launch is a one-shot process, not persistent shell
            // state -- so this can only warn about "missing", not
            // "different from what's active right now".
            if let Some(label) = &record.profile_label {
                let tool: Tool = if record.tool_kind == "codex" { Tool::Codex } else { Tool::Claude };
                if profile_registry().get(tool, label)?.is_none() {
                    println!(
                        "warning: this record's Profile '{label}' ({tool}) is not registered on this \
                         machine -- run `aam profile add` or `aam sync pull-account` first, or resume \
                         may fail"
                    );
                }
            }
            if let Some(backend) = &record.auth_backend {
                if backend != "oauth-subscription" {
                    println!(
                        "warning: this project was last touched via backend '{backend}', not the \
                         official subscription -- resuming an extended-thinking session under a \
                         different backend/account can fail with a signature error (see \
                         project-tracker's own troubleshooting notes)"
                    );
                }
            }

            // 05.3: only ever *tell* the user where to go -- never assume
            // the path exists just because a record mentions it (it may
            // well be a cross-device record from the mirror).
            if !std::path::Path::new(&record.path).is_dir() {
                let device_note = if record.device_id.is_empty() {
                    String::new()
                } else {
                    format!("（记录设备 id: {}）", record.device_id)
                };
                println!(
                    "本机未找到目录 '{}'{device_note}。如果这条记录来自另一台设备，请前往该设备继续。",
                    record.path
                );
                return Ok(());
            }

            println!("cd \"{}\"", record.path);
            match record.tool_kind.as_str() {
                "codex" => println!("codex resume {}", record.last_session_id),
                _ => println!("claude --resume {}", record.last_session_id),
            }
            Ok(())
        }

        ProjectAction::Link { path_a, path_b } => {
            let local = project_index();
            let mirror = aam_memory::remote_mirror_index();
            let project_id = aam_memory::link_projects(&local, &mirror, &path_a, &path_b)?;
            println!("linked '{path_a}' and '{path_b}' under projectId '{project_id}'");
            Ok(())
        }
    }
}

fn run_whoami(tool: Tool, config_dir: Option<PathBuf>) -> Result<(), Box<dyn Error>> {
    let dir = config_dir.unwrap_or_else(|| tool.actual_config_dir());
    let profile_label = profile_registry().find_by_config_dir(tool, &dir)?.map(|p| p.label);
    let device_id = aam_sync::local_identity(&aam_core::aam_home().join("sync"))
        .ok()
        .flatten()
        .map(|i| i.device_id);

    let output = serde_json::json!({
        "toolKind": tool.as_str(),
        "profileLabel": profile_label,
        "deviceId": device_id,
    });
    println!("{output}");
    Ok(())
}
