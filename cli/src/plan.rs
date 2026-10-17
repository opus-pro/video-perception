use crate::extensions;
use crate::registry::{self, Backend, SensorDef};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

pub const SENSE_PLAN_CONTRACT: &str = "video-perception.sense-plan/v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SensePlan {
    pub contract: String,
    pub capabilities: Vec<CapabilityRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_backend: Option<ModelBackendConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRequest {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<CapabilityOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CapabilityOptions {
    /// Scene-change confidence threshold in the inclusive range [0, 1].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    /// Merge detected scenes shorter than this duration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_ms: Option<u64>,
    /// Disable scene merging when false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModelBackendConfig {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl SensePlan {
    pub fn load(path: &Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|error| format!("read SensePlan {}: {error}", path.display()))?;
        let plan: Self = serde_json::from_str(&content)
            .map_err(|error| format!("parse SensePlan {}: {error}", path.display()))?;
        plan.validate()?;
        Ok(plan)
    }

    pub fn from_capabilities(ids: &[String]) -> Result<Self, String> {
        let plan = Self {
            contract: SENSE_PLAN_CONTRACT.to_string(),
            capabilities: ids
                .iter()
                .map(|id| CapabilityRequest {
                    id: id.clone(),
                    required: None,
                    provider: None,
                    policy: None,
                    language: None,
                    options: None,
                })
                .collect(),
            model_backend: None,
        };
        plan.validate()?;
        Ok(plan)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.contract != SENSE_PLAN_CONTRACT {
            return Err(format!(
                "unsupported SensePlan contract: {} (expected {SENSE_PLAN_CONTRACT})",
                self.contract
            ));
        }
        if self.capabilities.is_empty() {
            return Err("SensePlan must request at least one capability".into());
        }

        let mut seen = HashSet::new();
        for request in &self.capabilities {
            if !seen.insert(request.id.as_str()) {
                return Err(format!("duplicate capability in SensePlan: {}", request.id));
            }
            let definition = registry::find(&request.id).ok_or_else(|| {
                format!(
                    "unknown capability in SensePlan: {}. Run `video-perception catalog list`.",
                    request.id
                )
            })?;
            if definition.required && request.required == Some(false) {
                return Err(format!(
                    "capability {} is a required technical dependency and cannot be optional",
                    request.id
                ));
            }
            validate_provider_options(request)?;
            validate_capability_options(request)?;
        }

        if let Some(backend) = &self.model_backend
            && !matches!(backend.provider.as_str(), "ollama" | "gemini")
        {
            if backend.model.is_none() {
                return Err(format!(
                    "custom model backend {} requires an explicit model",
                    backend.provider
                ));
            }
            let capabilities = self
                .capabilities
                .iter()
                .filter(|request| {
                    registry::find(&request.id)
                        .is_some_and(|definition| matches!(definition.backend, Backend::Vlm(_)))
                })
                .collect::<Vec<_>>();
            if capabilities.is_empty() {
                return Err(format!(
                    "custom model backend {} requires at least one VLM capability",
                    backend.provider
                ));
            }
            for request in capabilities {
                match extensions::find_provider(&backend.provider, &request.id)? {
                    Some(provider)
                        if matches!(
                            provider.runtime.kind.as_str(),
                            "command" | "openai-compatible" | "ollama-compatible"
                        ) => {}
                    Some(provider) => {
                        return Err(format!(
                            "model backend provider {} uses non-executable runtime {}",
                            backend.provider, provider.runtime.kind
                        ));
                    }
                    None => {
                        return Err(format!(
                            "model backend provider {} does not implement capability {}; run `video-perception extension list --json`",
                            backend.provider, request.id
                        ));
                    }
                }
            }
        }

        self.resolved_plan()?;
        Ok(())
    }

    pub fn resolved_plan(&self) -> Result<Vec<&'static SensorDef>, String> {
        let mut ids: Vec<String> = self
            .capabilities
            .iter()
            .map(|request| request.id.clone())
            .collect();
        // Every Observation needs stable asset metadata and scene coordinates,
        // even when the consumer only requests a routed capability such as
        // transcript. These are artifact-level technical dependencies, not
        // consumer-selected analysis.
        for base in ["probe", "scenes"] {
            if !ids.iter().any(|id| id == base) {
                ids.push(base.to_string());
            }
        }
        registry::resolve_plan(&ids)
    }

    pub fn request_for(&self, id: &str) -> Option<&CapabilityRequest> {
        self.capabilities.iter().find(|request| request.id == id)
    }

    pub fn is_requested(&self, id: &str) -> bool {
        self.request_for(id).is_some()
    }

    pub fn is_required(&self, definition: &SensorDef) -> bool {
        self.request_for(definition.id)
            .and_then(|request| request.required)
            .unwrap_or(definition.required)
    }
}

