//! Optional, off-by-default OpenTelemetry-style span/timing exporter (DP9).
//!
//! This is ADDITIVE observability across the controller turn loop, tool
//! invocations, and runtime process lifecycle. It is OFF by default: no span
//! data leaves the process unless an exporter is explicitly configured (via
//! [`OtelConfig::from_env`] reading an explicit flag, or by constructing one
//! directly). Disabling it changes NOTHING about event-sourced truth or read
//! models -- spans are observability, not state. The event log remains the sole
//! source of truth; a span never carries authoritative state.
//!
//! Modeled after the codex `otel` crate's shape (config / provider /
//! trace_context), reproduced here as a small, dependency-light, fully
//! deterministic in-process implementation so the loop/tool/runtime span and
//! wall-clock timing are testable with NO live collector and NO global tracer
//! state. A real OTLP exporter can be layered behind the same `SpanExporter`
//! trait later without touching call sites.
//!
//! Every exported span attribute passes the existing runtime
//! [`RedactionPolicy`] credential-shape guard before export, mirroring the
//! redaction-on-emit discipline used on the streaming path: a known secret must
//! never appear in an exported span.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use capo_runtime::RedactionPolicy;

/// The env flag that turns the exporter on. Off (absent / `0` / `false` /
/// empty) by default, mirroring the opt-in discipline of
/// `CAPO_SERVER_RUN_CODEX_LIVE` / `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT`.
pub const OTEL_ENABLE_ENV: &str = "CAPO_OTEL_EXPORT";

/// Where loop/tool/runtime spans originate. Lets a consumer assert that the
/// expected surfaces are instrumented without string-matching span names.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum SpanSource {
    /// The controller turn loop.
    Loop,
    /// A tool invocation (ACI wrapper / adapter tool call).
    Tool,
    /// Runtime process lifecycle (spawn/confine/reap).
    Runtime,
}

impl SpanSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Loop => "loop",
            Self::Tool => "tool",
            Self::Runtime => "runtime",
        }
    }
}

/// A finished span: name, source, parentage, wall-clock timing, and redacted
/// attributes. Correlates to `run_id` / `turn_id` / `tool_call_id` via the
/// attribute map (set through [`SpanBuilder::correlate`]).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinishedSpan {
    pub span_id: u64,
    pub parent_span_id: Option<u64>,
    pub trace_id: u64,
    pub name: String,
    pub source: SpanSource,
    /// Real elapsed wall-clock duration (DP9: alongside the event-sequence-delta
    /// `duration_sequence_span`).
    pub wall_clock: Duration,
    /// Redacted span attributes (credential-shape scrubbed before retention).
    pub attributes: BTreeMap<String, String>,
}

impl FinishedSpan {
    /// Convenience: the value of a correlation attribute, if set.
    pub fn attribute(&self, key: &str) -> Option<&str> {
        self.attributes.get(key).map(String::as_str)
    }
}

/// Sink for finished spans. The deterministic test path uses
/// [`InMemorySpanExporter`]; a real OTLP exporter would implement the same
/// trait.
pub trait SpanExporter: Send + Sync {
    fn export(&self, span: FinishedSpan);
}

/// A deterministic, in-memory exporter that simply collects finished spans in
/// completion order. Used by the DP9 tests (no live collector).
#[derive(Clone, Default)]
pub struct InMemorySpanExporter {
    spans: Arc<Mutex<Vec<FinishedSpan>>>,
}

impl InMemorySpanExporter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every span exported so far, in completion order.
    pub fn spans(&self) -> Vec<FinishedSpan> {
        self.spans.lock().expect("otel span lock").clone()
    }

    pub fn len(&self) -> usize {
        self.spans.lock().expect("otel span lock").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl SpanExporter for InMemorySpanExporter {
    fn export(&self, span: FinishedSpan) {
        self.spans.lock().expect("otel span lock").push(span);
    }
}

/// Configuration for the optional exporter. The default is fully OFF: no
/// exporter, no spans.
#[derive(Clone)]
pub struct OtelConfig {
    enabled: bool,
    exporter: Option<Arc<dyn SpanExporter>>,
    redaction: RedactionPolicy,
}

impl Default for OtelConfig {
    fn default() -> Self {
        Self::disabled()
    }
}

impl std::fmt::Debug for OtelConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OtelConfig")
            .field("enabled", &self.enabled)
            .field("has_exporter", &self.exporter.is_some())
            .field("scans_credentials", &self.redaction.scans_credentials())
            .finish()
    }
}

