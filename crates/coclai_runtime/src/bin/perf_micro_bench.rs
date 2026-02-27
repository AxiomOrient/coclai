use std::fs;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use coclai_runtime::approvals::route_server_request;
use coclai_runtime::events::{Direction, Envelope, MsgKind};
use coclai_runtime::rpc::classify_message;
use coclai_runtime::state::{reduce_in_place, RuntimeState};
use coclai_runtime::{HookAction, HookContext, HookIssue, HookPhase, HookReport, PreHook};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug)]
struct CliConfig {
    out: PathBuf,
    baseline: Option<PathBuf>,
    max_regression: f64,
    max_hook_linearity: f64,
    iterations: u64,
    warmup: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkloadReport {
    name: String,
    iterations: u64,
    min_nanos: u64,
    p50_nanos: u64,
    p95_nanos: u64,
    p99_nanos: u64,
    max_nanos: u64,
    mean_nanos: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdapterOverheadReport {
    direct_p95_nanos: u64,
    dyn_p95_nanos: u64,
    p95_overhead_ratio: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MicroBenchReport {
    generated_at_unix_millis: u64,
    iterations: u64,
    warmup: u64,
    classify_message: WorkloadReport,
    reducer: WorkloadReport,
    approval_routing: WorkloadReport,
    #[serde(default)]
    hook_pre_h0: WorkloadReport,
    #[serde(default)]
    hook_pre_h1: WorkloadReport,
    #[serde(default)]
    hook_pre_h3: WorkloadReport,
    #[serde(default)]
    hook_pre_h5: WorkloadReport,
    #[serde(default)]
    adapter_direct_sync: WorkloadReport,
    #[serde(default)]
    adapter_dyn_sync: WorkloadReport,
    #[serde(default)]
    adapter_overhead: AdapterOverheadReport,
}

type AsyncBenchFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let cfg = parse_args()?;
    let report = run_micro_bench(cfg.iterations, cfg.warmup)?;

    if let Some(parent) = cfg.out.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create {:?}: {err}", parent))?;
        }
    }
    let serialized = serde_json::to_string_pretty(&report)
        .map_err(|err| format!("failed to serialize report: {err}"))?;
    fs::write(&cfg.out, serialized)
        .map_err(|err| format!("failed to write {:?}: {err}", cfg.out))?;

    if let Some(baseline_path) = cfg.baseline.as_ref() {
        let baseline_raw = fs::read_to_string(baseline_path)
            .map_err(|err| format!("failed to read baseline {:?}: {err}", baseline_path))?;
        let baseline: MicroBenchReport = serde_json::from_str(&baseline_raw)
            .map_err(|err| format!("invalid baseline JSON {:?}: {err}", baseline_path))?;
        let mut findings = regression_findings(&report, &baseline, cfg.max_regression);
        findings.extend(linearity_findings(&report, cfg.max_hook_linearity));
        if findings.is_empty() {
            println!(
                "perf regression check passed (max regression {:.2}%, hook linearity slack {:.2}%)",
                cfg.max_regression * 100.0,
                cfg.max_hook_linearity * 100.0
            );
        } else {
            for finding in &findings {
                eprintln!("{finding}");
            }
            return Err("perf regression check failed".to_owned());
        }
    } else {
        let findings = linearity_findings(&report, cfg.max_hook_linearity);
        if findings.is_empty() {
            println!(
                "hook linearity check passed (slack {:.2}%)",
                cfg.max_hook_linearity * 100.0
            );
        } else {
            for finding in &findings {
                eprintln!("{finding}");
            }
            return Err("hook linearity check failed".to_owned());
        }
    }

    println!(
        "adapter overhead (p95): direct={}ns dyn={}ns ratio={:.2}%",
        report.adapter_overhead.direct_p95_nanos,
        report.adapter_overhead.dyn_p95_nanos,
        report.adapter_overhead.p95_overhead_ratio * 100.0
    );
    Ok(())
}

