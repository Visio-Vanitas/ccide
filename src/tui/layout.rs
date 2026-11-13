//! Declarative Terminal Cell Typesetting & Column Alignment Engine (`src/tui/layout.rs`).
//!
//! Eliminates manual string-padding / width arithmetic across panels by providing ANSI- and
//! East-Asian (CJK) display-width-aware primitives (`Align`, `ColSpec`, `ColumnWidths`, `RowLayout`)
//! that mathematically guarantee `display_width(row) <= inner_w` at every terminal width.

use crate::tui::context::bar::render_prop_bar;
use crate::tui::width::{display_width, trunc_to_width};
use crate::tui::RESET;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

pub fn align_cell(text: &str, width: usize, align: Align) -> String {
    let w = display_width(text);
    if w > width {
        let t = trunc_to_width(text, width);
        let pad = " ".repeat(width.saturating_sub(display_width(&t)));
        return match align {
            Align::Left => format!("{t}{pad}"),
            Align::Right => format!("{pad}{t}"),
        };
    }
    let pad = " ".repeat(width - w);
    match align {
        Align::Left => format!("{text}{pad}"),
        Align::Right => format!("{pad}{text}"),
    }
}

pub struct ColumnWidths<const N: usize> {
    pub widths: [usize; N],
}

impl<const N: usize> ColumnWidths<N> {
    pub fn measure<S: AsRef<str>>(rows: &[[S; N]], min_widths: [usize; N]) -> Self {
        let mut widths = min_widths;
        for r in rows {
            for i in 0..N {
                widths[i] = widths[i].max(display_width(r[i].as_ref()));
            }
        }
        Self { widths }
    }

    pub fn fmt_col(&self, idx: usize, style: &str, plain: &str, align: Align) -> String {
        let aligned = align_cell(plain, self.widths[idx], align);
        if style.is_empty() {
            aligned
        } else {
            format!("{style}{aligned}{RESET}")
        }
    }
}

#[allow(dead_code)]
pub enum ColSpec<'a> {
    Fixed {
        text: &'a str,
        width: usize,
        align: Align,
        style: &'a str,
    },
    Styled(&'a str),
    FlexTrunc {
        text: &'a str,
        style: &'a str,
    },
    FillBar {
        pct: f64,
        fill_col: &'a str,
        track_col: &'a str,
        min_w: usize,
    },
}

pub struct RowLayout {
    inner_w: usize,
    pad_left: usize,
    pad_right: usize,
}

impl RowLayout {
    pub fn new(inner_w: usize) -> Self {
        Self {
            inner_w,
            pad_left: 1,
            pad_right: 1,
        }
    }

    pub fn padding(mut self, left: usize, right: usize) -> Self {
        self.pad_left = left;
        self.pad_right = right;
        self
    }

    pub fn split(&self, left_ansi: &str, right_ansi: &str, min_gap: usize) -> String {
        self.split_with_bg(left_ansi, right_ansi, min_gap, "")
    }

    pub fn split_with_bg(
        &self,
        left_ansi: &str,
        right_ansi: &str,
        min_gap: usize,
        row_bg: &str,
    ) -> String {
        let usable = self.inner_w.saturating_sub(self.pad_left + self.pad_right);
        if usable == 0 {
            return String::new();
        }
        let (lpad, rpad) = (" ".repeat(self.pad_left), " ".repeat(self.pad_right));
        let right_w = display_width(right_ansi);

        if right_w == 0 || right_w + min_gap + 4 > usable {
            let body = if display_width(left_ansi) > usable {
                trunc_to_width(left_ansi, usable)
            } else {
                left_ansi.to_string()
            };
            return format!("{lpad}{body}");
        }

        let left_budget = usable - right_w - min_gap;
        let raw_lw = display_width(left_ansi);
        let (left_fit, fit_lw) = if raw_lw > left_budget {
            let t = trunc_to_width(left_ansi, left_budget);
            let tw = display_width(&t);
            (format!("{t}{row_bg}"), tw)
        } else {
            (left_ansi.to_string(), raw_lw)
        };
        let gap = usable.saturating_sub(fit_lw + right_w);
        format!(
            "{lpad}{left_fit}{row_bg}{}{right_ansi}{row_bg}{rpad}",
            " ".repeat(gap)
        )
    }

