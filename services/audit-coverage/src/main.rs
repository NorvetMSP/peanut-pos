use walkdir::WalkDir;
use regex::Regex;
use serde::Serialize;
use rayon::prelude::*;
use structopt::StructOpt;
use std::{path::PathBuf, fs};
use anyhow::Result;
use syn::{visit::Visit, ItemFn};
use quote::ToTokens;
use toml::Value as TomlValue;

#[derive(StructOpt, Debug)]
#[structopt(name="audit-coverage")] 
struct Cli {
    /// Root directory (services) to scan
    #[structopt(parse(from_os_str), default_value = "..")] 
    root: PathBuf,
    /// Emit JSON report file path
    #[structopt(long, parse(from_os_str))]
    json: Option<PathBuf>,
    /// Verb patterns (comma separated)
    #[structopt(long, default_value = "create,update,delete,refund,void,adjust")] 
    verbs: String,
    /// Fail (non-zero exit) if uncovered handlers exceed threshold (legacy)
    #[structopt(long, default_value = "0")] 
    fail_over: usize,
    /// Minimum coverage ratio (0-100) required
    #[structopt(long, default_value = "0")] 
    min_ratio: f64,
    /// Emit Prometheus metrics file
    #[structopt(long, parse(from_os_str))]
    metrics_out: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
struct HandlerCoverage {
    file: String,
    handler_fn: String,
    verbs_detected: Vec<String>,
    audit_calls: usize,
    ignored: bool,
}

#[derive(Debug, Serialize)]
struct CoverageReport {
    total_handlers: usize,
    covered: usize,
    uncovered: usize,
    handlers: Vec<HandlerCoverage>,
}

#[derive(Default)]
struct Config {
    verbs: Vec<String>,
    ignore_handlers: Vec<String>,
    min_ratio: Option<f64>,
}

fn load_service_config(dir: &PathBuf) -> Config {
    let mut cfg = Config::default();
    let config_path = dir.join("audit_coverage.toml");
    if let Ok(text) = fs::read_to_string(&config_path) {
        if let Ok(val) = text.parse::<TomlValue>() {
            if let Some(v) = val.get("verbs").and_then(|v| v.as_array()) { cfg.verbs = v.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect(); }
            if let Some(v) = val.get("ignore_handlers").and_then(|v| v.as_array()) { cfg.ignore_handlers = v.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect(); }
            if let Some(mr) = val.get("min_ratio").and_then(|x| x.as_float()) { cfg.min_ratio = Some(mr); }
        }
    }
    cfg
}

struct FnCollector { fns: Vec<ItemFn> }
impl<'ast> Visit<'ast> for FnCollector {
    fn visit_item_fn(&mut self, i: &'ast ItemFn) { self.fns.push(i.clone()); syn::visit::visit_item_fn(self, i); }
}

fn main() -> Result<()> {
    let cli = Cli::from_args();
    let root_cfg = load_service_config(&cli.root);
    let mut verb_list: Vec<String> = if !cli.verbs.is_empty() { cli.verbs.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect() } else { vec![] };
    if verb_list.is_empty() && !root_cfg.verbs.is_empty() { verb_list = root_cfg.verbs.clone(); }
    if verb_list.is_empty() { verb_list = vec!["create","update","delete","refund","void","adjust"].into_iter().map(|s| s.to_string()).collect(); }
    let verb_regex = Regex::new(&format!("(?i)\\b({})\\b", verb_list.join("|")))?;
    let audit_call_regex = Regex::new(r"audit_producer\\.emit\\(")?;
    let ignore_tag = "// audit:ignore";

    // Collect rust files under *-service/src excluding target dirs
    let mut files = Vec::new();
    for entry in WalkDir::new(&cli.root) {
        let entry = entry?;
        if !entry.file_type().is_file() { continue; }
        if let Some(ext) = entry.path().extension() { if ext == "rs" { 
            let p = entry.path();
            if p.to_string_lossy().contains("target") { continue; }
            if p.to_string_lossy().contains("/common/") { continue; }
            if !p.to_string_lossy().contains("-service/") { continue; }
            files.push(p.to_path_buf());
        }}
    }

    let handlers: Vec<HandlerCoverage> = files.par_iter().flat_map(|path| {
        let content = fs::read_to_string(path).unwrap_or_default();
        let syntax: syn::File = match syn::parse_file(&content) { Ok(f) => f, Err(_) => return Vec::new() };
        let mut collector = FnCollector { fns: Vec::new() };
        collector.visit_file(&syntax);
        collector.fns.into_iter().filter_map(|f| {
            let is_pub = f.vis.to_token_stream().to_string().contains("pub");
            let is_async = f.sig.asyncness.is_some();
            if !is_pub || !is_async { return None; }
            let name = f.sig.ident.to_string();
            // Approximate body slice: find first '{' after signature and the matching ending '}' by naive balance
            let sig_str = f.sig.to_token_stream().to_string();
            if let Some(sig_pos) = content.find(&sig_str) {
                if let Some(body_start_rel) = content[sig_pos..].find('{') {
                    let body_start = sig_pos + body_start_rel;
                    // naive brace match
                    let mut depth = 0usize; let chars: Vec<char> = content[body_start..].chars().collect();
                    let mut end_offset = None;
                    for (i,ch) in chars.iter().enumerate() { if *ch=='{' { depth+=1; } else if *ch=='}' { depth-=1; if depth==0 { end_offset = Some(i); break; } } }
                    if let Some(end_i) = end_offset { let body_str: String = chars[..=end_i].iter().collect();
                        let ignored = body_str.contains(ignore_tag);
                        let verbs_detected: Vec<String> = verb_regex.find_iter(&body_str).map(|m| m.as_str().to_lowercase()).collect();
                        if verbs_detected.is_empty() { return None; }
                        let audit_calls = audit_call_regex.find_iter(&body_str).count();
                        return Some(HandlerCoverage { file: path.to_string_lossy().to_string(), handler_fn: name, verbs_detected, audit_calls, ignored });
                    }
                }
            }
            None
        }).collect::<Vec<_>>()
    }).collect();

    let covered = handlers.iter().filter(|h| h.audit_calls > 0 || h.ignored).count();
    let uncovered = handlers.iter().filter(|h| h.audit_calls == 0 && !h.ignored).count();
    let report = CoverageReport { total_handlers: handlers.len(), covered, uncovered, handlers };

    println!("Audit Coverage: covered={} uncovered={} total={} ratio={:.2}%", report.covered, report.uncovered, report.total_handlers, if report.total_handlers>0 { (report.covered as f64 / report.total_handlers as f64)*100.0 } else { 0.0 });

    if let Some(json_path) = cli.json.as_ref() {
        let serialized = serde_json::to_string_pretty(&report)?;
        fs::write(json_path, serialized)?;
        println!("Wrote JSON report to {}", json_path.display());
    }

    let ratio = if report.total_handlers>0 { (report.covered as f64 / report.total_handlers as f64)*100.0 } else { 0.0 };
    if let Some(mr) = root_cfg.min_ratio.or(Some(cli.min_ratio)).filter(|v| *v > 0.0) { if ratio < mr { eprintln!("Coverage ratio {:.2}% below minimum {:.2}%", ratio, mr); std::process::exit(1); } }
    if report.uncovered > cli.fail_over { eprintln!("Uncovered handlers {} exceed fail_over {}", report.uncovered, cli.fail_over); std::process::exit(1); }

    if let Some(metrics_path) = cli.metrics_out.as_ref() {
        let mut m = String::new();
        m.push_str("# HELP audit_handler_total Total audit-relevant handlers detected\n# TYPE audit_handler_total gauge\n");
        m.push_str(&format!("audit_handler_total {}\n", report.total_handlers));
        m.push_str("# HELP audit_handler_covered_total Total handlers with audit emits or ignore tag\n# TYPE audit_handler_covered_total gauge\n");
        m.push_str(&format!("audit_handler_covered_total {}\n", report.covered));
        m.push_str("# HELP audit_handler_uncovered_total Total handlers missing audit emits\n# TYPE audit_handler_uncovered_total gauge\n");
        m.push_str(&format!("audit_handler_uncovered_total {}\n", report.uncovered));
        m.push_str("# HELP audit_handler_coverage_ratio_percent Coverage ratio percent\n# TYPE audit_handler_coverage_ratio_percent gauge\n");
        m.push_str(&format!("audit_handler_coverage_ratio_percent {:.2}\n", ratio));
        if let Err(e) = fs::write(metrics_path, m) { eprintln!("Failed to write metrics: {e}"); }
        else { println!("Wrote metrics to {}", metrics_path.display()); }
    }

    Ok(())
}