fn parse_args() -> Result<CliConfig, String> {
    let mut out = PathBuf::from("target/perf/micro_latest.json");
    let mut baseline: Option<PathBuf> = None;
    let mut max_regression = 0.15f64;
    let mut max_hook_linearity = 0.50f64;
    let mut iterations = 150_000u64;
    let mut warmup = 20_000u64;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => {
                let Some(value) = args.next() else {
                    return Err("--out requires a value".to_owned());
                };
                out = PathBuf::from(value);
            }
            "--baseline" => {
                let Some(value) = args.next() else {
                    return Err("--baseline requires a value".to_owned());
                };
                baseline = Some(PathBuf::from(value));
            }
            "--max-regression" => {
                let Some(value) = args.next() else {
                    return Err("--max-regression requires a value".to_owned());
                };
                max_regression = value
                    .parse::<f64>()
                    .map_err(|err| format!("invalid --max-regression value: {err}"))?;
            }
            "--max-hook-linearity" => {
                let Some(value) = args.next() else {
                    return Err("--max-hook-linearity requires a value".to_owned());
                };
                max_hook_linearity = value
                    .parse::<f64>()
                    .map_err(|err| format!("invalid --max-hook-linearity value: {err}"))?;
            }
            "--iterations" => {
                let Some(value) = args.next() else {
                    return Err("--iterations requires a value".to_owned());
                };
                iterations = value
                    .parse::<u64>()
                    .map_err(|err| format!("invalid --iterations value: {err}"))?;
            }
            "--warmup" => {
                let Some(value) = args.next() else {
                    return Err("--warmup requires a value".to_owned());
                };
                warmup = value
                    .parse::<u64>()
                    .map_err(|err| format!("invalid --warmup value: {err}"))?;
            }
            "--help" | "-h" => {
                println!(
                    "Usage: perf_micro_bench [--out PATH] [--baseline PATH] [--max-regression FLOAT] [--max-hook-linearity FLOAT] [--iterations N] [--warmup N]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    Ok(CliConfig {
        out,
        baseline,
        max_regression,
        max_hook_linearity,
        iterations,
        warmup,
    })
}

fn run_micro_bench(iterations: u64, warmup: u64) -> Result<MicroBenchReport, String> {
    let classify_inputs = classify_inputs();
    let classify_message_report = run_workload("classify_message", iterations, warmup, |i| {
        let value = &classify_inputs[i % classify_inputs.len()];
        let kind = classify_message(std::hint::black_box(value));
        std::hint::black_box(kind);
    });

    let reduce_inputs = reduce_inputs();
    let mut state = RuntimeState::default();
    let reducer_report = run_workload("reducer", iterations, warmup, |i| {
        let envelope = &reduce_inputs[i % reduce_inputs.len()];
        reduce_in_place(
            std::hint::black_box(&mut state),
            std::hint::black_box(envelope),
        );
    });

    let approval_methods = [
        "item/fileChange/requestApproval",
        "item/commandExecution/requestApproval",
        "item/tool/requestUserInput",
        "item/unknown/requestApproval",
    ];
    let approval_routing_report = run_workload("approval_routing", iterations, warmup, |i| {
        let method = approval_methods[i % approval_methods.len()];
        let route = route_server_request(std::hint::black_box(method), std::hint::black_box(true));
        std::hint::black_box(route);
    });

    let hook_ctx = Arc::new(hook_context());
    let hooks_h0: Arc<Vec<Arc<dyn PreHook>>> = Arc::new(Vec::new());
    let hooks_h1 = Arc::new(build_noop_pre_hooks(1));
    let hooks_h3 = Arc::new(build_noop_pre_hooks(3));
    let hooks_h5 = Arc::new(build_noop_pre_hooks(5));

    let hook_pre_h0 = run_hook_workload(
        "hook_pre_h0",
        iterations,
        warmup,
        Arc::clone(&hook_ctx),
        Arc::clone(&hooks_h0),
    )?;
    let hook_pre_h1 = run_hook_workload(
        "hook_pre_h1",
        iterations,
        warmup,
        Arc::clone(&hook_ctx),
        Arc::clone(&hooks_h1),
    )?;
    let hook_pre_h3 = run_hook_workload(
        "hook_pre_h3",
        iterations,
        warmup,
        Arc::clone(&hook_ctx),
        Arc::clone(&hooks_h3),
    )?;
    let hook_pre_h5 = run_hook_workload(
        "hook_pre_h5",
        iterations,
        warmup,
        Arc::clone(&hook_ctx),
        Arc::clone(&hooks_h5),
    )?;

    let direct_adapter = BenchDirectAdapter;
    let dyn_adapter = BenchDynAdapter;
    let dyn_adapter_ref: &dyn BenchAdapterProbe = &dyn_adapter;

    let adapter_direct_sync = run_workload("adapter_direct_sync", iterations, warmup, |i| {
        let out = direct_adapter.step(i as u64);
        std::hint::black_box(out);
    });
    let adapter_dyn_sync = run_workload("adapter_dyn_sync", iterations, warmup, |i| {
        let out = dyn_adapter_ref.step(i as u64);
        std::hint::black_box(out);
    });

    let adapter_overhead = build_adapter_overhead_report(&adapter_direct_sync, &adapter_dyn_sync);

    Ok(MicroBenchReport {
        generated_at_unix_millis: now_unix_millis(),
        iterations,
        warmup,
        classify_message: classify_message_report,
        reducer: reducer_report,
        approval_routing: approval_routing_report,
        hook_pre_h0,
        hook_pre_h1,
        hook_pre_h3,
        hook_pre_h5,
        adapter_direct_sync,
        adapter_dyn_sync,
        adapter_overhead,
    })
}

