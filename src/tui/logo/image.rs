//! Terminal Inline Image Protocol Detection, Header Dimension Parser & Escape Encoders
//! (`OSC 1337` for iTerm2/WezTerm/Warp + `APC _G` Kitty Graphics Protocol for Kitty/Ghostty).

use std::env;

use crate::tui::RESET;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageProto {
    None,
    Osc1337,
    Kitty,
}

#[derive(Clone)]
pub struct ImageSprite {
    pub ext: String,
    pub width_px: u32,
    pub height_px: u32,
    pub b64: String,
}

pub fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(((data.len() + 2) / 3) * 4);
    let mut i = 0usize;
    while i + 3 <= data.len() {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8) | (data[i + 2] as u32);
        out.push(TABLE[((n >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3F) as usize] as char);
        out.push(TABLE[((n >> 6) & 0x3F) as usize] as char);
        out.push(TABLE[(n & 0x3F) as usize] as char);
        i += 3;
    }
    let rem = data.len() - i;
    if rem == 1 {
        let n = (data[i] as u32) << 16;
        out.push(TABLE[((n >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3F) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rem == 2 {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8);
        out.push(TABLE[((n >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3F) as usize] as char);
        out.push(TABLE[((n >> 6) & 0x3F) as usize] as char);
        out.push('=');
    }
    out
}

pub fn parse_image_dimensions(bytes: &[u8]) -> (u32, u32) {
    if bytes.len() >= 24 && &bytes[0..8] == b"\x89PNG\r\n\x1a\n" {
        let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        if w > 0 && h > 0 {
            return (w, h);
        }
    }
    if bytes.len() >= 10 && (&bytes[0..6] == b"GIF87a" || &bytes[0..6] == b"GIF89a") {
        let w = u16::from_le_bytes([bytes[6], bytes[7]]) as u32;
        let h = u16::from_le_bytes([bytes[8], bytes[9]]) as u32;
        if w > 0 && h > 0 {
            return (w, h);
        }
    }
    if bytes.len() >= 4 && bytes[0] == 0xFF && bytes[1] == 0xD8 {
        let mut idx = 2usize;
        while idx + 9 < bytes.len() {
            if bytes[idx] != 0xFF {
                idx += 1;
                continue;
            }
            while idx < bytes.len() && bytes[idx] == 0xFF {
                idx += 1;
            }
            if idx + 7 >= bytes.len() {
                break;
            }
            let marker = bytes[idx];
            idx += 1;
            if matches!(
                marker,
                0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF
            ) {
                let h = u16::from_be_bytes([bytes[idx + 3], bytes[idx + 4]]) as u32;
                let w = u16::from_be_bytes([bytes[idx + 5], bytes[idx + 6]]) as u32;
                if w > 0 && h > 0 {
                    return (w, h);
                }
                break;
            }
            if marker == 0xD9 || marker == 0xDA {
                break;
            }
            let seg_len = u16::from_be_bytes([bytes[idx], bytes[idx + 1]]) as usize;
            if seg_len < 2 {
                break;
            }
            idx += seg_len;
        }
    }
    (256, 256)
}

pub fn detect_terminal_image_proto() -> ImageProto {
    if env::var("TMUX").is_ok() {
        return ImageProto::None;
    }
    let term_prog = env::var("TERM_PROGRAM").unwrap_or_default();
    let lc_term = env::var("LC_TERMINAL").unwrap_or_default();
    let term = env::var("TERM").unwrap_or_default();

    if term_prog == "iTerm.app"
        || lc_term == "iTerm2"
        || env::var("ITERM_SESSION_ID").is_ok()
        || term_prog == "WezTerm"
        || term_prog == "WarpTerminal"
    {
        return ImageProto::Osc1337;
    }
    if env::var("KITTY_WINDOW_ID").is_ok()
        || term == "xterm-kitty"
        || term_prog.eq_ignore_ascii_case("ghostty")
        || env::var("GHOSTTY_RESOURCES_DIR").is_ok()
    {
        return ImageProto::Kitty;
    }
    ImageProto::None
}

pub fn format_kitty_png_escape(b64: &str, cols: usize, rows: usize) -> String {
    let mut out = String::with_capacity(b64.len() + 128);
    let total = b64.len();
    let mut offset = 0usize;
    let mut first = true;
    while offset < total {
        let end = (offset + 4096).min(total);
        let more = if end < total { 1 } else { 0 };
        let chunk = &b64[offset..end];
        if first {
            out.push_str(&format!(
                "\x1b_Ga=T,f=100,q=2,i=71,C=1,c={cols},r={rows},m={more};{chunk}\x1b\\"
            ));
            first = false;
        } else {
            out.push_str(&format!("\x1b_Gm={more};{chunk}\x1b\\"));
        }
        offset = end;
    }
    out
}

pub fn build_image_blit_sequence(
    proto: ImageProto,
    img: &ImageSprite,
    inner_cols: usize,
    interior_h: usize,
    img_col_1b: usize,
    render_cols: usize,
    render_rows: usize,
) -> String {
    let mut blit = String::with_capacity(img.b64.len() + 512);
    if proto == ImageProto::Kitty {
        blit.push_str("\x1b_Ga=d,d=i,i=71,q=2\x1b\\");
    }
    let blank = " ".repeat(inner_cols);
    blit.push_str(RESET);
    for ry in 2..(2 + interior_h) {
        blit.push_str(&format!("\x1b[{ry};2H{blank}"));
    }
    blit.push_str(&format!("\x1b[3;{img_col_1b}H"));
    match proto {
        ImageProto::Osc1337 => {
            blit.push_str(&format!(
                "\x1b]1337;File=inline=1;width={render_cols};height={render_rows};preserveAspectRatio=1;doNotMoveCursor=1:{}\x07",
                img.b64
            ));
        }
        ImageProto::Kitty => {
            blit.push_str(&format_kitty_png_escape(&img.b64, render_cols, render_rows));
        }
        ImageProto::None => {}
    }
    blit
}
