use std::str::FromStr;
use std::time::Duration;

use serde_json::{json, Value};

use super::super::*;

#[test]
fn maps_turn_start_params_to_wire_shape() {
    let params = TurnStartParams {
        input: vec![
            InputItem::Text {
                text: "hello".to_owned(),
            },
            InputItem::LocalImage {
                path: "/tmp/a.png".to_owned(),
            },
        ],
        cwd: Some("/tmp".to_owned()),
        approval_policy: Some(ApprovalPolicy::Never),
        sandbox_policy: Some(SandboxPolicy::Preset(SandboxPreset::WorkspaceWrite {
            writable_roots: vec!["/tmp".to_owned()],
            network_access: false,
        })),
        privileged_escalation_approved: true,
        model: Some("gpt-5".to_owned()),
        service_tier: Some(Some(ServiceTier::Fast)),
        effort: Some(ReasoningEffort::High),
        summary: Some("brief".to_owned()),
        personality: Some(Personality::Pragmatic),
        output_schema: Some(json!({"type":"object"})),
    };

    let wire = turn_start_params_to_wire("thr_1", &params);
    assert_eq!(wire["threadId"], "thr_1");
    assert_eq!(wire["input"][0]["type"], "text");
    assert_eq!(wire["input"][0]["text"], "hello");
    assert_eq!(wire["input"][1]["type"], "localImage");
    assert_eq!(wire["input"][1]["path"], "/tmp/a.png");
    assert_eq!(wire["approvalPolicy"], "never");
    assert_eq!(wire["sandboxPolicy"]["type"], "workspaceWrite");
    assert_eq!(wire["sandboxPolicy"]["writableRoots"][0], "/tmp");
    assert_eq!(wire["sandboxPolicy"]["networkAccess"], false);
    assert_eq!(wire["serviceTier"], "fast");
    assert_eq!(wire["personality"], "pragmatic");
    assert_eq!(wire["outputSchema"]["type"], "object");
}

#[test]
fn skills_list_params_and_response_are_camel_case() {
    let params = SkillsListParams {
        cwds: vec!["/repo".to_owned()],
        force_reload: true,
        per_cwd_extra_user_roots: Some(vec![SkillsListExtraRootsForCwd {
            cwd: "/repo".to_owned(),
            extra_user_roots: vec!["/shared-skills".to_owned()],
        }]),
    };
    let wire = serde_json::to_value(&params).expect("serialize skills list params");
    assert_eq!(wire["cwds"][0], "/repo");
    assert_eq!(wire["forceReload"], true);
    assert_eq!(wire["perCwdExtraUserRoots"][0]["cwd"], "/repo");
    assert_eq!(
        wire["perCwdExtraUserRoots"][0]["extraUserRoots"][0],
        "/shared-skills"
    );

    let response: SkillsListResponse = serde_json::from_value(json!({
        "data": [{
            "cwd": "/repo",
            "skills": [{
                "name": "skill-creator",
                "description": "Create or update a Codex skill",
                "shortDescription": "Create skills",
                "interface": {
                    "displayName": "Skill Creator",
                    "defaultPrompt": "Create a new skill"
                },
                "dependencies": {
                    "tools": [{
                        "type": "mcp",
                        "value": "github",
                        "description": "Needs GitHub MCP"
                    }]
                },
                "path": "/repo/.agents/skills/skill-creator/SKILL.md",
                "scope": "repo",
                "enabled": true
            }],
            "errors": [{
                "path": "/repo/.agents/skills/broken/SKILL.md",
                "message": "invalid frontmatter"
            }]
        }]
    }))
    .expect("deserialize skills list response");
    assert_eq!(response.data.len(), 1);
    assert_eq!(response.data[0].cwd, "/repo");
    assert_eq!(response.data[0].skills[0].scope, SkillScope::Repo);
    assert_eq!(
        response.data[0].skills[0]
            .interface
            .as_ref()
            .and_then(|v| v.display_name.as_deref()),
        Some("Skill Creator")
    );
    assert_eq!(
        response.data[0].skills[0]
            .dependencies
            .as_ref()
            .map(|v| v.tools.len()),
        Some(1)
    );
    assert_eq!(response.data[0].errors[0].message, "invalid frontmatter");
}

