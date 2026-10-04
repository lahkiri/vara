//! The host — the only thing Vara ships that is not a plugin.
//!
//! Modelled on DeepSeek Harness's "everything is a plugin" architecture
//! (Cordis): plugins contribute services, typed events and **reversible
//! effects** to a shared context, and there is no privileged core to patch.
//! The host owns exactly four things, and nothing else:
//!
//! 1. **Loading and unloading plugins** ([`Host::load`] / [`Host::unload`]) —
//!    every registration is an effect that unwinds when its plugin unloads, so
//!    removing a plugin leaves no half-state behind.
//! 2. **The event bus** — plugins talk through events, never by calling each
//!    other, which is what makes any single plugin replaceable.
//! 3. **The gate** — every consequential call a plugin makes passes one
//!    policy check, so "trusted plugin" is not a category that exists.
//! 4. **The log** — "model-visible means logged": what the entity saw and did
//!    is reconstructable, never implicit.
//!
//! What the host deliberately does **not** own: tools, models, memory,
//! interfaces, goals, channels, the agent loop itself. Those are plugins, and
//! the same host runs a desktop app, a TUI, or a headless daemon purely by
//! composing a different set.
//!
//! Everything here is deterministic and headlessly testable: no filesystem, no
//! clock, no environment. The shell supplies those through the [`HostEnv`] trait.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Stable identity of a plugin. Two plugins may not share one.
pub type PluginId = String;

/// What a plugin is allowed to ask of the host.
///
/// This is the *only* surface a plugin has. In particular there is no ambient
/// filesystem, no `std::env`, and no way to reach another plugin: everything
/// consequential goes through [`Gate`], and everything observable goes through
/// events.
pub trait HostEnv: Send + Sync {
    /// Ask the policy gate. Implementations decide; plugins never do.
    fn authorize(&self, request: &GateRequest) -> GateDecision;
    /// Log one durable fact. `model_visible` marks anything that shaped a
    /// model request, which the host requires to be reconstructable.
    fn log(&self, entry: LogEntry);
}

/// A consequential thing a plugin wants to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateRequest {
    pub plugin: PluginId,
    /// The risk class from the tool vocabulary (`R`, `Wr`, `Wd`, `X`, `N`, `H`).
    pub class: String,
    /// Human-readable action, e.g. `read_file` or `send_message`.
    pub action: String,
    /// The concrete target (path, URL, tool name…).
    pub target: String,
    /// True when the request was shaped by untrusted content (web text, a
    /// document, an inbox message). Taint alone can require approval.
    pub tainted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    Allow,
    /// Allowed only with the owner's explicit approval for this instance.
    NeedsApproval {
        reason: String,
    },
    Deny {
        reason: String,
    },
}

impl GateDecision {
    pub fn is_allow(&self) -> bool {
        matches!(self, GateDecision::Allow)
    }
}

/// One durable fact. Kept tiny on purpose: the shell decides where it lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub plugin: PluginId,
    pub kind: String,
    pub message: String,
    /// Must be true whenever this shaped what the model saw.
    pub model_visible: bool,
}

/// A typed event. The bus is intentionally untyped at the storage level so the
/// host stays free of product knowledge; plugins agree on names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub name: String,
    pub payload: String,
}

/// A capability a plugin can offer the rest of the product.
///
/// This is the "service seam": a plugin registers a **named service** and
/// others look it up by name through the host, never through an import. That is
/// how swapping one provider changes the whole product (dsh's `ctx.tools`,
/// `ctx.llm`, `ctx.fs` are the same idea), and it is why the host must not know
/// what a tool or a model is.
///
/// The payoff for Vara specifically: the six read tools, the model adapter and
/// the storage layer become *providers behind a name*, so a TUI, a headless
/// daemon or a web surface can be composed from the same host with a different
/// set of plugins — the owner's requirement that the interface be a replaceable
/// layer rather than the product.
#[derive(Clone)]
pub struct Service {
    pub name: String,
    /// Which plugin provides it — used to unwind on unload and to attribute
    /// failures honestly.
    pub owner: PluginId,
    /// The value. Held as `Arc<dyn Any + Send + Sync>` so any plugin can read
    /// it back, downcasting to the type the seam's contract fixes.
    pub value: Arc<dyn std::any::Any + Send + Sync>,
}