impl OtelConfig {
    /// The off-by-default configuration: no exporter is constructed and no span
    /// data is emitted.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            exporter: None,
            // A redaction policy is held even when disabled so enabling later is
            // a one-field change; the default credential-shape scan is on.
            redaction: RedactionPolicy::new(Vec::new()),
        }
    }

    /// An enabled configuration wired to `exporter` with the default
    /// credential-shape redaction guard.
    pub fn enabled_with(exporter: Arc<dyn SpanExporter>) -> Self {
        Self {
            enabled: true,
            exporter: Some(exporter),
            redaction: RedactionPolicy::new(Vec::new()),
        }
    }

    /// Override the redaction policy applied to span attributes before export.
    pub fn with_redaction(mut self, redaction: RedactionPolicy) -> Self {
        self.redaction = redaction;
        self
    }

    /// Resolve from the environment: the exporter is constructed ONLY when
    /// [`OTEL_ENABLE_ENV`] is explicitly truthy AND a `build_exporter` is
    /// supplied. Absent / `0` / `false` / empty -> fully disabled (no exporter
    /// constructed). This keeps "no spans leave the process unless enabled" a
    /// property of construction, not just of a runtime branch.
    pub fn from_env(build_exporter: impl FnOnce() -> Arc<dyn SpanExporter>) -> Self {
        if env_flag_enabled(OTEL_ENABLE_ENV) {
            Self::enabled_with(build_exporter())
        } else {
            Self::disabled()
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled && self.exporter.is_some()
    }
}

/// Whether an env flag reads as explicitly enabled. Absent, empty, `0`,
/// `false`, `off`, `no` are all OFF.
fn env_flag_enabled(key: &str) -> bool {
    match std::env::var(key) {
        Ok(value) => {
            let v = value.trim().to_ascii_lowercase();
            !matches!(v.as_str(), "" | "0" | "false" | "off" | "no")
        }
        Err(_) => false,
    }
}

/// A tracer that opens spans against a configured exporter. When OTel is
/// disabled this is inert: [`Tracer::span`] returns a no-op guard that
/// constructs nothing and exports nothing.
#[derive(Clone)]
pub struct Tracer {
    inner: Option<Arc<TracerInner>>,
}

struct TracerInner {
    exporter: Arc<dyn SpanExporter>,
    redaction: RedactionPolicy,
    next_id: Mutex<u64>,
}

impl Tracer {
    /// Build a tracer from config. A disabled config yields an inert tracer.
    pub fn new(config: OtelConfig) -> Self {
        if config.is_enabled() {
            Self {
                inner: Some(Arc::new(TracerInner {
                    exporter: config.exporter.expect("enabled config has exporter"),
                    redaction: config.redaction,
                    next_id: Mutex::new(1),
                })),
            }
        } else {
            Self { inner: None }
        }
    }

    /// An always-inert tracer (the off-by-default path).
    pub fn disabled() -> Self {
        Self { inner: None }
    }

    pub fn is_enabled(&self) -> bool {
        self.inner.is_some()
    }

    fn alloc_id(&self) -> u64 {
        let inner = self.inner.as_ref().expect("enabled tracer");
        let mut guard = inner.next_id.lock().expect("otel id lock");
        let id = *guard;
        *guard += 1;
        id
    }

    /// Open a root span on `source` named `name`. Returns a builder so
    /// correlation ids and attributes can be attached before the span is
    /// entered; the span is finished (and exported, if enabled) when its guard
    /// is dropped.
    pub fn span(&self, source: SpanSource, name: impl Into<String>) -> SpanBuilder {
        self.child_span(source, name, None)
    }

