use super::*;

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
        effort: Some(ReasoningEffort::High),
        summary: Some("brief".to_owned()),
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
    assert_eq!(wire["outputSchema"]["type"], "object");
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

#[test]
fn validate_prompt_attachments_rejects_missing_path() {
    let err = validate_prompt_attachments(
        "/tmp",
        &[PromptAttachment::AtPath {
            path: "definitely_missing_file_12345.txt".to_owned(),
            placeholder: None,
        }],
    )
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
        .attach_path_with_placeholder("Docs/CORE_API.md", "core-doc")
        .attach_image_url("https://example.com/a.png")
        .attach_local_image("/tmp/a.png")
        .attach_skill("checks", "/tmp/skill")
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
        } if path == "Docs/CORE_API.md" && placeholder == "core-doc"
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
        cwd: Some("/work".to_owned()),
        approval_policy: Some(ApprovalPolicy::OnRequest),
        sandbox_policy: Some(SandboxPolicy::Preset(SandboxPreset::ReadOnly)),
        privileged_escalation_approved: false,
    };

    let wire = thread_start_params_to_wire(&params);
    assert_eq!(wire["model"], "gpt-5");
    assert_eq!(wire["cwd"], "/work");
    assert_eq!(wire["approvalPolicy"], "on-request");
    assert_eq!(wire["sandbox"], "read-only");
}
