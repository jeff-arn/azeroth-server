"""Client report pairing, ingestion, auth, and inbox route tests."""

from __future__ import annotations

import io
import json
from datetime import datetime, timedelta, timezone

import pytest
from client_reports import (
    ClientReportAuthError,
    ClientReportError,
    ClientReportService,
    compact_pair_code,
    hash_secret,
    validate_report_payload,
)


class FakeRepository:
    def __init__(self):
        self.codes = {}
        self.installs = {}
        self.reports = {}
        self.files = {}

    def count_active_pair_codes(self, account_id, now):
        return sum(
            1
            for row in self.codes.values()
            if row["account_id"] == account_id and not row["used_at"] and row["expires_at"] > now
        )

    def create_pair_code(self, account, code_hash, expires_at, now):
        self.codes[code_hash] = {
            "account_id": account["id"],
            "account_name": account["username"],
            "expires_at": expires_at,
            "used_at": None,
        }

    def exchange_pair_code(self, code_hash, install, now):
        code = self.codes.get(code_hash)
        if not code or code["used_at"] or code["expires_at"] <= now:
            return None
        code["used_at"] = now
        self.installs[install["id"]] = {
            **install,
            "account_id": code["account_id"],
            "account_name": code["account_name"],
            "created_at": now,
            "last_seen_at": now,
            "revoked_at": None,
        }
        return code

    def get_install_by_token_hash(self, token_hash):
        return next(
            (row for row in self.installs.values() if row["token_hash"] == token_hash and not row["revoked_at"]),
            None,
        )

    def touch_install(self, install_id, now):
        self.installs[install_id]["last_seen_at"] = now

    def list_installs(self, account_id):
        return [row for row in self.installs.values() if row["account_id"] == account_id]

    def revoke_install(self, install_id, account_id, is_admin, now):
        row = self.installs.get(install_id)
        if not row or row["revoked_at"] or (not is_admin and row["account_id"] != account_id):
            return False
        row["revoked_at"] = now
        return True

    def find_report_id(self, install_id, client_report_id):
        return next(
            (
                row["id"]
                for row in self.reports.values()
                if row["install_id"] == install_id and row["client_report_id"] == client_report_id
            ),
            None,
        )

    def create_report(self, report, files):
        self.reports[report["id"]] = {**report, "status": "new", "files": files}
        for item in files:
            self.files[(report["id"], item["id"])] = item

    def list_reports(self, status=None, kind=None, limit=100):
        rows = list(self.reports.values())
        if status:
            rows = [row for row in rows if row["status"] == status]
        if kind:
            rows = [row for row in rows if row["kind"] == kind]
        return rows[:limit]

    def get_report(self, report_id):
        return self.reports.get(report_id)

    def set_status(self, report_id, status):
        row = self.reports.get(report_id)
        if not row:
            return False
        row["status"] = status
        return True

    def get_file(self, report_id, file_id):
        return self.files.get((report_id, file_id))


def sample_payload(**overrides):
    payload = {
        "schema_version": 1,
        "client_report_id": "bug-1",
        "kind": "bug",
        "title": "Door stuck at C:\\Users\\alice\\WoW",
        "body": "token=supersecretvalue",
        "severity": "normal",
        "occurred_at": "2026-11-21T12:00:00Z",
        "context": {
            "game": {"character": "Thrall", "realm": "Azeroth"},
            "pack": {"name": "azeroth-client-pack", "version": "0.1.0"},
            "client": {"build": 5875, "executable_sha256": "a" * 64},
        },
    }
    payload.update(overrides)
    return payload


@pytest.fixture
def service(tmp_path):
    repo = FakeRepository()
    return ClientReportService(repo, tmp_path), repo


def paired(service):
    pair = service.create_pair_code({"id": 7, "username": "ALICE"})
    exchanged = service.exchange_pair_code(pair["code"], "Gaming PC")
    return service.authenticate(exchanged["token"]), exchanged


def test_pair_code_creation_is_bounded(service):
    service, _ = service
    account = {"id": 7, "username": "ALICE"}
    for _ in range(5):
        service.create_pair_code(account)
    with pytest.raises(ClientReportError):
        service.create_pair_code(account)


def test_pair_code_is_one_time_and_token_is_hashed(service):
    service, repo = service
    install, exchanged = paired(service)
    assert install["account_name"] == "ALICE"
    assert repo.installs[install["id"]]["token_hash"] == hash_secret(exchanged["token"])
    assert exchanged["token"] not in str(repo.installs)
    with pytest.raises(ClientReportAuthError):
        service.exchange_pair_code("AAAAAA-AAAAAA", "Other")


def test_submit_redacts_and_deduplicates(service, tmp_path):
    service, _ = service
    install, _ = paired(service)
    result, created = service.submit_report(
        install,
        sample_payload(),
        [("error.txt", b"path C:\\Users\\alice\\WoW token=abc123456", "text/plain")],
    )
    assert created is True
    duplicate, created_again = service.submit_report(install, sample_payload(), [])
    assert created_again is False
    assert duplicate["id"] == result["id"]
    stored = service.get_report(result["id"])
    assert "alice" not in stored["title"].lower()
    assert "supersecretvalue" not in stored["body"]
    path, row = service.file_path(result["id"], stored["files"][0]["id"])
    assert "abc123456" not in path.read_text()
    assert path.parent == (tmp_path / result["id"])