impl ModelBackendConfig {
    pub fn model(&self) -> &str {
        self.model
            .as_deref()
            .unwrap_or_else(|| default_model(&self.provider))
    }
}

fn validate_provider_options(request: &CapabilityRequest) -> Result<(), String> {
    if request.id != "transcript" && request.language.is_some() {
        return Err(format!(
            "language is not supported for capability {}",
            request.id
        ));
    }
    if request.id != "transcript" && request.policy.is_some() && request.provider.is_none() {
        return Err(format!(
            "policy for capability {} requires an explicit provider",
            request.id
        ));
    }

    if let Some(provider) = request.provider.as_deref() {
        let built_in = provider == "auto"
            || (request.id == "transcript" && matches!(provider, "apple-speech" | "whisper-cpp"));
        if !built_in {
            match extensions::find_provider(provider, &request.id)? {
                Some(manifest) if manifest.runtime.kind == "command" => {}
                Some(manifest) => {
                    return Err(format!(
                        "capability provider {provider} uses runtime {}; select compatible model providers through model_backend",
                        manifest.runtime.kind
                    ));
                }
                None => {
                    return Err(format!(
                        "provider {provider} does not implement capability {}; run `video-perception extension list --json`",
                        request.id
                    ));
                }
            }
        }
    }

    if let Some(policy) = request.policy.as_deref() {
        if !matches!(policy, "prefer-local" | "local-only" | "remote-only") {
            return Err(format!(
                "invalid transcript policy: {policy}. Use prefer-local, local-only, or remote-only"
            ));
        }
    }

    Ok(())
}

fn validate_capability_options(request: &CapabilityRequest) -> Result<(), String> {
    let Some(options) = request.options.as_ref() else {
        return Ok(());
    };
    if request.id != "scenes" {
        return Err(format!(
            "technical options are not supported for capability {}",
            request.id
        ));
    }
    if let Some(threshold) = options.threshold {
        if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
            return Err("scenes options.threshold must be between 0 and 1".into());
        }
    }
    Ok(())
}