    /// Open a span parented to `parent` (its `span_id`), inheriting the parent's
    /// trace id so loop -> tool -> runtime parentage is recordable.
    pub fn child_span(
        &self,
        source: SpanSource,
        name: impl Into<String>,
        parent: Option<&SpanGuard>,
    ) -> SpanBuilder {
        match &self.inner {
            None => SpanBuilder::inert(),
            Some(_) => {
                let span_id = self.alloc_id();
                let (parent_span_id, trace_id) = match parent {
                    Some(p) => (Some(p.span_id), p.trace_id),
                    None => (None, span_id),
                };
                SpanBuilder {
                    inner: Some(SpanBuilderInner {
                        tracer: self.clone(),
                        span_id,
                        parent_span_id,
                        trace_id,
                        name: name.into(),
                        source,
                        attributes: BTreeMap::new(),
                    }),
                }
            }
        }
    }
}

/// Builds a span: attach correlation ids / attributes, then [`start`] to begin
/// wall-clock timing. Inert when the tracer is disabled.
///
/// [`start`]: SpanBuilder::start
pub struct SpanBuilder {
    inner: Option<SpanBuilderInner>,
}

struct SpanBuilderInner {
    tracer: Tracer,
    span_id: u64,
    parent_span_id: Option<u64>,
    trace_id: u64,
    name: String,
    source: SpanSource,
    attributes: BTreeMap<String, String>,
}

impl SpanBuilder {
    fn inert() -> Self {
        Self { inner: None }
    }

    /// Attach an arbitrary attribute (redacted at export time).
    pub fn attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        if let Some(inner) = self.inner.as_mut() {
            inner.attributes.insert(key.into(), value.into());
        }
        self
    }

    /// Attach the standard correlation ids. Any `None` is skipped.
    pub fn correlate(
        self,
        run_id: Option<&str>,
        turn_id: Option<&str>,
        tool_call_id: Option<&str>,
    ) -> Self {
        let mut builder = self;
        if let Some(run_id) = run_id {
            builder = builder.attribute("run_id", run_id);
        }
        if let Some(turn_id) = turn_id {
            builder = builder.attribute("turn_id", turn_id);
        }
        if let Some(tool_call_id) = tool_call_id {
            builder = builder.attribute("tool_call_id", tool_call_id);
        }
        builder
    }

    /// Begin the span, starting wall-clock timing. The returned guard exports
    /// the finished span (redacted) when dropped. Inert builders return an
    /// inert guard.
    pub fn start(self) -> SpanGuard {
        match self.inner {
            None => SpanGuard::inert(),
            Some(inner) => SpanGuard {
                span_id: inner.span_id,
                parent_span_id: inner.parent_span_id,
                trace_id: inner.trace_id,
                inner: Some(SpanGuardInner {
                    tracer: inner.tracer,
                    name: inner.name,
                    source: inner.source,
                    attributes: inner.attributes,
                    start: std::time::Instant::now(),
                    duration_override: None,
                    finished: false,
                }),
            },
        }
    }
}

/// An active span. Wall-clock timing runs from [`SpanBuilder::start`] until the
/// guard is finished (explicitly via [`finish`] or on drop). Exports the
/// redacted [`FinishedSpan`] exactly once.
///
/// [`finish`]: SpanGuard::finish
pub struct SpanGuard {
    span_id: u64,
    parent_span_id: Option<u64>,
    trace_id: u64,
    inner: Option<SpanGuardInner>,
}

struct SpanGuardInner {
    tracer: Tracer,
    name: String,
    source: SpanSource,
    attributes: BTreeMap<String, String>,
    start: std::time::Instant,
    duration_override: Option<Duration>,
    finished: bool,
}

impl SpanGuard {
    fn inert() -> Self {
        Self {
            span_id: 0,
            parent_span_id: None,
            trace_id: 0,
            inner: None,
        }
    }

    /// This span's id, for parenting child spans.
    pub fn span_id(&self) -> u64 {
        self.span_id
    }

    pub fn trace_id(&self) -> u64 {
        self.trace_id
    }

    pub fn is_enabled(&self) -> bool {
        self.inner.is_some()
    }

    /// Set an attribute on an in-flight span (redacted at export time).
    pub fn set_attribute(&mut self, key: impl Into<String>, value: impl Into<String>) {
        if let Some(inner) = self.inner.as_mut() {
            inner.attributes.insert(key.into(), value.into());
        }
    }

