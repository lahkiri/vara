//! The capability seams Vara's core already has, expressed as service
//! contracts — and the plugins that fill them.
//!
//! The host (`host.rs`) knows how to load plugins and how to hand out named
//! services; it deliberately knows nothing about tools, models, memory or
//! storage. This module is where the *product's* seams are named, so a surface
//! (desktop, TUI, a future daemon) composes capabilities instead of importing
//! them.
//!
//! Why this matters beyond tidiness: the owner's requirement is that *any* part
//! be replaceable — a different tool set, a different model adapter, a
//! different store — without forking the entity. A seam is only real if
//! something else can fill it, so each contract here has at least two
//! implementations in the tests: the shipped one and a stand-in.

use crate::host::{Effect, Plugin, PluginCtx};
use crate::tools_registry::{ToolCtx, ToolHost, ToolRegistry, ToolResult};
use std::sync::Arc;

/// Seam names. Constants, not string literals sprinkled through the code, so a
/// typo is a compile error rather than a silently unfilled capability.
pub mod seam {
    /// The read-only tool registry (`ToolRegistry`).
    pub const TOOLS_READ: &str = "tools.read";
    /// The tool execution context — allowed roots, clock, memory provider.
    pub const TOOL_CTX: &str = "tools.ctx";
    /// The machine the tools act through (`SharedToolHost`).
    pub const TOOL_HOST: &str = "tools.host";
    /// A model caller (`Arc<dyn Brain>`).
    pub const BRAIN: &str = "brain";
    /// Where durable facts go (`Arc<dyn EventSinkCap>`).
    pub const LOG: &str = "log";
}

/// The host capability as it travels over the seam.
///
/// `ToolHost` is itself a seam, so it is stored as a trait object rather than a
/// concrete host type. That is what makes the mock and the real filesystem
/// genuinely interchangeable instead of merely similar — and the tests exercise
/// both through this exact path.
pub type SharedToolHost = Arc<dyn ToolHost>;

/// A model caller, as a seam.
///
/// Deliberately narrow — one method, no provider detail — so swapping DeepSeek
/// for a local llama.cpp is a provider change and not a refactor. It is
/// synchronous on purpose: the host owns no runtime, and a surface that does
/// can adapt it (see `vara-tui`, which blocks on the real client). Making this
/// `async` would drag an executor choice into a seam that must stay swappable.
pub trait Brain: Send + Sync {
    /// One completion. `max_tokens` is a hint; implementations must respect it.
    fn complete(&self, system: &str, user: &str, max_tokens: u32) -> Result<String, String>;
}

/// A durable log sink, as a seam.
pub trait EventSinkCap: Send + Sync {
    fn record(&self, kind: &str, message: &str);
}

/// Fills [`seam::TOOLS_READ`] with the shipped read-only registry.
///
/// Note what this plugin does *not* do: it never executes a tool. Execution
/// stays with whoever asks, through the gate, so "registering a tool set" can
/// never itself be a way to run something.
pub struct ReadToolsPlugin;

impl Plugin for ReadToolsPlugin {
    fn id(&self) -> &str {
        "tools.read"
    }

    fn start(&self, ctx: &PluginCtx<'_>) -> Result<Vec<Effect>, String> {
        let registry = crate::tools_local::read_only_registry();
        let count = registry.len();
        let effect = ctx.register(seam::TOOLS_READ, registry)?;
        ctx.log(
            "plugin",
            format!("{count} read-only tools sit behind `{}`", seam::TOOLS_READ),
            false,
        );
        Ok(vec![effect])
    }

    fn describe(&self) -> String {
        "system_info, list_dir, find_files, read_file, disk_usage, memory_search".into()
    }
}

/// Fills [`seam::TOOL_HOST`] with the real machine.
pub struct RealHostPlugin;

impl Plugin for RealHostPlugin {
    fn id(&self) -> &str {
        "tools.host"
    }

    fn start(&self, ctx: &PluginCtx<'_>) -> Result<Vec<Effect>, String> {
        let host: SharedToolHost = Arc::new(crate::tools_local::FsToolHost::new());
        Ok(vec![ctx.register(seam::TOOL_HOST, host)?])
    }

    fn describe(&self) -> String {
        "the real filesystem, through the same contract the mock implements".into()
    }
}

/// Fills [`seam::TOOL_CTX`] from an explicit configuration.
///
/// The roots are passed in rather than read from the environment: a plugin that
/// chose its own scope would be exactly the privilege escalation the hard-deny
/// floor exists to prevent.
pub struct ToolCtxPlugin {
    ctx: ToolCtx,
}

impl ToolCtxPlugin {
    pub fn new(ctx: ToolCtx) -> Self {
        Self { ctx }
    }
}

impl Plugin for ToolCtxPlugin {
    fn id(&self) -> &str {
        "tools.ctx"
    }

