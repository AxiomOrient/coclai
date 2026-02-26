use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use coclai_runtime::{Runtime, RuntimeConfig, SchemaGuardConfig, StdioProcessSpec};
use serde_json::json;
use tokio::time::{sleep, timeout};

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default)
}

fn workspace_schema_guard() -> SchemaGuardConfig {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let active = manifest_dir.join("../../SCHEMAS/app-server/active");
    SchemaGuardConfig {
        active_schema_dir: active,
    }
}

fn python_soak_process() -> StdioProcessSpec {
    let script = r#"
import json
import sys

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        msg = json.loads(line)
    except Exception:
        continue

    method = msg.get("method")
    rpc_id = msg.get("id")

    if method == "initialize" and rpc_id is not None:
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ready": True}}) + "\n")
        sys.stdout.flush()
        continue

    if rpc_id is None:
        continue

    sys.stdout.write(json.dumps({
        "id": rpc_id,
        "result": {"echoMethod": method, "params": msg.get("params")}
    }) + "\n")
    sys.stdout.flush()
"#;

    let mut spec = StdioProcessSpec::new("python3");
    spec.args = vec!["-u".to_owned(), "-c".to_owned(), script.to_owned()];
    spec
}

fn current_rss_kb() -> u64 {
    let pid = std::process::id().to_string();
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", pid.as_str()])
        .output()
        .expect("run ps for rss");
    assert!(output.status.success(), "ps failed for rss query");
    let text = String::from_utf8(output.stdout).expect("rss output is utf8");
    text.trim()
        .parse::<u64>()
        .expect("rss output should be numeric")
}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn soak_runtime_rss_seq_and_pending_are_stable() {
    let duration_secs = env_u64("CODEX_SOAK_DURATION_SECS", 3_600);
    let warmup_secs_cfg = env_u64("CODEX_SOAK_WARMUP_SECS", 60);
    let loop_sleep_ms = env_u64("CODEX_SOAK_LOOP_SLEEP_MS", 5);
    let warmup_secs = warmup_secs_cfg.min(duration_secs.saturating_sub(1));

    let mut cfg = RuntimeConfig::new(python_soak_process(), workspace_schema_guard());
    cfg.live_channel_capacity = 4_096;
    let runtime = Runtime::spawn_local(cfg).await.expect("runtime spawn");
    let mut live_rx = runtime.subscribe_live();

    let started = Instant::now();
    let duration = Duration::from_secs(duration_secs);
    let warmup = Duration::from_secs(warmup_secs);
    let mut prev_seq = 0u64;
    let mut iteration = 0u64;
    let mut baseline_rss_kb: Option<u64> = None;
    let mut max_rss_after_warmup_kb = 0u64;

    while started.elapsed() < duration {
        let value = runtime
            .call_raw("echo/loop", json!({"index": iteration}))
            .await
            .expect("rpc call");
        assert_eq!(value["echoMethod"], "echo/loop");

        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        assert!(
            envelope.seq > prev_seq,
            "envelope seq must be strictly increasing"
        );
        prev_seq = envelope.seq;

        let snapshot = runtime.metrics_snapshot();
        assert_eq!(snapshot.pending_rpc_count, 0);
        assert_eq!(snapshot.pending_server_request_count, 0);

        let rss_kb = current_rss_kb();
        if started.elapsed() >= warmup {
            if baseline_rss_kb.is_none() {
                baseline_rss_kb = Some(rss_kb);
                max_rss_after_warmup_kb = rss_kb;
            } else if rss_kb > max_rss_after_warmup_kb {
                max_rss_after_warmup_kb = rss_kb;
            }
        }

        iteration = iteration.saturating_add(1);
        if loop_sleep_ms > 0 {
            sleep(Duration::from_millis(loop_sleep_ms)).await;
        }
    }

    let baseline = baseline_rss_kb.unwrap_or_else(current_rss_kb);
    let max_after = max_rss_after_warmup_kb.max(baseline);
    let growth_ratio = if baseline == 0 {
        0.0
    } else {
        ((max_after - baseline) as f64) / (baseline as f64)
    };
    assert!(
        growth_ratio <= 0.10,
        "rss growth ratio exceeded 10% after warmup: baseline={}KB max={}KB ratio={:.4}",
        baseline,
        max_after,
        growth_ratio
    );

    let final_snapshot = runtime.metrics_snapshot();
    assert_eq!(final_snapshot.pending_rpc_count, 0);
    assert_eq!(final_snapshot.pending_server_request_count, 0);

    runtime.shutdown().await.expect("shutdown");
}
