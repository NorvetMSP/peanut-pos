use once_cell::sync::Lazy;
use prometheus::{Registry, IntCounter, IntGauge, IntCounterVec, TextEncoder, Encoder};
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex};
use std::sync::atomic::{AtomicU64, Ordering};

pub static REGISTRY: Lazy<Registry> = Lazy::new(|| Registry::new());

pub static AUDIT_BUFFER_QUEUED: Lazy<IntGauge> = Lazy::new(|| {
    let g = IntGauge::new("audit_buffer_queued", "Current in-memory buffered audit events").unwrap();
    REGISTRY.register(Box::new(g.clone())).ok();
    g
});

pub static AUDIT_BUFFER_EMITTED: Lazy<IntCounter> = Lazy::new(|| {
    let c = IntCounter::new("audit_buffer_emitted_total", "Total audit events emitted from buffer").unwrap();
    REGISTRY.register(Box::new(c.clone())).ok();
    c
});

pub static AUDIT_BUFFER_DROPPED: Lazy<IntCounter> = Lazy::new(|| {
    let c = IntCounter::new("audit_buffer_dropped_total", "Total audit events dropped due to full buffer").unwrap();
    REGISTRY.register(Box::new(c.clone())).ok();
    c
});

pub static AUDIT_VIEW_REDACTIONS: Lazy<IntCounter> = Lazy::new(|| {
    let c = IntCounter::new("audit_view_redactions_total", "Total sensitive field redactions applied at view layer").unwrap();
    REGISTRY.register(Box::new(c.clone())).ok();
    c
});

pub static AUDIT_VIEW_REDACTIONS_LABELLED: Lazy<IntCounterVec> = Lazy::new(|| {
    let v = IntCounterVec::new(
        prometheus::Opts::new("audit_view_redactions_labelled_total", "Redactions broken down by tenant, role, field"),
        &["tenant_id", "role", "field"],
    ).unwrap();
    REGISTRY.register(Box::new(v.clone())).ok();
    v
});

// Last observed raw values to convert absolute snapshots into Prometheus counter deltas
static LAST_BUFFER_EMITTED: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(0));
static LAST_BUFFER_DROPPED: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(0));
static LAST_VIEW_REDACTIONS: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(0));
static LAST_VIEW_LABELLED: Lazy<Mutex<HashMap<(String,String,String), u64>>> = Lazy::new(|| Mutex::new(HashMap::new()));

// Whitelist of allowed redaction field labels (cardinality guard)
static REDACTION_FIELD_WHITELIST: Lazy<Option<HashSet<String>>> = Lazy::new(|| {
    std::env::var("AUDIT_VIEW_REDACTION_PATHS").ok().map(|v| {
        v.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<HashSet<_>>()
    })
});

// Cached coverage metrics content (sanitized) loaded once
static COVERAGE_METRICS: Lazy<Option<String>> = Lazy::new(|| {
    if let Ok(content) = std::fs::read_to_string("../audit_coverage_metrics.prom") {
        Some(content)
    } else { None }
});

pub fn update_buffer_metrics(queued: u64, emitted_total: u64, dropped_total: u64) {
    AUDIT_BUFFER_QUEUED.set(queued as i64);
    let prev_emitted = LAST_BUFFER_EMITTED.load(Ordering::Relaxed);
    if emitted_total > prev_emitted {
        AUDIT_BUFFER_EMITTED.inc_by(emitted_total - prev_emitted);
        LAST_BUFFER_EMITTED.store(emitted_total, Ordering::Relaxed);
    }
    let prev_dropped = LAST_BUFFER_DROPPED.load(Ordering::Relaxed);
    if dropped_total > prev_dropped {
        AUDIT_BUFFER_DROPPED.inc_by(dropped_total - prev_dropped);
        LAST_BUFFER_DROPPED.store(dropped_total, Ordering::Relaxed);
    }
}

pub fn update_redaction_counters(total_redactions: u64, labelled: &std::collections::HashMap<(String,String,String), u64>) {
    // Overall
    let prev_total = LAST_VIEW_REDACTIONS.load(Ordering::Relaxed);
    if total_redactions > prev_total {
        AUDIT_VIEW_REDACTIONS.inc_by(total_redactions - prev_total);
        LAST_VIEW_REDACTIONS.store(total_redactions, Ordering::Relaxed);
    }
    // Per label
    if let Ok(mut last_map) = LAST_VIEW_LABELLED.lock() {
        for ((tenant, role, field), count) in labelled.iter() {
            if let Some(whitelist) = &*REDACTION_FIELD_WHITELIST {
                if !whitelist.contains(field) { continue; }
            }
            let key = (tenant.clone(), role.clone(), field.clone());
            let prev = *last_map.get(&key).unwrap_or(&0);
            if *count > prev {
                AUDIT_VIEW_REDACTIONS_LABELLED.with_label_values(&[tenant, role, field]).inc_by(*count - prev);
                last_map.insert(key, *count);
            }
        }
    }
}

pub fn gather(extra_coverage_filter: bool) -> String {
    let mut buf = String::new();
    let encoder = TextEncoder::new();
    let mfs = REGISTRY.gather();
    let mut v = Vec::new();
    if encoder.encode(&mfs, &mut v).is_ok() {
        if let Ok(s) = String::from_utf8(v) { buf.push_str(&s); }
    }
    if extra_coverage_filter {
        if let Some(raw) = &*COVERAGE_METRICS {
            // Build set of existing metric names
            let mut existing = HashSet::new();
            for mf in &mfs { existing.insert(mf.get_name().to_string()); }
            let filtered: String = raw.lines().filter(|line| {
                if line.starts_with("# HELP ") {
                    let name = line.split_whitespace().nth(2).unwrap_or("");
                    return !existing.contains(name);
                }
                if line.starts_with("# TYPE ") {
                    let name = line.split_whitespace().nth(2).unwrap_or("");
                    return !existing.contains(name);
                }
                true
            }).collect::<Vec<_>>().join("\n");
            buf.push_str("\n# Coverage metrics (sanitized)\n");
            buf.push_str(&filtered);
            buf.push('\n');
        }
    }
    buf
}
