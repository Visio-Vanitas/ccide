#!/usr/bin/env python3
"""Local HTTP/SSE Workflow API Server (stdlib ThreadingHTTPServer, <= 280 lines)."""

import argparse
import json
import os
import sys
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ccide_workflow_runner as runner  # type: ignore[import-not-found]  # noqa: E402


def _extract_prompt(messages) -> str:
    if not isinstance(messages, list):
        return ""
    for msg in reversed(messages):
        if isinstance(msg, dict) and msg.get("role") == "user":
            c = msg.get("content")
            if isinstance(c, str):
                return c
            if isinstance(c, list):
                parts = [b.get("text", "") for b in c if isinstance(b, dict) and b.get("type") == "text"]
                return "\n".join(p for p in parts if p)
    return ""


def _format_event_text(ev: dict) -> str:
    kind, data = ev.get("event"), ev.get("data") or {}
    if kind == "workflow_result":
        res = data.get("result")
        if isinstance(res, str):
            return res
        return json.dumps(res if res is not None else data)
    if kind == "workflow_phase":
        return f"[phase] {data.get('title') or data.get('phase') or ''}\n"
    if kind == "workflow_log":
        return f"[log] {data.get('message', '')}\n"
    if kind == "workflow_agent":
        return f"[agent] {data.get('label') or data.get('status') or ''}\n"
    return ""


