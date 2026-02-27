use crate::runtime::Runtime;

use super::ClientError;

const DEFAULT_MIN_CODEX_VERSION: SemVerTriplet = SemVerTriplet {
    major: 0,
    minor: 104,
    patch: 0,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SemVerTriplet {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl SemVerTriplet {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

impl std::fmt::Display for SemVerTriplet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompatibilityGuard {
    pub require_initialize_user_agent: bool,
    pub min_codex_version: Option<SemVerTriplet>,
}

impl Default for CompatibilityGuard {
    fn default() -> Self {
        Self {
            require_initialize_user_agent: true,
            min_codex_version: Some(DEFAULT_MIN_CODEX_VERSION),
        }
    }
}

pub(super) fn validate_runtime_compatibility(
    runtime: &Runtime,
    guard: &CompatibilityGuard,
) -> Result<(), ClientError> {
    if !guard.require_initialize_user_agent && guard.min_codex_version.is_none() {
        return Ok(());
    }

    let Some(user_agent) = runtime.server_user_agent() else {
        if guard.require_initialize_user_agent {
            return Err(ClientError::MissingInitializeUserAgent);
        }
        return Ok(());
    };
    let (product, version) = parse_initialize_user_agent(&user_agent)
        .ok_or_else(|| ClientError::InvalidInitializeUserAgent(user_agent.clone()))?;
    let is_codex_product = product.starts_with("Codex ");

    if is_codex_product {
        if let Some(min_required) = guard.min_codex_version {
            if version < min_required {
                return Err(ClientError::IncompatibleCodexVersion {
                    detected: version.to_string(),
                    required: min_required.to_string(),
                    user_agent,
                });
            }
        }
    }

    Ok(())
}

pub(super) fn parse_initialize_user_agent(value: &str) -> Option<(String, SemVerTriplet)> {
    let slash = value.find('/')?;
    let product = value.get(..slash)?.trim().to_owned();
    if product.is_empty() {
        return None;
    }

    let version_part = value
        .get(slash + 1..)?
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '.')
        .collect::<String>();
    let mut parts = version_part.split('.');
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts.next()?.parse::<u32>().ok()?;
    let patch = parts.next()?.parse::<u32>().ok()?;

    Some((product, SemVerTriplet::new(major, minor, patch)))
}
