use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::*;
use crate::session::subagent_state::append_dismissed_subagents;

struct EnvGuard(&'static str, Option<String>);
impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.1 {
            Some(v) => std::env::set_var(self.0, v),
            None => std::env::remove_var(self.0),
        }
    }
}

fn write_f(p: &Path, body: &str) {
    if let Some(par) = p.parent() {
        fs::create_dir_all(par).unwrap();
    }
    fs::write(p, body).unwrap();
}

fn setup_swarm_fixture(tag: &str) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("ccide_swarm_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let agy = root.join("agy");
    let br = agy.join("brain");
    let tr = br.join("coord-001/.system_generated/logs/transcript.jsonl");
    write_f(
        &tr,
        include_str!("../../tests/fixtures/agy_swarm/brain/coord-001/.system_generated/logs/transcript.jsonl"),
    );
    write_f(
        &br.join("coord-001/.system_generated/swarm/members.json"),
        include_str!(
            "../../tests/fixtures/agy_swarm/brain/coord-001/.system_generated/swarm/members.json"
        ),
    );
    write_f(
        &br.join("coord-001/.system_generated/swarm/board.jsonl"),
        include_str!(
            "../../tests/fixtures/agy_swarm/brain/coord-001/.system_generated/swarm/board.jsonl"
        ),
    );
    write_f(
        &br.join("coord-001/.system_generated/subagents/sub-extra-001.json"),
        include_str!("../../tests/fixtures/agy_swarm/brain/coord-001/.system_generated/subagents/sub-extra-001.json"),
    );
    write_f(
        &br.join("worker-opus-001/.system_generated/logs/transcript.jsonl"),
        include_str!("../../tests/fixtures/agy_swarm/brain/worker-opus-001/.system_generated/logs/transcript.jsonl"),
    );
    write_f(
        &br.join("worker-argon-002/.system_generated/logs/transcript.jsonl"),
        include_str!("../../tests/fixtures/agy_swarm/brain/worker-argon-002/.system_generated/logs/transcript.jsonl"),
    );
    (root, tr)
}

