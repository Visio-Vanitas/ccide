#!/usr/bin/env python3
"""
iterm-pyenv.py — PyPI Mirror Latency Sniffer & Isolated `iterm2` Runtime Provisioner (`cc-ide`).

Provisions `~/.local/share/cc-ide/pyenv` with `iterm2` installed, bypassing PEP 668 externally-managed
Python restrictions without polluting Homebrew/system Python, and writes the resolved interpreter path
to `~/.cache/cc-ide/python_bin` for `ccide-core workspace` and `install.sh`.

CLI:
  python3 -I scripts/iterm-pyenv.py --ensure
  python3 -I scripts/iterm-pyenv.py --status [--json]
"""
import argparse
import concurrent.futures
import json
import os
import re
import shutil
import subprocess
import sys
import time
import urllib.request

DEFAULT_MIRRORS: tuple[tuple[str, str], ...] = (
    ("pypi", "https://pypi.org/simple"),
    ("tuna", "https://pypi.tuna.tsinghua.edu.cn/simple"),
    ("aliyun", "https://mirrors.aliyun.com/pypi/simple"),
    ("ustc", "https://mirrors.ustc.edu.cn/pypi/simple"),
)
MIRROR_CACHE_TTL_SECS = 86_400


def resolve_runtime_paths(home_dir: str) -> tuple[str, str, str]:
    venv_dir = os.environ.get("CCIDE_PYENV_DIR", "").strip() or os.path.join(
        home_dir, ".local", "share", "cc-ide", "pyenv"
    )
    cache_dir = os.environ.get("CCIDE_CACHE_DIR", "").strip() or os.path.join(
        home_dir, ".cache", "cc-ide"
    )
    return (
        venv_dir,
        os.path.join(cache_dir, "python_bin"),
        os.path.join(cache_dir, "pypi_mirror.json"),
    )


def can_import_iterm2(py_exe: str) -> bool:
    if not py_exe or not os.path.isfile(py_exe) or not os.access(py_exe, os.X_OK):
        return False
    try:
        res = subprocess.run(
            [py_exe, "-I", "-c", "import iterm2"],
            capture_output=True,
            timeout=5.0,
            check=False,
        )
        return res.returncode == 0
    except Exception:
        return False


def detect_user_pip_index(base_py: str) -> str | None:
    env_idx = os.environ.get("PIP_INDEX_URL", "").strip()
    if env_idx:
        return env_idx
    try:
        proc = subprocess.run(
            [base_py, "-I", "-m", "pip", "config", "list"],
            capture_output=True,
            text=True,
            timeout=3.0,
            check=False,
        )
        for line in proc.stdout.splitlines():
            m = re.match(r"^(?:global|install)\.index-url\s*=\s*['\"]?([^'\"]+)['\"]?$", line.strip())
            if m and m.group(1).strip():
                return m.group(1).strip()
    except Exception:
        pass
    return None


def normalize_probe_url(index_url: str) -> str:
    clean = index_url.rstrip("/")
    if not clean.endswith("/simple"):
        clean = clean + "/simple"
    return f"{clean}/iterm2/"


def _probe_one_mirror(name: str, index_url: str, timeout_s: float) -> dict:
    probe_url = normalize_probe_url(index_url)
    t0 = time.monotonic()
    req = urllib.request.Request(probe_url, headers={"User-Agent": "ccide-installer/0.5"})
    try:
        with urllib.request.urlopen(req, timeout=timeout_s) as resp:
            status = getattr(resp, "status", resp.getcode())
            _ = resp.read(512)
        dt_ms = round((time.monotonic() - t0) * 1000.0, 1)
        return {"name": name, "index_url": index_url, "ok": status == 200, "status": status, "latency_ms": dt_ms}
    except Exception as exc:
        closer = getattr(exc, "close", None)
        if callable(closer):
            try:
                closer()
            except Exception:
                pass
        dt_ms = round((time.monotonic() - t0) * 1000.0, 1)
        return {"name": name, "index_url": index_url, "ok": False, "error": str(exc), "latency_ms": dt_ms}


def select_pypi_index(
    base_py: str,
    mirror_cache_path: str,
    mirrors: list[tuple[str, str]] | None = None,
    timeout_s: float = 6.0,
) -> tuple[str | None, str]:
    user_idx = detect_user_pip_index(base_py)
    if user_idx:
        return user_idx, "user-configured"

    now_s = int(time.time())
    if os.path.isfile(mirror_cache_path):
        try:
            with open(mirror_cache_path, encoding="utf-8") as f:
                cached = json.load(f)
            if (
                isinstance(cached, dict)
                and cached.get("index_url")
                and (now_s - int(cached.get("probed_at", 0))) < MIRROR_CACHE_TTL_SECS
            ):
                return str(cached["index_url"]), "cached"
        except Exception:
            pass

    cand_list = list(mirrors) if mirrors is not None else list(DEFAULT_MIRRORS)
    extra = os.environ.get("CCIDE_EXTRA_PIP_INDEX", "").strip()
    if extra:
        cand_list.append(("extra", extra))

    results: list[dict] = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, len(cand_list))) as pool:
        futs = [pool.submit(_probe_one_mirror, nm, url, timeout_s) for nm, url in cand_list]
        for fut in concurrent.futures.as_completed(futs):
            results.append(fut.result())

    reachable = [r for r in results if r.get("ok")]
    reachable.sort(key=lambda r: float(r.get("latency_ms", 999_999.0)))
    if not reachable:
        return None, "unreachable"

    winner = reachable[0]
    os.makedirs(os.path.dirname(mirror_cache_path), exist_ok=True)
    tmp = mirror_cache_path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(
            {
                "index_url": winner["index_url"],
                "mirror_name": winner["name"],
                "latency_ms": winner["latency_ms"],
                "probed_at": now_s,
                "results": results,
            },
            f,
            indent=2,
        )
    os.replace(tmp, mirror_cache_path)
    return str(winner["index_url"]), f"probed ({winner['name']} {winner['latency_ms']}ms)"


