//! Reusable Multi-Panel TUI Rendering Engine (`tui`).
//!
//! Guarantees cross-window border alignment (`╭───╮`, `│...│`, `╰───╯`), CJK East-Asian
//! display column width calculation, and exact vertical height allocation (`sum == rows`).

pub mod grid;
pub mod logo;
pub mod palette;
pub mod tasks;
pub mod width;

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const SPARK_BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
pub const SPINNERS: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