fn run_hook_workload(
    name: &str,
    iterations: u64,
    warmup: u64,
    ctx: Arc<HookContext>,
    hooks: Arc<Vec<Arc<dyn PreHook>>>,
) -> Result<WorkloadReport, String> {
    run_workload_async(name, iterations, warmup, move |_| {
        let ctx = Arc::clone(&ctx);
        let hooks = Arc::clone(&hooks);
        Box::pin(async move {
            let mut report = HookReport::default();
            let executed = run_pre_hook_chain(hooks.as_ref(), ctx.as_ref(), &mut report).await;
            std::hint::black_box(executed);
            std::hint::black_box(report);
        })
    })
}

async fn run_pre_hook_chain(
    hooks: &[Arc<dyn PreHook>],
    ctx: &HookContext,
    report: &mut HookReport,
) -> usize {
    if hooks.is_empty() {
        return 0;
    }
    let mut decisions = 0usize;
    for hook in hooks {
        match hook.call(ctx).await {
            Ok(action) => {
                decisions = decisions.saturating_add(1);
                std::hint::black_box(action);
            }
            Err(issue) => report.push(issue),
        }
    }
    decisions
}

fn run_workload_async(
    name: &str,
    iterations: u64,
    warmup: u64,
    mut f: impl FnMut(usize) -> AsyncBenchFuture,
) -> Result<WorkloadReport, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|err| format!("failed to build async benchmark runtime: {err}"))?;
    Ok(runtime.block_on(async move {
        for i in 0..warmup {
            f(i as usize).await;
        }

        let mut elapsed_nanos = Vec::<u64>::with_capacity(iterations as usize);
        for i in 0..iterations {
            let started = std::time::Instant::now();
            f(i as usize).await;
            elapsed_nanos.push(started.elapsed().as_nanos() as u64);
        }

        finalize_workload_report(name, iterations, elapsed_nanos)
    }))
}

fn run_workload(
    name: &str,
    iterations: u64,
    warmup: u64,
    mut f: impl FnMut(usize),
) -> WorkloadReport {
    for i in 0..warmup {
        f(i as usize);
    }

    let mut elapsed_nanos = Vec::<u64>::with_capacity(iterations as usize);
    for i in 0..iterations {
        let started = std::time::Instant::now();
        f(i as usize);
        elapsed_nanos.push(started.elapsed().as_nanos() as u64);
    }

    finalize_workload_report(name, iterations, elapsed_nanos)
}

fn finalize_workload_report(
    name: &str,
    iterations: u64,
    mut elapsed_nanos: Vec<u64>,
) -> WorkloadReport {
    elapsed_nanos.sort_unstable();
    let sum = elapsed_nanos
        .iter()
        .copied()
        .fold(0u128, |acc, v| acc.saturating_add(v as u128));
    let mean_nanos = if iterations == 0 {
        0
    } else {
        (sum / iterations as u128) as u64
    };

    WorkloadReport {
        name: name.to_owned(),
        iterations,
        min_nanos: *elapsed_nanos.first().unwrap_or(&0),
        p50_nanos: percentile(&elapsed_nanos, 50),
        p95_nanos: percentile(&elapsed_nanos, 95),
        p99_nanos: percentile(&elapsed_nanos, 99),
        max_nanos: *elapsed_nanos.last().unwrap_or(&0),
        mean_nanos,
    }
}

fn percentile(sorted_nanos: &[u64], p: u64) -> u64 {
    if sorted_nanos.is_empty() {
        return 0;
    }
    let n = sorted_nanos.len() as u64;
    let rank = ((n.saturating_mul(p)).saturating_add(99) / 100).saturating_sub(1);
    sorted_nanos[rank as usize]
}

fn classify_inputs() -> Vec<Value> {
    vec![
        json!({"id":1,"result":{"ok":true}}),
        json!({"id":2,"method":"item/fileChange/requestApproval","params":{"itemId":"it_1"}}),
        json!({"method":"turn/started","params":{"threadId":"thr_1","turnId":"turn_1"}}),
        json!({"foo":"bar"}),
    ]
}

