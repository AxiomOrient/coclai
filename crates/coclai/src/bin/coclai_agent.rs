#[path = "../adapters/inbound/cli/mod.rs"]
mod cli;
#[path = "../adapters/inbound/http/mod.rs"]
mod http;
#[path = "../adapters/inbound/invoke_contract.rs"]
mod invoke_contract;
#[path = "coclai_agent/process_lock.rs"]
mod process_lock;
#[path = "../adapters/inbound/stdio/mod.rs"]
mod stdio;
#[path = "../adapters/inbound/ws/mod.rs"]
mod ws;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use coclai::build_agent;
use serde_json::{json, Value};

pub use coclai::{
    capability_registry, AgentDispatchError, CapabilityIngress, CapabilityInvocation,
    CapabilityResponse, CoclaiAgent,
};

use cli::{
    parse_ingress_options, parse_invocation_options, parse_serve_options, parse_start_options,
    usage, DEFAULT_AGENT_BIND_ADDR,
};
use http::{http_capabilities, http_health, http_invoke, AgentIngressState};
use invoke_contract::{render_dispatch_error_json, render_invoke_success_json};
use process_lock::{
    acquire_single_instance_lock, inspect_process_state, runtime_paths, start_background,
    stop_background,
};
use stdio::{invoke_capability, list_capabilities_json};
use ws::ws_upgrade;

fn run_network_server(
    agent: CoclaiAgent,
    lock_path: PathBuf,
    bind_addr: SocketAddr,
    status: &'static str,
) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("failed to build tokio runtime for serve: {err}"))?;

    runtime.block_on(async move {
        let state = AgentIngressState {
            agent: Arc::new(agent),
        };
        let app = Router::new()
            .route("/health", get(http_health))
            .route("/capabilities", get(http_capabilities))
            .route("/invoke", post(http_invoke))
            .route("/ws", get(ws_upgrade))
            .with_state(state);

        let listener = tokio::net::TcpListener::bind(bind_addr)
            .await
            .map_err(|err| format!("failed to bind {bind_addr}: {err}"))?;
        let local_addr = listener
            .local_addr()
            .map_err(|err| format!("failed to inspect bound address: {err}"))?;

        print_json(&json!({
            "status": status,
            "pid": std::process::id(),
            "lock_path": lock_path.display().to_string(),
            "bind_addr": local_addr.to_string(),
            "routes": ["/health", "/capabilities", "/invoke", "/ws"],
        }))?;

        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .map_err(|err| format!("agent network server failed: {err}"))
    })
}

