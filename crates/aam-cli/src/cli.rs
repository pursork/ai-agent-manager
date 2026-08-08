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
    /// Manage devices in a WebDAV-synced vault (`docs/04-webdav-sync-security.md`).
    Device {
        #[command(subcommand)]
        action: DeviceAction,
    },
    /// WebDAV-synced vault: create it, re-encrypt after a device joins,
    /// and push/pull Provider config or account login credentials
    /// (`docs/04-webdav-sync-security.md` §§4.6/4.10).
    Sync {
        #[command(subcommand)]
        action: SyncAction,
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

#[derive(Subcommand)]
pub enum DeviceAction {
    /// Joins an existing vault as a new device (`04.3` steps 1-5). Does
    /// **not** grant access to existing blobs yet -- run `aam sync
    /// reencrypt` on an already-authorized device afterwards.
    Join {
        #[arg(long)]
        webdav_url: String,
        #[arg(long)]
        webdav_user: String,
        #[arg(long)]
        label: String,
    },
    /// Lists devices in the vault (label / id / revoked status).
    List {
        #[arg(long)]
        webdav_url: String,
        #[arg(long)]
        webdav_user: String,
    },
    /// Marks a device revoked (`04.4`). Run `aam sync reencrypt` afterwards
    /// so future pushes exclude it.
    Revoke {
        #[arg(long)]
        webdav_url: String,
        #[arg(long)]
        webdav_user: String,
        device_id: String,
    },
}

#[derive(Subcommand)]
pub enum SyncAction {
    /// Creates a brand new vault at this WebDAV location (this device
    /// becomes its first device). Errors if a vault already exists there
    /// -- use `aam device join` instead.
    Init {
        #[arg(long)]
        webdav_url: String,
        #[arg(long)]
        webdav_user: String,
        #[arg(long)]
        label: String,
    },
    /// Re-encrypts every provider config this device's local registry
    /// knows about, to the vault's current device list (`04.3` step 6's
    /// manual version) -- run this after a new device joins.
    Reencrypt {
        #[arg(long)]
        webdav_url: String,
        #[arg(long)]
        webdav_user: String,
    },
    /// Pushes one provider's config + API key to the vault.
    Push {
        #[arg(long)]
        webdav_url: String,
        #[arg(long)]
        webdav_user: String,
        #[arg(long)]
        provider: String,
    },
    /// Pulls one provider's config + API key from the vault into the local
    /// registry (creating or overwriting it).
    Pull {
        #[arg(long)]
        webdav_url: String,
        #[arg(long)]
        webdav_user: String,
        #[arg(long)]
        provider: String,
    },
    /// Pushes a Profile's official login credential (`.credentials.json` /
    /// `auth.json`, not the rest of its config directory) to the vault
    /// (`04.10`). Claude's WebDAV key is `label` itself; Codex's is a
    /// fingerprint derived from the credential's own JWT claims (shown in
    /// `aam sync list-accounts` afterwards).
    PushAccount {
        #[arg(long)]
        webdav_url: String,
        #[arg(long)]
        webdav_user: String,
        #[arg(long)]
        tool: ToolArg,
        /// The local Profile whose credential file to push.
        #[arg(long)]
        label: String,
    },
    /// Lists accounts pushed to this vault (tool / key / label hint / email
    /// hint) -- run this before `pull-account` to see what's available,
    /// since there's no WebDAV directory listing to discover it otherwise.
    ListAccounts {
        #[arg(long)]
        webdav_url: String,
        #[arg(long)]
        webdav_user: String,
    },
    /// Pulls an account credential from the vault. Creates the local
    /// Profile named `--as` if none exists yet for `--tool`, then writes
    /// the decrypted credential file into it -- no separate `profile add`
    /// needed first.
    PullAccount {
        #[arg(long)]
        webdav_url: String,
        #[arg(long)]
        webdav_user: String,
        #[arg(long)]
        tool: ToolArg,
        /// The vault key from `aam sync list-accounts` (a label for
        /// Claude, a fingerprint for Codex).
        #[arg(long)]
        key: String,
        /// Local Profile label to create/overwrite with the pulled
        /// credential.
        #[arg(long = "as")]
        as_label: String,
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
