use super::*;
use crate::manual::{
    catalog::{self, Document, Model, Policy, Protocol, Source},
    metadata, sync,
};
use serde_json::{json, Value};

fn source(id: &str, model: Model) -> Source {
    Source {
        id: id.into(),
        name: id.into(),
        base_url: "https://fixture.invalid/v1".into(),
        key_env: String::new(),
        key_ref: None,
        copilot: None,
        protocol: Protocol::OpenaiChat,
        enabled: true,
        models: vec![model],
    }
}
fn model(id: &str) -> Model {
    Model {
        id: id.into(),
        enabled: true,
        ..Default::default()
    }
}

#[test]
fn matches_requested_families_without_broad_substring_or_version_matches() {
    for id in [
        "gpt-5.6",
        "gpt5.6",
        "GPT_5.6-luna",
        "openai/gpt-5.6-sol-fast",
        "gpt-6",
        "gpt-6-astra",
        "gpt-6.1-mini",
        "FW-gpt-6-astra",
        "Kimi-K2.7-Code",
        "FW-Kimi-K3",
        "moonshot/kimi-k2.7",
        "KimiK2",
        "GLM-4.7",
        "FW-GLM-5.3",
        "zai/glm5",
    ] {
        assert!(matches(id), "{id}");
    }
    for id in [
        "gpt-5.5",
        "gpt-5.60",
        "gpt-60",
        "gpt-7",
        "gpt-6abc",
        "mygpt-6",
        "gemini-gpt-6",
        "claude-sonnet",
        "qwen3-coder",
        "not-kimi",
        "kimicoder",
        "glmiddleware",
        "",
        "vendor/",
    ] {
        assert!(!matches(id), "{id}");
    }
}

#[test]
fn changes_only_limits_and_marks_them_as_user_configuration() {
    let mut item = model("gpt-5.6-sol-fast");
    item.context_window = Some(1_050_000);
    item.max_output_tokens = Some(64_000);
    item.tools = Some(false);
    item.reasoning = Some(false);
    item.metadata = json!({"id":item.id,"context_window":1_050_000,"max_output_tokens":64_000});
    let original = item.clone();
    apply(&mut item);
    assert_eq!(item.context_window, Some(1_000_000));
    assert_eq!(item.max_output_tokens, Some(128_000));
    assert_eq!(item.limits_source, Some(LimitSource::UserOverride));
    assert_eq!(item.id, original.id);
    assert_eq!(item.metadata, original.metadata);
    assert_eq!(item.tools, original.tools);
    assert_eq!(item.reasoning, original.reasoning);
    assert_eq!(item.input_modalities, original.input_modalities);
    assert_eq!(item.protocol_override, original.protocol_override);
    let mut unrelated = model("qwen3-coder");
    unrelated.context_window = Some(32_768);
    unrelated.max_output_tokens = Some(4096);
    let before = unrelated.clone();
    apply(&mut unrelated);
    assert_eq!(unrelated, before);
    let mut unknown = model("gpt-5.5");
    apply(&mut unknown);
    assert!(unknown.context_window.is_none());
    assert!(unknown.max_output_tokens.is_none());
}

#[test]
fn missing_deployment_fields_cannot_erase_effective_limits_or_change_routing_ids() {
    let mut declared = model("gpt-6-astra");
    declared.context_window = Some(1_050_000);
    declared.max_output_tokens = Some(128_000);
    declared.metadata = json!({"context_window":1_050_000,"max_output_tokens":128_000});
    let canonical = catalog::group_id(&Protocol::OpenaiResponses, &declared);
    let legacy = catalog::group_id(&Protocol::OpenaiChat, &declared);
    let mut doc = Document {
        providers: vec![
            source("declared", declared),
            source("unknown", model("gpt-6-astra")),
        ],
        ..Default::default()
    };
    let policy = Policy {
        failover: false,
        ..Default::default()
    };
    doc.policies.insert(canonical.clone(), policy.clone());
    let before = serde_json::to_value(&doc).unwrap();
    let groups = doc.groups();
    assert_eq!(groups.len(), 1);
    let group = &groups[0];
    assert_eq!(group.id, canonical);
    assert!(group.legacy_ids.contains(&legacy));
    assert_eq!(group.provider_ids.len(), 2);
    assert_eq!(group.policy, policy);
    assert_eq!(group.model.context_window, Some(1_000_000));
    assert_eq!(group.model.max_output_tokens, Some(128_000));
    assert_eq!(serde_json::to_value(&doc).unwrap(), before);
    let public = catalog::public_models(&doc);
    assert_eq!(public["data"][0]["context_window"], 1_000_000);
    assert_eq!(public["data"][0]["max_output_tokens"], 128_000);
    assert_eq!(public["data"][0]["limits_source"], "user_override");
}

