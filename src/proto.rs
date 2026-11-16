//! Protocol Version Constants & `ccide-core proto [--json]` Subcommand (`src/proto.rs`).

use crate::json::json_escape;
use crate::platform::try_ipc_dir;

pub const IPC_PROTO: u32 = 1;
pub const PATCH_PROTO: u32 = 1;
pub const PATCH_RULES_VERSION: &str = "2";
pub const SPEC_VERSION: u8 = 2;

pub fn run(args: &[String]) {
    let ver = env!("CARGO_PKG_VERSION");
    if args.iter().any(|a| a == "--json") {
        let ipc_field = match try_ipc_dir() {
            Some(d) => format!("\"{}\"", json_escape(&d.to_string_lossy())),
            None => "null".to_string(),
        };
        println!(
            "{{\"ccide_version\":\"{}\",\"ipc_proto\":{},\"patch_proto\":{},\"patch_rules_version\":\"{}\",\"spec_version\":{},\"ipc_dir\":{}}}",
            json_escape(ver),
            IPC_PROTO,
            PATCH_PROTO,
            json_escape(PATCH_RULES_VERSION),
            SPEC_VERSION,
            ipc_field
        );
    } else {
        println!(
            "ccide-core {ver} ipc_proto={IPC_PROTO} patch_proto={PATCH_PROTO} patch_rules={PATCH_RULES_VERSION} spec={SPEC_VERSION}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_frozen_v010_protocol_constants() {
        assert_eq!(IPC_PROTO, 1);
        assert_eq!(PATCH_PROTO, 1);
        assert_eq!(PATCH_RULES_VERSION, "2");
        assert_eq!(SPEC_VERSION, 2);
    }
}
