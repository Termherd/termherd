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
    }
}