#[test]
fn maps_text_with_elements_input_to_wire_shape() {
    let input = InputItem::TextWithElements {
        text: "check @README.md".to_owned(),
        text_elements: vec![TextElement {
            byte_range: ByteRange { start: 6, end: 16 },
            placeholder: Some("README".to_owned()),
        }],
    };
    let wire = input_item_to_wire(&input);
    assert_eq!(wire["type"], "text");
    assert_eq!(wire["text"], "check @README.md");
    assert_eq!(wire["text_elements"][0]["byteRange"]["start"], 6);
    assert_eq!(wire["text_elements"][0]["byteRange"]["end"], 16);
    assert_eq!(wire["text_elements"][0]["placeholder"], "README");
}

#[test]
fn builds_prompt_input_with_at_path_attachment() {
    let input = build_prompt_inputs(
        "summarize",
        &[PromptAttachment::AtPath {
            path: "README.md".to_owned(),
            placeholder: None,
        }],
    );
    assert_eq!(input.len(), 1);
    match &input[0] {
        InputItem::TextWithElements {
            text,
            text_elements,
        } => {
            assert_eq!(text, "summarize\n@README.md");
            assert_eq!(text_elements.len(), 1);
            assert_eq!(text_elements[0].byte_range.start, 10);
            assert_eq!(text_elements[0].byte_range.end, 20);
        }
        other => panic!("unexpected input variant: {other:?}"),
    }
}

#[test]
fn parses_policy_and_effort_from_str() {
    assert_eq!(
        ApprovalPolicy::from_str("on-request").expect("parse approval"),
        ApprovalPolicy::OnRequest
    );
    assert_eq!(
        ReasoningEffort::from_str("xhigh").expect("parse effort"),
        ReasoningEffort::XHigh
    );
    assert_eq!(
        ThreadListSortKey::from_str("updated_at").expect("parse thread list sort key"),
        ThreadListSortKey::UpdatedAt
    );
    assert!(ApprovalPolicy::from_str("always").is_err());
    assert!(ReasoningEffort::from_str("ultra").is_err());
    assert!(ThreadListSortKey::from_str("latest").is_err());

    let known_item_type: ThreadItemType =
        serde_json::from_value(json!("agentMessage")).expect("parse known item type");
    assert_eq!(known_item_type, ThreadItemType::AgentMessage);

    let unknown_item_type: ThreadItemType =
        serde_json::from_value(json!("futureType")).expect("parse unknown item type");
    assert_eq!(
        unknown_item_type,
        ThreadItemType::Unknown("futureType".to_owned())
    );
    assert_eq!(
        serde_json::to_value(&unknown_item_type).expect("serialize unknown item type"),
        json!("futureType")
    );
}

