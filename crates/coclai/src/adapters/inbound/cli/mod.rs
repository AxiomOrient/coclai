use std::net::SocketAddr;
use std::str::FromStr;

use crate::CapabilityIngress;
use serde_json::{json, Value};

pub(crate) const DEFAULT_AGENT_BIND_ADDR: &str = "127.0.0.1:8787";

#[derive(Clone, Debug)]
pub(crate) struct ServeOptions {
    pub(crate) bind_addr: SocketAddr,
}

#[derive(Clone, Debug)]
pub(crate) struct StartOptions {
    pub(crate) foreground: bool,
    pub(crate) bind_addr: SocketAddr,
}

#[derive(Debug)]
pub(crate) struct InvocationOptions {
    pub(crate) ingress: CapabilityIngress,
    pub(crate) payload: Value,
    pub(crate) caller_addr: Option<String>,
    pub(crate) auth_token: Option<String>,
}

pub(crate) fn usage() {
    eprintln!(
        "usage:
  coclai-agent serve [--bind <host:port>]
  coclai-agent start [--foreground] [--bind <host:port>]
  coclai-agent stop
  coclai-agent status
  coclai-agent list-capabilities [--ingress stdio|http|ws]
  coclai-agent invoke <capability_id> [--ingress stdio|http|ws] [--payload <json>] [--caller <host[:port]>] [--token <secret>]"
    );
}

pub(crate) fn parse_ingress_options(args: &[String]) -> Result<CapabilityIngress, String> {
    let mut ingress = CapabilityIngress::Stdio;
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "--ingress" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--ingress requires one value".to_owned())?;
                ingress = CapabilityIngress::from_str(value).map_err(str::to_owned)?;
                index += 2;
            }
            unknown if unknown.starts_with("--") => {
                return Err(format!("unknown option: {unknown}"));
            }
            unexpected => {
                return Err(format!("unexpected positional argument: {unexpected}"));
            }
        }
    }

    Ok(ingress)
}

pub(crate) fn parse_invocation_options(args: &[String]) -> Result<InvocationOptions, String> {
    let mut ingress = CapabilityIngress::Stdio;
    let mut payload = json!({});
    let mut caller_addr = None;
    let mut auth_token = None;
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "--ingress" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--ingress requires one value".to_owned())?;
                ingress = CapabilityIngress::from_str(value).map_err(str::to_owned)?;
                index += 2;
            }
            "--payload" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--payload requires one json string".to_owned())?;
                payload = serde_json::from_str(value)
                    .map_err(|err| format!("invalid --payload json: {err}"))?;
                index += 2;
            }
            "--caller" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--caller requires one host[:port] value".to_owned())?;
                caller_addr = Some(value.clone());
                index += 2;
            }
            "--token" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--token requires one value".to_owned())?;
                auth_token = Some(value.clone());
                index += 2;
            }
            unknown if unknown.starts_with("--") => {
                return Err(format!("unknown option: {unknown}"));
            }
            unexpected => {
                return Err(format!("unexpected positional argument: {unexpected}"));
            }
        }
    }

    Ok(InvocationOptions {
        ingress,
        payload,
        caller_addr,
        auth_token,
    })
}

fn parse_bind_addr(input: &str) -> Result<SocketAddr, String> {
    input
        .parse::<SocketAddr>()
        .map_err(|err| format!("invalid bind address `{input}`: {err}"))
}

fn resolve_bind_addr(explicit: Option<String>) -> Result<SocketAddr, String> {
    if let Some(raw) = explicit {
        return parse_bind_addr(&raw);
    }
    if let Ok(raw) = std::env::var("COCLAI_AGENT_BIND_ADDR") {
        if !raw.trim().is_empty() {
            return parse_bind_addr(raw.trim());
        }
    }
    parse_bind_addr(DEFAULT_AGENT_BIND_ADDR)
}

pub(crate) fn parse_serve_options(args: &[String]) -> Result<ServeOptions, String> {
    let mut bind_addr = None::<String>;
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "--bind" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--bind requires one host:port value".to_owned())?;
                bind_addr = Some(value.clone());
                index += 2;
            }
            unknown if unknown.starts_with("--") => {
                return Err(format!("unknown option: {unknown}"));
            }
            unexpected => return Err(format!("unexpected positional argument: {unexpected}")),
        }
    }
    Ok(ServeOptions {
        bind_addr: resolve_bind_addr(bind_addr)?,
    })
}

pub(crate) fn parse_start_options(args: &[String]) -> Result<StartOptions, String> {
    let mut foreground = false;
    let mut bind_addr = None::<String>;
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "--bind" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--bind requires one host:port value".to_owned())?;
                bind_addr = Some(value.clone());
                index += 2;
            }
            "--foreground" => {
                foreground = true;
                index += 1;
            }
            unknown if unknown.starts_with("--") => {
                return Err(format!("unknown option: {unknown}"));
            }
            unexpected => return Err(format!("unexpected positional argument: {unexpected}")),
        }
    }
    Ok(StartOptions {
        foreground,
        bind_addr: resolve_bind_addr(bind_addr)?,
    })
}

pub(crate) fn normalize_token(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}
