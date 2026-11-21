use super::*;
use crate::session::agy_swarm::{load_agy_swarm, read_board_tail};
use std::collections::HashMap;
use std::path::Path;

#[test]
fn parses_agy_fixture_main_ops_and_subagents() {
    let tmp = std::env::temp_dir().join(format!("ccide_agy_test_{}.jsonl", std::process::id()));
    fs::write(&tmp, include_str!("../../tests/fixtures/agy/main.jsonl")).unwrap();
    let mut tr = JsonlTracker::from_tail(Some(tmp.clone()));
    tr.offset = 0;
    let ops = agy_main_file_ops(&tmp, &mut tr).unwrap();
    assert_eq!(
        (ops.tool.as_str(), ops.file.as_str(), ops.seq),
        ("Write", "/workspace/demo/src/helper.rs", 3)
    );
    assert_eq!(ops.model, "Claude Opus 4.6 (Thinking)");
    let mut str_tr = JsonlTracker::from_tail(Some(tmp.clone()));
    let subs = agy_subagents(&tmp, &mut str_tr);
    assert_eq!(subs.len(), 2);
    assert_eq!(
        (subs[0].full_id.as_str(), subs[0].desc.as_str()),
        ("tc-sub-1", "reviewer")
    );
    assert_eq!(subs[0].model_raw.as_deref(), Some("gemini-3.8-flash"));
    assert_eq!(
        (subs[1].desc.as_str(), subs[1].model_raw.as_deref()),
        ("worker-1", Some("gpt-5.4"))
    );
    assert!(agy_transcript_path("../evil").as_os_str().is_empty());
    assert!(agy_transcript_path("a/b").as_os_str().is_empty());

    let args_j = r#"{"Path":"\"src/quoted\\\"f.rs\"","StartLine":"80","EndLine":"95"}"#;
    assert_eq!(
        extract_agy_arg_str(args_j, "Path").as_deref(),
        Some("src/quoted\"f.rs")
    );
    assert_eq!(extract_agy_arg_u64(args_j, "StartLine"), Some(80));
    let tc_args = r#"{"name":"replace_file_content","args":{"TargetFile":"\"src/lib.rs\"","StartLine":"12","EndLine":"15"}}"#;
    assert_eq!(
        map_agy_tool_call(tc_args),
        Some(("Edit", "src/lib.rs".into(), 12, 4))
    );
    let tc_int = r#"{"name":"read_file","args":{"Path":"/Users/u/.gemini/antigravity/brain/c1/x.md","StartLine":1}}"#;
    assert_eq!(map_agy_tool_call(tc_int), None);
    let _ = fs::remove_file(&tmp);
}

#[test]
fn handles_corrupt_oversized_and_unsafe_swarm_files() {
    let root = std::env::temp_dir().join(format!("ccide_agy_bad_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let sg = root.join("brain/c1/.system_generated");
    let tr = sg.join("logs/transcript.jsonl");
    let wf = |p: &Path, b: &str| {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b).unwrap();
    };
    wf(&tr, "{}\n");
    wf(&sg.join("swarm/board.jsonl"), "{bad\n");
    assert_eq!(
        read_board_tail(&root.join("missing.jsonl"), 65_536),
        (0, vec![])
    );
    assert_eq!(
        read_board_tail(&sg.join("swarm/board.jsonl"), 65_536),
        (0, vec![])
    );
    wf(&sg.join("swarm/members.json"), &"{".repeat(262_200));
    let (mut subs, mut fb) = (HashMap::new(), JsonlTracker::from_tail(None));
    assert!(!load_agy_swarm(&root.to_string_lossy(), "c1", &tr, &mut subs, &mut fb).active);
    wf(
        &sg.join("swarm/members.json"),
        r#"{"title":"T","members":{"bad":{"conversation_id":"../evil","phase":"RUNNING"}}}"#,
    );
    wf(
        &sg.join("subagents/bad.json"),
        r#"{"conversationId":"a/b","state":"SUBAGENT_STATE_ALIVE"}"#,
    );
    let snap = load_agy_swarm(&root.to_string_lossy(), "c1", &tr, &mut subs, &mut fb);
    assert!(snap.members.is_empty());
    let _ = fs::remove_dir_all(&root);
}