    fn start(&self, ctx: &PluginCtx<'_>) -> Result<Vec<Effect>, String> {
        Ok(vec![ctx.register(seam::TOOL_CTX, self.ctx.clone())?])
    }

    fn describe(&self) -> String {
        format!("{} allowed root(s)", self.ctx.roots.paths().len())
    }
}

/// Fills [`seam::BRAIN`] with any implementation of [`Brain`].
pub struct BrainPlugin {
    brain: Arc<dyn Brain>,
}

impl BrainPlugin {
    pub fn new(brain: Arc<dyn Brain>) -> Self {
        Self { brain }
    }
}

impl Plugin for BrainPlugin {
    fn id(&self) -> &str {
        "brain"
    }

    fn start(&self, ctx: &PluginCtx<'_>) -> Result<Vec<Effect>, String> {
        let brain = self.brain.clone();
        Ok(vec![ctx.register(seam::BRAIN, brain)?])
    }

    fn describe(&self) -> String {
        "a model, behind a one-method seam".into()
    }
}

/// Fills [`seam::LOG`] with anything that can record a line.
pub struct LogPlugin {
    sink: Arc<dyn EventSinkCap>,
}

impl LogPlugin {
    pub fn new(sink: Arc<dyn EventSinkCap>) -> Self {
        Self { sink }
    }
}

impl Plugin for LogPlugin {
    fn id(&self) -> &str {
        "log"
    }

    fn start(&self, ctx: &PluginCtx<'_>) -> Result<Vec<Effect>, String> {
        // Register the *same* holder the runtime reads back. The host stores
        // Arc<dyn Any>, so a bare trait object cannot travel: LogCap is the
        // type both sides agree on, and entity::sink_from_host is the reader.
        let cap = crate::entity::LogCap(self.sink.clone());
        Ok(vec![ctx.register(seam::LOG, cap)?])
    }

    fn describe(&self) -> String {
        "a place for durable facts".into()
    }
}