def test_validate_report_rejects_bad_shapes_and_files(service):
    service, _ = service
    install, _ = paired(service)
    with pytest.raises(ClientReportError):
        validate_report_payload(sample_payload(kind="surprise"))
    with pytest.raises(ClientReportError):
        service.submit_report(install, sample_payload(client_report_id="bug-2"), [("dump.dmp", b"x", "")])


@pytest.fixture
def app(monkeypatch):
    import sys

    sys.modules.pop("dashboard_app", None)
    import importlib.util
    from pathlib import Path

    src = Path(__file__).resolve().parent.parent / "dashboard.py"
    spec = importlib.util.spec_from_file_location("dashboard_app", src)
    mod = importlib.util.module_from_spec(spec)
    sys.modules["dashboard_app"] = mod
    spec.loader.exec_module(mod)
    mod.app.config["TESTING"] = True
    return mod


class RouteFakeService:
    def __init__(self, tmp_path):
        self.tmp_path = tmp_path
        self.report = {
            "id": "report-1",
            "client_report_id": "bug-1",
            "install_id": "install-1",
            "account_id": 7,
            "account_name": "ALICE",
            "kind": "bug",
            "title": "<script>alert(1)</script>",
            "body": "details",
            "severity": "normal",
            "status": "new",
            "occurred_at": "2026-11-21T12:00:00Z",
            "received_at": "2026-11-21T12:01:00Z",
            "character_name": "Thrall",
            "realm_name": "Azeroth",
            "pack_name": "azeroth-client-pack",
            "pack_version": "0.1.0",
            "client_build": 5875,
            "client_sha256": "a" * 64,
            "context": {"game": {"character": "Thrall"}},
            "files": [
                {
                    "id": "file-1",
                    "filename": "error.txt",
                    "content_type": "text/plain",
                    "size_bytes": 5,
                    "sha256": "b" * 64,
                }
            ],
        }

    def create_pair_code(self, account):
        return {"code": "ABCDEF-GHIJKL", "expires_at": "2026-11-21T12:10:00Z"}

    def list_installs(self, account_id):
        return []

    def list_reports(self, status=None, kind=None, limit=100):
        return [{**self.report}]

    def get_report(self, report_id):
        return dict(self.report)

    def set_status(self, report_id, status):
        self.report["status"] = status
        return dict(self.report)

    def exchange_pair_code(self, code, label):
        return {"install_id": "install-1", "token": "token", "account_id": 7, "account_name": "ALICE"}

    def authenticate(self, token):
        if token != "good-token":
            raise ClientReportAuthError()
        return {"id": "install-1", "account_id": 7, "account_name": "ALICE"}

    def ping(self, install):
        return {"ok": True, "install_id": install["id"], "account_name": "ALICE"}

    def submit_report(self, install, payload, files):
        self.last_files = files
        return {"id": self.report["id"], "client_report_id": payload["client_report_id"], "duplicate": False}, True

    def file_path(self, report_id, file_id):
        path = self.tmp_path / "error.txt"
        path.write_text("hello")
        return path, self.report["files"][0]


@pytest.fixture
def route_client(app, tmp_path, monkeypatch):
    fake = RouteFakeService(tmp_path)
    monkeypatch.setattr(app, "client_report_service", fake)
    return app.app.test_client(), fake


def login(client, gmlevel=0):
    with client.session_transaction() as session:
        session["account_id"] = 7
        session["username"] = "ALICE"
        session["gmlevel"] = gmlevel
        session["char_name"] = "Thrall"
        session["client_reports_csrf"] = "test-csrf"


def test_pair_page_and_inbox_escape_report_titles(route_client):
    client, _ = route_client
    login(client, gmlevel=2)
    blocked = client.post("/client-reports/pair")
    assert b"form expired" in blocked.data.lower()
    pair = client.post("/client-reports/pair", data={"csrf_token": "test-csrf"})
    assert pair.status_code == 200
    assert b"ABCDEF-GHIJKL" in pair.data
    assert b"&lt;script&gt;alert(1)&lt;/script&gt;" in pair.data
    assert b"<script>alert(1)</script>" not in pair.data


def test_submit_requires_valid_bearer_token(route_client):
    client, _ = route_client
    payload = sample_payload()
    data = {
        "report": json.dumps(payload),
        "files": (io.BytesIO(b"error"), "error.txt"),
    }
    rejected = client.post("/api/client-reports", data=data, content_type="multipart/form-data")
    assert rejected.status_code == 401
    data = {
        "report": json.dumps(payload),
        "files": (io.BytesIO(b"error"), "error.txt"),
    }
    accepted = client.post(
        "/api/client-reports",
        data=data,
        content_type="multipart/form-data",
        headers={"Authorization": "Bearer good-token"},
    )
    assert accepted.status_code == 201


def test_admin_routes_reject_regular_players(route_client):
    client, _ = route_client
    login(client, gmlevel=0)
    assert client.get("/api/client-reports").status_code == 403
    assert client.get("/client-reports/report-1").status_code == 403
    login(client, gmlevel=2)
    assert client.get("/api/client-reports").status_code == 200
    detail = client.get("/client-reports/report-1")
    assert detail.status_code == 200
    assert b"Download" in detail.data
    download = client.get("/client-reports/report-1/files/file-1")
    assert download.data == b"hello"


def test_pair_code_expiry_window_is_bounded():
    now = datetime.now(timezone.utc)
    assert now + timedelta(minutes=10) > now
    assert len(compact_pair_code("abcd-efgh-ijkl")) == 12