fn print_json(value: &Value) -> Result<(), String> {
    let rendered = serde_json::to_string_pretty(value)
        .map_err(|err| format!("failed to render json output: {err}"))?;
    println!("{rendered}");
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let Some(command_name) = args.get(1).map(String::as_str) else {
        usage();
        return ExitCode::from(2);
    };
    let rest = &args[2..];
    let agent = build_agent();
    let paths = runtime_paths();

    match command_name {
        "serve" => {
            let options = match parse_serve_options(rest) {
                Ok(options) => options,
                Err(err) => {
                    eprintln!("{err}");
                    return ExitCode::from(2);
                }
            };
            let lock_guard = match acquire_single_instance_lock(&paths) {
                Ok(guard) => guard,
                Err(err) => {
                    eprintln!("{err}");
                    return ExitCode::from(2);
                }
            };
            let _keep_lock_guard = lock_guard;
            if let Err(err) = run_network_server(
                agent.clone(),
                paths.lock_path.clone(),
                options.bind_addr,
                "serving",
            ) {
                eprintln!("{err}");
                return ExitCode::from(2);
            }
            ExitCode::SUCCESS
        }
        "start" => {
            let options = match parse_start_options(rest) {
                Ok(options) => options,
                Err(err) => {
                    eprintln!("{err}");
                    return ExitCode::from(2);
                }
            };
            if options.foreground {
                let lock_guard = match acquire_single_instance_lock(&paths) {
                    Ok(guard) => guard,
                    Err(err) => {
                        eprintln!("{err}");
                        return ExitCode::from(2);
                    }
                };
                let _keep_lock_guard = lock_guard;
                if let Err(err) = run_network_server(
                    agent.clone(),
                    paths.lock_path.clone(),
                    options.bind_addr,
                    "serving_foreground",
                ) {
                    eprintln!("{err}");
                    return ExitCode::from(2);
                }
                return ExitCode::SUCCESS;
            }

            match start_background(&paths, options.bind_addr) {
                Ok(output) => match print_json(&output) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(err) => {
                        eprintln!("{err}");
                        ExitCode::from(2)
                    }
                },
                Err(err) => {
                    eprintln!("{err}");
                    ExitCode::from(2)
                }
            }
        }
        "stop" => {
            if !rest.is_empty() {
                usage();
                return ExitCode::from(2);
            }
            match stop_background(&paths) {
                Ok(output) => match print_json(&output) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(err) => {
                        eprintln!("{err}");
                        ExitCode::from(2)
                    }
                },
                Err(err) => {
                    eprintln!("{err}");
                    ExitCode::from(2)
                }
            }
        }
        "status" => {
            let health = agent.health();
            let process = inspect_process_state(&paths);
            match print_json(&json!({
                "agent": {
                    "status": health.status,
                    "registry_size": health.registry_size,
                    "full_parity_gaps": health.full_parity_gaps,
                    "network_ingress_loopback_only": health.network_ingress_loopback_only,
                    "network_ingress_token_configured": health.network_ingress_token_configured,
                },
                "process": {
                    "running": process.running,
                    "pid": process.pid,
                    "stale_lock": process.stale_lock,
                    "lock_path": process.lock_path.display().to_string(),
                },
                "network": {
                    "bind_addr": std::env::var("COCLAI_AGENT_BIND_ADDR").unwrap_or_else(|_| DEFAULT_AGENT_BIND_ADDR.to_owned()),
                    "routes": ["/health", "/capabilities", "/invoke", "/ws"],
                },
            })) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("{err}");
                    ExitCode::from(2)
                }
            }
        }
        "list-capabilities" => {
            let ingress = match parse_ingress_options(rest) {
                Ok(ingress) => ingress,
                Err(err) => {
                    eprintln!("{err}");
                    return ExitCode::from(2);
                }
            };
            match print_json(&list_capabilities_json(ingress)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("{err}");
                    ExitCode::from(2)
                }
            }
        }
        "invoke" => {
            let Some(capability_id) = rest.first().cloned() else {
                usage();
                return ExitCode::from(2);
            };
            let option_args = &rest[1..];
            let options = match parse_invocation_options(option_args) {
                Ok(options) => options,
                Err(err) => {
                    eprintln!("{err}");
                    return ExitCode::from(2);
                }
            };

            match invoke_capability(&agent, capability_id, options) {
                Ok(response) => match print_json(&render_invoke_success_json(response)) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(err) => {
                        eprintln!("{err}");
                        ExitCode::from(2)
                    }
                },
                Err(err) => match print_json(&render_dispatch_error_json(err)) {
                    Ok(()) => ExitCode::from(2),
                    Err(render_err) => {
                        eprintln!("{render_err}");
                        ExitCode::from(2)
                    }
                },
            }
        }
        _ => {
            usage();
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::header::AUTHORIZATION;
    use axum::http::{HeaderMap, HeaderValue};

    use super::cli::{
        parse_ingress_options, parse_invocation_options, parse_serve_options, parse_start_options,
    };
    use super::http::token_from_headers;
    use super::process_lock::{parse_lock_pid, runtime_paths};

    #[test]
    fn parse_lock_pid_reads_expected_format() {
        let pid = parse_lock_pid("pid=1234\nstarted_unix=42\n");
        assert_eq!(pid, Some(1234));
    }

    #[test]
    fn parse_ingress_options_rejects_unknown_flags() {
        let args = vec!["--unknown".to_owned()];
        let err = parse_ingress_options(&args).expect_err("unknown flag must fail");
        assert!(err.contains("unknown option"));
    }

    #[test]
    fn parse_start_options_accepts_foreground_only() {
        let args = vec!["--foreground".to_owned()];
        let parsed = parse_start_options(&args).expect("foreground option should parse");
        assert!(parsed.foreground);
    }

    #[test]
    fn parse_start_options_accepts_bind_override() {
        let args = vec!["--bind".to_owned(), "127.0.0.1:9911".to_owned()];
        let parsed = parse_start_options(&args).expect("bind option should parse");
        assert_eq!(parsed.bind_addr.to_string(), "127.0.0.1:9911");
    }

    #[test]
    fn parse_serve_options_accepts_bind_override() {
        let args = vec!["--bind".to_owned(), "127.0.0.1:8822".to_owned()];
        let parsed = parse_serve_options(&args).expect("serve bind option should parse");
        assert_eq!(parsed.bind_addr.to_string(), "127.0.0.1:8822");
    }

    #[test]
    fn parse_serve_options_rejects_invalid_bind() {
        let args = vec!["--bind".to_owned(), "127.0.0.1".to_owned()];
        let err = parse_serve_options(&args).expect_err("invalid bind must fail");
        assert!(err.contains("invalid bind address"));
    }

    #[test]
    fn parse_invocation_options_accepts_token_and_caller_for_invoke() {
        let args = vec![
            "--ingress".to_owned(),
            "http".to_owned(),
            "--caller".to_owned(),
            "127.0.0.1:39000".to_owned(),
            "--token".to_owned(),
            "test-token".to_owned(),
            "--payload".to_owned(),
            "{\"k\":\"v\"}".to_owned(),
        ];
        let parsed = parse_invocation_options(&args).expect("invoke options should parse");
        assert_eq!(parsed.ingress.as_str(), "http(localhost)");
        assert_eq!(parsed.caller_addr.as_deref(), Some("127.0.0.1:39000"));
        assert_eq!(parsed.auth_token.as_deref(), Some("test-token"));
        assert_eq!(parsed.payload["k"], "v");
    }

    #[test]
    fn parse_ingress_options_rejects_token_option() {
        let args = vec!["--token".to_owned(), "secret".to_owned()];
        let err = parse_ingress_options(&args).expect_err("token must be rejected");
        assert!(err.contains("unknown option"));
    }

    #[test]
    fn runtime_paths_has_lock_file_name() {
        let paths = runtime_paths();
        assert_eq!(
            paths.lock_path.file_name().and_then(|f| f.to_str()),
            Some("agent.lock")
        );
    }

    #[test]
    fn token_from_headers_reads_x_coclai_token_first() {
        let mut headers = HeaderMap::new();
        headers.insert("x-coclai-token", HeaderValue::from_static("token-a"));
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer token-b"));
        let token = token_from_headers(&headers);
        assert_eq!(token.as_deref(), Some("token-a"));
    }

    #[test]
    fn token_from_headers_reads_bearer_authorization() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer token-b"));
        let token = token_from_headers(&headers);
        assert_eq!(token.as_deref(), Some("token-b"));
    }
}