fn default_model(provider: &str) -> &'static str {
    match provider {
        "ollama" => "gemma4",
        "gemini" => "gemini-2.5-flash",
        _ => "default",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(id: &str) -> CapabilityRequest {
        CapabilityRequest {
            id: id.into(),
            required: None,
            provider: None,
            policy: None,
            language: None,
            options: None,
        }
    }

    #[test]
    fn explicit_optional_capability_uses_registry_default() {
        let plan = SensePlan {
            contract: SENSE_PLAN_CONTRACT.into(),
            capabilities: vec![request("transcript")],
            model_backend: None,
        };
        plan.validate().unwrap();
        assert!(!plan.is_required(registry::find("transcript").unwrap()));
    }

    #[test]
    fn requested_capability_can_be_promoted_to_required() {
        let mut transcript = request("transcript");
        transcript.required = Some(true);
        let plan = SensePlan {
            contract: SENSE_PLAN_CONTRACT.into(),
            capabilities: vec![transcript],
            model_backend: None,
        };
        plan.validate().unwrap();
        assert!(plan.is_required(registry::find("transcript").unwrap()));
    }

    #[test]
    fn registry_required_capability_cannot_be_demoted() {
        let mut probe = request("probe");
        probe.required = Some(false);
        let plan = SensePlan {
            contract: SENSE_PLAN_CONTRACT.into(),
            capabilities: vec![probe],
            model_backend: None,
        };
        assert!(plan.validate().unwrap_err().contains("cannot be optional"));
    }

    #[test]
    fn dependencies_are_resolved_without_adding_unrelated_capabilities() {
        let plan = SensePlan::from_capabilities(&["blur".into()]).unwrap();
        let ids: Vec<&str> = plan
            .resolved_plan()
            .unwrap()
            .iter()
            .map(|definition| definition.id)
            .collect();
        assert_eq!(ids, vec!["probe", "scenes", "stats", "blur"]);
    }

    #[test]
    fn observation_base_is_a_technical_dependency() {
        let plan = SensePlan::from_capabilities(&["transcript".into()]).unwrap();
        let ids: Vec<&str> = plan
            .resolved_plan()
            .unwrap()
            .iter()
            .map(|definition| definition.id)
            .collect();
        assert_eq!(ids, vec!["probe", "scenes", "transcript"]);
        assert!(!plan.is_requested("probe"));
        assert!(!plan.is_requested("scenes"));
    }

    #[test]
    fn duplicate_capability_is_rejected() {
        let plan = SensePlan {
            contract: SENSE_PLAN_CONTRACT.into(),
            capabilities: vec![request("scenes"), request("scenes")],
            model_backend: None,
        };
        assert!(
            plan.validate()
                .unwrap_err()
                .contains("duplicate capability")
        );
    }

    #[test]
    fn provider_options_are_capability_specific() {
        let mut scenes = request("scenes");
        scenes.policy = Some("local-only".into());
        let plan = SensePlan {
            contract: SENSE_PLAN_CONTRACT.into(),
            capabilities: vec![scenes],
            model_backend: None,
        };
        assert!(
            plan.validate()
                .unwrap_err()
                .contains("requires an explicit provider")
        );
    }

    #[test]
    fn scene_options_are_validated_and_capability_specific() {
        let mut scenes = request("scenes");
        scenes.options = Some(CapabilityOptions {
            threshold: Some(0.15),
            merge_ms: Some(250),
            merge: Some(false),
        });
        let mut plan = SensePlan {
            contract: SENSE_PLAN_CONTRACT.into(),
            capabilities: vec![scenes],
            model_backend: None,
        };
        plan.validate().unwrap();

        plan.capabilities[0].options.as_mut().unwrap().threshold = Some(1.5);
        assert!(
            plan.validate()
                .unwrap_err()
                .contains("threshold must be between 0 and 1")
        );

        let mut stats = request("stats");
        stats.options = Some(CapabilityOptions {
            threshold: Some(0.15),
            merge_ms: None,
            merge: None,
        });
        plan.capabilities = vec![stats];
        assert!(
            plan.validate()
                .unwrap_err()
                .contains("technical options are not supported")
        );
    }

    #[test]
    fn invalid_contract_and_unknown_capability_are_rejected() {
        let mut wrong_contract = SensePlan::from_capabilities(&["probe".into()]).unwrap();
        wrong_contract.contract = "video-perception.sense-plan/v0".into();
        assert!(
            wrong_contract
                .validate()
                .unwrap_err()
                .contains("unsupported SensePlan")
        );

        assert!(
            SensePlan::from_capabilities(&["consumer-verdict".into()])
                .unwrap_err()
                .contains("unknown capability")
        );
    }

    #[test]
    fn invalid_provider_policy_and_model_backend_are_rejected() {
        let mut transcript = request("transcript");
        transcript.provider = Some("mystery".into());
        let mut plan = SensePlan {
            contract: SENSE_PLAN_CONTRACT.into(),
            capabilities: vec![transcript],
            model_backend: None,
        };
        assert!(
            plan.validate()
                .unwrap_err()
                .contains("does not implement capability transcript")
        );

        plan.capabilities[0].provider = Some("auto".into());
        plan.capabilities[0].policy = Some("upload-everything".into());
        assert!(
            plan.validate()
                .unwrap_err()
                .contains("invalid transcript policy")
        );

        plan.capabilities[0].policy = Some("local-only".into());
        plan.model_backend = Some(ModelBackendConfig {
            provider: "consumer-service".into(),
            model: None,
        });
        assert!(
            plan.validate()
                .unwrap_err()
                .contains("custom model backend")
        );
    }
}
