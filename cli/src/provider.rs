use serde::Serialize;
use std::fmt;

/// Where a capability executes. Consumers can use this to enforce data-locality
/// requirements without naming a concrete provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Locality {
    Local,
    Remote,
}

impl Locality {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
        }
    }
}

/// Execution constraint applied before provider availability is considered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionPolicy {
    PreferLocal,
    LocalOnly,
    RemoteOnly,
}

impl ExecutionPolicy {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "prefer-local" | "auto" => Ok(Self::PreferLocal),
            "local-only" => Ok(Self::LocalOnly),
            "remote-only" => Ok(Self::RemoteOnly),
            other => Err(format!(
                "Unknown provider policy: {other}. Use: prefer-local, local-only, remote-only"
            )),
        }
    }

    pub(crate) fn allows(self, locality: Locality) -> bool {
        match self {
            Self::PreferLocal => true,
            Self::LocalOnly => locality == Locality::Local,
            Self::RemoteOnly => locality == Locality::Remote,
        }
    }
}

/// Stable, capability-independent provider identity written into observations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderDescriptor {
    pub id: &'static str,
    pub locality: Locality,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AttemptOutcome {
    Unavailable,
    Failed,
    Selected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderAttempt {
    pub provider: String,
    pub outcome: AttemptOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A provider implements one capability. The resolver owns ordering and
/// fallback; implementations own only availability and execution.
pub trait CapabilityProvider {
    type Request: ?Sized;
    type Output;

    fn descriptor(&self) -> ProviderDescriptor;
    fn availability(&self) -> Result<(), String>;
    fn execute(&self, request: &Self::Request) -> Result<Self::Output, String>;
}

pub struct Resolution<T> {
    pub output: T,
    pub provider: ProviderDescriptor,
    pub attempts: Vec<ProviderAttempt>,
}

#[derive(Debug)]
pub struct AvailabilityResolution {
    pub provider: ProviderDescriptor,
    pub attempts: Vec<ProviderAttempt>,
}

#[derive(Debug)]
pub struct ResolutionError {
    pub message: String,
    pub attempts: Vec<ProviderAttempt>,
}

impl fmt::Display for ResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)?;
        if !self.attempts.is_empty() {
            let details = self
                .attempts
                .iter()
                .map(|attempt| {
                    let reason = attempt.reason.as_deref().unwrap_or("selected");
                    let outcome = match attempt.outcome {
                        AttemptOutcome::Unavailable => "unavailable",
                        AttemptOutcome::Failed => "failed",
                        AttemptOutcome::Selected => "selected",
                    };
                    format!("{} [{outcome}]: {reason}", attempt.provider)
                })
                .collect::<Vec<_>>()
                .join("; ");
            write!(f, " ({details})")?;
        }
        Ok(())
    }
}

/// Resolve the provider that would be selected from availability alone.
/// This is intended for diagnostics: it applies the same exact-provider and
/// locality-policy rules as execution without invoking the capability.
pub fn resolve_available<I: ?Sized, O>(
    providers: &[&dyn CapabilityProvider<Request = I, Output = O>],
    requested: &str,
    policy: ExecutionPolicy,
) -> Result<AvailabilityResolution, ResolutionError> {
    let exact = requested != "auto";
    if exact
        && !providers
            .iter()
            .any(|provider| provider.descriptor().id == requested)
    {
        return Err(ResolutionError {
            message: format!("Unknown provider: {requested}"),
            attempts: Vec::new(),
        });
    }

    let mut attempts = Vec::new();
    for provider in providers {
        let descriptor = provider.descriptor();
        if exact && descriptor.id != requested {
            continue;
        }
        if !policy.allows(descriptor.locality) {
            if exact {
                return Err(ResolutionError {
                    message: format!(
                        "Provider {} is {} but the selected policy does not allow it",
                        descriptor.id,
                        descriptor.locality.as_str()
                    ),
                    attempts,
                });
            }
            continue;
        }

        match provider.availability() {
            Ok(()) => {
                attempts.push(ProviderAttempt {
                    provider: descriptor.id.to_string(),
                    outcome: AttemptOutcome::Selected,
                    reason: None,
                });
                return Ok(AvailabilityResolution {
                    provider: descriptor,
                    attempts,
                });
            }
            Err(reason) => {
                attempts.push(ProviderAttempt {
                    provider: descriptor.id.to_string(),
                    outcome: AttemptOutcome::Unavailable,
                    reason: Some(reason),
                });
                if exact {
                    break;
                }
            }
        }
    }

    Err(ResolutionError {
        message: if exact {
            format!("Provider {requested} is unavailable")
        } else {
            "No eligible provider is available".to_string()
        },
        attempts,
    })
}

