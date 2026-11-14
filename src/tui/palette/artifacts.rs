//! Companion Theme File Renderers (`ccide.tmTheme`, `glow-style.json`, `ghostty-theme.conf`).

use super::Palette;

pub(crate) fn build_tm_theme(pal: &Palette) -> String {
    let (u, s) = (&pal.ui_specs, &pal.syntax);
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\"><dict>\n  <key>name</key><string>ccide-{}</string>\n  <key>settings</key><array>\n\
         <dict><key>settings</key><dict><key>background</key><string>{}</string><key>foreground</key><string>{}</string>\
         <key>caret</key><string>{}</string><key>selection</key><string>{}</string><key>lineHighlight</key><string>{}</string></dict></dict>\n",
        pal.preset,
        s.code_bg.to_hex_string(),
        u[0].to_hex_string(),
        u[3].to_hex_string(),
        u[11].to_hex_string(),
        s.code_bg.to_hex_string()
    );
    for (scope, fg, style) in [
        ("comment, punctuation.definition.comment", s.comment, Some("italic")),
        ("string, string.quoted, string.template", s.string, None),
        ("constant.character.escape, string.regexp", s.operator, None),
        ("constant.numeric, constant.language, constant.character, keyword.other.unit", s.number, None),
        ("keyword, keyword.control, storage, storage.type, storage.modifier", s.keyword, None),
        ("keyword.operator, punctuation.accessor, punctuation.separator", s.operator, None),
        ("entity.name.function, support.function, meta.function-call, variable.function, entity.name.function.macro", s.function, None),
        ("entity.name.type, entity.name.class, entity.name.struct, entity.name.enum, entity.name.trait, support.type, support.class", s.type_name, None),
        ("entity.name.tag, support.type.property-name, meta.mapping.key string, meta.structure.dictionary.key, entity.other.attribute-name", s.variable, None),
        ("variable.parameter, variable.other.member, variable.other.property", u[9], None),
        ("variable.language", u[8], Some("italic")),
        ("entity.name.namespace, entity.name.module", u[5], None),
        ("markup.heading", u[4], Some("bold")),
        ("markup.inserted", s.diff_add_fg, None),
        ("markup.deleted", s.diff_del_fg, None),
    ] {
        let st_xml = style
            .map(|st| format!("<key>fontStyle</key><string>{st}</string>"))
            .unwrap_or_default();
        out.push_str(&format!(
            "    <dict><key>scope</key><string>{scope}</string><key>settings</key><dict><key>foreground</key><string>{}</string>{st_xml}</dict></dict>\n",
            fg.to_hex_string()
        ));
    }
    out.push_str("  </array>\n</dict></plist>\n");
    out
}