#[test]
fn loads_swarm_fixture_board_subagents_and_dismissal() {
    let (root, tr) = setup_swarm_fixture("load");
    let home = root.join("home");
    fs::create_dir_all(&home).unwrap();
    let home_s = home.to_string_lossy().into_owned();
    let (mut subs, mut fb) = (HashMap::new(), JsonlTracker::from_tail(None));

    let snap = load_agy_swarm(&home_s, "coord-001", &tr, &mut subs, &mut fb);
    assert!(snap.active);
    assert_eq!(snap.title, "Demo Swarm");
    assert_eq!(
        snap.coordinator_model.as_deref(),
        Some("gemini-flash-worker-agy")
    );
    assert_eq!(snap.board_head, 6);
    assert_eq!(snap.board_latest.len(), 3);
    assert_eq!(snap.board_latest[0].0, "coordinator");
    assert_eq!(snap.board_latest[0].1, 6);
    assert!(snap.board_latest[0]
        .2
        .contains("Wait for backends completion"));
    assert_eq!(snap.worker_transcripts.len(), 4);

    let ids: Vec<_> = snap
        .members
        .iter()
        .map(|m| (m.full_id.as_str(), m.status))
        .collect();
    assert_eq!(
        ids,
        vec![
            ("sub-extra-001", "running"),
            ("worker-argon-002", "running"),
            ("worker-opus-001", "waiting"),
            ("worker-stopped-004", "stopped"),
            ("worker-killed-003", "killed"),
        ]
    );
    assert_eq!(snap.members[0].desc, "Codebase Researcher");
    assert_eq!(snap.members[0].raw_type, "research");
    assert_eq!(
        (snap.members[1].tool.as_str(), snap.members[1].file.as_str()),
        ("Edit", "src/swarm.rs")
    );
    assert_eq!(
        (snap.members[2].tool.as_str(), snap.members[2].file.as_str()),
        ("Read", "src/arch.rs")
    );
    assert_eq!(snap.members[4].footer_rank, 5);
    assert!(is_subagent_alive("running") && is_subagent_alive("waiting"));
    assert!(!is_subagent_alive("killed") && !is_subagent_alive("stopped"));

    append_dismissed_subagents(
        &home_s,
        "coord-001",
        &["worker-killed-003".into(), "worker-opus-001".into()],
    );
    let snap2 = load_agy_swarm(&home_s, "coord-001", &tr, &mut subs, &mut fb);
    assert!(!snap2
        .members
        .iter()
        .any(|m| m.full_id == "worker-killed-003"));
    assert!(snap2.members.iter().any(|m| m.full_id == "worker-opus-001"));
    assert_eq!(snap2.worker_transcripts.len(), 4);

    let (h_miss, l_miss) = read_board_tail(&root.join("missing.jsonl"), 65_536);
    assert_eq!((h_miss, l_miss.len()), (0, 0));
    let (h_tail, _) = read_board_tail(
        &root.join("agy/brain/coord-001/.system_generated/swarm/board.jsonl"),
        200,
    );
    assert_eq!(h_tail, 6);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn rejects_unsafe_ids_and_resolves_cid_precedence() {
    let (root, tr) = setup_swarm_fixture("resolve");
    let bad_members =
        r#"{"title":"T","members":{"bad":{"conversation_id":"../evil","phase":"RUNNING"}}}"#;
    write_f(
        &root.join("agy/brain/coord-001/.system_generated/swarm/members.json"),
        bad_members,
    );
    write_f(
        &root.join("agy/brain/coord-001/.system_generated/subagents/bad.json"),
        r#"{"conversationId":"a/b","state":"SUBAGENT_STATE_ALIVE"}"#,
    );
    let (mut subs, mut fb) = (HashMap::new(), JsonlTracker::from_tail(None));
    let snap = load_agy_swarm(
        &root.to_string_lossy(),
        "coord-001",
        &tr,
        &mut subs,
        &mut fb,
    );
    assert!(!snap
        .members
        .iter()
        .any(|m| m.full_id == "../evil" || m.full_id == "a/b"));

    let _g1 = EnvGuard("CCIDE_AGY_HOME", std::env::var("CCIDE_AGY_HOME").ok());
    let _g2 = EnvGuard(
        "CCIDE_AGY_CONVERSATION",
        std::env::var("CCIDE_AGY_CONVERSATION").ok(),
    );
    std::env::set_var("CCIDE_AGY_HOME", root.join("agy"));
    std::env::remove_var("CCIDE_AGY_CONVERSATION");
    let home = root.join("home");
    let home_s = home.to_string_lossy().into_owned();

    assert_eq!(
        agy_resolve_cid(&home_s, "y1", "/workspace/demo").as_deref(),
        Some("coord-001")
    );
    write_f(
        &root.join("agy/cache/last_conversations.json"),
        r#"{"/workspace/demo":{"conversationId":"worker-argon-002"}}"#,
    );
    assert_eq!(
        agy_resolve_cid(&home_s, "y1", "/workspace/demo").as_deref(),
        Some("worker-argon-002")
    );
    std::env::set_var("CCIDE_AGY_CONVERSATION", "worker-opus-001");
    assert_eq!(
        agy_resolve_cid(&home_s, "y1", "/workspace/demo").as_deref(),
        Some("worker-opus-001")
    );
    write_f(&home.join(".cache/cc-ide/tab_conv_y1.txt"), "coord-001\n");
    assert_eq!(
        agy_resolve_cid(&home_s, "y1", "/workspace/demo").as_deref(),
        Some("coord-001")
    );
    write_f(&home.join(".cache/cc-ide/tab_conv_y1.txt"), "../evil\n");
    assert_eq!(
        agy_resolve_cid(&home_s, "y1", "/workspace/demo").as_deref(),
        Some("worker-opus-001")
    );
    let _ = fs::remove_dir_all(&root);
}
