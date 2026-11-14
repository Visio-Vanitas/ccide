#!/usr/bin/env python3
"""
sync-pricing.py — Synchronizes `~/.config/cc-ide/pricing.json` with community-maintained
official vendor pricing from `BerriAI/litellm` and `pydantic/genai-prices` on GitHub
(following `tokscale` conventions, excluding subscription `$0` `github_copilot/` entries)
while preserving user `custom_overrides` and `cc-ide` curated Claude/Gemini/DeepSeek rates.
"""
import datetime
import json
import os
import re
import urllib.request

GENAI_PRICES_URL = "https://raw.githubusercontent.com/pydantic/genai-prices/main/prices/data_slim.json"
LITELLM_PRICES_URL = "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json"
TARGET_PROVIDERS = {"anthropic", "google", "deepseek", "openai", "x-ai"}
LITELLM_PREFIX_FILTER = ("claude-", "gemini-", "deepseek-", "gpt-", "o3", "o4", "grok-")


def strip_jsonc(raw: str) -> str:
    no_block = re.sub(r"/\*.*?\*/", "", raw, flags=re.DOTALL)
    no_line = re.sub(r"(?m)^\s*//.*$|(?<=\s)//.*$", "", no_block)
    return re.sub(r",\s*([}\]])", r"\1", no_line)


def extract_tier_val(field) -> tuple[float | None, int | None, float | None]:
    if isinstance(field, (int, float)):
        return float(field), None, None
    if isinstance(field, dict) and field.get("base") is not None:
        try:
            base = float(field["base"])
        except (TypeError, ValueError):
            return None, None, None
        tiers = field.get("tiers") or []
        if tiers and isinstance(tiers[0], dict):
            raw_start = tiers[0].get("start")
            raw_price = tiers[0].get("price")
            try:
                start = int(raw_start) if raw_start is not None else 200000
                price = float(raw_price) if raw_price is not None else base
            except (TypeError, ValueError):
                return base, None, None
            return base, start, price
        return base, None, None
    return None, None, None


def convert_genai_price(prices_spec) -> dict | None:
    if isinstance(prices_spec, list):
        if not prices_spec:
            return None
        chosen = prices_spec[-1]
        p = chosen.get("prices") if isinstance(chosen, dict) else None
    elif isinstance(prices_spec, dict):
        p = prices_spec
    else:
        return None
    if not isinstance(p, dict):
        return None

    in_base, above, in_long = extract_tier_val(p.get("input_mtok"))
    out_base, _, out_long = extract_tier_val(p.get("output_mtok"))
    if in_base is None or out_base is None:
        return None
    cw_base, _, cw_long = extract_tier_val(p.get("cache_write_mtok"))
    cr_base, _, cr_long = extract_tier_val(p.get("cache_read_mtok"))

    entry: dict[str, float | int] = {
        "input": round(in_base, 6),
        "output": round(out_base, 6),
        "cache_write_5m": round(cw_base if cw_base is not None else in_base, 6),
        "cache_read": round(cr_base if cr_base is not None else in_base * 0.1, 6),
    }
    if above is not None:
        entry["above_tokens"] = above
        entry["long_input"] = round(in_long if in_long is not None else in_base, 6)
        entry["long_output"] = round(out_long if out_long is not None else out_base, 6)
        entry["long_cache_write"] = round(
            cw_long if cw_long is not None else float(entry["cache_write_5m"]), 6
        )
        entry["long_cache_read"] = round(
            cr_long if cr_long is not None else float(entry["cache_read"]), 6
        )
    return entry


def fetch_json(url: str):
    req = urllib.request.Request(url, headers={"User-Agent": "cc-ide-pricing-sync/0.5"})
    with urllib.request.urlopen(req, timeout=20) as resp:
        return json.loads(resp.read().decode("utf-8"))


