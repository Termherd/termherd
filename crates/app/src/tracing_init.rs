//! Tracing bootstrap — the one logging stack (Q3), initialised once at
//! startup. Split out of `main` so the default filter and its parse guard
//! live together.

use tracing_subscriber::EnvFilter;

/// Default tracing filter: our crates at `info`; the iced/wgpu/winit stack
/// pinned to `warn` because it dumps verbose `info` startup blocks (full
/// `WindowAttributes`, compositor settings, adapter lists) through `tracing`,
/// which otherwise floods the terminal. `RUST_LOG` overrides this when set.
const DEFAULT_FILTER: &str = "info,\
    iced_winit=warn,iced_wgpu=warn,wgpu_core=warn,wgpu_hal=warn,\
    naga=warn,cosmic_text=warn,winit=warn";

pub fn init_tracing() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

#[cfg(test)]
mod tests {
    use super::DEFAULT_FILTER;
    use tracing_subscriber::EnvFilter;

    #[test]
    fn default_filter_parses_cleanly() {
        // A typo would make `EnvFilter` silently drop the bad directive and
        // re-enable the dependency flood; fail the build instead.
        let filter = EnvFilter::builder().parse(DEFAULT_FILTER);
        assert!(filter.is_ok(), "DEFAULT_FILTER must be valid: {filter:?}");
    }
}
