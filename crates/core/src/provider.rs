//! Agent CLI providers and their static capability descriptors.
//!
//! Every provider-conditional decision in `core`, `pty`, and `app` reads
//! [`Provider::capabilities`] rather than branching on ad-hoc provider checks.
//! Keeping the capability matrix in one pure module guarantees UI affordances,
//! keymap actions, and MCP tools stay in lockstep as Codex and Antigravity
//! slices land.

/// An agent CLI that TermHerd can host inside a terminal pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    Claude,
    Codex,
    Antigravity,
}

/// How much live activity signal a provider supplies while running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivitySupport {
    /// Reliable `Busy` / `Attention` / `Idle` transitions (e.g. Claude OSC).
    Full,
    /// Only process liveness (`Unmonitored` while running, `Exited` on exit).
    ProcessOnly,
}

/// How tab renaming behaves for a provider's pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameSupport {
    /// Typed `/rename` slash command inside the PTY, behind confirmation.
    SlashCommand,
    /// Local TermHerd tab title only (never mutates provider-owned files).
    LocalTabOnly,
}

/// Static integration capabilities of a [`Provider`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderCapabilities {
    /// Whether the PTY adapter implements command construction for this
    /// provider. Keeps UI launch affordances from offering a provider whose
    /// command builder has not shipped yet.
    pub launch_supported: bool,
    /// Accepts a caller-minted session UUID at fresh launch (`--session-id`).
    pub mint_session_id: bool,
    /// Supports resuming a discovered conversation/thread by provider ID.
    pub resume_by_id: bool,
    /// Discovers local session history into the sidebar.
    pub history_discovery: bool,
    /// Live activity signal fidelity while the CLI is running.
    pub live_activity: ActivitySupport,
    /// How renaming a tab hosting this provider works.
    pub native_rename: RenameSupport,
    /// Supports provider-native accent color (`/color`).
    pub native_color: bool,
    /// Supports typed slash commands with confirmation overlay.
    pub slash_commands: bool,
    /// Exposes a live peer/agent name (`copy-agent-name`).
    pub agent_name: bool,
    /// Supports handing the session off to a desktop app (`/desktop`).
    pub send_to_desktop: bool,
    /// Has a discoverable transcript file on disk (`reveal-transcript`).
    pub reveal_transcript: bool,
    /// Accepts TermHerd's loopback MCP server configuration at launch.
    pub mcp_bridge_injection: bool,
}

impl Provider {
    /// Every provider compiled into TermHerd, in display order.
    pub const ALL: [Self; 3] = [Self::Claude, Self::Codex, Self::Antigravity];

    /// Stable lowercase identifier used in snapshots, layout persistence, and
    /// MCP payloads.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Antigravity => "antigravity",
        }
    }

    /// Human-readable label shown in tooltips and hover cards.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::Antigravity => "Antigravity",
        }
    }

    /// Executable stem probed on `$PATH` at startup.
    #[must_use]
    pub const fn binary_name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Antigravity => "agy",
        }
    }

    /// Parse a stable lowercase provider identifier.
    #[must_use]
    pub fn parse(id: &str) -> Option<Self> {
        match id {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "antigravity" => Some(Self::Antigravity),
            _ => None,
        }
    }

    /// The capability descriptor for this provider.
    #[must_use]
    pub const fn capabilities(self) -> ProviderCapabilities {
        match self {
            Self::Claude => ProviderCapabilities {
                launch_supported: true,
                mint_session_id: true,
                resume_by_id: true,
                history_discovery: true,
                live_activity: ActivitySupport::Full,
                native_rename: RenameSupport::SlashCommand,
                native_color: true,
                slash_commands: true,
                agent_name: true,
                send_to_desktop: true,
                reveal_transcript: true,
                mcp_bridge_injection: true,
            },
            Self::Codex => ProviderCapabilities {
                launch_supported: false,
                mint_session_id: false,
                resume_by_id: true,
                history_discovery: true,
                live_activity: ActivitySupport::ProcessOnly,
                native_rename: RenameSupport::LocalTabOnly,
                native_color: false,
                slash_commands: false,
                agent_name: false,
                send_to_desktop: false,
                reveal_transcript: true,
                mcp_bridge_injection: false,
            },
            Self::Antigravity => ProviderCapabilities {
                launch_supported: true,
                mint_session_id: false,
                resume_by_id: true,
                history_discovery: true,
                live_activity: ActivitySupport::ProcessOnly,
                native_rename: RenameSupport::LocalTabOnly,
                native_color: false,
                slash_commands: false,
                agent_name: false,
                send_to_desktop: false,
                reveal_transcript: true,
                mcp_bridge_injection: false,
            },
        }
    }
}