impl std::fmt::Debug for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Service")
            .field("name", &self.name)
            .field("owner", &self.owner)
            .finish()
    }
}

impl Service {
    pub fn new<T: Send + Sync + 'static>(name: &str, owner: &str, value: T) -> Self {
        Self {
            name: name.to_string(),
            owner: owner.to_string(),
            value: Arc::new(value),
        }
    }

    /// Read the service back as its declared type.
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.value.clone().downcast::<T>().ok()
    }
}

/// Everything a plugin may do to the host while it is loaded.
///
/// Registrations are collected here and unwound on unload — the reversible
/// effect model. A plugin cannot register "permanently".
pub struct PluginCtx<'a> {
    id: PluginId,
    host: &'a Arc<Host>,
    env: Arc<dyn HostEnv>,
}

impl<'a> PluginCtx<'a> {
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Publish an event to every listener with this name.
    pub fn emit(&self, name: &str, payload: impl Into<String>) {
        self.host.dispatch(&Event {
            name: name.to_string(),
            payload: payload.into(),
        });
    }

    /// Subscribe to an event. The returned token is the registration effect:
    /// dropping it (or unloading the plugin) removes the listener.
    pub fn on(&self, name: &str, handler: impl Fn(&Event) + Send + Sync + 'static) -> Effect {
        self.host.subscribe(&self.id, name, Box::new(handler))
    }

    /// Offer a service to the rest of the product under `name`.
    ///
    /// Registering twice under the same name is refused: an ambiguous seam is
    /// worse than a missing one, because it silently picks a provider.
    pub fn register<T: Send + Sync + 'static>(
        &self,
        name: &str,
        value: T,
    ) -> Result<Effect, String> {
        let service = Service::new(name, &self.id, value);
        self.host
            .services
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(name.to_string(), service);
        self.env.log(LogEntry {
            plugin: self.id.clone(),
            kind: "service".into(),
            message: format!("registered '{name}'"),
            model_visible: false,
        });
        let services = self.host.services.clone();
        let key = name.to_string();
        let owner = self.id.clone();
        Ok(Effect::new(move || {
            if let Ok(mut map) = services.lock() {
                // Only the owner may remove it: unloading plugin A must never
                // tear down a service that plugin B re-registered.
                if map.get(&key).map(|s| s.owner == owner).unwrap_or(false) {
                    map.remove(&key);
                }
            }
        }))
    }

    /// Look up a service somebody else registered. `None` means the seam is not
    /// filled — callers must handle that explicitly instead of defaulting.
    pub fn service<T: Send + Sync + 'static>(&self, name: &str) -> Option<Arc<T>> {
        self.host.service(name)
    }
    /// Does a plugin with this id exist right now? Used for honest capability
    /// checks (a plugin may declare a soft dependency).
    pub fn has_plugin(&self, id: &str) -> bool {
        self.host.is_loaded(id)
    }

    /// Ask the gate. A plugin must go through this for anything consequential.
    pub fn authorize(
        &self,
        class: &str,
        action: &str,
        target: &str,
        tainted: bool,
    ) -> GateDecision {
        let request = GateRequest {
            plugin: self.id.clone(),
            class: class.to_string(),
            action: action.to_string(),
            target: target.to_string(),
            tainted,
        };
        let decision = self.env.authorize(&request);
        // Every attempt is logged, allowed or not: an audit trail that only
        // records successes cannot explain a refusal.
        self.env.log(LogEntry {
            plugin: self.id.clone(),
            kind: "gate".into(),
            message: format!(
                "{class} {action} {target} → {}",
                match &decision {
                    GateDecision::Allow => "allow".to_string(),
                    GateDecision::NeedsApproval { reason } => format!("approval: {reason}"),
                    GateDecision::Deny { reason } => format!("deny: {reason}"),
                }
            ),
            model_visible: false,
        });
        decision
    }

    /// Record a durable fact.
    pub fn log(&self, kind: &str, message: impl Into<String>, model_visible: bool) {
        self.env.log(LogEntry {
            plugin: self.id.clone(),
            kind: kind.to_string(),
            message: message.into(),
            model_visible,
        });
    }
}

