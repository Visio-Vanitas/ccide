//! Built-in Theme Presets & Per-Slot Override Applier for `cc-ide`.

use super::syntax::SyntaxPalette;
use super::{extract_color_field, resolve_palette_path, strip_jsonc_comments, ColorSpec, Palette};
use crate::json::{extract_json_bool, extract_json_str};
use crate::tui::RESET;

pub(crate) fn hex_nib(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub const PRESET_NAMES: [&str; 6] = [
    "default",
    "catppuccin-mocha",
    "tokyo-night",
    "nord",
    "gruvbox-dark",
    "dracula",
];

#[derive(Clone, Debug)]
pub struct StatuslinePalette {
    pub bypass_bg: ColorSpec,
    pub bypass_fg: ColorSpec,
    pub auto_bg: ColorSpec,
    pub auto_fg: ColorSpec,
    pub plan_bg: ColorSpec,
    pub plan_fg: ColorSpec,
    pub accept_bg: ColorSpec,
    pub accept_fg: ColorSpec,
    pub manual_bg: ColorSpec,
    pub manual_fg: ColorSpec,
    pub model_bg: ColorSpec,
    pub model_fg: ColorSpec,
    pub ctx_ok_bg: ColorSpec,
    pub ctx_ok_fg: ColorSpec,
    pub ctx_warn_bg: ColorSpec,
    pub ctx_warn_fg: ColorSpec,
    pub ctx_crit_bg: ColorSpec,
    pub ctx_crit_fg: ColorSpec,
    pub git_bg: ColorSpec,
    pub git_fg: ColorSpec,
    pub sys_bg: ColorSpec,
    pub sys_fg: ColorSpec,
}

pub(crate) type UiColorSpecs = [ColorSpec; 12];

fn hex_rgb_at(s: &str, idx: usize) -> ColorSpec {
    let (b, o) = (s.as_bytes(), idx * 6);
    let byte = |p: usize| (hex_nib(b[p]).unwrap_or(0) << 4) | hex_nib(b[p + 1]).unwrap_or(0);
    ColorSpec::Rgb(byte(o), byte(o + 2), byte(o + 4))
}

fn decode_rgb_arr<const N: usize>(hex: &str) -> [ColorSpec; N] {
    let mut out = [ColorSpec::Rgb(0, 0, 0); N];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = hex_rgb_at(hex, i);
    }
    out
}

fn decode_syn(hex: &str) -> SyntaxPalette {
    let c = |i: usize| hex_rgb_at(hex, i);
    SyntaxPalette {
        keyword: c(0),
        function: c(1),
        type_name: c(2),
        string: c(3),
        number: c(4),
        comment: c(5),
        operator: c(6),
        variable: c(7),
        code_bg: c(8),
        diff_add_fg: c(9),
        diff_add_bg: c(10),
        diff_del_fg: c(11),
        diff_del_bg: c(12),
    }
}

pub(crate) struct RawThemeColors {
    pub ui: [ColorSpec; 15],
    pub sl: [ColorSpec; 22],
    pub syntax: SyntaxPalette,
}