def main() -> int:
    home = os.path.expanduser("~")
    script_dir = os.path.dirname(os.path.realpath(__file__))
    repo_pricing = os.path.join(os.path.dirname(script_dir), "assets", "pricing.json")
    user_pricing = os.path.join(home, ".config", "cc-ide", "pricing.json")

    base_data: dict = {"custom_overrides": {}, "models": {}}
    curated_keys: set[str] = set()
    if os.path.isfile(repo_pricing):
        try:
            with open(repo_pricing, "r", encoding="utf-8") as f:
                curated = json.loads(strip_jsonc(f.read()))
            if isinstance(curated.get("models"), dict):
                base_data["models"].update(curated["models"])
                curated_keys.update(curated["models"].keys())
        except Exception:
            pass
    if os.path.isfile(user_pricing):
        try:
            with open(user_pricing, "r", encoding="utf-8") as f:
                user_loaded = json.loads(strip_jsonc(f.read()))
            if isinstance(user_loaded.get("custom_overrides"), dict):
                base_data["custom_overrides"].update(user_loaded["custom_overrides"])
        except Exception:
            pass

    synced = 0
    # 1. BerriAI/litellm (`tokscale` primary catalog)
    try:
        litellm = fetch_json(LITELLM_PRICES_URL)
        if isinstance(litellm, dict):
            for raw_k, spec in litellm.items():
                if not isinstance(spec, dict) or str(raw_k).startswith("github_copilot/"):
                    continue
                k = str(raw_k).split("/")[-1].strip().lower()
                if not k.startswith(LITELLM_PREFIX_FILTER) or k in curated_keys:
                    continue
                in_tok = spec.get("input_cost_per_token")
                out_tok = spec.get("output_cost_per_token")
                if not isinstance(in_tok, (int, float)) or not isinstance(out_tok, (int, float)):
                    continue
                if in_tok <= 0 and out_tok <= 0:
                    continue
                in_m = round(float(in_tok) * 1e6, 6)
                out_m = round(float(out_tok) * 1e6, 6)
                cw_t = spec.get("cache_creation_input_token_cost")
                cr_t = spec.get("cache_read_input_token_cost")
                entry: dict[str, float | int] = {
                    "input": in_m,
                    "output": out_m,
                    "cache_write_5m": round(float(cw_t) * 1e6, 6) if isinstance(cw_t, (int, float)) else in_m,
                    "cache_read": round(float(cr_t) * 1e6, 6) if isinstance(cr_t, (int, float)) else round(in_m * 0.1, 6),
                }
                ab_in = spec.get("input_cost_per_token_above_200k_tokens")
                ab_out = spec.get("output_cost_per_token_above_200k_tokens")
                if isinstance(ab_in, (int, float)) and isinstance(ab_out, (int, float)):
                    entry["above_tokens"] = 200000
                    entry["long_input"] = round(float(ab_in) * 1e6, 6)
                    entry["long_output"] = round(float(ab_out) * 1e6, 6)
                base_data["models"][k] = entry
                synced += 1
    except Exception:
        pass

    # 2. pydantic/genai-prices (authoritative provider YAML compilation)
    try:
        upstream = fetch_json(GENAI_PRICES_URL)
        if isinstance(upstream, list):
            for prov in upstream:
                if not isinstance(prov, dict) or prov.get("id") not in TARGET_PROVIDERS:
                    continue
                for md in prov.get("models") or []:
                    if not isinstance(md, dict):
                        continue
                    mid = str(md.get("id") or "").strip().lower()
                    if not mid or mid in curated_keys or any(x in mid for x in ("embed", "tts", "transcribe", "whisper")):
                        continue
                    conv = convert_genai_price(md.get("prices"))
                    if conv is not None:
                        existing = base_data["models"].get(mid, {})
                        if "cache_write_1h" in existing and "cache_write_1h" not in conv:
                            conv["cache_write_1h"] = existing["cache_write_1h"]
                        base_data["models"][mid] = conv
                        synced += 1
    except Exception:
        pass

    os.makedirs(os.path.dirname(user_pricing), exist_ok=True)
    today = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d")
    out_obj = {
        "updated_at": today,
        "source": "BerriAI/litellm + pydantic/genai-prices + cc-ide curated catalog",
        "custom_overrides": base_data["custom_overrides"],
        "models": dict(sorted(base_data["models"].items())),
    }
    tmp_p = user_pricing + ".tmp"
    with open(tmp_p, "w", encoding="utf-8") as f:
        json.dump(out_obj, f, indent=2, ensure_ascii=False)
        f.write("\n")
    os.replace(tmp_p, user_pricing)
    print(f"[OK] Synced {synced} upstream rates -> {user_pricing} (total {len(out_obj['models'])} models)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