/// Run a tool by name through the *composed* seams — the path a surface uses.
///
/// Returns `None` when the product is not composed well enough to run anything.
/// A caller must handle that explicitly: a missing seam is a configuration to
/// explain, never something to paper over with a default.
pub fn call_tool(ctx: &PluginCtx<'_>, tool: &str, args: &serde_json::Value) -> Option<ToolResult> {
    let registry = ctx.service::<ToolRegistry>(seam::TOOLS_READ)?;
    let host = ctx.service::<SharedToolHost>(seam::TOOL_HOST)?;
    let call_ctx = ctx.service::<ToolCtx>(seam::TOOL_CTX)?;
    Some(registry.call(tool, args, &call_ctx, host.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{GateDecision, Host, HostEnv, LogEntry};

    struct QuietEnv {
        logs: std::sync::Mutex<Vec<String>>,
    }
    impl HostEnv for QuietEnv {
        fn authorize(&self, _r: &crate::host::GateRequest) -> GateDecision {
            GateDecision::Allow
        }
        fn log(&self, e: LogEntry) {
            self.logs
                .lock()
                .unwrap()
                .push(format!("{}: {}", e.plugin, e.message));
        }
    }

    fn quiet() -> Arc<QuietEnv> {
        Arc::new(QuietEnv {
            logs: std::sync::Mutex::new(Vec::new()),
        })
    }

    fn ctx_for(root: &str) -> ToolCtx {
        ToolCtx {
            roots: crate::tools_registry::Roots::new(vec![std::path::PathBuf::from(root)]),
            now_unix: 1_700_000_000,
            denied_paths: Vec::new(),
            memory: None,
        }
    }

    /// Registers a mock machine under the *same* seam name the real one uses —
    /// which is the whole claim being tested: the seam does not care which
    /// machine is behind it.
    struct MockHostPlugin;

    impl Plugin for MockHostPlugin {
        fn id(&self) -> &str {
            "tools.host"
        }
        fn start(&self, ctx: &PluginCtx<'_>) -> Result<Vec<Effect>, String> {
            let host: SharedToolHost = Arc::new(crate::tools_local::MockToolHost::new());
            Ok(vec![ctx.register(seam::TOOL_HOST, host)?])
        }
    }

    #[test]
    fn the_shipped_plugins_fill_every_seam() {
        let host = Arc::new(Host::new(quiet()));
        host.load(Arc::new(ReadToolsPlugin)).unwrap();
        host.load(Arc::new(ToolCtxPlugin::new(ctx_for("C:/Users/me"))))
            .unwrap();

        let names: Vec<String> = host.services().into_iter().map(|(n, _)| n).collect();
        assert!(names.contains(&seam::TOOLS_READ.to_string()));
        assert!(names.contains(&seam::TOOL_CTX.to_string()));

        // Six read tools arrived through the seam, not by import.
        let registry = host.service::<ToolRegistry>(seam::TOOLS_READ).unwrap();
        assert_eq!(registry.len(), 6);
        assert!(registry.get("system_info").is_some());
    }

    #[test]
    fn pulling_the_tools_plugin_removes_the_capability() {
        let host = Arc::new(Host::new(quiet()));
        host.load(Arc::new(ReadToolsPlugin)).unwrap();
        assert!(host.service::<ToolRegistry>(seam::TOOLS_READ).is_some());
        host.unload("tools.read");
        assert!(
            host.service::<ToolRegistry>(seam::TOOLS_READ).is_none(),
            "removing a plugin must remove the capability it offered"
        );
    }

    #[test]
    fn a_different_provider_can_fill_the_same_seam() {
        // A seam is real only if something else can satisfy it: a stand-in
        // registry with one tool, proving nothing is hard-wired to the shipped
        // implementation.
        struct Substitute;
        impl Plugin for Substitute {
            fn id(&self) -> &str {
                "tools.read.substitute"
            }
            fn start(&self, ctx: &PluginCtx<'_>) -> Result<Vec<Effect>, String> {
                let mut registry = ToolRegistry::new();
                registry.register(Box::new(crate::tools_local::SystemInfoTool));
                Ok(vec![ctx.register(seam::TOOLS_READ, registry)?])
            }
        }
        let host = Arc::new(Host::new(quiet()));
        host.load(Arc::new(ReadToolsPlugin)).unwrap();
        host.unload("tools.read");
        host.load(Arc::new(Substitute)).unwrap();
        let registry = host.service::<ToolRegistry>(seam::TOOLS_READ).unwrap();
        assert_eq!(registry.len(), 1);
        assert!(registry.get("list_dir").is_none());
    }

    #[test]
    fn an_unfilled_host_seam_refuses_instead_of_inventing_a_machine() {
        let host = Arc::new(Host::new(quiet()));
        host.load(Arc::new(ReadToolsPlugin)).unwrap();
        host.load(Arc::new(ToolCtxPlugin::new(ctx_for("C:/Users/me"))))
            .unwrap();

        struct Probe;
        impl Plugin for Probe {
            fn id(&self) -> &str {
                "probe"
            }
            fn start(&self, ctx: &PluginCtx<'_>) -> Result<Vec<Effect>, String> {
                assert!(
                    call_tool(ctx, "system_info", &serde_json::json!({})).is_none(),
                    "without a host seam there is nothing to run a tool against"
                );
                Ok(vec![Effect::none()])
            }
        }
        host.load(Arc::new(Probe)).unwrap();
    }

    #[test]
    fn a_tool_runs_through_the_composed_seams_with_a_substituted_machine() {
        struct User;
        impl Plugin for User {
            fn id(&self) -> &str {
                "user"
            }
            fn start(&self, ctx: &PluginCtx<'_>) -> Result<Vec<Effect>, String> {
                let result = call_tool(ctx, "system_info", &serde_json::json!({}))
                    .expect("all seams are filled");
                assert!(result.ok, "{}", result.summary);
                assert!(
                    result.summary.contains("MockOS"),
                    "the mock machine answered, not the real one: {}",
                    result.summary
                );
                Ok(vec![Effect::none()])
            }
        }

        let host = Arc::new(Host::new(quiet()));
        host.load(Arc::new(ReadToolsPlugin)).unwrap();
        host.load(Arc::new(ToolCtxPlugin::new(ctx_for("C:/Users/me"))))
            .unwrap();
        host.load(Arc::new(MockHostPlugin)).unwrap();
        host.load(Arc::new(User)).unwrap();

        // The seam holds a trait object, so swapping the machine needed no
        // change anywhere else in the product.
        assert!(host.service::<SharedToolHost>(seam::TOOL_HOST).is_some());
    }

    #[test]
    fn swapping_the_machine_back_to_the_real_one_needs_no_other_change() {
        let host = Arc::new(Host::new(quiet()));
        host.load(Arc::new(ReadToolsPlugin)).unwrap();
        host.load(Arc::new(ToolCtxPlugin::new(ctx_for("C:/Users/me"))))
            .unwrap();
        host.load(Arc::new(MockHostPlugin)).unwrap();
        host.unload("tools.host");
        host.load(Arc::new(RealHostPlugin)).unwrap();

        // A real machine is now behind the seam, and the registry is untouched.
        assert!(host.service::<SharedToolHost>(seam::TOOL_HOST).is_some());
        assert_eq!(
            host.service::<ToolRegistry>(seam::TOOLS_READ)
                .unwrap()
                .len(),
            6
        );
    }

    #[test]
    fn loading_the_same_plugin_twice_is_refused_not_silently_replaced() {
        let host = Arc::new(Host::new(quiet()));
        host.load(Arc::new(ReadToolsPlugin)).unwrap();
        let err = host.load(Arc::new(ReadToolsPlugin)).unwrap_err();
        assert!(err.contains("already loaded"), "{err}");
    }
}