#[test]
fn discovery_and_existing_documents_use_the_same_rule_without_trusting_upstream_provenance() {
    let raw = json!({"id":"FW-Kimi-K2.7-Code","limitsSource":"vendor","supports_tools":false});
    let discovered = metadata::parse_model(&raw, true).unwrap();
    assert!(discovered.context_window.is_none());
    assert!(discovered.limits_source.is_none());
    let doc = Document {
        providers: vec![source("fixture", discovered)],
        ..Default::default()
    };
    let serialized = serde_json::to_string(&doc).unwrap();
    let reloaded: Document = serde_json::from_str(&serialized).unwrap();
    assert_eq!(reloaded.providers[0].models[0].metadata, raw);
    let group = reloaded.groups().remove(0);
    assert_eq!(group.model.context_window, Some(CONTEXT_WINDOW));
    assert_eq!(group.model.max_output_tokens, Some(MAX_OUTPUT_TOKENS));
    let mut forged: Model =
        serde_json::from_value(json!({"id":"unrelated","limitsSource":"user_override"})).unwrap();
    assert!(forged.limits_source.is_none());
    apply(&mut forged);
    assert!(forged.limits_source.is_none());
}

#[test]
fn pi_and_codex_projections_use_effective_context_without_inventing_other_capabilities() {
    let doc = Document {
        providers: vec![source("fixture", model("gpt-6-astra"))],
        ..Default::default()
    };
    let group = doc.groups().remove(0);
    let pi = metadata::pi_model(&group.model, &group.id);
    assert_eq!(pi["contextWindow"], 1_000_000);
    assert_eq!(pi["maxTokens"], 128_000);
    assert!(pi.get("reasoning").is_none());
    assert!(pi.get("limitsSource").is_none());
    let mut codex = json!({"context_window":16_384,"max_context_window":16_384});
    metadata::enrich_codex_entry(&mut codex, &group.model);
    assert_eq!(codex["context_window"], 1_000_000);
    assert_eq!(codex["max_context_window"], 1_000_000);
    // The current Codex projection has no confirmed max-output schema field; don't fabricate one.
    assert!(codex.get("max_output_tokens").is_none());
    assert_eq!(codex["input_modalities"], json!(["text"]));
}

#[test]
fn explicit_sync_writes_limits_only_into_an_isolated_agent_fixture() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().canonicalize().unwrap();
    let doc = Document {
        providers: vec![source("fixture", model("FW-GLM-5.3"))],
        ..Default::default()
    };
    let plan = sync::Plan::build(&home, &doc, sync::Agent::Pi, "http://127.0.0.1:18080").unwrap();
    let file = home.join(".pi/agent/models.json");
    assert!(!file.exists());
    plan.apply(doc.revision).unwrap();
    let config: Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    let provider = config["providers"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    assert_eq!(provider["models"][0]["contextWindow"], 1_000_000);
    assert_eq!(provider["models"][0]["maxTokens"], 128_000);
    assert_eq!(provider["baseUrl"], "http://127.0.0.1:18080/v1");
    assert!(!home.join(".pi/agent/settings.json").exists());
}

#[test]
fn blocked_models_stay_out_of_the_public_and_sync_catalog() {
    let mut doc = Document {
        providers: vec![source("fixture", model("gpt-6-astra"))],
        ..Default::default()
    };
    doc.blocked_models.insert("gpt-6-astra".into());
    assert!(doc.groups()[0].blocked);
    assert_eq!(catalog::public_models(&doc)["data"], json!([]));
}