fn reduce_inputs() -> Vec<Envelope> {
    vec![
        envelope(
            1,
            "thread/started",
            Some("thr_perf"),
            None,
            None,
            json!({"threadId":"thr_perf"}),
        ),
        envelope(
            2,
            "turn/started",
            Some("thr_perf"),
            Some("turn_perf"),
            None,
            json!({"threadId":"thr_perf","turnId":"turn_perf"}),
        ),
        envelope(
            3,
            "item/started",
            Some("thr_perf"),
            Some("turn_perf"),
            Some("item_perf"),
            json!({"threadId":"thr_perf","turnId":"turn_perf","itemId":"item_perf","itemType":"agentMessage"}),
        ),
        envelope(
            4,
            "item/agentMessage/delta",
            Some("thr_perf"),
            Some("turn_perf"),
            Some("item_perf"),
            json!({"threadId":"thr_perf","turnId":"turn_perf","itemId":"item_perf","delta":""}),
        ),
        envelope(
            5,
            "item/completed",
            Some("thr_perf"),
            Some("turn_perf"),
            Some("item_perf"),
            json!({"threadId":"thr_perf","turnId":"turn_perf","itemId":"item_perf","status":"completed"}),
        ),
        envelope(
            6,
            "turn/completed",
            Some("thr_perf"),
            Some("turn_perf"),
            None,
            json!({"threadId":"thr_perf","turnId":"turn_perf"}),
        ),
    ]
}

fn envelope(
    seq: u64,
    method: &str,
    thread_id: Option<&str>,
    turn_id: Option<&str>,
    item_id: Option<&str>,
    params: Value,
) -> Envelope {
    Envelope {
        seq,
        ts_millis: 0,
        direction: Direction::Inbound,
        kind: MsgKind::Notification,
        rpc_id: None,
        method: Some(method.to_owned()),
        thread_id: thread_id.map(ToOwned::to_owned),
        turn_id: turn_id.map(ToOwned::to_owned),
        item_id: item_id.map(ToOwned::to_owned),
        json: json!({
            "method": method,
            "params": params
        }),
    }
}

fn now_unix_millis() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => duration_as_millis_u64(d),
        Err(_) => 0,
    }
}

fn duration_as_millis_u64(d: Duration) -> u64 {
    d.as_millis() as u64
}

fn hook_context() -> HookContext {
    HookContext {
        phase: HookPhase::PreRun,
        thread_id: Some("thr_perf".to_owned()),
        turn_id: Some("turn_perf".to_owned()),
        cwd: Some("/tmp".to_owned()),
        model: Some("gpt-perf".to_owned()),
        main_status: Some("ok".to_owned()),
        correlation_id: "perf-hook".to_owned(),
        ts_ms: 0,
        metadata: json!({"scope":"micro-bench"}),
    }
}

fn build_noop_pre_hooks(count: usize) -> Vec<Arc<dyn PreHook>> {
    const NAMES: [&str; 5] = [
        "bench_noop_pre_0",
        "bench_noop_pre_1",
        "bench_noop_pre_2",
        "bench_noop_pre_3",
        "bench_noop_pre_4",
    ];
    let capped = count.min(NAMES.len());
    NAMES[..capped]
        .iter()
        .map(|name| Arc::new(BenchNoopPreHook { name }) as Arc<dyn PreHook>)
        .collect()
}

struct BenchNoopPreHook {
    name: &'static str,
}

impl PreHook for BenchNoopPreHook {
    fn name(&self) -> &'static str {
        self.name
    }

    fn call<'a>(
        &'a self,
        _ctx: &'a HookContext,
    ) -> Pin<Box<dyn Future<Output = Result<HookAction, HookIssue>> + Send + 'a>> {
        Box::pin(async move { Ok(HookAction::Noop) })
    }
}

trait BenchAdapterProbe {
    fn step(&self, value: u64) -> u64;
}

struct BenchDirectAdapter;

impl BenchDirectAdapter {
    fn step(&self, value: u64) -> u64 {
        value.wrapping_add(1)
    }
}

struct BenchDynAdapter;

impl BenchAdapterProbe for BenchDynAdapter {
    fn step(&self, value: u64) -> u64 {
        value.wrapping_add(1)
    }
}

fn build_adapter_overhead_report(
    adapter_direct_sync: &WorkloadReport,
    adapter_dyn_sync: &WorkloadReport,
) -> AdapterOverheadReport {
    let direct = adapter_direct_sync.p95_nanos;
    let dyn_cost = adapter_dyn_sync.p95_nanos;
    AdapterOverheadReport {
        direct_p95_nanos: direct,
        dyn_p95_nanos: dyn_cost,
        p95_overhead_ratio: ratio_overhead(dyn_cost, direct),
    }
}