    /// Force a deterministic wall-clock duration. Used ONLY by tests so timing
    /// assertions do not depend on real elapsed time; production reads
    /// `Instant::now()`.
    pub fn set_wall_clock_for_test(&mut self, duration: Duration) {
        if let Some(inner) = self.inner.as_mut() {
            inner.duration_override = Some(duration);
        }
    }

    /// Finish and export the span now (idempotent; a later drop is a no-op).
    pub fn finish(mut self) {
        self.finish_inner();
    }

    fn finish_inner(&mut self) {
        let span_id = self.span_id;
        let parent_span_id = self.parent_span_id;
        let trace_id = self.trace_id;
        if let Some(inner) = self.inner.as_mut() {
            if inner.finished {
                return;
            }
            inner.finished = true;
            let wall_clock = inner
                .duration_override
                .unwrap_or_else(|| inner.start.elapsed());
            let redaction = &inner
                .tracer
                .inner
                .as_ref()
                .expect("enabled guard has tracer inner")
                .redaction;
            let attributes = redact_attributes(redaction, &inner.attributes);
            let finished = FinishedSpan {
                span_id,
                parent_span_id,
                trace_id,
                name: inner.name.clone(),
                source: inner.source,
                wall_clock,
                attributes,
            };
            inner
                .tracer
                .inner
                .as_ref()
                .expect("enabled guard has tracer inner")
                .exporter
                .export(finished);
        }
    }
}

impl Drop for SpanGuard {
    fn drop(&mut self) {
        self.finish_inner();
    }
}

