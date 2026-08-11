use serde_json::json;

use crate::{
    Capability, EchoHost, Engine, Host, Journal, JournalEntry, RunOptions, WorkflowError,
    run_with_host,
};

const DEMO: &str = r#"
let meta = #{
    name: "demo",
    description: "hash-resume demo",
    args_schema: #{
        type: "object",
        properties: #{
            topic: #{ type: "string", minLength: 1 }
        },
        required: ["topic"]
    },
    phases: [#{ title: "Work", detail: "Do work" }]
};

phase("Work");
let answer = agent("research: " + args.topic, #{ label: "worker" });
let reviews = parallel([
    #{ prompt: "check-" + args.topic, label: "a" },
    #{ prompt: "simp-" + args.topic, label: "b" }
]);
complete(#{ answer: answer.output, reviews: reviews });
"#;

#[test]
fn runs_and_resumes_with_matching_checksums() -> Result<(), WorkflowError> {
    let engine = Engine::new(EchoHost);
    let first = engine.run_script(
        DEMO,
        RunOptions {
            args: Some(json!({ "topic": "hashing" })),
            ..RunOptions::default()
        },
    )?;
    assert_eq!(first.phases, vec!["Work".to_owned()]);
    assert_eq!(first.journal.entries().len(), 2);
    assert_eq!(first.workflow_hash.as_str().len(), 64);
    assert_eq!(first.input_hash.as_str().len(), 64);
    assert_eq!(first.journal.workflow_hash(), Some(&first.workflow_hash));
    assert_eq!(first.journal.input_hash(), Some(&first.input_hash));

    let second = engine.run_script(
        DEMO,
        RunOptions {
            args: Some(json!({ "topic": "hashing" })),
            journal: first.journal.clone(),
            ..RunOptions::default()
        },
    )?;
    assert_eq!(second.complete, first.complete);
    assert_eq!(second.journal, first.journal);
    Ok(())
}

#[test]
fn rejects_resume_when_input_hash_changes() -> Result<(), WorkflowError> {
    let first = run_with_host(
        EchoHost,
        DEMO,
        RunOptions {
            args: Some(json!({ "topic": "hashing" })),
            ..RunOptions::default()
        },
    )?;

    let error = run_with_host(
        EchoHost,
        DEMO,
        RunOptions {
            args: Some(json!({ "topic": "changed" })),
            journal: first.journal,
            ..RunOptions::default()
        },
    )
    .expect_err("input change must diverge");

    assert!(matches!(error, WorkflowError::JournalDivergence(_)));
    assert!(error.to_string().contains("input_hash"));
    Ok(())
}

#[test]
fn rejects_resume_when_workflow_hash_changes() -> Result<(), WorkflowError> {
    let first = run_with_host(
        EchoHost,
        DEMO,
        RunOptions {
            args: Some(json!({ "topic": "hashing" })),
            ..RunOptions::default()
        },
    )?;

    let altered = DEMO.replace("hash-resume demo", "altered description");

    let error = run_with_host(
        EchoHost,
        &altered,
        RunOptions {
            args: Some(json!({ "topic": "hashing" })),
            journal: first.journal,
            ..RunOptions::default()
        },
    )
    .expect_err("workflow change must diverge");

    assert!(matches!(error, WorkflowError::JournalDivergence(_)));
    assert!(error.to_string().contains("workflow_hash"));
    Ok(())
}

#[test]
fn journal_round_trips_through_json() -> Result<(), WorkflowError> {
    let result = run_with_host(
        EchoHost,
        DEMO,
        RunOptions {
            args: Some(json!({ "topic": "hashing" })),
            ..RunOptions::default()
        },
    )?;
    let encoded = result.journal.to_json()?;
    let decoded = Journal::from_json(&encoded)?;
    assert_eq!(decoded, result.journal);
    Ok(())
}

#[test]
fn resume_from_checkpoint_file_matches_original_run() -> Result<(), WorkflowError> {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let path = std::env::temp_dir().join(format!(
        "sui-workflow-resume-{}-{}.json",
        std::process::id(),
        stamp
    ));

    let first = run_with_host(
        EchoHost,
        DEMO,
        RunOptions {
            args: Some(json!({ "topic": "hashing" })),
            checkpoint: Some(path.clone()),
            ..RunOptions::default()
        },
    )?;

    let on_disk = fs::read_to_string(&path).map_err(|error| WorkflowError::io(&path, error))?;
    let loaded = Journal::from_json(&on_disk)?;
    assert_eq!(loaded, first.journal);
    assert_eq!(loaded.workflow_hash(), Some(&first.workflow_hash));
    assert_eq!(loaded.input_hash(), Some(&first.input_hash));

    let second = run_with_host(
        EchoHost,
        DEMO,
        RunOptions {
            args: Some(json!({ "topic": "hashing" })),
            journal: loaded,
            ..RunOptions::default()
        },
    )?;

    assert_eq!(second.complete, first.complete);
    assert_eq!(second.workflow_hash, first.workflow_hash);
    assert_eq!(second.input_hash, first.input_hash);
    assert_eq!(second.journal, first.journal);
    assert_eq!(second.phases, first.phases);
    assert_eq!(second.logs, first.logs);

    let _ = fs::remove_file(&path);
    Ok(())
}

#[test]
fn rejects_resume_when_prompt_request_hash_changes() -> Result<(), WorkflowError> {
    let first = run_with_host(
        EchoHost,
        DEMO,
        RunOptions {
            args: Some(json!({ "topic": "hashing" })),
            ..RunOptions::default()
        },
    )?;

    let mut entries = first.journal.entries().to_vec();
    let JournalEntry::Agent {
        request,
        request_hash,
        ..
    } = &mut entries[0]
    else {
        return Err(WorkflowError::InvalidConfig(
            "expected first journal entry to be an agent call".into(),
        ));
    };
    request.prompt = "tampered-prompt".into();
    *request_hash = request.content_hash()?;

    let journal = Journal::from_parts(
        first.journal.workflow_hash().cloned(),
        first.journal.input_hash().cloned(),
        entries,
    )?;

    let error = run_with_host(
        EchoHost,
        DEMO,
        RunOptions {
            args: Some(json!({ "topic": "hashing" })),
            journal,
            ..RunOptions::default()
        },
    )
    .expect_err("prompt checksum change must diverge");

    assert!(matches!(error, WorkflowError::JournalDivergence(_)));
    assert!(error.to_string().contains("request_hash"));
    Ok(())
}

#[test]
fn await_user_pauses_then_resumes() -> Result<(), WorkflowError> {
    let script = r#"
let meta = #{
    name: "gate",
    description: "await user gate"
};
await_user("approval", "continue?");
complete(#{ done: true });
"#;
    let first = run_with_host(EchoHost, script, RunOptions::default())?;
    assert!(first.complete.is_none());
    assert_eq!(
        first.paused.as_ref().map(|pause| pause.kind.as_str()),
        Some("approval")
    );
    assert_eq!(first.journal.entries().len(), 1);

    let second = run_with_host(
        EchoHost,
        script,
        RunOptions {
            journal: first.journal,
            ..RunOptions::default()
        },
    )?;
    assert_eq!(second.complete, Some(json!({ "done": true })));
    assert!(second.paused.is_none());
    Ok(())
}

#[test]
fn rejects_tampered_result_hash() -> Result<(), WorkflowError> {
    let first = run_with_host(
        EchoHost,
        DEMO,
        RunOptions {
            args: Some(json!({ "topic": "hashing" })),
            ..RunOptions::default()
        },
    )?;
    let mut entries = first.journal.entries().to_vec();
    let JournalEntry::Agent { result, .. } = &mut entries[0] else {
        return Err(WorkflowError::InvalidConfig(
            "expected first journal entry to be an agent call".into(),
        ));
    };
    result.output = "forged".into();

    let error = Journal::from_parts(
        first.journal.workflow_hash().cloned(),
        first.journal.input_hash().cloned(),
        entries,
    )
    .expect_err("result payload/hash mismatch must fail validation");
    assert!(matches!(error, WorkflowError::JournalDivergence(_)));
    assert!(error.to_string().contains("result_hash"));
    Ok(())
}

#[test]
fn denies_capability_above_host_grant() {
    #[derive(Default)]
    struct ReadOnlyHost;

    impl Host for ReadOnlyHost {
        fn granted_capability(&self) -> Capability {
            Capability::ReadOnly
        }

        fn run_agent(
            &self,
            request: &crate::AgentRequest,
        ) -> Result<crate::AgentResult, crate::HostError> {
            EchoHost.run_agent(request)
        }
    }

    let script = r#"
let meta = #{
    name: "cap",
    description: "capability check"
};
agent("edit files", #{ capability_mode: "execute" });
complete(#{ ok: true });
"#;
    let error = run_with_host(ReadOnlyHost, script, RunOptions::default())
        .expect_err("execute must be denied");
    assert!(matches!(error, WorkflowError::CapabilityDenied { .. }));
}

#[test]
fn validates_args_schema_before_run() {
    let error = run_with_host(
        EchoHost,
        DEMO,
        RunOptions {
            args: Some(json!({})),
            ..RunOptions::default()
        },
    )
    .expect_err("missing topic must fail schema validation");
    assert!(matches!(error, WorkflowError::InvalidConfig(_)));
}

#[test]
fn fingerprint_helper_is_sha256_hex() -> Result<(), WorkflowError> {
    let script = r#"
let meta = #{
    name: "fp",
    description: "fingerprint"
};
complete(#{ id: fingerprint("abc") });
"#;
    let result = run_with_host(EchoHost, script, RunOptions::default())?;
    assert_eq!(
        result.complete,
        Some(json!({
            "id": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        }))
    );
    Ok(())
}