class WorkflowHandler(BaseHTTPRequestHandler):
    server_version = "ccide-workflow-server/0.7.0"

    def log_message(self, format: str, *args):  # noqa: A002
        if os.environ.get("CCIDE_WORKFLOW_SERVER_QUIET") != "1":
            super().log_message(format, *args)

    def _check_auth(self) -> bool:
        tok = getattr(self.server, "bearer_token", None)
        if not tok:
            return True
        auth = self.headers.get("Authorization") or ""
        if auth == f"Bearer {tok}":
            return True
        self._send_json(401, {"error": "unauthorized"})
        return False

    def _read_json(self) -> dict:
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length) if length > 0 else b"{}"
        return json.loads(raw.decode("utf-8"))

    def _send_json(self, code: int, payload: dict):
        body = json.dumps(payload).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _begin_sse(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "close")
        self.end_headers()

    def _write_sse(self, event_name: str, data):
        payload = data if isinstance(data, str) else json.dumps(data)
        chunk = f"event: {event_name}\ndata: {payload}\n\n" if event_name else f"data: {payload}\n\n"
        self.wfile.write(chunk.encode("utf-8"))
        self.wfile.flush()

    def do_GET(self):
        if not self._check_auth():
            return
        path = self.path.split("?", 1)[0]
        if path == "/health":
            self._send_json(200, {"ok": True, "service": "ccide-workflow-server"})
        elif path == "/v1/workflows":
            self._send_json(200, runner.build_catalog())
        else:
            self._send_json(404, {"error": f"unknown path: {path}"})

    def do_POST(self):
        if not self._check_auth():
            return
        path = self.path.split("?", 1)[0]
        try:
            body = self._read_json()
        except Exception as exc:
            self._send_json(400, {"error": f"invalid json: {exc}"})
            return

        if path == "/v1/workflows/run":
            self._handle_workflows_run(body)
        elif path == "/v1/messages":
            self._handle_messages(body)
        elif path == "/v1/chat/completions":
            self._handle_chat_completions(body)
        else:
            self._send_json(404, {"error": f"unknown path: {path}"})

    def _handle_workflows_run(self, body: dict):
        wf = body.get("workflow")
        script = body.get("script")
        if not wf and not script:
            self._send_json(400, {"error": "workflow or script is required"})
            return
        gen = runner.execute_workflow_stream(
            workflow=wf,
            script=script,
            args=body.get("args"),
            preset=body.get("preset"),
            roles=body.get("roles"),
            resume_from_run_id=body.get("resume_from_run_id"),
            cwd=body.get("cwd"),
        )
        try:
            if body.get("stream"):
                self._begin_sse()
                for ev in gen:
                    self._write_sse(ev["event"], ev["data"])
            else:
                events, final = [], {}
                for ev in gen:
                    events.append(ev)
                    if ev["event"] == "workflow_result":
                        final = ev["data"]
                self._send_json(200, {**final, "events": events})
        except FileNotFoundError as exc:
            self._send_json(404, {"error": str(exc)})
        except Exception as exc:
            if not body.get("stream"):
                self._send_json(500, {"error": str(exc)})

    def _prepare_model_workflow(self, body: dict):
        model = body.get("model") or ""
        if not isinstance(model, str) or not model.startswith("workflow/"):
            self._send_json(400, {"error": "model must start with 'workflow/'"})
            return None
        wf_name = model[len("workflow/") :]
        prompt = _extract_prompt(body.get("messages"))
        wf_args = {}
        if isinstance(body.get("metadata"), dict):
            wf_args.update(body["metadata"])
        if prompt:
            wf_args["prompt"] = prompt
        gen = runner.execute_workflow_stream(
            workflow=wf_name,
            args=wf_args,
            preset=wf_args.get("preset"),
            roles=wf_args.get("roles"),
            cwd=wf_args.get("cwd"),
        )
        return model, gen

    def _handle_messages(self, body: dict):
        prep = self._prepare_model_workflow(body)
        if not prep:
            return
        model, gen = prep
        msg_id = f"msg_{uuid.uuid4().hex[:16]}"
        try:
            if body.get("stream"):
                self._begin_sse()
                self._write_sse("message_start", {"type": "message_start", "message": {"id": msg_id, "type": "message", "role": "assistant", "model": model, "content": [], "stop_reason": None}})
                self._write_sse("content_block_start", {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}})
                for ev in gen:
                    txt = _format_event_text(ev)
                    if txt:
                        self._write_sse("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": txt}})
                self._write_sse("content_block_stop", {"type": "content_block_stop", "index": 0})
                self._write_sse("message_delta", {"type": "message_delta", "delta": {"stop_reason": "end_turn"}})
                self._write_sse("message_stop", {"type": "message_stop"})
            else:
                final_txt = ""
                for ev in gen:
                    if ev["event"] == "workflow_result":
                        final_txt = _format_event_text(ev)
                self._send_json(200, {"id": msg_id, "type": "message", "role": "assistant", "model": model, "content": [{"type": "text", "text": final_txt}], "stop_reason": "end_turn"})
        except FileNotFoundError as exc:
            self._send_json(404, {"error": str(exc)})

    def _handle_chat_completions(self, body: dict):
        prep = self._prepare_model_workflow(body)
        if not prep:
            return
        model, gen = prep
        chat_id = f"chatcmpl-{uuid.uuid4().hex[:16]}"
        try:
            if body.get("stream"):
                self._begin_sse()
                self._write_sse("", {"id": chat_id, "object": "chat.completion.chunk", "model": model, "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": None}]})
                for ev in gen:
                    txt = _format_event_text(ev)
                    if txt:
                        self._write_sse("", {"id": chat_id, "object": "chat.completion.chunk", "model": model, "choices": [{"index": 0, "delta": {"content": txt}, "finish_reason": None}]})
                self._write_sse("", {"id": chat_id, "object": "chat.completion.chunk", "model": model, "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]})
                self._write_sse("", "[DONE]")
            else:
                final_txt = ""
                for ev in gen:
                    if ev["event"] == "workflow_result":
                        final_txt = _format_event_text(ev)
                self._send_json(200, {"id": chat_id, "object": "chat.completion", "model": model, "choices": [{"index": 0, "message": {"role": "assistant", "content": final_txt}, "finish_reason": "stop"}]})
        except FileNotFoundError as exc:
            self._send_json(404, {"error": str(exc)})


def create_server(host: str = "127.0.0.1", port: int = 18099, token: str | None = None) -> ThreadingHTTPServer:
    srv = ThreadingHTTPServer((host, port), WorkflowHandler)
    srv.bearer_token = token  # type: ignore[attr-defined]
    return srv


def main(argv=None) -> int:
    p = argparse.ArgumentParser(prog="ccide workflow serve", description="Local HTTP/SSE Workflow API Server")
    p.add_argument("--host", default="127.0.0.1")
    p.add_argument("--port", type=int, default=18099)
    p.add_argument("--token", default=os.environ.get("CCIDE_WORKFLOW_TOKEN"))
    ns = p.parse_args(argv)
    srv = create_server(ns.host, ns.port, ns.token)
    print(f"ccide-workflow-server listening on http://{ns.host}:{srv.server_address[1]}", flush=True)
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        srv.server_close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