pub(crate) fn build_glow_style_json(pal: &Palette) -> String {
    let (u, s) = (&pal.ui_specs, &pal.syntax);
    let [txt, dim, brd, cyn, blu, lav, grn, mnt, bg] = [
        u[0].to_hex_string(),
        u[1].to_hex_string(),
        u[2].to_hex_string(),
        u[3].to_hex_string(),
        u[4].to_hex_string(),
        u[5].to_hex_string(),
        u[7].to_hex_string(),
        u[9].to_hex_string(),
        s.code_bg.to_hex_string(),
    ];
    let [kw, fnc, typ, stg, num, cmt, opr, var, add, del] = [
        s.keyword.to_hex_string(),
        s.function.to_hex_string(),
        s.type_name.to_hex_string(),
        s.string.to_hex_string(),
        s.number.to_hex_string(),
        s.comment.to_hex_string(),
        s.operator.to_hex_string(),
        s.variable.to_hex_string(),
        s.diff_add_fg.to_hex_string(),
        s.diff_del_fg.to_hex_string(),
    ];
    format!(
        "{{\n  \"document\": {{\"color\": \"{txt}\", \"margin\": 0}},\n  \
         \"block_quote\": {{\"indent\": 1, \"indent_token\": \"▎ \", \"color\": \"{lav}\", \"italic\": true}},\n  \
         \"h1\": {{\"prefix\": \"▌ \", \"suffix\": \" \", \"color\": \"{txt}\", \"background_color\": \"{blu}\", \"bold\": true}},\n  \
         \"h2\": {{\"prefix\": \"▍ \", \"color\": \"{blu}\", \"bold\": true}}, \"h3\": {{\"prefix\": \"▸ \", \"color\": \"{cyn}\", \"bold\": true}},\n  \
         \"h4\": {{\"prefix\": \"• \", \"color\": \"{lav}\", \"bold\": true}}, \"h5\": {{\"prefix\": \"› \", \"color\": \"{grn}\", \"bold\": true}},\n  \
         \"h6\": {{\"prefix\": \"· \", \"color\": \"{dim}\", \"bold\": true}}, \"strong\": {{\"bold\": true, \"color\": \"{txt}\"}},\n  \
         \"hr\": {{\"color\": \"{brd}\", \"format\": \"────────────────────────\"}},\n  \
         \"item\": {{\"block_prefix\": \"• \"}}, \"enumeration\": {{\"block_prefix\": \". \"}}, \"task\": {{\"ticked\": \"[✓] \", \"unticked\": \"[ ] \"}},\n  \
         \"link\": {{\"color\": \"{blu}\", \"underline\": true}}, \"link_text\": {{\"color\": \"{cyn}\", \"bold\": true}},\n  \
         \"code\": {{\"color\": \"{mnt}\", \"background_color\": \"{bg}\"}},\n  \
         \"code_block\": {{\"color\": \"{txt}\", \"margin\": 0, \"chroma\": {{\n    \
           \"text\": {{\"color\": \"{txt}\"}}, \"comment\": {{\"color\": \"{cmt}\"}}, \"comment_preproc\": {{\"color\": \"{opr}\"}},\n    \
           \"keyword\": {{\"color\": \"{kw}\"}}, \"keyword_reserved\": {{\"color\": \"{kw}\"}}, \"keyword_namespace\": {{\"color\": \"{kw}\"}},\n    \
           \"keyword_declaration\": {{\"color\": \"{kw}\"}}, \"keyword_constant\": {{\"color\": \"{num}\"}}, \"keyword_pseudo\": {{\"color\": \"{kw}\"}},\n    \
           \"keyword_type\": {{\"color\": \"{typ}\"}}, \"operator\": {{\"color\": \"{opr}\"}}, \"punctuation\": {{\"color\": \"{txt}\"}},\n    \
           \"name\": {{\"color\": \"{var}\"}}, \"name_builtin\": {{\"color\": \"{opr}\"}}, \"name_function\": {{\"color\": \"{fnc}\"}},\n    \
           \"name_class\": {{\"color\": \"{typ}\", \"bold\": true}}, \"name_decorator\": {{\"color\": \"{typ}\"}},\n    \
           \"literal_number\": {{\"color\": \"{num}\"}}, \"literal_string\": {{\"color\": \"{stg}\"}},\n    \
           \"literal_string_escape\": {{\"color\": \"{opr}\"}}, \"generic_deleted\": {{\"color\": \"{del}\"}},\n    \
           \"generic_inserted\": {{\"color\": \"{add}\"}}, \"background\": {{\"background_color\": \"{bg}\"}}\n  }}}},\n  \
         \"table\": {{\"center_separator\": \"┼\", \"column_separator\": \"│\", \"row_separator\": \"─\"}}\n}}\n"
    )
}

pub(crate) fn build_ghostty_theme(pal: &Palette) -> String {
    let (u, s) = (&pal.ui_specs, &pal.syntax);
    let cols = [
        s.code_bg,
        u[8],
        u[7],
        u[6],
        u[4],
        u[5],
        u[3],
        u[0],
        u[1],
        s.diff_del_fg,
        s.diff_add_fg,
        u[6],
        s.function,
        s.keyword,
        u[9],
        u[0],
    ];
    let mut out = format!(
        "# Auto-generated by cc-ide (preset: {})\nforeground = {}\nbackground = {}\nselection-background = {}\nselection-foreground = {}\ncursor-color = {}\n",
        pal.preset,
        u[0].to_hex_string(),
        s.code_bg.to_hex_string(),
        u[10].to_hex_string(),
        u[0].to_hex_string(),
        u[3].to_hex_string()
    );
    for (i, c) in cols.iter().enumerate() {
        out.push_str(&format!("palette = {i}={}\n", c.to_hex_string()));
    }
    out
}
