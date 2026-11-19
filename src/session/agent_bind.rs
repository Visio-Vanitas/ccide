//! Non-Claude (`Agy` / `Codex`) Hook-First + Disk-Fallback Resolution (`src/session/agent_bind.rs`).

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::ffi::{is_pid_alive, ProcToken};
use crate::json::{extract_json_str, extract_json_u64};
use crate::platform::cache_dir;
use crate::session::agent::{is_safe_id, AgentKind};
use crate::session::agy::{agy_settings_model, agy_transcript_path};
use crate::session::agy_swarm::{agy_parent_coordinator, agy_resolve_cid_detail};
use crate::session::binding::{commit_binding, SessionBinding};
use crate::session::codex::{
    codex_config, find_rollout_by_id, find_rollout_for_cwd, rollout_session_id,
};
use crate::session::is_claimed_by_live_sibling;
use crate::session::model::{
    classify_model_family_for, is_valid_model_name, load_config_models, short_model_label,
    ConfigModels,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentSession {
    pub kind: AgentKind,
    pub session_id: String,
    pub agent_pid: u32,
    pub transcript_path: PathBuf,
    pub model_id: String,
    pub cwd: String,
}

pub fn agent_session_to_binding(s: &AgentSession) -> SessionBinding {
    SessionBinding {
        session_id: s.session_id.clone(),
        ui_pid: s.agent_pid,
        data_pid: s.agent_pid,
        agent: s.kind,
        transcript_path: s.transcript_path.to_string_lossy().into_owned(),
    }
}

fn live_agent_pid(txt: &str) -> Option<u32> {
    let tok = extract_json_str(txt, "agent_pid")
        .and_then(|s| ProcToken::parse(&s))
        .or_else(|| {
            extract_json_u64(txt, "agent_pid").and_then(|n| ProcToken::parse(&n.to_string()))
        })?;
    let alive = if tok.start_ms > 0 {
        tok.is_alive_match()
    } else {
        is_pid_alive(tok.pid)
    };
    alive.then_some(tok.pid)
}

fn read_live_sess_model(cdir: &Path, sid: &str) -> String {
    if !is_safe_id(sid) {
        return String::new();
    }
    fs::read_to_string(cdir.join(format!("live_sess_{}.json", sid)))
        .ok()
        .and_then(|t| extract_json_str(&t, "model_id").or_else(|| extract_json_str(&t, "model")))
        .unwrap_or_default()
}

pub fn resolve_agent_session(
    home: &str,
    yid: &str,
    cwd: &str,
    _shell_pid: u32,
    kind: AgentKind,
) -> Option<AgentSession> {
    if kind == AgentKind::Claude {
        return None;
    }
    let cdir = cache_dir(home);
    let agy_explicit = if kind == AgentKind::Agy {
        agy_resolve_cid_detail(home, yid, cwd)
            .filter(|(_, exp)| *exp)
            .map(|(c, _)| c)
    } else {
        None
    };

    if is_safe_id(yid) {
        if let Ok(txt) = fs::read_to_string(cdir.join(format!("live_tab_{}.json", yid))) {
            let ok_kind = extract_json_str(&txt, "agent").and_then(|a| AgentKind::from_name(&a))
                == Some(kind);
            let tpath_s = extract_json_str(&txt, "transcript_path").unwrap_or_default();
            let mut tpath = PathBuf::from(&tpath_s);
            let dotdot = tpath
                .components()
                .any(|c| c == std::path::Component::ParentDir);
            if ok_kind && !tpath_s.is_empty() && !dotdot && tpath.is_file() {
                if let Some(pid) = live_agent_pid(&txt) {
                    let raw_sid = extract_json_str(&txt, "session_id");
                    let bad_raw = raw_sid.as_ref().is_some_and(|s| !is_safe_id(s));
                    let mut sid = raw_sid.unwrap_or_else(|| {
                        if kind == AgentKind::Codex {
                            rollout_session_id(&tpath)
                        } else {
                            String::new()
                        }
                    });
                    let skip_exp = kind == AgentKind::Agy
                        && agy_explicit.as_ref().is_some_and(|exp| exp != &sid);
                    if !skip_exp && !bad_raw && (sid.is_empty() || is_safe_id(&sid)) {
                        if kind == AgentKind::Agy && !sid.is_empty() {
                            if let Some((c_cid, c_tp)) = agy_parent_coordinator(&sid, &tpath) {
                                (sid, tpath) = (c_cid, c_tp);
                            }
                        }
                        if sid.is_empty() || !is_claimed_by_live_sibling(&cdir, &sid, yid) {
                            commit_binding(&cdir, &sid, yid);
                            let model_id = extract_json_str(&txt, "model_id")
                                .or_else(|| extract_json_str(&txt, "model"))
                                .unwrap_or_default();
                            let hcwd =
                                extract_json_str(&txt, "cwd").unwrap_or_else(|| cwd.to_string());
                            return Some(AgentSession {
                                kind,
                                session_id: sid,
                                agent_pid: pid,
                                transcript_path: tpath,
                                model_id,
                                cwd: hcwd,
                            });
                        }
                    }
                }
            }
        }
    }

    match kind {
        AgentKind::Agy => {
            let (mut cid, is_explicit) = agy_resolve_cid_detail(home, yid, cwd)?;
            if !is_safe_id(&cid) {
                return None;
            }
            let mut tpath = agy_transcript_path(&cid);
            if !is_explicit {
                if let Some((c_cid, c_tp)) = agy_parent_coordinator(&cid, &tpath) {
                    (cid, tpath) = (c_cid, c_tp);
                }
            }
            if is_claimed_by_live_sibling(&cdir, &cid, yid) {
                return None;
            }
            let mt = fs::metadata(&tpath).and_then(|m| m.modified()).ok()?;
            if !is_explicit
                && SystemTime::now().duration_since(mt).unwrap_or_default()
                    > Duration::from_secs(86_400)
            {
                return None;
            }
            commit_binding(&cdir, &cid, yid);
            let mut model_id = read_live_sess_model(&cdir, &cid);
            if model_id.is_empty() {
                model_id = agy_settings_model().unwrap_or_else(|| "auto".to_string());
            }
            Some(AgentSession {
                kind,
                session_id: cid,
                agent_pid: 0,
                transcript_path: tpath,
                model_id,
                cwd: cwd.to_string(),
            })
        }
        AgentKind::Codex => {
            let tpath = find_rollout_for_cwd(cwd)?;
            let sid = rollout_session_id(&tpath);
            if !is_safe_id(&sid) || is_claimed_by_live_sibling(&cdir, &sid, yid) {
                return None;
            }
            commit_binding(&cdir, &sid, yid);
            let mut model_id = read_live_sess_model(&cdir, &sid);
            if model_id.is_empty() {
                model_id = codex_config().model.unwrap_or_default();
            }
            Some(AgentSession {
                kind,
                session_id: sid,
                agent_pid: 0,
                transcript_path: tpath,
                model_id,
                cwd: cwd.to_string(),
            })
        }
        AgentKind::Claude => None,
    }
}

fn last_codex_rollout_model(session_id: &str) -> Option<String> {
    let mut f = fs::File::open(find_rollout_by_id(session_id)?).ok()?;
    let len = f.metadata().ok()?.len();
    let _ = f.seek(SeekFrom::Start(len.saturating_sub(65_536)));
    let mut raw = Vec::new();
    f.read_to_end(&mut raw).ok()?;
    String::from_utf8_lossy(&raw)
        .lines()
        .filter(|ln| ln.contains("\"turn_context\""))
        .filter_map(|ln| extract_json_str(ln, "model").filter(|m| is_valid_model_name(m)))
        .next_back()
}

pub fn load_config_models_for(
    agent: AgentKind,
    home: &str,
    session_id: &str,
    tab_yazi_id: &str,
    live_model: Option<&str>,
) -> ConfigModels {
    if agent == AgentKind::Claude {
        return load_config_models(home, session_id, tab_yazi_id, None, None);
    }
    let live = live_model
        .map(str::trim)
        .filter(|m| is_valid_model_name(m))
        .map(str::to_string);
    let raw_main = match agent {
        AgentKind::Agy => live
            .filter(|m| !m.eq_ignore_ascii_case("auto"))
            .or_else(agy_settings_model)
            .unwrap_or_else(|| "auto".to_string()),
        AgentKind::Codex => live
            .or_else(|| last_codex_rollout_model(session_id))
            .or_else(|| codex_config().model)
            .unwrap_or_default(),
        AgentKind::Claude => unreachable!(),
    };
    let main_family = classify_model_family_for(agent, &raw_main);
    let main_model_short = short_model_label(&raw_main);
    ConfigModels {
        main_family,
        main_model_raw: raw_main,
        main_model_short: main_model_short.clone(),
        sub_family: main_family,
        sub_model_short: main_model_short,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::ipc_path;

    #[test]
    fn resolves_live_hook_record_and_respects_sibling_guard() {
        let base = std::env::temp_dir().join(format!("ccide_abind_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let cdir = base.join(".cache/cc-ide");
        fs::create_dir_all(&cdir).unwrap();
        let tr = base.join("rollout-s1.jsonl");
        fs::write(&tr, "{}\n").unwrap();
        let self_tok = ProcToken::self_token().encode();
        let tr_esc = tr.to_string_lossy().replace('\\', "\\\\");
        let body = format!(
            "{{\"agent\":\"codex\",\"session_id\":\"s1\",\"agent_pid\":\"{self_tok}\",\
            \"transcript_path\":\"{tr_esc}\",\"model_id\":\"gpt-5.4\",\"cwd\":\"/repo\"}}"
        );
        fs::write(cdir.join("live_tab_y1.json"), body).unwrap();
        let home = base.to_string_lossy().into_owned();
        let sess = resolve_agent_session(&home, "y1", "/repo", 0, AgentKind::Codex).unwrap();
        assert_eq!(sess.session_id, "s1");
        assert_eq!(sess.agent_pid, std::process::id());
        let sb = agent_session_to_binding(&sess);
        assert_eq!(
            (sb.agent, sb.ui_pid, sb.data_pid),
            (AgentKind::Codex, sess.agent_pid, sess.agent_pid)
        );
        let sib_yid = format!("abind_sib_{}", std::process::id());
        let _ = fs::create_dir_all(ipc_path(""));
        let sib_tab = ipc_path(&format!("sidebar_tab_{}.json", sib_yid));
        fs::write(&sib_tab, format!(r#"{{"pid":"{}"}}"#, self_tok)).unwrap();
        fs::write(cdir.join("session_s1.yazi"), &sib_yid).unwrap();
        assert!(resolve_agent_session(&home, "y1", "/repo", 0, AgentKind::Codex).is_none());
        let _ = fs::remove_file(&sib_tab);
        let _ = fs::remove_dir_all(&base);
    }
}