impl RawThemeColors {
    pub(crate) fn from_preset(name: &str) -> (&'static str, Self) {
        let (canon, ui_h, sl_h, syn_h) = match name.trim().to_ascii_lowercase().as_str() {
            "catppuccin-mocha" | "catppuccin" | "mocha" => (
                "catppuccin-mocha",
                "585B7045475A313244CDD6F49399B289DCEBA6E3A1F9E2AFF38BA889B4FA94E2D5CBA6F7313244453A5C45475A",
                "D20F39EFF1F58839EFEFF1F5209FB5EFF1F540A02BEFF1F5585B70CDD6F41E66F5EFF1F545475ACDD6F4DF8E1D1E1E2ED20F39EFF1F5313244BAC2DE181825A6ADC8",
                "CBA6F789B4FAF9E2AFA6E3A1FAB3876C708689DCEBF5C2E71E1E2EA6E3A11E3228F38BA83B202B",
            ),
            "tokyo-night" | "tokyonight" => (
                "tokyo-night",
                "414868343B58292E42C0CAF5737AA27DCFFF9ECE6AE0AF68F7768E7AA2F773DACABB9AF728345736305633467C",
                "DB4B4BF7FAFF9D7CD8F7FAFF1ABC9C1A1B2673DACA1A1B26414868C0CAF53D59A1F7FAFF3B4261C0CAF5E0AF681A1B26DB4B4BF7FAFF292E42A9B1D61F23358B93BA",
                "BB9AF77AA2F72AC3DE9ECE6AFF9E64565F8989DDFF7DCFFF1A1B269ECE6A20302BF7768E37222C",
            ),
            "nord" => (
                "nord",
                "4C566A434C5E3B4252ECEFF4909AAE88C0D0A3BE8CEBCB8BBF616A81A1C18FBCBBB48EAD3B4252434C5E4C566A",
                "BF616AECEFF4B48EAD2E344088C0D02E3440A3BE8C2E34404C566AECEFF45E81ACECEFF4434C5EE5E9F0D087702E3440BF616AECEFF43B4252D8DEE92E3440B4BCCA",
                "81A1C188C0D08FBCBBA3BE8CB48EAD616E8881A1C1D087702E3440A3BE8C1F3328BF616A3A1F24",
            ),
            "gruvbox-dark" | "gruvbox" => (
                "gruvbox-dark",
                "665C545049453C3836EBDBB2A8998483A598B8BB26FABD2FFB49344585888EC07CD3869B3C3836504945665C54",
                "CC241DFBF1C7B16286FBF1C7689D6A28282898971A282828504945EBDBB2458588FBF1C7504945EBDBB2D79921282828CC241DFBF1C73C3836D5C4A1282828A89984",
                "FB4934B8BB26FABD2FB8BB26D3869B9283748EC07C83A598282828B8BB262B331EFB49343C201E",
            ),
            "dracula" => (
                "dracula",
                "6272A444475A343746F8F8F28B94B88BE9FD50FA7BF1FA8CFF55556272A450FA7BBD93F944475A4E426E585C82",
                "FF5555F8F8F2BD93F9282A368BE9FD282A3650FA7B282A3644475AF8F8F26272A4F8F8F244475AF8F8F2FFB86C282A36FF5555F8F8F2343746F8F8F2282A36BDC3DA",
                "FF79C650FA7B8BE9FDF1FA8CBD93F96272A4FF79C6FFB86C282A3650FA7B1D3828FF55553E2025",
            ),
            _ => {
                let sl_256 = [
                    131, 255, 61, 255, 30, 255, 29, 255, 239, 252, 24, 255, 240, 253, 136, 255,
                    131, 255, 237, 250, 235, 247,
                ];
                let mut sl = [ColorSpec::Ansi256(0); 22];
                for i in 0..22 { sl[i] = ColorSpec::Ansi256(sl_256[i]); }
                return ("default", Self {
                    ui: decode_rgb_arr("444A5E3034462D3244E8ECF8787E9656D6FF5CE49AFFC25CEE7C6462A4FF38DAACC08EFF203E5C36304E2A5276"),
                    sl,
                    syntax: decode_syn("C08EFF62A4FFFFC25C5CE49AFFC25C787E9656D6FFFF9E641E222E5CE49A1A3327EE7C64382022"),
                });
            }
        };
        (
            canon,
            Self {
                ui: decode_rgb_arr(ui_h),
                sl: decode_rgb_arr(sl_h),
                syntax: decode_syn(syn_h),
            },
        )
    }

    pub(crate) fn apply_overrides(&mut self, clean: &str) {
        const UI_KEYS: &str = "border divider grid_dot text dim cyan green amber coral blue mint lavender active_view_bg cursor_bg active_cursor_bg";
        const SL_KEYS: &str = "sl_bypass_bg sl_bypass_fg sl_auto_bg sl_auto_fg sl_plan_bg sl_plan_fg sl_accept_bg sl_accept_fg sl_manual_bg sl_manual_fg sl_model_bg sl_model_fg sl_ctx_ok_bg sl_ctx_ok_fg sl_ctx_warn_bg sl_ctx_warn_fg sl_ctx_crit_bg sl_ctx_crit_fg sl_git_bg sl_git_fg sl_sys_bg sl_sys_fg";
        for (i, k) in UI_KEYS.split_ascii_whitespace().enumerate() {
            if let Some(v) = extract_color_field(clean, k) {
                self.ui[i] = v;
            }
        }
        for (i, k) in SL_KEYS.split_ascii_whitespace().enumerate() {
            if let Some(v) = extract_color_field(clean, k) {
                self.sl[i] = v;
            }
        }
        self.syntax.apply_overrides(clean);
    }

