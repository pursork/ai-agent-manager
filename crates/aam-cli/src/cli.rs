use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "aam", about = "ai-agent-manager: Claude Code + Codex CLI account/provider switcher")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Manage Profiles (tool + account, `docs/03-credential-account-module.md` §3.6).
    Profile {
        #[command(subcommand)]
        action: ProfileAction,
    },
    /// Register or update a third-party Provider's shared config (base_url/model/key).
    Provider {
        #[command(subcommand)]
        action: ProviderAction,
    },
    /// Launch `claude` with a Profile's CLAUDE_CONFIG_DIR (and Provider env vars, if attached).
    Claude {
        label: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        extra: Vec<String>,
    },
    /// Launch `codex` with a Profile's CODEX_HOME.
    Codex {
        label: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        extra: Vec<String>,
    },
    /// Skills management (`docs/09-skills-management.md`), Phase 1 subset.
    Skills {
        #[command(subcommand)]
        action: SkillsAction,
    },
}

#[derive(Subcommand)]
pub enum ProfileAction {
    /// List known Profiles.
    List {
        #[arg(long)]
        tool: Option<ToolArg>,
    },
    /// Create a new Profile (just the isolated config directory + Skills
    /// consistency link for Claude -- doesn't log in; run `aam claude/codex
    /// <label>` afterwards to do that interactively).
    Add {
        #[arg(long)]
        tool: ToolArg,
        label: String,
    },
    /// Runs the tool's official liveness check
    /// (`claude auth status` / `codex login status`) under this Profile.
    Verify {
        #[arg(long)]
        tool: ToolArg,
        label: String,
    },
    /// Attaches an already-`provider add`-ed Provider to a Profile
    /// (writes+verifies for Codex; verifies+records for Claude).
    UseProvider {
        #[arg(long)]
        tool: ToolArg,
        label: String,
        #[arg(long)]
        provider: String,
    },
}

#[derive(Subcommand)]
pub enum ProviderAction {
    /// Registers or updates a Provider's shared, non-secret config; the
    /// API key is prompted for (or reused if left blank and one is
    /// already saved) and stored separately in aam-vault.
    Add {
        #[arg(long)]
        kind: ProviderKindArg,
        /// Defaults to `kind`'s name if omitted.
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        base_url: String,
        /// Required for `cpa`; ignored for `deepseek-v4-flash` (fixed model).
        #[arg(long)]
        model: Option<String>,
        /// If omitted, prompted for on stdin (leave blank to reuse a
        /// previously-saved key for this id).
        #[arg(long)]
        api_key: Option<String>,
        #[arg(long, default_value_t = false)]
        supports_websockets: bool,
        #[arg(long, default_value = "high")]
        reasoning_effort: String,
        #[arg(long, default_value = "high")]
        plan_reasoning_effort: String,
    },
    /// Lists registered Providers (config only, never prints keys).
    List,
}

#[derive(Subcommand)]
pub enum SkillsAction {
    /// Lists skills in the canonical `~/.claude/skills` store.
    List,
    /// Shows link/git status for the canonical store and each skill.
    Status,
    /// Shares an already-canonical skill with Codex (and/or other Claude
    /// Profiles) by Junction/symlink. Phase 1 subset: only skills already
    /// at `~/.claude/skills/<name>` (no scan/adopt-from-elsewhere yet,
    /// that's Phase 3).
    Adopt {
        name: String,
        /// Comma-separated targets. Phase 1 supports `codex`.
        #[arg(long = "share-with")]
        share_with: String,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ToolArg {
    Claude,
    Codex,
}

impl From<ToolArg> for aam_switcher::Tool {
    fn from(value: ToolArg) -> Self {
        match value {
            ToolArg::Claude => aam_switcher::Tool::Claude,
            ToolArg::Codex => aam_switcher::Tool::Codex,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ProviderKindArg {
    Cpa,
    #[value(name = "deepseek-v4-flash")]
    DeepseekV4Flash,
}

impl From<ProviderKindArg> for aam_switcher::ProviderKind {
    fn from(value: ProviderKindArg) -> Self {
        match value {
            ProviderKindArg::Cpa => aam_switcher::ProviderKind::Cpa,
            ProviderKindArg::DeepseekV4Flash => aam_switcher::ProviderKind::DeepseekV4Flash,
        }
    }
}