    pub fn columns(&self, specs: &[ColSpec<'_>], col_gap: usize) -> String {
        let usable = self.inner_w.saturating_sub(self.pad_left + self.pad_right);
        if usable == 0 || specs.is_empty() {
            return String::new();
        }
        let fixed_and_gaps = |include_bar: bool| -> usize {
            let (mut count, mut sum) = (0usize, 0usize);
            for sp in specs {
                match *sp {
                    ColSpec::Fixed { width, .. } => {
                        sum += width;
                        count += 1;
                    }
                    ColSpec::Styled(s) => {
                        sum += display_width(s);
                        count += 1;
                    }
                    ColSpec::FlexTrunc { .. } => count += 1,
                    ColSpec::FillBar { .. } if include_bar => count += 1,
                    ColSpec::FillBar { .. } => {}
                }
            }
            sum + count.saturating_sub(1) * col_gap
        };

        let bar_min = specs
            .iter()
            .find_map(|s| match *s {
                ColSpec::FillBar { min_w, .. } => Some(min_w),
                _ => None,
            })
            .unwrap_or(0);
        let include_bar = bar_min > 0 && usable >= fixed_and_gaps(true) + bar_min;
        let flex_budget = usable.saturating_sub(fixed_and_gaps(include_bar));

        let mut rendered: Vec<String> = Vec::with_capacity(specs.len());
        for sp in specs {
            match *sp {
                ColSpec::Fixed {
                    text,
                    width,
                    align,
                    style,
                } => {
                    let cell = align_cell(text, width, align);
                    rendered.push(if style.is_empty() {
                        cell
                    } else {
                        format!("{style}{cell}{RESET}")
                    });
                }
                ColSpec::Styled(s) => rendered.push(s.to_string()),
                ColSpec::FlexTrunc { text, style } => {
                    let fit = trunc_to_width(text, flex_budget.max(4));
                    rendered.push(if style.is_empty() {
                        fit
                    } else {
                        format!("{style}{fit}{RESET}")
                    });
                }
                ColSpec::FillBar {
                    pct,
                    fill_col,
                    track_col,
                    min_w,
                } if include_bar => {
                    rendered.push(render_prop_bar(
                        pct,
                        flex_budget.max(min_w),
                        fill_col,
                        track_col,
                    ));
                }
                ColSpec::FillBar { .. } => {}
            }
        }
        let full = format!(
            "{}{}{}",
            " ".repeat(self.pad_left),
            rendered.join(&" ".repeat(col_gap)),
            " ".repeat(self.pad_right)
        );
        if display_width(&full) > self.inner_w {
            trunc_to_width(&full, self.inner_w)
        } else {
            full
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_and_columns_never_overflow_across_all_widths() {
        for inner_w in 16usize..=72 {
            let row = RowLayout::new(inner_w).split(
                "\x1b[38;2;90;170;255m●\x1b[0m \x1b[1mdeepseek-v4.1-flash\x1b[0m",
                "\x1b[38;2;255;190;80m$122.48\x1b[0m \x1b[2m99.9%\x1b[0m",
                2,
            );
            assert!(display_width(&row) <= inner_w);
            if inner_w >= 26 {
                assert_eq!(display_width(&row), inner_w);
            }
            let crow = RowLayout::new(inner_w).columns(
                &[
                    ColSpec::Styled("\x1b[36m●\x1b[0m"),
                    ColSpec::Fixed {
                        text: "TOOL IO",
                        width: 7,
                        align: Align::Left,
                        style: "",
                    },
                    ColSpec::FillBar {
                        pct: 42.0,
                        fill_col: "\x1b[36m",
                        track_col: "\x1b[90m",
                        min_w: 4,
                    },
                    ColSpec::Fixed {
                        text: "183k",
                        width: 5,
                        align: Align::Right,
                        style: "",
                    },
                    ColSpec::Fixed {
                        text: "41%",
                        width: 4,
                        align: Align::Right,
                        style: "",
                    },
                ],
                1,
            );
            assert!(display_width(&crow) <= inner_w);
        }
    }
}
