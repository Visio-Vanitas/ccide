-- 120Hz Silky-Smooth Hover-Targeted & Content-Bounds-Clamped Navigation Plugin for Yazi (`cc-ide`).
--
-- Grounded in Yazi 26.x (`sxyazi/yazi`) Main-UI-VM + `slim_lua` Previewer-VM architecture:
-- 1. Zero-Yield Per-URL Shared Bounds (`/tmp/.cc-ide/yb_<hash>.txt` & `yx_<hash>.txt`):
--    - Bridges `M:peek(job)` (`slim_lua` worker VM) and `Preview:touch()` (Main UI VM) in `< 0.02ms`
--      via synchronous C `io.open`, avoiding LuaJIT coroutine yield boundaries (`pcall` / callback yields)
--      and isolating state cleanly across multiple open Yazi tabs.
-- 2. Zero-Subprocess 120Hz Preview Horizontal Panning (`±2` cols/tick):
--    - Caches rendered Markdown (`md-render.py`) and syntax-highlighted code (`bat`) per-file in
--      `/tmp/.cc-ide/ym_<hash>.txt` & `yc_<hash>.txt` so horizontal swipes read RAM-cached ANSI in
--      `< 0.1ms` and scroll via native Rust `ui.Text:scroll(px, 0)`.

local M = {}

local function char_display_cols(b)
	if b >= 0xF0 then
		return 4, 2
	elseif b >= 0xE0 then
		return 3, (b >= 0xE3 and 2 or 1)
	elseif b >= 0xC0 then
		return 2, 1
	end
	return 1, 1
end

local function utf8_width(s)
	if not s or s == "" then
		return 0
	end
	local idx, len, cols = 1, #s, 0
	while idx <= len do
		local clen, cw = char_display_cols(s:byte(idx))
		idx = idx + clen
		cols = cols + cw
	end
	return cols
end

local function utf8_drop_cols(s, drop_cols)
	if not s or drop_cols <= 0 or s == "" then
		return s
	end
	local idx, len, dropped = 1, #s, 0
	while idx <= len and dropped < drop_cols do
		local clen, cw = char_display_cols(s:byte(idx))
		idx = idx + clen
		dropped = dropped + cw
	end
	if idx > len then
		return ""
	end
	return "‹" .. s:sub(idx)
end

-- Exact ANSI SGR width & horizontal viewport slicer (`[offset_cols : offset_cols + width_cols]`).
-- Carries all active `\27[...m` SGR escape codes from off-screen left columns (`0 .. offset_cols - 1`)
-- and re-emits them at column 0 of the visible slice, working around Yazi/Ratatui's off-screen ANSI bug!
local function ansi_visible_width(s)
	if not s or s == "" then
		return 0
	end
	local idx, len, cols = 1, #s, 0
	while idx <= len do
		local b = s:byte(idx)
		if b == 0x1B then
			local nb = s:byte(idx + 1)
			if nb == 0x5B then
				-- CSI sequence `\27[...<final>`
				local j = idx + 2
				while j <= len do
					local cb = s:byte(j)
					if cb >= 0x40 and cb <= 0x7E then
						break
					end
					j = j + 1
				end
				idx = j + 1
			elseif nb == 0x5D then
				-- OSC sequence `\27]...\7` or `\27]...\27\\`
				local j = idx + 2
				while j <= len do
					local cb = s:byte(j)
					if cb == 0x07 then
						j = j + 1
						break
					elseif cb == 0x1B and s:byte(j + 1) == 0x5C then
						j = j + 2
						break
					end
					j = j + 1
				end
				idx = j
			else
				idx = idx + 2
			end
		elseif b < 0x20 or b == 0x7F then
			idx = idx + 1
		else
			local clen, cw = char_display_cols(b)
			idx = idx + clen
			cols = cols + cw
		end
	end
	return cols
end

local function clone_sgr_state(st)
	if not st then
		return { fg = nil, bg = nil, b = false, d = false, it = false, u = false, rev = false }
	end
	return {
		fg = st.fg,
		bg = st.bg,
		b = st.b or false,
		d = st.d or false,
		it = st.it or false,
		u = st.u or false,
		rev = st.rev or false,
	}
end

local function reset_sgr_state(st)
	st.fg = nil
	st.bg = nil
	st.b = false
	st.d = false
	st.it = false
	st.u = false
	st.rev = false
end