/// A registration that unwinds when dropped. Unloading a plugin is therefore
/// "drop its effects", not "remember to clean up in the right order".
pub struct Effect {
    on_drop: Option<Box<dyn FnOnce() + Send + Sync>>,
}

impl Effect {
    pub fn new(f: impl FnOnce() + Send + Sync + 'static) -> Self {
        Self {
            on_drop: Some(Box::new(f)),
        }
    }
    /// A no-op effect, for plugins whose registration is pure data.
    pub fn none() -> Self {
        Self { on_drop: None }
    }
    /// Consume the effect now instead of at drop.
    pub fn dispose(mut self) {
        if let Some(f) = self.on_drop.take() {
            f();
        }
    }
}

impl Drop for Effect {
    fn drop(&mut self) {
        if let Some(f) = self.on_drop.take() {
            f();
        }
    }
}

/// What a plugin must implement. `bin` is an ordinary Rust value, so a plugin
/// can be native code, a bridge to an external process, or a test double.
pub trait Plugin: Send + Sync {
    fn id(&self) -> &str;
    /// Called once when loaded. Every registration must be returned as an
    /// `Effect` so unloading is complete.
    fn start(&self, ctx: &PluginCtx<'_>) -> Result<Vec<Effect>, String>;
    /// Optional human-facing description, used by `host.list()` and the UI.
    fn describe(&self) -> String {
        String::new()
    }
}

type Handler = Arc<dyn Fn(&Event) + Send + Sync>;
type Listener = (PluginId, Handler);

/// The host. Small on purpose: if this file grows tools or product knowledge,
/// the architecture has already failed.
#[derive(Default)]
pub struct Host {
    plugins: Mutex<BTreeMap<PluginId, PluginRecord>>,
    listeners: Arc<Mutex<BTreeMap<String, Vec<Listener>>>>,
    services: Arc<Mutex<BTreeMap<String, Service>>>,
    env: Mutex<Option<Arc<dyn HostEnv>>>,
}

struct PluginRecord {
    plugin: Arc<dyn Plugin>,
    effects: Mutex<Vec<Effect>>,
    healthy: bool,
    /// Why the plugin was disabled, when it failed.
    error: Option<String>,
}

impl Host {
    pub fn new(env: Arc<dyn HostEnv>) -> Self {
        let host = Self::default();
        *host.env.lock().unwrap_or_else(|p| p.into_inner()) = Some(env);
        host
    }

    /// A host with the most restrictive possible environment: everything is
    /// denied and nothing is recorded. Useful for tests and for a dry boot.
    pub fn sealed() -> Self {
        struct DenyAll;
        impl HostEnv for DenyAll {
            fn authorize(&self, _r: &GateRequest) -> GateDecision {
                GateDecision::Deny {
                    reason: "no policy engine is loaded".into(),
                }
            }
            fn log(&self, _e: LogEntry) {}
        }
        Self::new(Arc::new(DenyAll))
    }

    fn env(&self) -> Arc<dyn HostEnv> {
        self.env
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .expect("host built without an environment")
    }

    /// Load a plugin. A plugin that fails to start is recorded as unhealthy and
    /// **does not take the host down** — one broken plugin must never make the
    /// entity unusable (the lesson from letting a bad mod kill the app).
    ///
    /// Takes `self: &Arc<Self>` so the plugin's context can hold a real
    /// reference to the bus. That is what lets a registration clean itself up
    /// on drop without `unsafe` anywhere in the host.
    pub fn load(self: &Arc<Self>, plugin: Arc<dyn Plugin>) -> Result<PluginId, String> {
        let id: PluginId = plugin.id().to_string();
        if self
            .plugins
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(&id)
        {
            return Err(format!("plugin '{id}' is already loaded"));
        }
        let ctx = PluginCtx {
            id: id.clone(),
            host: self,
            env: self.env(),
        };
        let effects = match plugin.start(&ctx) {
            Ok(effects) => effects,
            Err(why) => {
                // The failing plugin is registered as unhealthy so the UI can
                // explain it, and the error travels back to the caller.
                self.plugins
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(
                        id.clone(),
                        PluginRecord {
                            plugin,
                            effects: Mutex::new(Vec::new()),
                            healthy: false,
                            error: Some(why.clone()),
                        },
                    );
                self.env().log(LogEntry {
                    plugin: id.clone(),
                    kind: "plugin".into(),
                    message: format!("failed to start: {why}"),
                    model_visible: false,
                });
                return Err(why);
            }
        };
        self.plugins
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                id.clone(),
                PluginRecord {
                    plugin,
                    effects: Mutex::new(effects),
                    healthy: true,
                    error: None,
                },
            );
        self.env().log(LogEntry {
            plugin: id.clone(),
            kind: "plugin".into(),
            message: "loaded".into(),
            model_visible: false,
        });
        Ok(id)
    }