#[test]
fn parses_thread_item_payload_variants() {
    let agent: ThreadItemView = serde_json::from_value(json!({
        "id": "item_a",
        "type": "agentMessage",
        "text": "hello"
    }))
    .expect("parse agent item");
    assert_eq!(agent.id, "item_a");
    assert_eq!(agent.item_type, ThreadItemType::AgentMessage);
    match agent.payload {
        ThreadItemPayloadView::AgentMessage(data) => assert_eq!(data.text, "hello"),
        other => panic!("unexpected payload: {other:?}"),
    }

    let command: ThreadItemView = serde_json::from_value(json!({
        "id": "item_c",
        "type": "commandExecution",
        "command": "echo hi",
        "commandActions": [],
        "cwd": "/tmp",
        "status": "completed"
    }))
    .expect("parse command item");
    match command.payload {
        ThreadItemPayloadView::CommandExecution(data) => {
            assert_eq!(data.command, "echo hi");
            assert_eq!(data.cwd, "/tmp");
            assert_eq!(data.status, "completed");
        }
        other => panic!("unexpected payload: {other:?}"),
    }

    let unknown: ThreadItemView = serde_json::from_value(json!({
        "id": "item_u",
        "type": "futureType",
        "foo": "bar"
    }))
    .expect("parse unknown item");
    assert_eq!(
        unknown.item_type,
        ThreadItemType::Unknown("futureType".to_owned())
    );
    match unknown.payload {
        ThreadItemPayloadView::Unknown(fields) => {
            assert_eq!(fields.get("foo"), Some(&json!("bar")));
        }
        other => panic!("unexpected payload: {other:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn validate_prompt_attachments_rejects_missing_path() {
    let err = validate_prompt_attachments(
        "/tmp",
        &[PromptAttachment::AtPath {
            path: "definitely_missing_file_12345.txt".to_owned(),
            placeholder: None,
        }],
    )
    .await
    .expect_err("must fail");
    match err {
        PromptRunError::AttachmentNotFound(path) => {
            assert!(path.ends_with("/tmp/definitely_missing_file_12345.txt"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn prompt_run_params_defaults_are_explicit() {
    let params = PromptRunParams::new("/work", "hello");
    assert_eq!(params.cwd, "/work");
    assert_eq!(params.prompt, "hello");
    assert_eq!(params.effort, Some(DEFAULT_REASONING_EFFORT));
    assert_eq!(params.approval_policy, ApprovalPolicy::Never);
    assert_eq!(
        params.sandbox_policy,
        SandboxPolicy::Preset(SandboxPreset::ReadOnly)
    );
    assert!(!params.privileged_escalation_approved);
    assert_eq!(params.timeout, Duration::from_secs(120));
    assert_eq!(params.output_schema, None);
    assert!(params.attachments.is_empty());
}

#[test]
fn prompt_run_params_builder_overrides_defaults() {
    let params = PromptRunParams::new("/work", "hello")
        .with_model("gpt-5-codex")
        .with_effort(ReasoningEffort::High)
        .with_approval_policy(ApprovalPolicy::OnRequest)
        .with_sandbox_policy(SandboxPolicy::Preset(SandboxPreset::WorkspaceWrite {
            writable_roots: vec!["/work".to_owned()],
            network_access: false,
        }))
        .allow_privileged_escalation()
        .attach_path("README.md")
        .attach_path_with_placeholder("docs/API_REFERENCE.md", "core-doc")
        .attach_image_url("https://example.com/a.png")
        .attach_local_image("/tmp/a.png")
        .attach_skill("checks", "/tmp/skill")
        .with_output_schema(json!({"type":"object","required":["answer"]}))
        .with_timeout(Duration::from_secs(30));

    assert_eq!(params.cwd, "/work");
    assert_eq!(params.prompt, "hello");
    assert_eq!(params.model.as_deref(), Some("gpt-5-codex"));
    assert_eq!(params.effort, Some(ReasoningEffort::High));
    assert_eq!(params.approval_policy, ApprovalPolicy::OnRequest);
    assert_eq!(
        params.sandbox_policy,
        SandboxPolicy::Preset(SandboxPreset::WorkspaceWrite {
            writable_roots: vec!["/work".to_owned()],
            network_access: false,
        })
    );
    assert!(params.privileged_escalation_approved);
    assert_eq!(params.timeout, Duration::from_secs(30));
    assert_eq!(
        params.output_schema,
        Some(json!({"type":"object","required":["answer"]}))
    );
    assert_eq!(params.attachments.len(), 5);
    assert!(matches!(
        params.attachments[0],
        PromptAttachment::AtPath {
            ref path,
            placeholder: None
        } if path == "README.md"
    ));
    assert!(matches!(
        params.attachments[1],
        PromptAttachment::AtPath {
            ref path,
            placeholder: Some(ref placeholder)
        } if path == "docs/API_REFERENCE.md" && placeholder == "core-doc"
    ));
    assert!(matches!(
        params.attachments[2],
        PromptAttachment::ImageUrl { ref url } if url == "https://example.com/a.png"
    ));
    assert!(matches!(
        params.attachments[3],
        PromptAttachment::LocalImage { ref path } if path == "/tmp/a.png"
    ));
    assert!(matches!(
        params.attachments[4],
        PromptAttachment::Skill {
            ref name,
            ref path
        } if name == "checks" && path == "/tmp/skill"
    ));
}

#[test]
fn maps_thread_start_params_to_wire_shape() {
    let params = ThreadStartParams {
        model: Some("gpt-5".to_owned()),
        model_provider: Some("openai".to_owned()),
        service_tier: Some(None),
        cwd: Some("/work".to_owned()),
        approval_policy: Some(ApprovalPolicy::OnRequest),
        sandbox_policy: Some(SandboxPolicy::Preset(SandboxPreset::ReadOnly)),
        config: Some(serde_json::Map::from_iter([(
            "telemetry".to_owned(),
            json!(true),
        )])),
        service_name: Some("codex".to_owned()),
        base_instructions: Some("base".to_owned()),
        developer_instructions: Some("dev".to_owned()),
        personality: Some(Personality::Friendly),
        ephemeral: Some(true),
        privileged_escalation_approved: false,
    };

    let wire = thread_start_params_to_wire(&params);
    assert_eq!(wire["model"], "gpt-5");
    assert_eq!(wire["modelProvider"], "openai");
    assert_eq!(wire["serviceTier"], Value::Null);
    assert_eq!(wire["cwd"], "/work");
    assert_eq!(wire["approvalPolicy"], "on-request");
    assert_eq!(wire["sandbox"]["type"], "readOnly");
    assert_eq!(wire["config"]["telemetry"], true);
    assert_eq!(wire["serviceName"], "codex");
    assert_eq!(wire["baseInstructions"], "base");
    assert_eq!(wire["developerInstructions"], "dev");
    assert_eq!(wire["personality"], "friendly");
    assert_eq!(wire["ephemeral"], true);
}

#[test]
fn maps_thread_resume_overrides_to_supported_subset() {
    let params = ThreadStartParams {
        model: Some("gpt-5".to_owned()),
        model_provider: Some("openai".to_owned()),
        service_tier: Some(Some(ServiceTier::Flex)),
        cwd: Some("/work".to_owned()),
        approval_policy: Some(ApprovalPolicy::OnRequest),
        sandbox_policy: Some(SandboxPolicy::Preset(SandboxPreset::ReadOnly)),
        config: Some(serde_json::Map::from_iter([(
            "telemetry".to_owned(),
            json!(true),
        )])),
        service_name: Some("codex".to_owned()),
        base_instructions: Some("base".to_owned()),
        developer_instructions: Some("dev".to_owned()),
        personality: Some(Personality::Friendly),
        ephemeral: Some(true),
        privileged_escalation_approved: false,
    };

    let wire = super::super::wire::thread_overrides_to_wire(&params);
    assert_eq!(wire["model"], "gpt-5");
    assert_eq!(wire["modelProvider"], "openai");
    assert_eq!(wire["serviceTier"], "flex");
    assert_eq!(wire["sandbox"]["type"], "readOnly");
    assert_eq!(wire["config"]["telemetry"], true);
    assert_eq!(wire["baseInstructions"], "base");
    assert_eq!(wire["developerInstructions"], "dev");
    assert_eq!(wire["personality"], "friendly");
    assert!(!wire.contains_key("serviceName"));
    assert!(!wire.contains_key("ephemeral"));
}
