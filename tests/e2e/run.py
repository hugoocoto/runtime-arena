#!/usr/bin/env python3
"""Plays a short contest against a running server with the real judge.

Usage: run.py SERVER_URL ADMIN_TOKEN JOIN_CODE (see scripts/e2e.sh).
"""
import io
import json
import sys
import tarfile
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PROBLEM = ROOT / "crates/judge/tests/fixtures/sum"
PROGRAMS = ROOT / "tests/sandbox/out"
EXPECTED = {"ok": "accepted", "wa": "wrong_answer", "re": "runtime_error"}
DEADLINE_S = 30


def request(method, url, token=None, body=None, content_type=None):
    headers = {}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    if content_type:
        headers["Content-Type"] = content_type
    req = urllib.request.Request(url, data=body, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=10) as response:
            return response.read()
    except urllib.error.HTTPError as e:
        sys.exit(f"{method} {url}: {e.code} {e.read().decode(errors='replace')}")


def post_json(url, data, token=None):
    body = json.dumps(data).encode()
    return json.loads(request("POST", url, token, body, "application/json"))


def wait_healthy(server):
    deadline = time.monotonic() + DEADLINE_S
    while time.monotonic() < deadline:
        try:
            with urllib.request.urlopen(f"{server}/healthz", timeout=2) as response:
                if response.status == 200:
                    return
        except OSError:
            pass
        time.sleep(0.2)
    sys.exit("the server did not become healthy")


def problem_archive():
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w:gz") as tar:
        for path in sorted(PROBLEM.rglob("*")):
            if path.is_file():
                tar.add(path, arcname=path.relative_to(PROBLEM).as_posix())
    return buffer.getvalue()


def multipart(fields):
    boundary = uuid.uuid4().hex
    body = io.BytesIO()
    for name, value in fields.items():
        body.write(f'--{boundary}\r\nContent-Disposition: form-data; name="{name}"; '.encode())
        body.write(f'filename="{name}"\r\n\r\n'.encode())
        body.write(value)
        body.write(b"\r\n")
    body.write(f"--{boundary}--\r\n".encode())
    return body.getvalue(), f"multipart/form-data; boundary={boundary}"


def submit(server, token, program):
    body, content_type = multipart(
        {
            "problem": b"sum",
            "executable": (PROGRAMS / program).read_bytes(),
            "source": b"end-to-end test",
        }
    )
    created = json.loads(request("POST", f"{server}/api/submissions", token, body, content_type))
    return created["id"]


def wait_judged(server, token, submission):
    deadline = time.monotonic() + DEADLINE_S
    while time.monotonic() < deadline:
        info = json.loads(request("GET", f"{server}/api/submissions/{submission}", token))
        if info["status"] in ("done", "failed"):
            return info
        time.sleep(0.1)
    sys.exit(f"submission {submission} was not judged in {DEADLINE_S} s")


def main():
    server, admin, join_code = sys.argv[1:4]
    wait_healthy(server)

    request("PUT", f"{server}/api/admin/problems/sum", admin, problem_archive())
    credentials = {"name": "e2e-player", "password": "e2e-password"}
    post_json(f"{server}/api/signup", {**credentials, "display_name": None, "join_code": join_code})
    token = post_json(f"{server}/api/login", credentials)["token"]
    round_id = post_json(
        f"{server}/api/admin/rounds",
        {"name": "e2e", "duration_minutes": 5, "cooldown_seconds": 0, "problems": ["sum"]},
        admin,
    )["id"]
    request("POST", f"{server}/api/admin/rounds/{round_id}/start", admin)

    failed = False
    for program, expected in EXPECTED.items():
        verdict = wait_judged(server, token, submit(server, token, program))["verdict"]
        good = verdict == expected
        failed |= not good
        print(f"{'ok  ' if good else 'FAIL'}  {program}: {verdict} (expected {expected})")

    boards = request("GET", f"{server}/api/rounds/{round_id}/boards", token).decode()
    on_board = credentials["name"] in boards
    failed |= not on_board
    print(f"{'ok  ' if on_board else 'FAIL'}  the accepted solution is on the leaderboard")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