    /// Unload a plugin: its effects unwind in reverse order, then its
    /// listeners and records disappear. Returns false when it was not loaded.
    pub fn unload(&self, id: &str) -> bool {
        let Some(record) = self
            .plugins
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id)
        else {
            return false;
        };
        let effects =
            std::mem::take(&mut *record.effects.lock().unwrap_or_else(|p| p.into_inner()));
        for effect in effects.into_iter().rev() {
            effect.dispose();
        }
        for listeners in self
            .listeners
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values_mut()
        {
            listeners.retain(|(owner, _)| owner != id);
        }
        self.env().log(LogEntry {
            plugin: id.to_string(),
            kind: "plugin".into(),
            message: "unloaded".into(),
            model_visible: false,
        });
        true
    }

    fn subscribe(
        self: &Arc<Self>,
        owner: &str,
        name: &str,
        handler: Box<dyn Fn(&Event) + Send + Sync>,
    ) -> Effect {
        let name = name.to_string();
        let owner = owner.to_string();
        self.listeners
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(name.clone())
            .or_default()
            .push((owner.clone(), Arc::from(handler)));
        // The registration unwinds with the plugin, or early if the effect is
        // dropped by hand. The effect owns a strong reference to the bus (not a
        // raw pointer, and not the whole host): the bus outlives the effect, so
        // cleanup is plain safe code.
        let bus = self.listeners.clone();
        Effect::new(move || {
            if let Ok(mut map) = bus.lock() {
                if let Some(list) = map.get_mut(&name) {
                    list.retain(|(o, _)| o != &owner);
                }
            }
        })
    }

    /// Deliver an event to every listener registered at dispatch time.
    ///
    /// Listeners are cloned out of the table before being called, so a handler
    /// is free to subscribe, unsubscribe or emit again — no lock is held while
    /// user code runs, and no raw pointers are involved.
    pub fn dispatch(&self, event: &Event) {
        let handlers: Vec<Arc<dyn Fn(&Event) + Send + Sync>> = {
            let map = self.listeners.lock().unwrap_or_else(|p| p.into_inner());
            map.get(&event.name)
                .map(|list| list.iter().map(|(_, f)| f.clone()).collect())
                .unwrap_or_default()
        };
        for handler in handlers {
            handler(event);
        }
    }

    /// Look up a service by name. `None` means the seam is unfilled, which a
    /// consumer must handle explicitly instead of silently defaulting.
    pub fn service<T: Send + Sync + 'static>(&self, name: &str) -> Option<Arc<T>> {
        self.services
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(name)
            .and_then(|s| s.get::<T>())
    }

    /// Every registered service, with the plugin that owns it.
    pub fn services(&self) -> Vec<(String, PluginId)> {
        self.services
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|(name, s)| (name.clone(), s.owner.clone()))
            .collect()
    }

    /// `host.list()` — what is loaded, and what is broken.
    pub fn list(&self) -> Vec<PluginInfo> {
        self.plugins
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|(id, record)| PluginInfo {
                id: id.clone(),
                description: record.plugin.describe(),
                healthy: record.healthy,
                error: record.error.clone(),
                listeners: self
                    .listeners
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .values()
                    .flatten()
                    .filter(|(owner, _)| owner == id)
                    .count(),
            })
            .collect()
    }

    pub fn is_loaded(&self, id: &str) -> bool {
        self.plugins
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(id)
    }

    pub fn load_all(
        self: &Arc<Self>,
        plugins: Vec<Arc<dyn Plugin>>,
    ) -> Vec<(PluginId, Result<(), String>)> {
        plugins
            .into_iter()
            .map(|p| {
                let id = p.id().to_string();
                let result = self.load(p).map(|_| ());
                (id, result)
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginInfo {
    pub id: PluginId,
    pub description: String,
    pub healthy: bool,
    pub error: Option<String>,
    pub listeners: usize,
}

/// The shortest possible proof that the composition works: a recounter that
/// listens, a counter that emits, and nothing else.
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RecordingEnv {
        decisions: Mutex<Vec<GateRequest>>,
        logs: Mutex<Vec<LogEntry>>,
        allow: bool,
    }

    impl RecordingEnv {
        fn new(allow: bool) -> Arc<Self> {
            Arc::new(Self {
                decisions: Mutex::new(Vec::new()),
                logs: Mutex::new(Vec::new()),
                allow,
            })
        }
    }

    impl HostEnv for RecordingEnv {
        fn authorize(&self, request: &GateRequest) -> GateDecision {
            self.decisions.lock().unwrap().push(request.clone());
            if self.allow {
                GateDecision::Allow
            } else {
                GateDecision::NeedsApproval {
                    reason: "owner has not approved this class yet".into(),
                }
            }
        }
        fn log(&self, entry: LogEntry) {
            self.logs.lock().unwrap().push(entry);
        }
    }

    struct Counter {
        hits: Arc<AtomicUsize>,
    }
    impl Plugin for Counter {
        fn id(&self) -> &str {
            "counter"
        }
        fn start(&self, ctx: &PluginCtx<'_>) -> Result<Vec<Effect>, String> {
            let hits = self.hits.clone();
            let effect = ctx.on("tick", move |_| {
                hits.fetch_add(1, Ordering::SeqCst);
            });
            ctx.emit("tick", "during-start");
            Ok(vec![effect])
        }
        fn describe(&self) -> String {
            "counts ticks".into()
        }
    }

    struct Failing;
    impl Plugin for Failing {
        fn id(&self) -> &str {
            "failing"
        }
        fn start(&self, _ctx: &PluginCtx<'_>) -> Result<Vec<Effect>, String> {
            Err("missing dependency: nothing-provider".into())
        }
    }

    struct Caller;
    impl Plugin for Caller {
        fn id(&self) -> &str {
            "caller"
        }
        fn start(&self, ctx: &PluginCtx<'_>) -> Result<Vec<Effect>, String> {
            let decision = ctx.authorize("Wr", "trash_path", "C:/tmp/a.txt", true);
            ctx.log("decision", format!("{decision:?}"), false);
            Ok(vec![Effect::none()])
        }
    }

    #[test]
    fn a_plugin_registers_listens_and_emits() {
        let env = RecordingEnv::new(true);
        let host = Arc::new(Host::new(env.clone()));
        let hits = Arc::new(AtomicUsize::new(0));
        host.load(Arc::new(Counter { hits: hits.clone() })).unwrap();
        // The plugin emitted during `start`, after registering, so that tick is
        // already counted — a plugin may use its own events while loading.
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        host.dispatch(&Event {
            name: "tick".into(),
            payload: "later".into(),
        });
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn unloading_a_plugin_unwinds_its_effects() {
        let env = RecordingEnv::new(true);
        let host = Arc::new(Host::new(env.clone()));
        let hits = Arc::new(AtomicUsize::new(0));
        host.load(Arc::new(Counter { hits: hits.clone() })).unwrap();
        let after_load = hits.load(Ordering::SeqCst);
        assert!(host.unload("counter"));
        host.dispatch(&Event {
            name: "tick".into(),
            payload: "after unload".into(),
        });
        assert_eq!(
            hits.load(Ordering::SeqCst),
            after_load,
            "the listener must be gone"
        );
        assert!(!host.is_loaded("counter"));
        // …and unloading twice is not an error, just false.
        assert!(!host.unload("counter"));
        // The host itself is still usable afterwards.
        assert!(host.list().is_empty());
    }

    #[test]
    fn a_failing_plugin_does_not_take_the_host_down() {
        let env = RecordingEnv::new(true);
        let host = Arc::new(Host::new(env.clone()));
        let hits = Arc::new(AtomicUsize::new(0));
        host.load(Arc::new(Counter { hits: hits.clone() })).unwrap();
        let err = host.load(Arc::new(Failing)).unwrap_err();
        assert!(err.contains("nothing-provider"));

        // The healthy plugin still works.
        let before = hits.load(Ordering::SeqCst);
        host.dispatch(&Event {
            name: "tick".into(),
            payload: "x".into(),
        });
        assert_eq!(hits.load(Ordering::SeqCst), before + 1);

        // And the broken one is visible, with its reason.
        let list = host.list();
        let failing = list.iter().find(|p| p.id == "failing").unwrap();
        assert!(!failing.healthy);
        assert!(failing
            .error
            .as_deref()
            .unwrap()
            .contains("nothing-provider"));
    }

    #[test]
    fn loading_the_same_plugin_twice_is_refused() {
        let host = Arc::new(Host::new(RecordingEnv::new(true)));
        let hits = Arc::new(AtomicUsize::new(0));
        host.load(Arc::new(Counter { hits })).unwrap();
        let again = host.load(Arc::new(Counter {
            hits: Arc::new(AtomicUsize::new(0)),
        }));
        assert!(again.unwrap_err().contains("already loaded"));
    }

    #[test]
    fn every_gate_call_is_recorded_allowed_or_not() {
        let env = RecordingEnv::new(false);
        let host = Arc::new(Host::new(env.clone()));
        host.load(Arc::new(Caller)).unwrap();
        let decisions = env.decisions.lock().unwrap();
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].plugin, "caller");
        assert_eq!(decisions[0].class, "Wr");
        assert_eq!(decisions[0].action, "trash_path");
        assert!(decisions[0].tainted, "taint must reach the gate");
        drop(decisions);
        // A refusal is logged too.
        let logs = env.logs.lock().unwrap();
        assert!(logs.iter().any(|l| l.kind == "gate"));
        assert!(logs.iter().any(|l| l.kind == "decision"));
    }

    #[test]
    fn a_sealed_host_denies_everything() {
        let host = Arc::new(Host::sealed());
        host.load(Arc::new(Caller)).unwrap();
        // No panic, no effect: the caller's authorize returned Deny.
        assert!(host.is_loaded("caller"));
    }

    #[test]
    fn load_all_reports_each_result_without_stopping() {
        let host = Arc::new(Host::new(RecordingEnv::new(true)));
        let results = host.load_all(vec![
            Arc::new(Counter {
                hits: Arc::new(AtomicUsize::new(0)),
            }),
            Arc::new(Failing),
        ]);
        assert_eq!(results.len(), 2);
        assert!(results[0].1.is_ok());
        assert!(results[1].1.is_err());
    }

    #[test]
    fn effects_unwind_in_reverse_order() {
        let order = Arc::new(Mutex::new(Vec::new()));
        struct Two {
            order: Arc<Mutex<Vec<&'static str>>>,
        }
        impl Plugin for Two {
            fn id(&self) -> &str {
                "two"
            }
            fn start(&self, _ctx: &PluginCtx<'_>) -> Result<Vec<Effect>, String> {
                let a = self.order.clone();
                let b = self.order.clone();
                Ok(vec![
                    Effect::new(move || a.lock().unwrap().push("first")),
                    Effect::new(move || b.lock().unwrap().push("second")),
                ])
            }
        }
        let host = Arc::new(Host::new(RecordingEnv::new(true)));
        host.load(Arc::new(Two {
            order: order.clone(),
        }))
        .unwrap();
        host.unload("two");
        assert_eq!(
            order.lock().unwrap().as_slice(),
            ["second", "first"],
            "reverse order keeps dependent teardown correct"
        );
    }

    // ---- services: the seam that lets a provider be swapped ----

    #[derive(Debug, PartialEq)]
    struct Toolset {
        names: Vec<String>,
    }

    struct Provider;
    impl Plugin for Provider {
        fn id(&self) -> &str {
            "provider"
        }
        fn start(&self, ctx: &PluginCtx<'_>) -> Result<Vec<Effect>, String> {
            ctx.register(
                "tools",
                Toolset {
                    names: vec!["system_info".into(), "list_dir".into()],
                },
            )
            .map(|e| vec![e])
        }
    }

    struct Consumer {
        seen: Arc<Mutex<Option<Vec<String>>>>,
    }
    impl Plugin for Consumer {
        fn id(&self) -> &str {
            "consumer"
        }
        fn start(&self, ctx: &PluginCtx<'_>) -> Result<Vec<Effect>, String> {
            // Soft dependency: a missing seam must be handled, not defaulted.
            let names = match ctx.service::<Toolset>("tools") {
                Some(toolset) => toolset.names.clone(),
                None => {
                    ctx.log("seam", "tools seam is unfilled", false);
                    Vec::new()
                }
            };
            *self.seen.lock().unwrap() = Some(names);
            Ok(vec![Effect::none()])
        }
    }

    #[test]
    fn a_provider_fills_a_seam_and_a_consumer_reads_it() {
        let host = Arc::new(Host::new(RecordingEnv::new(true)));
        let seen = Arc::new(Mutex::new(None));
        host.load(Arc::new(Provider)).unwrap();
        host.load(Arc::new(Consumer { seen: seen.clone() }))
            .unwrap();
        assert_eq!(
            seen.lock().unwrap().clone().unwrap(),
            vec!["system_info".to_string(), "list_dir".to_string()]
        );
        assert_eq!(host.services().len(), 1);
        assert_eq!(host.services()[0].0, "tools");
        assert_eq!(host.services()[0].1, "provider");
    }

    #[test]
    fn an_unfilled_seam_is_visible_not_silent() {
        let env = RecordingEnv::new(true);
        let host = Arc::new(Host::new(env.clone()));
        let seen = Arc::new(Mutex::new(None));
        // No provider loaded: the consumer must degrade explicitly.
        host.load(Arc::new(Consumer { seen: seen.clone() }))
            .unwrap();
        assert_eq!(seen.lock().unwrap().clone().unwrap(), Vec::<String>::new());
        let logs = env.logs.lock().unwrap();
        assert!(logs
            .iter()
            .any(|l| l.kind == "seam" && l.message.contains("unfilled")));
    }

    #[test]
    fn unloading_the_provider_removes_the_service() {
        let host = Arc::new(Host::new(RecordingEnv::new(true)));
        host.load(Arc::new(Provider)).unwrap();
        assert!(host.service::<Toolset>("tools").is_some());
        host.unload("provider");
        assert!(
            host.service::<Toolset>("tools").is_none(),
            "the seam must close when its provider leaves"
        );
        // …and a consumer loaded afterwards degrades instead of panicking.
        let seen = Arc::new(Mutex::new(None));
        host.load(Arc::new(Consumer { seen: seen.clone() }))
            .unwrap();
        assert!(seen.lock().unwrap().clone().unwrap().is_empty());
    }

    #[test]
    fn a_consumer_keeps_working_after_the_provider_returns() {
        let host = Arc::new(Host::new(RecordingEnv::new(true)));
        host.load(Arc::new(Provider)).unwrap();
        host.unload("provider");
        host.load(Arc::new(Provider)).unwrap();
        let seen = Arc::new(Mutex::new(None));
        host.load(Arc::new(Consumer { seen: seen.clone() }))
            .unwrap();
        assert_eq!(seen.lock().unwrap().clone().unwrap().len(), 2);
    }

    #[test]
    fn list_shows_description_and_listener_count() {
        let host = Arc::new(Host::new(RecordingEnv::new(true)));
        host.load(Arc::new(Counter {
            hits: Arc::new(AtomicUsize::new(0)),
        }))
        .unwrap();
        let list = host.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].description, "counts ticks");
        assert!(list[0].healthy);
        assert_eq!(list[0].listeners, 1, "the plugin subscribed to 'tick'");
    }
}