def write_python_bin_record(record_path: str, py_exe: str) -> None:
    os.makedirs(os.path.dirname(record_path), exist_ok=True)
    tmp = record_path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        f.write(py_exe + "\n")
    os.replace(tmp, record_path)


def print_offline_fallback(venv_dir: str, index_hint: str) -> None:
    idx = index_hint or "https://pypi.tuna.tsinghua.edu.cn/simple"
    print(
        "  [WARN] Could not auto-install `iterm2` Python package (network/venv unavailable).\n"
        "         Tier 1/2 native iTerm2 splits disabled for now; falling back to Tier 3 (`tmux`).\n"
        "         To enable Tier 1/2 manually once online, paste any ONE of the following:\n"
        f"           1) python3 -m venv {venv_dir} && {venv_dir}/bin/python3 -m pip install -i {idx} iterm2\n"
        f"           2) python3 -m pip install --user --break-system-packages -i {idx} iterm2\n"
        f"           3) {venv_dir}/bin/python3 -m pip install --no-index --find-links=/path/to/wheels iterm2"
    )


def ensure_iterm_pyenv(home_dir: str, mirrors: list[tuple[str, str]] | None = None) -> int:
    venv_dir, py_record, mirror_cache = resolve_runtime_paths(home_dir)
    venv_py = os.path.join(venv_dir, "bin", "python3")

    if can_import_iterm2(venv_py):
        write_python_bin_record(py_record, venv_py)
        print(f"  [OK] Dedicated iTerm2 Python runtime ready ({venv_py}, cached)")
        return 0

    for existing_cand in (
        os.path.join(home_dir, "miniconda3", "bin", "python3"),
        sys.executable,
    ):
        if os.environ.get("CCIDE_FORCE_VENV", "") != "1" and can_import_iterm2(existing_cand):
            write_python_bin_record(py_record, existing_cand)
            print(f"  [OK] Reusing Python runtime with `iterm2` pre-installed ({existing_cand})")
            return 0

    base_py = shutil.which("python3") or sys.executable or "/usr/bin/python3"
    chosen_idx, source_desc = select_pypi_index(base_py, mirror_cache, mirrors=mirrors)
    if not chosen_idx:
        print_offline_fallback(venv_dir, "")
        return 0

    print(f"  [INFO] Selected PyPI index ({source_desc}): {chosen_idx}")
    try:
        os.makedirs(os.path.dirname(venv_dir), exist_ok=True)
        if not os.path.isfile(venv_py):
            subprocess.run([base_py, "-m", "venv", venv_dir], check=True, timeout=45.0)
        subprocess.run(
            [
                venv_py,
                "-m",
                "pip",
                "install",
                "--disable-pip-version-check",
                "--quiet",
                "-i",
                chosen_idx,
                "iterm2",
            ],
            check=True,
            timeout=120.0,
        )
        if can_import_iterm2(venv_py):
            write_python_bin_record(py_record, venv_py)
            print(f"  [OK] Installed `iterm2` into isolated venv -> {venv_py}")
            return 0
    except Exception as exc:
        print(f"  [WARN] Venv `iterm2` provisioning encountered an error: {exc}")

    print_offline_fallback(venv_dir, chosen_idx)
    return 0


def main() -> None:
    ap = argparse.ArgumentParser(description="cc-ide Isolated iTerm2 Python Environment Manager")
    ap.add_argument("--ensure", action="store_true", help="Ensure venv + `iterm2` package are ready")
    ap.add_argument("--status", action="store_true", help="Inspect venv & python_bin resolution")
    ap.add_argument("--json", action="store_true", help="Print status as JSON")
    args = ap.parse_args()

    home_dir = os.environ.get("HOME", os.path.expanduser("~"))
    if args.ensure:
        sys.exit(ensure_iterm_pyenv(home_dir))

    venv_dir, py_record, mirror_cache = resolve_runtime_paths(home_dir)
    venv_py = os.path.join(venv_dir, "bin", "python3")
    rec_val = ""
    if os.path.isfile(py_record):
        with open(py_record, encoding="utf-8") as f:
            rec_val = f.read().strip()
    payload = {
        "venv_python": venv_py,
        "venv_ready": can_import_iterm2(venv_py),
        "recorded_python": rec_val,
        "recorded_ready": can_import_iterm2(rec_val) if rec_val else False,
        "mirror_cache": mirror_cache,
    }
    if args.json:
        print(json.dumps(payload, indent=2))
    else:
        print(f"venv_python={venv_py} (ready={payload['venv_ready']})")


if __name__ == "__main__":
    main()