/// Resolve and invoke a provider. `auto` tries providers in caller-supplied
/// order; an explicit provider never falls back to a different implementation.
pub fn resolve<I: ?Sized, O>(
    providers: &[&dyn CapabilityProvider<Request = I, Output = O>],
    request: &I,
    requested: &str,
    policy: ExecutionPolicy,
) -> Result<Resolution<O>, ResolutionError> {
    let exact = requested != "auto";
    if exact
        && !providers
            .iter()
            .any(|provider| provider.descriptor().id == requested)
    {
        return Err(ResolutionError {
            message: format!("Unknown provider: {requested}"),
            attempts: Vec::new(),
        });
    }

    let mut attempts = Vec::new();
    for provider in providers {
        let descriptor = provider.descriptor();
        if exact && descriptor.id != requested {
            continue;
        }
        if !policy.allows(descriptor.locality) {
            if exact {
                return Err(ResolutionError {
                    message: format!(
                        "Provider {} is {} but the selected policy does not allow it",
                        descriptor.id,
                        descriptor.locality.as_str()
                    ),
                    attempts,
                });
            }
            continue;
        }

        if let Err(reason) = provider.availability() {
            attempts.push(ProviderAttempt {
                provider: descriptor.id.to_string(),
                outcome: AttemptOutcome::Unavailable,
                reason: Some(reason),
            });
            if exact {
                break;
            }
            continue;
        }

        match provider.execute(request) {
            Ok(output) => {
                attempts.push(ProviderAttempt {
                    provider: descriptor.id.to_string(),
                    outcome: AttemptOutcome::Selected,
                    reason: None,
                });
                return Ok(Resolution {
                    output,
                    provider: descriptor,
                    attempts,
                });
            }
            Err(reason) => {
                attempts.push(ProviderAttempt {
                    provider: descriptor.id.to_string(),
                    outcome: AttemptOutcome::Failed,
                    reason: Some(reason),
                });
                if exact {
                    break;
                }
            }
        }
    }

    let all_unavailable = attempts
        .iter()
        .all(|attempt| attempt.outcome == AttemptOutcome::Unavailable);
    Err(ResolutionError {
        message: match (exact, all_unavailable) {
            (true, true) => format!("Provider {requested} is unavailable"),
            (false, true) => "No eligible provider is available".to_string(),
            (true, false) => format!("Provider {requested} could not execute the capability"),
            (false, false) => "No eligible provider could execute the capability".to_string(),
        },
        attempts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeProvider {
        id: &'static str,
        locality: Locality,
        available: bool,
        result: Result<&'static str, &'static str>,
    }

    impl CapabilityProvider for FakeProvider {
        type Request = str;
        type Output = String;

        fn descriptor(&self) -> ProviderDescriptor {
            ProviderDescriptor {
                id: self.id,
                locality: self.locality,
                model: None,
                timing: None,
            }
        }

        fn availability(&self) -> Result<(), String> {
            self.available
                .then_some(())
                .ok_or_else(|| "not installed".to_string())
        }

        fn execute(&self, _request: &str) -> Result<String, String> {
            self.result.map(str::to_owned).map_err(str::to_owned)
        }
    }

    #[test]
    fn auto_falls_back_and_records_attempts() {
        let local = FakeProvider {
            id: "local",
            locality: Locality::Local,
            available: true,
            result: Err("model failed"),
        };
        let remote = FakeProvider {
            id: "remote",
            locality: Locality::Remote,
            available: true,
            result: Ok("done"),
        };
        let providers: [&dyn CapabilityProvider<Request = str, Output = String>; 2] =
            [&local, &remote];

        let result = resolve(&providers, "request", "auto", ExecutionPolicy::PreferLocal).unwrap();

        assert_eq!(result.output, "done");
        assert_eq!(result.provider.id, "remote");
        assert_eq!(result.attempts.len(), 2);
        assert_eq!(result.attempts[0].outcome, AttemptOutcome::Failed);
        assert_eq!(result.attempts[1].outcome, AttemptOutcome::Selected);
    }

    #[test]
    fn exact_provider_does_not_fall_back() {
        let local = FakeProvider {
            id: "local",
            locality: Locality::Local,
            available: false,
            result: Ok("unused"),
        };
        let remote = FakeProvider {
            id: "remote",
            locality: Locality::Remote,
            available: true,
            result: Ok("would hide the failure"),
        };
        let providers: [&dyn CapabilityProvider<Request = str, Output = String>; 2] =
            [&local, &remote];

        let error = resolve(&providers, "request", "local", ExecutionPolicy::PreferLocal)
            .err()
            .unwrap();

        assert_eq!(error.attempts.len(), 1);
        assert_eq!(error.attempts[0].provider, "local");
        assert_eq!(error.message, "Provider local is unavailable");
        assert!(
            error
                .to_string()
                .contains("local [unavailable]: not installed")
        );
    }

    #[test]
    fn locality_policy_filters_auto_candidates() {
        let local = FakeProvider {
            id: "local",
            locality: Locality::Local,
            available: true,
            result: Ok("local"),
        };
        let remote = FakeProvider {
            id: "remote",
            locality: Locality::Remote,
            available: true,
            result: Ok("remote"),
        };
        let providers: [&dyn CapabilityProvider<Request = str, Output = String>; 2] =
            [&local, &remote];

        let result = resolve(&providers, "request", "auto", ExecutionPolicy::RemoteOnly).unwrap();

        assert_eq!(result.provider.id, "remote");
        assert_eq!(result.output, "remote");
    }

    #[test]
    fn availability_resolution_applies_policy_without_executing() {
        let local = FakeProvider {
            id: "local",
            locality: Locality::Local,
            available: true,
            result: Err("must not execute"),
        };
        let remote = FakeProvider {
            id: "remote",
            locality: Locality::Remote,
            available: true,
            result: Err("must not execute"),
        };
        let providers: [&dyn CapabilityProvider<Request = str, Output = String>; 2] =
            [&local, &remote];

        let selected = resolve_available(&providers, "auto", ExecutionPolicy::RemoteOnly).unwrap();

        assert_eq!(selected.provider.id, "remote");
        assert_eq!(selected.attempts.len(), 1);
        assert_eq!(selected.attempts[0].outcome, AttemptOutcome::Selected);
    }

    #[test]
    fn availability_resolution_rejects_disallowed_exact_provider() {
        let remote = FakeProvider {
            id: "remote",
            locality: Locality::Remote,
            available: true,
            result: Ok("unused"),
        };
        let providers: [&dyn CapabilityProvider<Request = str, Output = String>; 1] = [&remote];

        let error =
            resolve_available(&providers, "remote", ExecutionPolicy::LocalOnly).unwrap_err();

        assert!(error.to_string().contains("does not allow"));
    }
}