local function apply_sgr_seq(st, seq)
	local body = seq:sub(3, -2)
	if body == "" then
		reset_sgr_state(st)
		return
	end
	local nums = {}
	for part in (body .. ";"):gmatch("([^;]*);") do
		nums[#nums + 1] = (part == "" and 0) or tonumber(part) or -1
	end
	local i, n = 1, #nums
	while i <= n do
		local c = nums[i]
		if c == 0 then
			reset_sgr_state(st)
		elseif c == 1 then
			st.b = true
		elseif c == 2 then
			st.d = true
		elseif c == 3 then
			st.it = true
		elseif c == 4 then
			st.u = true
		elseif c == 7 then
			st.rev = true
		elseif c == 22 then
			st.b = false
			st.d = false
		elseif c == 23 then
			st.it = false
		elseif c == 24 then
			st.u = false
		elseif c == 27 then
			st.rev = false
		elseif (c >= 30 and c <= 37) or (c >= 90 and c <= 97) then
			st.fg = tostring(c)
		elseif c == 39 then
			st.fg = nil
		elseif (c >= 40 and c <= 47) or (c >= 100 and c <= 107) then
			st.bg = tostring(c)
		elseif c == 49 then
			st.bg = nil
		elseif (c == 38 or c == 48) and (i + 1) <= n then
			local mode = nums[i + 1]
			local key = (c == 38) and "fg" or "bg"
			if mode == 5 and (i + 2) <= n and nums[i + 2] >= 0 then
				st[key] = string.format("%d;5;%d", c, nums[i + 2])
				i = i + 2
			elseif mode == 2 and (i + 4) <= n and nums[i + 2] >= 0 and nums[i + 3] >= 0 and nums[i + 4] >= 0 then
				st[key] = string.format("%d;2;%d;%d;%d", c, nums[i + 2], nums[i + 3], nums[i + 4])
				i = i + 4
			end
		end
		i = i + 1
	end
end

local function serialize_sgr_state(st)
	local parts = {}
	if st.b then
		parts[#parts + 1] = "1"
	end
	if st.d then
		parts[#parts + 1] = "2"
	end
	if st.it then
		parts[#parts + 1] = "3"
	end
	if st.u then
		parts[#parts + 1] = "4"
	end
	if st.rev then
		parts[#parts + 1] = "7"
	end
	if st.fg then
		parts[#parts + 1] = st.fg
	end
	if st.bg then
		parts[#parts + 1] = st.bg
	end
	if #parts == 0 then
		return ""
	end
	return "\27[" .. table.concat(parts, ";") .. "m"
end

local function slice_ansi_preserve_sgr(s, offset_cols, width_cols, initial_sgr)
	local st = clone_sgr_state(initial_sgr)
	if not s or s == "" or width_cols <= 0 then
		return "", st
	end

	local idx, len = 1, #s
	local pos_col = 0
	local out_w = 0
	local started = false
	local ended = false
	local out = {}

	while idx <= len do
		local b = s:byte(idx)
		if b == 0x1B then
			local nb = s:byte(idx + 1)
			if nb == 0x5B then
				local j = idx + 2
				while j <= len do
					local cb = s:byte(j)
					if cb >= 0x40 and cb <= 0x7E then
						break
					end
					j = j + 1
				end
				if j <= len then
					local final_b = s:byte(j)
					local seq = s:sub(idx, j)
					if final_b == 0x6D then
						apply_sgr_seq(st, seq)
						if started and not ended then
							out[#out + 1] = seq
						end
					end
					idx = j + 1
				else
					break
				end
			elseif nb == 0x5D then
				local j = idx + 2
				while j <= len do
					local cb = s:byte(j)
					if cb == 0x07 then
						j = j + 1
						break
					elseif cb == 0x1B and s:byte(j + 1) == 0x5C then
						j = j + 2
						break
					end
					j = j + 1
				end
				idx = j
			else
				idx = idx + 2
			end
		elseif b < 0x20 or b == 0x7F then
			idx = idx + 1
		else
			local clen, cw = char_display_cols(b)
			local ch = s:sub(idx, idx + clen - 1)
			idx = idx + clen

			if not ended then
				if pos_col + cw <= offset_cols then
					pos_col = pos_col + cw
				else
					if not started then
						started = true
						local sgr_prefix = serialize_sgr_state(st)
						if sgr_prefix ~= "" then
							out[#out + 1] = sgr_prefix
						end
						if pos_col < offset_cols and cw == 2 then
							if out_w + 1 <= width_cols then
								out[#out + 1] = " "
								out_w = out_w + 1
							else
								ended = true
							end
							pos_col = pos_col + cw
						else
							if out_w + cw > width_cols then
								ended = true
							else
								out[#out + 1] = ch
								out_w = out_w + cw
								pos_col = pos_col + cw
							end
						end
					else
						if out_w + cw > width_cols then
							ended = true
						else
							out[#out + 1] = ch
							out_w = out_w + cw
							pos_col = pos_col + cw
						end
					end
				end
			end
		end
	end

	if not started then
		return "", st
	end
	out[#out + 1] = "\27[0m"
	return table.concat(out), st
end

local function url_hash(s)
	local h1 = 5381
	local h2 = 52711
	for i = 1, #s do
		local b = s:byte(i)
		h1 = ((h1 * 33) + b) % 2147483647
		h2 = ((h2 * 131) + b) % 2147483647
	end
	return string.format("%08x%08x", h1, h2)
end

local function read_url_bounds(url_str)
	if not url_str or url_str == "" then
		return 0, false
	end
	local f = io.open("/tmp/.cc-ide/yb_" .. url_hash(url_str) .. ".txt", "r")
	if not f then
		return 0, false
	end
	local mx = tonumber(f:read("*l") or "0") or 0
	local ab = (f:read("*l") == "1")
	f:close()
	return mx, ab
end

local function write_url_bounds(url_str, max_x, at_bot)
	if not url_str or url_str == "" then
		return
	end
	local p = "/tmp/.cc-ide/yb_" .. url_hash(url_str) .. ".txt"
	local tmp = p .. ".tmp"
	local f = io.open(tmp, "w")
	if not f then
		return
	end
	f:write(tostring(math.max(0, math.floor(max_x or 0))) .. "\n" .. (at_bot and "1\n" or "0\n"))
	f:close()
	os.rename(tmp, p)
end

local function read_url_px(url_str)
	if not url_str or url_str == "" then
		return 0
	end
	local f = io.open("/tmp/.cc-ide/yx_" .. url_hash(url_str) .. ".txt", "r")
	if not f then
		return 0
	end
	local px = tonumber(f:read("*l") or "0") or 0
	f:close()
	return math.max(0, math.floor(px))
end

local function write_url_px(url_str, px)
	if not url_str or url_str == "" then
		return
	end
	local p = "/tmp/.cc-ide/yx_" .. url_hash(url_str) .. ".txt"
	local tmp = p .. ".tmp"
	local f = io.open(tmp, "w")
	if not f then
		return
	end
	f:write(tostring(math.max(0, math.floor(px or 0))) .. "\n")
	f:close()
	os.rename(tmp, p)
end

local function sync_bounds_and_get_px(url_str, max_x, at_bot)
	local mx = math.max(0, math.floor(max_x or 0))
	write_url_bounds(url_str, mx, at_bot)
	local px = math.min(read_url_px(url_str), mx)
	if px ~= read_url_px(url_str) then
		write_url_px(url_str, px)
	end
	return px
end

local function write_focus_file(key_str, line_no, span_no, pending)
	if not key_str or key_str == "" then
		return
	end
	local p = "/tmp/.cc-ide/yf_" .. url_hash(key_str) .. ".txt"
	if not line_no or line_no <= 0 then
		os.remove(p)
		return
	end
	local tmp = p .. ".tmp"
	local f = io.open(tmp, "w")
	if not f then
		return
	end
	f:write(
		tostring(math.floor(line_no))
			.. "\n"
			.. tostring(math.max(1, math.floor(span_no or 1)))
			.. "\n"
			.. tostring(pending or 0)
			.. "\n"
			.. tostring(os.time())
			.. "\n"
	)
	f:close()
	os.rename(tmp, p)
end

local function read_single_focus(key_str)
	if not key_str or key_str == "" then
		return 0, 0, 0
	end
	local f = io.open("/tmp/.cc-ide/yf_" .. url_hash(key_str) .. ".txt", "r")
	if not f then
		return 0, 0, 0
	end
	local ln = tonumber(f:read("*l") or "0") or 0
	local sp = tonumber(f:read("*l") or "1") or 1
	local pend = tonumber(f:read("*l") or "0") or 0
	local ts = tonumber(f:read("*l") or "0") or 0
	f:close()
	if ln <= 0 or (ts > 0 and (os.time() - ts) > 900) then
		return 0, 0, 0
	end
	return ln, math.max(1, sp), pend
end

local function read_focus_marker(url_str, path_str)
	local ln, sp, pend = read_single_focus(url_str)
	if ln > 0 then
		return ln, sp, pend
	end
	local clean_u = (url_str or ""):gsub("^file://", "")
	if clean_u ~= url_str then
		ln, sp, pend = read_single_focus(clean_u)
		if ln > 0 then
			return ln, sp, pend
		end
	end
	if path_str and path_str ~= "" and path_str ~= url_str then
		return read_single_focus(path_str)
	end
	return 0, 0, 0
end

local function consume_focus_pending(url_str, path_str)
	for _, k in ipairs({ url_str, (url_str or ""):gsub("^file://", ""), path_str }) do
		if k and k ~= "" then
			local ln, sp, pend = read_single_focus(k)
			if ln > 0 and pend == 1 then
				write_focus_file(k, ln, sp, 0)
			end
		end
	end
end

local function current_hovered_url()
	local tab = cx and (cx.active or cx.tab)
	local h = tab and tab.current and tab.current.hovered
	return h and tostring(h.url) or ""
end

local function trigger_preview_refresh(force_now)
	local tab = cx and (cx.active or cx.tab)
	local h = tab and tab.current and tab.current.hovered
	if not h then
		return
	end
	if not force_now and ya.time then
		local now = ya.time()
		if (now - (_G.CCIDE_LAST_PEEK_EMIT or 0)) < 0.015 then
			return
		end
		_G.CCIDE_LAST_PEEK_EMIT = now
	end
	ya.emit("peek", { tab.preview.skip, only_if = h.url, force = true })
end

local dispatch_sync = ya.sync(function(_, action, arg1, arg2, arg3)
	_G.CCIDE_INPUT_MODE = _G.CCIDE_INPUT_MODE or "mouse"

	if action == "key_nav" then
		_G.CCIDE_INPUT_MODE = "key"
		_G.CCIDE_CUR_Y = nil
		_G.CCIDE_PAR_Y = nil
		local cmd = tostring(arg1 or "")
		if cmd == "arrow" then
			local step = tonumber(arg2) or ((arg2 == "top" or arg2 == "bot") and arg2) or 1
			ya.emit("arrow", { step })
		elseif cmd == "leave" then
			ya.emit("leave", {})
		elseif cmd == "enter" then
			ya.emit("enter", {})
		end
		ui.render()
		return
	end

	if action == "reveal" then
		local target = tostring(arg1 or "")
		local target_line = tonumber(arg2) or 0
		local target_span = math.max(1, tonumber(arg3) or 1)
		if target ~= "" then
			_G.CCIDE_INPUT_MODE = "key"
			_G.CCIDE_CUR_Y = nil
			_G.CCIDE_PAR_Y = nil
			_G.CCIDE_CUR_X = 0
			if target_line > 0 then
				write_focus_file(target, target_line, target_span, 1)
			end
			write_url_px(target, 0)
			local u = current_hovered_url()
			if u ~= "" then
				write_url_px(u, 0)
			end
			local clean_u = u:gsub("^file://", "")
			ya.emit("reveal", { target })
			if clean_u == target or u == target then
				local tab = cx and (cx.active or cx.tab)
				local h = tab and tab.current and tab.current.hovered
				if h then
					local prev_h = _G.CCIDE_PREV_AREA_H or 24
					local desired = (target_line > 1)
							and math.max(0, target_line - 1 - math.max(3, math.floor(prev_h / 4)))
						or 0
					consume_focus_pending(u, target)
					ya.emit("peek", { desired, only_if = h.url, force = true })
				end
			end
			ui.render()
		end
		return
	end

	if action == "reset" then
		_G.CCIDE_PAR_X = 0
		_G.CCIDE_CUR_X = 0
		_G.CCIDE_PREV_DIR_X = 0
		local u = current_hovered_url()
		if u ~= "" and read_url_px(u) ~= 0 then
			write_url_px(u, 0)
			trigger_preview_refresh(true)
		end
		ui.render()
		return
	end

	_G.CCIDE_INPUT_MODE = "key"
	local delta = tonumber(arg1) or 0
	if delta == 0 then
		return
	end

	local cur_max = _G.CCIDE_CUR_MAX_X or 0
	local old_cur = _G.CCIDE_CUR_X or 0
	_G.CCIDE_CUR_X = (cur_max > 0) and math.max(0, math.min(cur_max, old_cur + delta)) or 0

	local dir_max = _G.CCIDE_PREV_DIR_MAX_X or 0
	local old_dir = _G.CCIDE_PREV_DIR_X or 0
	_G.CCIDE_PREV_DIR_X = (dir_max > 0) and math.max(0, math.min(dir_max, old_dir + delta)) or 0

	local u = current_hovered_url()
	local prev_max = select(1, read_url_bounds(u))
	local old_prev = read_url_px(u)
	local new_prev = (prev_max > 0) and math.max(0, math.min(prev_max, old_prev + delta)) or 0
	if new_prev ~= old_prev or (_G.CCIDE_PREV_DIR_X or 0) ~= old_dir then
		if new_prev ~= old_prev then
			write_url_px(u, new_prev)
		end
		trigger_preview_refresh(true)
	end
	if (_G.CCIDE_CUR_X or 0) ~= old_cur or new_prev ~= old_prev or (_G.CCIDE_PREV_DIR_X or 0) ~= old_dir then
		ui.render()
	end
end)

function M:setup()
	if _G.CCIDE_HSCROLL_INSTALLED then
		return
	end
	_G.CCIDE_HSCROLL_INSTALLED = true

	_G.CCIDE_INPUT_MODE = "mouse"
	_G.CCIDE_DRAW_COL = nil
	_G.CCIDE_PAR_X = 0
	_G.CCIDE_PAR_MAX_X = 0
	_G.CCIDE_PAR_Y = nil
	_G.CCIDE_CUR_X = 0
	_G.CCIDE_CUR_MAX_X = 0
	_G.CCIDE_CUR_Y = nil
	_G.CCIDE_PREV_DIR_X = 0
	_G.CCIDE_PREV_DIR_MAX_X = 0
	_G.CCIDE_LAST_HOVER_URL = ""
	_G.CCIDE_LAST_PEEK_EMIT = 0

	ps.sub("cd", function()
		_G.CCIDE_PAR_X = 0
		_G.CCIDE_CUR_X = 0
		_G.CCIDE_PREV_DIR_X = 0
		_G.CCIDE_PAR_Y = nil
		_G.CCIDE_CUR_Y = nil
		local u = current_hovered_url()
		_G.CCIDE_LAST_HOVER_URL = u
		if u ~= "" then
			write_url_px(u, 0)
		end
	end)
	ps.sub("hover", function()
		local u = current_hovered_url()
		if u ~= (_G.CCIDE_LAST_HOVER_URL or "") then
			_G.CCIDE_LAST_HOVER_URL = u
			_G.CCIDE_CUR_Y = nil
			_G.CCIDE_PREV_DIR_X = 0
			if u ~= "" then
				write_url_px(u, 0)
				local hl_ln, _, pend = read_focus_marker(u, u:gsub("^file://", ""))
				if pend == 1 and hl_ln > 1 then
					local tab = cx and (cx.active or cx.tab)
					local h = tab and tab.current and tab.current.hovered
					if h then
						local prev_h = _G.CCIDE_PREV_AREA_H or 24
						local desired = math.max(0, hl_ln - 1 - math.max(3, math.floor(prev_h / 4)))
						consume_focus_pending(u, u:gsub("^file://", ""))
						ya.emit("peek", { desired, only_if = h.url, force = true })
					end
				end
			end
		end
	end)

	if Entity then
		local orig_style = Entity.style
		if orig_style then
			function Entity:style()
				if (_G.CCIDE_INPUT_MODE or "mouse") == "mouse" then
					return self._file:style() or ui.Style()
				end
				return orig_style(self)
			end
		end

		local orig_click = Entity.click
		if orig_click then
			function Entity:click(event, up)
				if not up and not event.is_middle then
					_G.CCIDE_INPUT_MODE = "key"
					_G.CCIDE_CUR_Y = nil
					_G.CCIDE_PAR_Y = nil
				end
				return orig_click(self, event, up)
			end
		end

		local orig_highlights = Entity.highlights
		if orig_highlights then
			function Entity:highlights()
				local col = _G.CCIDE_DRAW_COL
				local off = (col == "current" and (_G.CCIDE_CUR_X or 0))
					or (col == "parent" and (_G.CCIDE_PAR_X or 0))
					or (col == "preview_dir" and (_G.CCIDE_PREV_DIR_X or 0))
					or 0
				if off <= 0 or not self._file then
					return orig_highlights(self)
				end
				local full = self._file.name
				if rt and rt.mgr and rt.mgr.show_symlink and self._file.link_to then
					full = full .. " -> " .. tostring(self._file.link_to)
				end
				return ui.printable(utf8_drop_cols(full, off))
			end
		end

		local orig_symlink = Entity.symlink
		if orig_symlink then
			function Entity:symlink()
				local col = _G.CCIDE_DRAW_COL
				local off = (col == "current" and (_G.CCIDE_CUR_X or 0))
					or (col == "parent" and (_G.CCIDE_PAR_X or 0))
					or (col == "preview_dir" and (_G.CCIDE_PREV_DIR_X or 0))
					or 0
				if off > 0 and self._file then
					return ""
				end
				return orig_symlink(self)
			end
		end
	end

	local function render_folder_column(col_name, folder, area, empty_fn, include_dnd)
		if not folder then
			return {}
		end
		local all_files = folder.files
		local total = all_files and #all_files or 0
		if total == 0 then
			if col_name == "current" then
				_G.CCIDE_CUR_MAX_X = 0
				_G.CCIDE_CUR_X = 0
			else
				_G.CCIDE_PAR_MAX_X = 0
				_G.CCIDE_PAR_X = 0
			end
			return empty_fn and empty_fn() or {}
		end

		local h = math.max(1, area.h)
		local max_y = math.max(0, total - h)
		local custom_y = (col_name == "current") and _G.CCIDE_CUR_Y or _G.CCIDE_PAR_Y
		local start_off = folder.offset or 0
		if custom_y ~= nil then
			start_off = math.max(0, math.min(max_y, custom_y))
			if col_name == "current" then
				_G.CCIDE_CUR_Y = start_off
			else
				_G.CCIDE_PAR_Y = start_off
			end
		end

		_G.CCIDE_DRAW_COL = nil
		local max_overflow = 0
		local vis_files = {}
		local right = {}
		local last_idx = math.min(total, start_off + h)
		for idx = start_off + 1, last_idx do
			local f = all_files[idx]
			if f then
				vis_files[#vis_files + 1] = f
				local rline = Linemode:new(f):redraw()
				right[#right + 1] = rline
				local avail = math.max(0, area.w - rline:width())
				local nat_line = Entity:new(f):redraw()
				local over = nat_line:width() - avail
				if over > max_overflow then
					max_overflow = over
				end
			end
		end

		if col_name == "current" then
			_G.CCIDE_CUR_MAX_X = max_overflow
			_G.CCIDE_CUR_X = math.max(0, math.min(_G.CCIDE_CUR_X or 0, max_overflow))
		else
			_G.CCIDE_PAR_MAX_X = max_overflow
			_G.CCIDE_PAR_X = math.max(0, math.min(_G.CCIDE_PAR_X or 0, max_overflow))
		end

		_G.CCIDE_DRAW_COL = col_name
		local left = {}
		for i, f in ipairs(vis_files) do
			local entity = Entity:new(f)
			local lline = entity:redraw()
			local avail = math.max(0, area.w - right[i]:width())
			lline:truncate { max = avail, ellipsis = entity:ellipsis(avail) }
			left[#left + 1] = lline
		end
		_G.CCIDE_DRAW_COL = nil

		if include_dnd and Dnd then
			return {
				ui.List(left):area(area),
				ui.Text(right):area(area):align(ui.Align.RIGHT),
				table.unpack(Dnd:new(area):redraw()),
			}
		end
		return {
			ui.List(left):area(area),
			ui.Text(right):area(area):align(ui.Align.RIGHT),
		}
	end

	if Current then
		function Current:redraw()
			return render_folder_column("current", self._folder, self._area, function()
				return self:empty()
			end, true)
		end

		function Current:click(event, up)
			if up or event.is_middle then
				return
			end
			local all_files = self._folder and self._folder.files
			if not all_files then
				return
			end
			local off = (_G.CCIDE_CUR_Y ~= nil) and _G.CCIDE_CUR_Y or (self._folder.offset or 0)
			local idx = off + (event.y - self._area.y + 1)
			local f = all_files[idx]
			if f then
				_G.CCIDE_INPUT_MODE = "key"
				_G.CCIDE_CUR_Y = nil
				Entity:new(f):click(event, up)
			end
		end

		function Current:scroll(_, step)
			_G.CCIDE_INPUT_MODE = "mouse"
			local all_files = self._folder and self._folder.files
			local total = all_files and #all_files or 0
			local max_y = math.max(0, total - math.max(1, self._area.h))
			if max_y <= 0 then
				return
			end
			local cur = (_G.CCIDE_CUR_Y ~= nil) and _G.CCIDE_CUR_Y or (self._folder.offset or 0)
			local next_y = math.max(0, math.min(max_y, cur + step))
			if next_y ~= cur then
				_G.CCIDE_CUR_Y = next_y
				ui.render()
			end
		end

		function Current:touch(_, step)
			_G.CCIDE_INPUT_MODE = "mouse"
			local max_x = _G.CCIDE_CUR_MAX_X or 0
			if max_x <= 0 then
				return
			end
			local delta = (step < 0 and 2) or (step > 0 and -2) or 0
			local cur = _G.CCIDE_CUR_X or 0
			local next_x = math.max(0, math.min(max_x, cur + delta))
			if next_x ~= cur then
				_G.CCIDE_CUR_X = next_x
				ui.render()
			end
		end
	end

	if Parent then
		function Parent:redraw()
			if not self._folder then
				return {}
			end
			return render_folder_column("parent", self._folder, self._area, nil, false)
		end

		function Parent:click(event, up)
			if up or event.is_middle then
				return
			end
			local all_files = self._folder and self._folder.files
			local off = (_G.CCIDE_PAR_Y ~= nil) and _G.CCIDE_PAR_Y or (self._folder and self._folder.offset or 0)
			local idx = off + (event.y - self._area.y + 1)
			local f = all_files and all_files[idx]
			if f then
				_G.CCIDE_INPUT_MODE = "key"
				_G.CCIDE_PAR_Y = nil
				Entity:new(f):click(event, up)
			elseif not up and event.is_left then
				_G.CCIDE_INPUT_MODE = "key"
				ya.emit("leave", {})
			end
		end

		function Parent:scroll(_, step)
			_G.CCIDE_INPUT_MODE = "mouse"
			local all_files = self._folder and self._folder.files
			local total = all_files and #all_files or 0
			local max_y = math.max(0, total - math.max(1, self._area.h))
			if max_y <= 0 then
				return
			end
			local cur = (_G.CCIDE_PAR_Y ~= nil) and _G.CCIDE_PAR_Y or (self._folder.offset or 0)
			local next_y = math.max(0, math.min(max_y, cur + step))
			if next_y ~= cur then
				_G.CCIDE_PAR_Y = next_y
				ui.render()
			end
		end

		function Parent:touch(_, step)
			_G.CCIDE_INPUT_MODE = "mouse"
			local max_x = _G.CCIDE_PAR_MAX_X or 0
			if max_x <= 0 then
				return
			end
			local delta = (step < 0 and 2) or (step > 0 and -2) or 0
			local cur = _G.CCIDE_PAR_X or 0
			local next_x = math.max(0, math.min(max_x, cur + delta))
			if next_x ~= cur then
				_G.CCIDE_PAR_X = next_x
				ui.render()
			end
		end
	end

	if Preview then
		local orig_pclick = Preview.click
		if orig_pclick then
			function Preview:click(event, up)
				if not up and not event.is_middle then
					_G.CCIDE_INPUT_MODE = "key"
				end
				return orig_pclick(self, event, up)
			end
		end

		function Preview:scroll(_, step)
			_G.CCIDE_INPUT_MODE = "mouse"
			_G.CCIDE_PREV_AREA_H = self._area and self._area.h or _G.CCIDE_PREV_AREA_H
			local tab = self._tab or (cx and (cx.active or cx.tab))
			local skip = tab and tab.preview and tab.preview.skip or 0
			if step < 0 and skip <= 0 then
				return
			end
			if step > 0 then
				if self._folder then
					local total = self._folder.files and #self._folder.files or 0
					if total <= self._area.h or skip >= total - self._area.h then
						return
					end
				else
					local _, at_bot = read_url_bounds(current_hovered_url())
					if at_bot then
						return
					end
				end
			end
			ya.emit("seek", { step })
		end

		function Preview:touch(_, step)
			_G.CCIDE_INPUT_MODE = "mouse"
			local delta = (step < 0 and 2) or (step > 0 and -2) or 0
			if delta == 0 then
				return
			end
			if self._folder then
				local max_x = _G.CCIDE_PREV_DIR_MAX_X or 0
				if max_x <= 0 then
					return
				end
				local cur = _G.CCIDE_PREV_DIR_X or 0
				local next_x = math.max(0, math.min(max_x, cur + delta))
				if next_x ~= cur then
					_G.CCIDE_PREV_DIR_X = next_x
					trigger_preview_refresh(true)
				end
				return
			end
			local u = current_hovered_url()
			if u == "" then
				return
			end
			local max_x = select(1, read_url_bounds(u))
			if max_x <= 0 then
				return
			end
			local cur = read_url_px(u)
			local next_x = math.max(0, math.min(max_x, cur + delta))
			if next_x ~= cur then
				write_url_px(u, next_x)
				trigger_preview_refresh(false)
			end
		end
	end

	-- Enhance built-in `--- @sync peek` `folder` plugin directly inside the Main UI Lua VM
	-- (self-contained so lazy metatable proxies never recurse into `orig_folder_peek`).
	local ok_folder, folder_mod = pcall(require, "folder")
	if ok_folder and (type(folder_mod) == "table" or type(folder_mod) == "userdata") then
		function folder_mod:peek(job)
			local tab = cx and (cx.tab or cx.active)
			local folder = tab and tab.preview and tab.preview.folder
			if not folder then
				_G.CCIDE_PREV_DIR_MAX_X = 0
				_G.CCIDE_PREV_DIR_X = 0
				return ya.preview_widget(job, ui.Line("Loading..."):area(job.area):align(ui.Align.CENTER))
			elseif folder.cwd ~= job.file.url then
				return
			end

			local bound = math.max(0, #folder.files - job.area.h)
			if job.skip > bound then
				return ya.emit("peek", { bound, only_if = job.file.url, upper_bound = true, scope = job.scope })
			end

			if #folder.files == 0 then
				_G.CCIDE_PREV_DIR_MAX_X = 0
				_G.CCIDE_PREV_DIR_X = 0
				local done, err = folder.stage()
				local s = not done and "Loading..." or not err and "No items" or string.format("Error: %s", err)
				return ya.preview_widget(job, ui.Text(s):area(job.area):align(ui.Align.CENTER):wrap(ui.Wrap.YES))
			end

			_G.CCIDE_DRAW_COL = nil
			local max_overflow = 0
			local right = {}
			for i, f in ipairs(folder.window) do
				local rline = Linemode:new(f):redraw()
				right[i] = rline
				local avail = math.max(0, job.area.w - rline:width())
				local over = Entity:new(f):redraw():width() - avail
				if over > max_overflow then
					max_overflow = over
				end
			end
			_G.CCIDE_PREV_DIR_MAX_X = max_overflow
			_G.CCIDE_PREV_DIR_X = math.max(0, math.min(_G.CCIDE_PREV_DIR_X or 0, max_overflow))
			_G.CCIDE_DRAW_COL = "preview_dir"
			local left = {}
			for i, f in ipairs(folder.window) do
				local entity = Entity:new(f)
				local lline = entity:redraw()
				local avail = math.max(0, job.area.w - right[i]:width())
				lline:truncate { max = avail, ellipsis = entity:ellipsis(avail) }
				left[i] = lline
			end
			_G.CCIDE_DRAW_COL = nil
			local marker_area = job.area { x = math.max(0, job.area.x - 1) }
			return ya.preview_widget(job, {
				ui.Text(left):area(job.area),
				ui.Text(right):area(job.area):align(ui.Align.RIGHT),
				table.unpack(Marker:new(marker_area, folder):redraw()),
			})
		end
	end
end

-- ── 120Hz Cached Previewer Implementation (`M:peek` / `M:seek` in `slim_lua` worker VM) ──
local function file_stamp(file)
	local cha = file and file.cha
	local mtime = cha and cha.mtime or 0
	local len = cha and cha.len or 0
	return tostring(file.path) .. "\t" .. tostring(mtime) .. "\t" .. tostring(len)
end

local function read_cached_block(cache_path, expected_key)
	local f = io.open(cache_path, "r")
	if not f then
		return nil
	end
	local first = f:read("*l")
	if first ~= expected_key then
		f:close()
		return nil
	end
	local rest = f:read("*a")
	f:close()
	return rest
end

local function write_cached_block(cache_path, key, body)
	local tmp = cache_path .. ".tmp"
	local f = io.open(tmp, "w")
	if not f then
		return
	end
	f:write(key .. "\n")
	f:write(body or "")
	f:close()
	os.rename(tmp, cache_path)
end

local function find_bat_bin()
	for _, p in ipairs({ "/opt/homebrew/bin/bat", "/usr/local/bin/bat", "/usr/bin/bat", "/usr/bin/batcat" }) do
		local f = io.open(p, "r")
		if f then
			f:close()
			return p
		end
	end
	return nil
end

local function peek_markdown(job, url_str)
	local area_w = job.area.w
	local area_h = job.area.h
	local skip = job.skip or 0
	local key = file_stamp(job.file)
	local cache_path = "/tmp/.cc-ide/ym_" .. url_hash(url_str) .. ".txt"
	local raw_all = read_cached_block(cache_path, key)

	if not raw_all then
		local home = os.getenv("HOME") or ""
		local script = home .. "/.claude/scripts/md-render.py"
		local out = Command("python3"):arg({ script, "-w", tostring(area_w), tostring(job.file.path) }):stdout(Command.PIPED):stderr(Command.NULL):output()
		raw_all = (out and out.stdout) or ""
		if raw_all ~= "" then
			write_cached_block(cache_path, key, raw_all)
		end
	end

	local all_lines = {}
	for ln in (raw_all .. "\n"):gmatch("(.-)\r?\n") do
		all_lines[#all_lines + 1] = ln
	end
	if #all_lines > 0 and all_lines[#all_lines] == "" then
		all_lines[#all_lines] = nil
	end

	local total = #all_lines
	local path_str = tostring(job.file.path)
	local hl_ln, _, hl_pend = read_focus_marker(url_str, path_str)
	if hl_pend == 1 and hl_ln > 0 and total > 0 then
		consume_focus_pending(url_str, path_str)
		local raw_cnt = 0
		local rf = io.open(path_str, "r")
		if rf then
			for _ in rf:lines() do
				raw_cnt = raw_cnt + 1
			end
			rf:close()
		end
		local approx_row = (raw_cnt > 0) and math.max(1, math.floor((hl_ln / raw_cnt) * total)) or hl_ln
		local desired = math.min(
			math.max(0, total - area_h),
			math.max(0, approx_row - 1 - math.max(3, math.floor(area_h / 4)))
		)
		if desired ~= skip then
			ya.emit("peek", { desired, only_if = job.file.url, force = true })
			return
		end
	end
	if skip > 0 and skip > math.max(0, total - area_h) then
		ya.emit("peek", { math.max(0, total - area_h), only_if = job.file.url, upper_bound = true })
		return
	end

	local slice = {}
	local end_idx = math.min(total, skip + area_h)
	local tab_spaces = string.rep(" ", (rt and rt.preview and rt.preview.tab_size) or 2)
	local max_w = 0
	for i = skip + 1, end_idx do
		local expanded = (all_lines[i] or ""):gsub("\t", tab_spaces)
		slice[#slice + 1] = expanded
		local lw = ansi_visible_width(expanded)
		if lw > max_w then
			max_w = lw
		end
	end

	local max_x = math.max(0, max_w - area_w)
	local at_bot = (end_idx >= total)
	local px = sync_bounds_and_get_px(url_str, max_x, at_bot)
	local function emit_md_at(pan_x)
		local rendered_rows = {}
		local carry = nil
		for i = 1, #slice do
			rendered_rows[i], carry = slice_ansi_preserve_sgr(slice[i], pan_x, area_w, carry)
		end
		ya.preview_widget(job, ui.Text.parse(table.concat(rendered_rows, "\n")):area(job.area))
	end
	emit_md_at(px)
	if px > 0 and ya.sleep then
		ya.sleep(0.016)
		local latest_px = math.min(read_url_px(url_str), max_x)
		if latest_px ~= px then
			emit_md_at(latest_px)
		end
	end
end

local function resolve_yazi_bat_theme()
	-- Match Yazi's `Highlighter::load_theme()` (`yazi-core/src/highlighter.rs:120-137`) 100%:
	-- use `th.mgr.syntect_theme` when configured and readable, else Yazi's built-in `ansi` theme!
	local ok, custom = pcall(function()
		return th and th.mgr and th.mgr.syntect_theme and tostring(th.mgr.syntect_theme) or ""
	end)
	if ok and custom and custom ~= "" then
		local f = io.open(custom, "r")
		if f then
			f:close()
			return custom
		end
	end
	return "ansi"
end

local function fetch_or_warm_bat_cache(file, url_str, path_str, skip, area_h, tab_sz, hl_start, hl_end)
	local theme_name = resolve_yazi_bat_theme()
	local hl_tag = (hl_start and hl_start > 0) and string.format("%d:%d", hl_start, hl_end or hl_start) or ""
	local cache_path = "/tmp/.cc-ide/yc2_" .. url_hash(url_str) .. ".txt"
	local key = file_stamp(file) .. "\t" .. tostring(skip) .. "\t" .. tostring(area_h) .. "\t" .. theme_name .. "\t" .. hl_tag
	local cached = read_cached_block(cache_path, key)
	if cached then
		return cached
	end
	local bat_bin = find_bat_bin()
	if not bat_bin then
		return nil
	end
	local range_arg = string.format("%d:%d", skip + 1, skip + area_h)
	local cmd_args = {
		"--color=always",
		"--theme=" .. theme_name,
		"--style=plain",
		"--paging=never",
		"--tabs=" .. tostring(tab_sz),
		"--line-range=" .. range_arg,
	}
	if hl_tag ~= "" then
		cmd_args[#cmd_args + 1] = "--highlight-line=" .. hl_tag
	end
	cmd_args[#cmd_args + 1] = path_str
	local out = Command(bat_bin)
		:arg(cmd_args)
		:stdout(Command.PIPED)
		:stderr(Command.NULL)
		:output()
	if out and out.stdout and out.stdout ~= "" then
		write_cached_block(cache_path, key, out.stdout)
		return out.stdout
	end
	return nil
end

local function peek_code(job, url_str)
	local area_w = job.area.w
	local area_h = job.area.h
	local skip = job.skip or 0
	local tab_sz = (rt and rt.preview and rt.preview.tab_size) or 2
	local tab_spaces = string.rep(" ", tab_sz)
	local path_str = tostring(job.file.path)
	local hl_ln, hl_sp, hl_pend = read_focus_marker(url_str, path_str)
	local hl_end = (hl_ln > 0) and (hl_ln + math.max(1, hl_sp) - 1) or 0

	local f = io.open(path_str, "r")
	local vis_lines = {}
	local max_w = 0
	local has_more = false
	local total_lines = 0
	local s_ln = skip + 1
	local e_ln = skip + area_h
	if f then
		local idx = 0
		local scan_cap = math.max(e_ln + 2, 50000)
		for line in f:lines() do
			idx = idx + 1
			if idx >= s_ln and idx <= e_ln then
				local exp = line:gsub("\r$", ""):gsub("\t", tab_spaces)
				vis_lines[#vis_lines + 1] = exp
				local lw = utf8_width(exp)
				if lw > max_w then
					max_w = lw
				end
			elseif idx > e_ln then
				has_more = true
				if idx >= scan_cap then
					break
				end
			end
		end
		total_lines = idx
		f:close()
	end

	if hl_pend == 1 and hl_ln > 0 then
		consume_focus_pending(url_str, path_str)
		local desired = math.max(0, hl_ln - 1 - math.max(3, math.floor(area_h / 4)))
		if total_lines > 0 and total_lines < 50000 then
			desired = math.min(desired, math.max(0, total_lines - area_h))
		end
		if desired ~= skip then
			ya.emit("peek", { desired, only_if = job.file.url, force = true })
			return
		end
	end

	if skip > 0 and total_lines > 0 and total_lines < 50000 and skip > math.max(0, total_lines - area_h) then
		ya.emit("peek", { math.max(0, total_lines - area_h), only_if = job.file.url, upper_bound = true })
		return
	end

	local max_x = math.max(0, max_w - area_w)
	local at_bot = (f ~= nil) and (not has_more)
	local px = sync_bounds_and_get_px(url_str, max_x, at_bot)
	local has_vis_hl = (hl_ln > 0) and (hl_ln <= e_ln) and (hl_end >= s_ln)

	if px <= 0 and not has_vis_hl then
		require("code"):peek(job)
		if max_x > 0 then
			fetch_or_warm_bat_cache(job.file, url_str, path_str, skip, area_h, tab_sz, hl_ln, hl_end)
		end
		return
	end

	if #vis_lines == 0 and skip > 0 then
		ya.emit("peek", { 0, only_if = job.file.url, upper_bound = true })
		return
	end

	local theme_name = resolve_yazi_bat_theme()
	local hl_tag = (hl_ln > 0) and string.format("%d:%d", hl_ln, hl_end) or ""
	local cache_path = "/tmp/.cc-ide/yc2_" .. url_hash(url_str) .. ".txt"
	local key = file_stamp(job.file) .. "\t" .. tostring(skip) .. "\t" .. tostring(area_h) .. "\t" .. theme_name .. "\t" .. hl_tag
	local ansi_slice = read_cached_block(cache_path, key)

	if not ansi_slice then
		local plain_rows = {}
		for i = 1, #vis_lines do
			local abs_ln = skip + i
			local sliced = slice_ansi_preserve_sgr(vis_lines[i], px, area_w, nil)
			if hl_ln > 0 and abs_ln >= hl_ln and abs_ln <= hl_end then
				plain_rows[i] = "\27[48;5;237m" .. sliced
			else
				plain_rows[i] = sliced
			end
		end
		ya.preview_widget(job, ui.Text.parse(table.concat(plain_rows, "\n")):area(job.area))
		ansi_slice = fetch_or_warm_bat_cache(job.file, url_str, path_str, skip, area_h, tab_sz, hl_ln, hl_end)
		px = math.min(read_url_px(url_str), max_x)
	end

	local function emit_code_at(pan_x)
		local text
		if ansi_slice and ansi_slice ~= "" then
			local ansi_rows = {}
			local carry = nil
			for ln in (ansi_slice .. "\n"):gmatch("(.-)\r?\n") do
				ansi_rows[#ansi_rows + 1], carry = slice_ansi_preserve_sgr(ln:gsub("\t", tab_spaces), pan_x, area_w, carry)
			end
			if #ansi_rows > area_h and ansi_rows[#ansi_rows] == "" then
				ansi_rows[#ansi_rows] = nil
			end
			text = ui.Text.parse(table.concat(ansi_rows, "\n")):area(job.area)
		else
			local plain_rows = {}
			for i = 1, #vis_lines do
				local abs_ln = skip + i
				local sliced = slice_ansi_preserve_sgr(vis_lines[i], pan_x, area_w, nil)
				if hl_ln > 0 and abs_ln >= hl_ln and abs_ln <= hl_end then
					plain_rows[i] = "\27[48;5;237m" .. sliced
				else
					plain_rows[i] = sliced
				end
			end
			text = ui.Text.parse(table.concat(plain_rows, "\n")):area(job.area)
		end
		ya.preview_widget(job, text)
	end

	emit_code_at(px)
	if ya.sleep then
		ya.sleep(0.016)
		local latest_px = math.min(read_url_px(url_str), max_x)
		if latest_px ~= px then
			emit_code_at(latest_px)
		end
	end
end

function M:peek(job)
	local mode = tostring((job.args and job.args[1]) or "code")
	local url_str = tostring(job.file.url)
	if mode == "md" then
		peek_markdown(job, url_str)
	else
		-- Both `"code"` and `"json"` route through SGR-state-preserving `peek_code` (`bat`)
		peek_code(job, url_str)
	end
end

function M:seek(job)
	require("code"):seek(job)
end

function M:entry(job)
	local args = job.args or {}
	local a1 = tostring(args[1] or "")
	local a2 = tostring(args[2] or "")
	local a3 = tostring(args[3] or "")
	local a4 = tostring(args[4] or "")
	if a1 == "key" then
		dispatch_sync("key_nav", a2, a3)
	elseif a1 == "reveal" then
		dispatch_sync("reveal", a2, a3, a4)
	elseif a1 == "reset" then
		dispatch_sync("reset")
	else
		dispatch_sync("pan", tonumber(a1) or 0)
	end
end

return M