    pub(crate) fn build_palette(self, preset: &str, tint_logo: bool, clean: &str) -> Palette {
        let [mc, mg, md, mo, mk] = [
            extract_color_field(clean, "model_claude").unwrap_or(self.ui[8]),
            extract_color_field(clean, "model_gemini").unwrap_or(self.ui[9]),
            extract_color_field(clean, "model_deepseek").unwrap_or(self.ui[5]),
            extract_color_field(clean, "model_openai").unwrap_or(self.ui[10]),
            extract_color_field(clean, "model_grok").unwrap_or(self.ui[11]),
        ];
        let (u, s) = (self.ui, self.sl);
        Palette {
            preset: preset.to_string(),
            tint_logo,
            border: u[0].fg_ansi(),
            divider: u[1].fg_ansi(),
            grid_dot: format!("{}·{}", u[2].fg_ansi(), RESET),
            text: u[3].fg_ansi(),
            dim: u[4].fg_ansi(),
            cyan: u[5].fg_ansi(),
            green: u[6].fg_ansi(),
            amber: u[7].fg_ansi(),
            coral: u[8].fg_ansi(),
            blue: u[9].fg_ansi(),
            mint: u[10].fg_ansi(),
            lavender: u[11].fg_ansi(),
            active_view_bg: u[12].bg_ansi(),
            cursor_bg: u[13].bg_ansi(),
            active_cursor_bg: u[14].bg_ansi(),
            model_claude: mc.fg_ansi(),
            model_claude_rgb: mc.to_rgb_approx(),
            model_gemini: mg.fg_ansi(),
            model_gemini_rgb: mg.to_rgb_approx(),
            model_deepseek: md.fg_ansi(),
            model_deepseek_rgb: md.to_rgb_approx(),
            model_openai: mo.fg_ansi(),
            model_openai_rgb: mo.to_rgb_approx(),
            model_grok: mk.fg_ansi(),
            model_grok_rgb: mk.to_rgb_approx(),
            statusline: StatuslinePalette {
                bypass_bg: s[0],
                bypass_fg: s[1],
                auto_bg: s[2],
                auto_fg: s[3],
                plan_bg: s[4],
                plan_fg: s[5],
                accept_bg: s[6],
                accept_fg: s[7],
                manual_bg: s[8],
                manual_fg: s[9],
                model_bg: s[10],
                model_fg: s[11],
                ctx_ok_bg: s[12],
                ctx_ok_fg: s[13],
                ctx_warn_bg: s[14],
                ctx_warn_fg: s[15],
                ctx_crit_bg: s[16],
                ctx_crit_fg: s[17],
                git_bg: s[18],
                git_fg: s[19],
                sys_bg: s[20],
                sys_fg: s[21],
            },
            syntax: self.syntax,
            ui_specs: [
                u[3], u[4], u[0], u[5], u[9], u[11], u[7], u[6], u[8], u[10], u[12], u[13],
            ],
        }
    }
}

impl Palette {
    pub fn default_palette() -> Self {
        Self::from_json_str("")
    }

    pub fn from_json_str(raw_src: &str) -> Self {
        let clean = strip_jsonc_comments(raw_src);
        let req_preset = extract_json_str(&clean, "preset").unwrap_or_default();
        let (canonical_preset, mut c) = RawThemeColors::from_preset(&req_preset);
        let tint_logo = extract_json_bool(&clean, "tint_logo").unwrap_or(false);
        c.apply_overrides(&clean);
        c.build_palette(canonical_preset, tint_logo, &clean)
    }

    pub fn load(home: &str) -> Self {
        std::fs::read_to_string(resolve_palette_path(home))
            .map(|t| Self::from_json_str(&t))
            .unwrap_or_else(|_| Self::default_palette())
    }

    pub fn family_accent(&self, family: &str) -> (&str, (u8, u8, u8)) {
        match family {
            "gemini" => (&self.model_gemini, self.model_gemini_rgb),
            "deepseek" => (&self.model_deepseek, self.model_deepseek_rgb),
            "openai" => (&self.model_openai, self.model_openai_rgb),
            "grok" => (&self.model_grok, self.model_grok_rgb),
            _ => (&self.model_claude, self.model_claude_rgb),
        }
    }
}