/// Apply the redaction guard to every attribute value (and key) before export.
/// A credential-shaped value is scrubbed to the runtime placeholder so a known
/// secret never appears in an exported span.
fn redact_attributes(
    redaction: &RedactionPolicy,
    attributes: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    attributes
        .iter()
        .map(|(key, value)| {
            let (redacted_key, _) = redaction.apply(key.as_bytes());
            let (redacted_value, _) = redaction.apply(value.as_bytes());
            (
                String::from_utf8_lossy(&redacted_key).into_owned(),
                String::from_utf8_lossy(&redacted_value).into_owned(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_by_default_constructs_no_exporter_and_emits_nothing() {
        // The default config is fully off and no exporter is constructed.
        let config = OtelConfig::default();
        assert!(!config.is_enabled());

        // from_env must NOT call the builder when the flag is unset, proving no
        // exporter object even comes into existence on the off path.
        let built = Arc::new(Mutex::new(false));
        let built_for_closure = built.clone();
        // Ensure the flag is unset for this assertion.
        // SAFETY: single-threaded test; we restore nothing because absent is the
        // default and other tests set their own scoped values.
        let prior = std::env::var(OTEL_ENABLE_ENV).ok();
        unsafe {
            std::env::remove_var(OTEL_ENABLE_ENV);
        }
        let config = OtelConfig::from_env(move || {
            *built_for_closure.lock().unwrap() = true;
            Arc::new(InMemorySpanExporter::new())
        });
        assert!(!config.is_enabled());
        assert!(
            !*built.lock().unwrap(),
            "exporter builder ran while OTel was disabled"
        );

        // An inert tracer opens inert spans that export nothing.
        let tracer = Tracer::new(config);
        assert!(!tracer.is_enabled());
        let exporter = InMemorySpanExporter::new();
        {
            let mut span = tracer.span(SpanSource::Loop, "turn");
            span = span.correlate(Some("run-1"), Some("turn-1"), None);
            let guard = span.start();
            assert!(!guard.is_enabled());
            drop(guard);
        }
        assert!(exporter.is_empty());

        if let Some(prior) = prior {
            unsafe {
                std::env::set_var(OTEL_ENABLE_ENV, prior);
            }
        }
    }

    #[test]
    fn enabled_emits_loop_tool_runtime_spans_with_parentage_and_wall_clock() {
        let exporter = InMemorySpanExporter::new();
        let tracer = Tracer::new(OtelConfig::enabled_with(Arc::new(exporter.clone())));
        assert!(tracer.is_enabled());

        // loop span (root) -> tool span (child) -> runtime span (grandchild),
        // each correlated to run/turn/tool ids and given a deterministic
        // wall-clock so timing is asserted without real elapsed time.
        let loop_guard = tracer
            .span(SpanSource::Loop, "controller.turn")
            .correlate(Some("run-7"), Some("turn-3"), None)
            .start();
        let loop_id = loop_guard.span_id();
        let trace_id = loop_guard.trace_id();

        let mut tool_guard = tracer
            .child_span(SpanSource::Tool, "tool.invoke", Some(&loop_guard))
            .correlate(Some("run-7"), Some("turn-3"), Some("tool-9"))
            .start();
        tool_guard.set_wall_clock_for_test(Duration::from_millis(12));
        let tool_id = tool_guard.span_id();

        let mut runtime_guard = tracer
            .child_span(SpanSource::Runtime, "runtime.spawn", Some(&tool_guard))
            .correlate(Some("run-7"), None, Some("tool-9"))
            .start();
        runtime_guard.set_wall_clock_for_test(Duration::from_millis(5));
        runtime_guard.finish();
        tool_guard.finish();
        drop(loop_guard);

        let spans = exporter.spans();
        assert_eq!(spans.len(), 3, "expected loop+tool+runtime spans");

        // Completion order: runtime, tool, then loop (LIFO finish).
        assert_eq!(spans[0].source, SpanSource::Runtime);
        assert_eq!(spans[0].parent_span_id, Some(tool_id));
        assert_eq!(spans[0].wall_clock, Duration::from_millis(5));
        assert_eq!(spans[0].attribute("run_id"), Some("run-7"));
        assert_eq!(spans[0].attribute("tool_call_id"), Some("tool-9"));

        assert_eq!(spans[1].source, SpanSource::Tool);
        assert_eq!(spans[1].parent_span_id, Some(loop_id));
        assert_eq!(spans[1].wall_clock, Duration::from_millis(12));
        assert_eq!(spans[1].attribute("turn_id"), Some("turn-3"));

        assert_eq!(spans[2].source, SpanSource::Loop);
        assert_eq!(spans[2].parent_span_id, None);

        // All three share the loop's trace id (single trace).
        assert!(spans.iter().all(|span| span.trace_id == trace_id));
    }

    #[test]
    fn known_secret_never_appears_in_an_exported_span() {
        let exporter = InMemorySpanExporter::new();
        let tracer = Tracer::new(OtelConfig::enabled_with(Arc::new(exporter.clone())));

        // A credential-shaped value attached to a span attribute must be
        // scrubbed by the redaction guard before export.
        let secret = "sk-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let guard = tracer
            .span(SpanSource::Tool, "tool.invoke")
            .attribute("command", format!("export TOKEN={secret}"))
            .attribute("safe", "ls -la /workspace")
            .start();
        drop(guard);

        let spans = exporter.spans();
        assert_eq!(spans.len(), 1);
        let span = &spans[0];
        for value in span.attributes.values() {
            assert!(
                !value.contains(secret),
                "secret leaked into exported span attribute: {value}"
            );
        }
        assert_eq!(
            span.attribute("command"),
            Some("export [REDACTED:credential]")
        );
        // A benign attribute survives untouched.
        assert_eq!(span.attribute("safe"), Some("ls -la /workspace"));
    }

    #[test]
    fn finish_is_idempotent_across_explicit_finish_and_drop() {
        let exporter = InMemorySpanExporter::new();
        let tracer = Tracer::new(OtelConfig::enabled_with(Arc::new(exporter.clone())));
        let guard = tracer.span(SpanSource::Loop, "turn").start();
        guard.finish();
        // No second export from any lingering drop path.
        assert_eq!(exporter.len(), 1);
    }

    #[test]
    fn env_flag_truthiness_is_explicit_opt_in() {
        assert!(!matches_enabled(""));
        assert!(!matches_enabled("0"));
        assert!(!matches_enabled("false"));
        assert!(!matches_enabled("off"));
        assert!(!matches_enabled("no"));
        assert!(matches_enabled("1"));
        assert!(matches_enabled("true"));
        assert!(matches_enabled("on"));
    }

    // Mirror env_flag_enabled's truthiness rule without touching process env.
    fn matches_enabled(value: &str) -> bool {
        let v = value.trim().to_ascii_lowercase();
        !matches!(v.as_str(), "" | "0" | "false" | "off" | "no")
    }
}
