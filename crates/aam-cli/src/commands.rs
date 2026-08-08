use crate::cli::{Command, ProfileAction, ProviderAction, SkillsAction};
use aam_switcher::{
    build_provider, claude_backend, codex_backend, provider_secret_store, ApplyCodexProvider,
    Profile, ProfileRegistry, Provider, ProviderRecord, ProviderRegistry, Tool,
};
use std::error::Error;
use std::io::{self, Write};

pub fn run(command: Command) -> Result<(), Box<dyn Error>> {
    match command {
        Command::Profile { action } => run_profile(action),
        Command::Provider { action } => run_provider(action),
        Command::Claude { label, extra } => run_launch(Tool::Claude, &label, &extra),
        Command::Codex { label, extra } => run_launch(Tool::Codex, &label, &extra),
        Command::Skills { action } => run_skills(action),
    }
}

fn profile_registry() -> ProfileRegistry {
    ProfileRegistry::open_default()
}

fn provider_registry() -> ProviderRegistry {
    ProviderRegistry::open_default()
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
    }
}