fn ratio_overhead(observed: u64, baseline: u64) -> f64 {
    if baseline == 0 {
        return 0.0;
    }
    (observed as f64 / baseline as f64) - 1.0
}

fn regression_findings(
    current: &MicroBenchReport,
    baseline: &MicroBenchReport,
    max_regression_ratio: f64,
) -> Vec<String> {
    let mut out = Vec::new();
    compare_workload(
        &mut out,
        "classify_message",
        current.classify_message.p95_nanos,
        baseline.classify_message.p95_nanos,
        max_regression_ratio,
    );
    compare_workload(
        &mut out,
        "reducer",
        current.reducer.p95_nanos,
        baseline.reducer.p95_nanos,
        max_regression_ratio,
    );
    compare_workload(
        &mut out,
        "approval_routing",
        current.approval_routing.p95_nanos,
        baseline.approval_routing.p95_nanos,
        max_regression_ratio,
    );
    compare_workload(
        &mut out,
        "hook_pre_h0",
        current.hook_pre_h0.p95_nanos,
        baseline.hook_pre_h0.p95_nanos,
        max_regression_ratio,
    );
    compare_workload(
        &mut out,
        "hook_pre_h1",
        current.hook_pre_h1.p95_nanos,
        baseline.hook_pre_h1.p95_nanos,
        max_regression_ratio,
    );
    compare_workload(
        &mut out,
        "hook_pre_h3",
        current.hook_pre_h3.p95_nanos,
        baseline.hook_pre_h3.p95_nanos,
        max_regression_ratio,
    );
    compare_workload(
        &mut out,
        "hook_pre_h5",
        current.hook_pre_h5.p95_nanos,
        baseline.hook_pre_h5.p95_nanos,
        max_regression_ratio,
    );
    compare_workload(
        &mut out,
        "adapter_direct_sync",
        current.adapter_direct_sync.p95_nanos,
        baseline.adapter_direct_sync.p95_nanos,
        max_regression_ratio,
    );
    compare_workload(
        &mut out,
        "adapter_dyn_sync",
        current.adapter_dyn_sync.p95_nanos,
        baseline.adapter_dyn_sync.p95_nanos,
        max_regression_ratio,
    );
    out
}

fn linearity_findings(current: &MicroBenchReport, max_hook_linearity_ratio: f64) -> Vec<String> {
    let mut out = Vec::new();
    let h1 = current.hook_pre_h1.p95_nanos;
    let h3 = current.hook_pre_h3.p95_nanos;
    let h5 = current.hook_pre_h5.p95_nanos;

    if h1 == 0 || h3 == 0 || h5 == 0 {
        return out;
    }

    if h3 < h1 {
        out.push(format!(
            "hook linearity: hook_pre_h3 p95 {}ns is smaller than hook_pre_h1 p95 {}ns",
            h3, h1
        ));
    }
    if h5 < h3 {
        out.push(format!(
            "hook linearity: hook_pre_h5 p95 {}ns is smaller than hook_pre_h3 p95 {}ns",
            h5, h3
        ));
    }

    compare_hook_scale(
        &mut out,
        "hook_pre_h3",
        h3,
        h1,
        3.0,
        max_hook_linearity_ratio,
    );
    compare_hook_scale(
        &mut out,
        "hook_pre_h5",
        h5,
        h1,
        5.0,
        max_hook_linearity_ratio,
    );
    out
}

fn compare_hook_scale(
    out: &mut Vec<String>,
    workload: &str,
    observed: u64,
    base: u64,
    scale: f64,
    slack_ratio: f64,
) {
    if base == 0 {
        return;
    }
    let allowed = (base as f64) * scale * (1.0 + slack_ratio);
    if (observed as f64) > allowed {
        out.push(format!(
            "hook linearity: {workload} p95 {}ns exceeds expected linear bound {:.0}ns (base={}ns scale={} slack={:.2}%)",
            observed,
            allowed,
            base,
            scale,
            slack_ratio * 100.0
        ));
    }
}

fn compare_workload(
    out: &mut Vec<String>,
    workload: &str,
    current_p95_nanos: u64,
    baseline_p95_nanos: u64,
    max_regression_ratio: f64,
) {
    if baseline_p95_nanos == 0 {
        return;
    }
    let allowed = (baseline_p95_nanos as f64) * (1.0 + max_regression_ratio);
    if (current_p95_nanos as f64) > allowed {
        out.push(format!(
            "regression: {workload} p95 {}ns exceeds baseline {}ns by more than {:.2}%",
            current_p95_nanos,
            baseline_p95_nanos,
            max_regression_ratio * 100.0
        ));
    }
}
