use super::*;

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
    let _ = fs::remove_file(&tmp);
}
