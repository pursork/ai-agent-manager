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
    /// Reads/updates the project Memory-Bank index (`docs/05-session-memory-bank-module.md`),
    /// the same `project-index.json` `~/.claude/skills/project-tracker` already maintains.
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },
    /// Session discovery and adoption (`05.7`-`05.9`): find sessions on
    /// disk not yet in the Memory-Bank index, and explicitly bring them in.
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },
    /// Read-only diagnostic: prints which Profile (if any) and device this
    /// process is running under, as JSON. Primarily for hook scripts (e.g.
    /// the bundled `project-tracker` skill's `track-session.ps1`) to shell
    /// out to, rather than re-implementing aam's DPAPI/registry lookups
    /// themselves (`docs/09-skills-management.md`).
    Whoami {
        #[arg(long)]
        tool: ToolArg,
        /// Overrides the config directory to look up (mainly for testing);
        /// defaults to $CLAUDE_CONFIG_DIR/$CODEX_HOME, or the tool's OS
        /// default if that's unset.
        #[arg(long)]
        config_dir: Option<std::path::PathBuf>,
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
    /// Finds skill directories not yet tracked in `aam`'s skills index
    /// (`docs/09-skills-management.md` §9.6): the canonical store itself
    /// (pre-existing skills never adopted), Codex's own skills dir, and
    /// any Claude Profile whose `skills/` isn't already linked back to
    /// the canonical store. Read-only -- run `adopt` on what it reports.
    Scan,
    /// Brings a skill under `aam`'s management. Two forms:
    /// - `aam skills adopt <name>` -- moves an already-discovered (via
    ///   `scan`) local directory into the canonical store and links its
    ///   old location back (or, if it's already at the canonical
    ///   location, just records it in the index).
    /// - `aam skills adopt <name> --source <git-url>[@ref]` -- clones a
    ///   new skill from git straight into the canonical store.
    Adopt {
        name: String,
        /// Comma-separated share targets, applied after adopting.
        /// Currently supports `codex`.
        #[arg(long = "share-with")]
        share_with: Option<String>,
        /// `<git-url>[@ref]` -- adopt by cloning from git instead of
        /// moving an existing local directory.
        #[arg(long)]
        source: Option<String>,
        /// `manual` (default) or `auto` -- only meaningful with
        /// `--source`; controls whether `update --all-auto` includes it.
        #[arg(long = "update-mode", default_value = "manual")]
        update_mode: String,
    },
    /// Installs a skill shipped with `aam` itself (e.g. `project-tracker`)
    /// into `~/.claude/skills/<name>`. Refuses to overwrite an existing,
    /// differently-content directory unless `--force` is given. Does
    /// **not** register any Claude Code hooks -- see the installed skill's
    /// own `SKILL.md` for that manual step.
    InstallBundled {
        /// Omit to install every bundled skill.
        name: Option<String>,
        #[arg(long)]
        force: bool,
    },
    /// Checks git-sourced skills (`adopt --source`) against their
    /// upstream for updates (`docs/09-skills-management.md` §9.7).
    CheckUpdates,
    /// Applies an upstream update to a git-sourced skill (`git reset
    /// --hard @{upstream}` -- these directories are pristine upstream
    /// mirrors, not meant to be locally edited), or with `--all-auto`,
    /// to every skill adopted with `--update-mode auto`.
    Update {
        name: Option<String>,
        #[arg(long = "all-auto", default_value_t = false)]
        all_auto: bool,
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

#[derive(Subcommand)]
pub enum ProjectAction {
    /// Lists every project record, local and cross-device-mirrored
    /// (`05.6`; simple concatenation, not deduplicated -- see `link`).
    List,
    /// Shows every entry matching `name` (fuzzy: name or trailing path
    /// segment, case-insensitive), local and mirrored.
    Show { name: String },
    /// Prints the `cd` + resume command for a project. Never runs it --
    /// this project cannot change your shell's working directory
    /// (`05.3`'s "只提示，不搬迁" rule). If the record's path doesn't
    /// exist on this machine (e.g. it's a mirrored record from another
    /// device), says so instead of printing commands that would fail.
    Resume { name: String },
    /// Manually declares that two records (by path, local and/or
    /// mirrored) are the same logical project, giving them a shared
    /// `projectId` (`08` #8). No automatic matching -- this is the only
    /// way two records get linked.
    Link { path_a: String, path_b: String },
}

#[derive(Subcommand)]
pub enum SessionAction {
    /// Scans every registered Profile's config directory for sessions not
    /// yet in the Memory-Bank index. Read-only -- writes nothing (`05.7`).
    Scan,
    /// Adopts every session `scan` would report: writes it into the index
    /// with `discoverySource: scan`, `syncApproved: false` (`05.8`).
    Adopt {
        /// Generate `autoStatus` via a Provider for sessions that don't
        /// already have one (mainly Codex -- Claude gets `ai-title` for
        /// free). Requires `--profile`; the named Profile must have a
        /// third-party Provider attached (the official subscription has
        /// no `Provider` to call, `03.1`).
        #[arg(long)]
        summarize: bool,
        /// Which Profile's Provider to summarize with. Never picked
        /// silently -- required whenever `--summarize` is set (`05.8`).
        #[arg(long)]
        profile: Option<String>,
    },
    /// Marks previously-adopted (`scan`-sourced) records as approved for
    /// sync (`05.9`). Does not sync anything itself -- `aam session sync`
    /// is what actually pushes to WebDAV.
    ApproveSync {
        /// Project paths to approve (see `aam project list`).
        names: Vec<String>,
        #[arg(long = "all-scanned")]
        all_scanned: bool,
    },
    /// Syncs the Memory-Bank index with the vault: pulls the current
    /// shared set, replaces this device's own slice with its current
    /// `syncApproved` records, pushes the result (`05.6`). Cross-device
    /// records land in a local mirror, never in `project-tracker`'s own
    /// `project-index.json` (`08` #9).
    Sync {
        #[arg(long)]
        webdav_url: String,
        #[arg(long)]
        webdav_user: String,
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
