//! Reusable Multi-Panel TUI Rendering Engine (`tui`).
//!
//! Guarantees cross-window border alignment (`╭───╮`, `│...│`, `╰───╯`), CJK East-Asian
//! display column width calculation, and exact vertical height allocation (`sum == rows`).

pub mod grid;
pub mod logo;
pub mod width;

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[38;2;120;126;150m";
pub const GRAY: &str = "\x1b[38;2;68;74;94m";
pub const WHITE: &str = "\x1b[38;2;232;236;248m";
pub const CYAN: &str = "\x1b[38;2;86;214;255m";
pub const GREEN: &str = "\x1b[38;2;92;228;154m";
pub const AMBER: &str = "\x1b[38;2;255;194;92m";
pub const CORAL: &str = "\x1b[38;2;238;124;100m";
pub const BLUE: &str = "\x1b[38;2;98;164;255m";
pub const MINT: &str = "\x1b[38;2;56;218;172m";
pub const LAVENDER: &str = "\x1b[38;2;192;142;255m";
pub const ACTIVE_VIEW_BG: &str = "\x1b[48;2;32;62;92m";
pub const GRID_DOT: &str = "\x1b[38;2;45;50;68m·\x1b[0m";
pub const SPARK_BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
pub const SPINNERS: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