/// Which agent CLIs were detected on the host at startup. Constructed once in
/// `main()` and injected into the shell so `core` stays pure of filesystem and
/// environment reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvailableProviders {
    claude: bool,
    codex: bool,
    antigravity: bool,
}

impl Default for AvailableProviders {
    fn default() -> Self {
        Self {
            claude: true,
            codex: false,
            antigravity: false,
        }
    }
}

impl AvailableProviders {
    /// Construct an explicit availability snapshot (used by startup detection
    /// and tests).
    #[must_use]
    pub const fn new(claude: bool, codex: bool, antigravity: bool) -> Self {
        Self {
            claude,
            codex,
            antigravity,
        }
    }

    /// Whether `provider`'s CLI was detected on the host.
    #[must_use]
    pub const fn is_available(self, provider: Provider) -> bool {
        match provider {
            Provider::Claude => self.claude,
            Provider::Codex => self.codex,
            Provider::Antigravity => self.antigravity,
        }
    }

    /// Every provider that is both installed on this host and wired for launch.
    #[must_use]
    pub fn launchable(self) -> Vec<Provider> {
        Provider::ALL
            .into_iter()
            .filter(|&provider| {
                self.is_available(provider) && provider.capabilities().launch_supported
            })
            .collect()
    }

    /// Launchable providers other than `primary`, in display order. Drives the
    /// sidebar's secondary agent picker: empty means no picker arrow is shown.
    #[must_use]
    pub fn secondary_launchable(self, primary: Provider) -> Vec<Provider> {
        self.launchable()
            .into_iter()
            .filter(|&provider| provider != primary)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_provider_round_trips_through_id_and_parse() {
        for provider in Provider::ALL {
            assert_eq!(Provider::parse(provider.id()), Some(provider));
            assert!(!provider.display_name().is_empty());
        }
        assert_eq!(Provider::parse("unknown"), None);
        assert_eq!(Provider::parse("Claude"), None);
    }

    #[test]
    fn claude_retains_full_capabilities() {
        let caps = Provider::Claude.capabilities();
        assert!(caps.mint_session_id);
        assert!(caps.resume_by_id);
        assert!(caps.history_discovery);
        assert_eq!(caps.live_activity, ActivitySupport::Full);
        assert_eq!(caps.native_rename, RenameSupport::SlashCommand);
        assert!(caps.native_color);
        assert!(caps.slash_commands);
        assert!(caps.agent_name);
        assert!(caps.send_to_desktop);
        assert!(caps.reveal_transcript);
        assert!(caps.mcp_bridge_injection);
    }

    #[test]
    fn codex_and_antigravity_do_not_claim_unverified_capabilities() {
        for provider in [Provider::Codex, Provider::Antigravity] {
            let caps = provider.capabilities();
            assert!(!caps.mint_session_id);
            assert!(caps.resume_by_id);
            assert_eq!(caps.live_activity, ActivitySupport::ProcessOnly);
            assert_eq!(caps.native_rename, RenameSupport::LocalTabOnly);
            assert!(!caps.native_color);
            assert!(!caps.slash_commands);
            assert!(!caps.agent_name);
            assert!(!caps.send_to_desktop);
            assert!(!caps.mcp_bridge_injection);
        }
        assert!(Provider::Antigravity.capabilities().launch_supported);
        assert!(!Provider::Codex.capabilities().launch_supported);
    }

    #[test]
    fn available_providers_filters_to_installed_and_launch_supported() {
        let all_installed = AvailableProviders::new(true, true, true);
        assert_eq!(
            all_installed.launchable(),
            vec![Provider::Claude, Provider::Antigravity]
        );
        assert_eq!(
            all_installed.secondary_launchable(Provider::Claude),
            vec![Provider::Antigravity]
        );

        let claude_only = AvailableProviders::default();
        assert_eq!(claude_only.launchable(), vec![Provider::Claude]);
        assert!(
            claude_only
                .secondary_launchable(Provider::Claude)
                .is_empty()
        );
    }
}
