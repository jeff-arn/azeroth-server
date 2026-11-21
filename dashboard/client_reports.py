"""Client bug/crash report pairing, validation, storage, and queries."""

from __future__ import annotations

import hashlib
import json
import re
import secrets
import uuid
from datetime import datetime, timedelta, timezone
from pathlib import Path

SCHEMA_VERSION = 1
PAIR_CODE_TTL_SECONDS = 600
MAX_TITLE_CHARS = 200
MAX_BODY_CHARS = 10000
MAX_CONTEXT_BYTES = 64 * 1024
MAX_FILE_BYTES = 1024 * 1024
MAX_TOTAL_FILE_BYTES = 4 * 1024 * 1024
ALLOWED_KINDS = {"bug", "lua_error", "crash"}
ALLOWED_SEVERITIES = {"low", "normal", "high", "critical"}
ALLOWED_STATUSES = {"new", "triaged", "resolved", "closed"}
ALLOWED_SUFFIXES = {".txt", ".log", ".json"}
_REPORT_ID_RE = re.compile(r"^[A-Za-z0-9_.:-]{1,128}$")
_CODE_ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789"
_SECRET_PATTERNS = (
    re.compile(r"(?i)(bearer\s+)[A-Za-z0-9._~+/=-]{8,}"),
    re.compile(r"(?i)((?:password|passwd|token|secret|api[_-]?key)\s*[=:]\s*)[^\s,;]+"),
    re.compile(r"/Users/[^/\s]+"),
    re.compile(r"(?i)[A-Z]:\\Users\\[^\\/\s]+"),
)


class ClientReportError(ValueError):
    def __init__(self, message, status=400):
        super().__init__(message)
        self.status = status


class ClientReportAuthError(ClientReportError):
    def __init__(self, message="Invalid or revoked install token"):
        super().__init__(message, status=401)


class ClientReportNotFound(ClientReportError):
    def __init__(self, message="Client report not found"):
        super().__init__(message, status=404)


def utc_now():
    return datetime.now(timezone.utc)


def format_dt(value):
    return value.astimezone(timezone.utc).strftime("%Y-%m-%d %H:%M:%S")


def iso_z(value):
    if value is None:
        return None
    if isinstance(value, str):
        return value
    if value.tzinfo is None:
        value = value.replace(tzinfo=timezone.utc)
    return value.astimezone(timezone.utc).isoformat().replace("+00:00", "Z")


def hash_secret(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def generate_pair_code() -> str:
    raw = "".join(secrets.choice(_CODE_ALPHABET) for _ in range(12))
    return raw[:6] + "-" + raw[6:]


def compact_pair_code(code: str) -> str:
    return re.sub(r"[^A-Za-z0-9]", "", code or "").upper()


def redact_value(value):
    if isinstance(value, str):
        out = value
        for pattern in _SECRET_PATTERNS:
            if pattern.groups:
                out = pattern.sub(lambda match: match.group(1) + "<redacted>", out)
            else:
                out = pattern.sub("<path>", out)
        return out
    if isinstance(value, list):
        return [redact_value(item) for item in value]
    if isinstance(value, dict):
        return {key: redact_value(item) for key, item in value.items()}
    return value


def parse_occurred_at(raw):
    if isinstance(raw, (int, float)) and raw > 0:
        return datetime.fromtimestamp(raw, tz=timezone.utc)
    if not isinstance(raw, str) or not raw.strip():
        raise ClientReportError("occurred_at is required")
    text = raw.strip()
    try:
        parsed = datetime.fromisoformat(text.replace("Z", "+00:00"))
    except ValueError:
        raise ClientReportError("occurred_at must be an ISO-8601 timestamp") from None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


def validate_report_payload(payload):
    if not isinstance(payload, dict):
        raise ClientReportError("report must be a JSON object")
    if payload.get("schema_version") != SCHEMA_VERSION:
        raise ClientReportError("unsupported report schema_version")
    client_report_id = str(payload.get("client_report_id") or "")
    if not _REPORT_ID_RE.match(client_report_id):
        raise ClientReportError("client_report_id is invalid")
    kind = str(payload.get("kind") or "")
    if kind not in ALLOWED_KINDS:
        raise ClientReportError("kind must be one of: bug, lua_error, crash")
    title = str(payload.get("title") or "").strip()
    if not title:
        raise ClientReportError("title is required")
    if len(title) > MAX_TITLE_CHARS:
        raise ClientReportError("title exceeds %d characters" % MAX_TITLE_CHARS)
    body = str(payload.get("body") or "")
    if len(body) > MAX_BODY_CHARS:
        raise ClientReportError("body exceeds %d characters" % MAX_BODY_CHARS)
    severity = str(payload.get("severity") or "normal")
    if severity not in ALLOWED_SEVERITIES:
        raise ClientReportError("severity is invalid")
    context = payload.get("context") or {}
    if not isinstance(context, dict):
        raise ClientReportError("context must be an object")
    context = redact_value(context)
    context_json = json.dumps(context, ensure_ascii=False, separators=(",", ":"))
    if len(context_json.encode("utf-8")) > MAX_CONTEXT_BYTES:
        raise ClientReportError("context exceeds the size limit")
    game = context.get("game") or {}
    pack = context.get("pack") or {}
    client = context.get("client") or {}
    return {
        "client_report_id": client_report_id,
        "kind": kind,
        "title": redact_value(title),
        "body": redact_value(body),
        "severity": severity,
        "occurred_at": parse_occurred_at(payload.get("occurred_at")),
        "character_name": str(game.get("character") or "")[:64] or None,
        "realm_name": str(game.get("realm") or "")[:64] or None,
        "pack_name": str(pack.get("name") or "")[:64] or None,
        "pack_version": str(pack.get("version") or "")[:32] or None,
        "client_build": client.get("build") if isinstance(client.get("build"), int) else None,
        "client_sha256": str(client.get("executable_sha256") or "")[:64] or None,
        "context": context,
        "context_json": context_json,
    }


class MySQLClientReportRepository:
    """MySQL persistence for the small client-report tables in `logs`."""

    def __init__(self, connect):
        self._connect = connect

    def _connection(self):
        return self._connect("logs")

    def count_active_pair_codes(self, account_id, now):
        conn = self._connection()
        try:
            cursor = conn.cursor()
            cursor.execute(
                """
                SELECT COUNT(*) FROM client_report_pair_codes
                WHERE account_id = %s AND used_at IS NULL AND expires_at > %s
                """,
                (account_id, format_dt(now)),
            )
            row = cursor.fetchone()
            cursor.close()
            return int(row[0]) if row else 0
        finally:
            conn.close()

    def create_pair_code(self, account, code_hash, expires_at, now):
        conn = self._connection()
        try:
            cursor = conn.cursor()
            cursor.execute(
                """
                INSERT INTO client_report_pair_codes
                    (code_hash, account_id, account_name, expires_at, created_at)
                VALUES (%s, %s, %s, %s, %s)
                """,
                (code_hash, account["id"], account["username"], format_dt(expires_at), format_dt(now)),
            )
            conn.commit()
            cursor.close()
        finally:
            conn.close()

    def exchange_pair_code(self, code_hash, install, now):
        conn = self._connection()
        try:
            cursor = conn.cursor(dictionary=True)
            cursor.execute(
                "SELECT * FROM client_report_pair_codes WHERE code_hash = %s FOR UPDATE",
                (code_hash,),
            )
            code = cursor.fetchone()
            if not code:
                conn.rollback()
                return None
            expires = code["expires_at"]
            if isinstance(expires, str):
                expires = datetime.fromisoformat(expires).replace(tzinfo=timezone.utc)
            elif expires.tzinfo is None:
                expires = expires.replace(tzinfo=timezone.utc)
            if code.get("used_at") or expires <= now:
                conn.rollback()
                return None
            cursor.execute(
                "UPDATE client_report_pair_codes SET used_at = %s WHERE code_hash = %s",
                (format_dt(now), code_hash),
            )
            cursor.execute(
                """
                INSERT INTO client_report_installs
                    (id, account_id, account_name, token_hash, label, created_at, last_seen_at)
                VALUES (%s, %s, %s, %s, %s, %s, %s)
                """,
                (
                    install["id"],
                    code["account_id"],
                    code["account_name"],
                    install["token_hash"],
                    install.get("label"),
                    format_dt(now),
                    format_dt(now),
                ),
            )
            conn.commit()
            cursor.close()
            return code
        finally:
            conn.close()

    def get_install_by_token_hash(self, token_hash):
        conn = self._connection()
        try:
            cursor = conn.cursor(dictionary=True)
            cursor.execute(
                "SELECT * FROM client_report_installs WHERE token_hash = %s AND revoked_at IS NULL",
                (token_hash,),
            )
            row = cursor.fetchone()
            cursor.close()
            return row
        finally:
            conn.close()

    def touch_install(self, install_id, now):
        conn = self._connection()
        try:
            cursor = conn.cursor()
            cursor.execute(
                "UPDATE client_report_installs SET last_seen_at = %s WHERE id = %s",
                (format_dt(now), install_id),
            )
            conn.commit()
            cursor.close()
        finally:
            conn.close()

    def list_installs(self, account_id):
        conn = self._connection()
        try:
            cursor = conn.cursor(dictionary=True)
            cursor.execute(
                """
                SELECT id, account_id, account_name, label, created_at, last_seen_at, revoked_at
                FROM client_report_installs WHERE account_id = %s ORDER BY created_at DESC
                """,
                (account_id,),
            )
            rows = cursor.fetchall()
            cursor.close()
            return rows
        finally:
            conn.close()

    def revoke_install(self, install_id, account_id, is_admin, now):
        conn = self._connection()
        try:
            cursor = conn.cursor()
            if is_admin:
                cursor.execute(
                    "UPDATE client_report_installs SET revoked_at = %s WHERE id = %s AND revoked_at IS NULL",
                    (format_dt(now), install_id),
                )
            else:
                cursor.execute(
                    """
                    UPDATE client_report_installs SET revoked_at = %s
                    WHERE id = %s AND account_id = %s AND revoked_at IS NULL
                    """,
                    (format_dt(now), install_id, account_id),
                )
            changed = cursor.rowcount
            conn.commit()
            cursor.close()
            return changed > 0
        finally:
            conn.close()

    def find_report_id(self, install_id, client_report_id):
        conn = self._connection()
        try:
            cursor = conn.cursor(dictionary=True)
            cursor.execute(
                "SELECT id FROM client_reports WHERE install_id = %s AND client_report_id = %s",
                (install_id, client_report_id),
            )
            row = cursor.fetchone()
            cursor.close()
            return row["id"] if row else None
        finally:
            conn.close()

    def create_report(self, report, files):
        conn = self._connection()
        try:
            cursor = conn.cursor()
            cursor.execute(
                """
                INSERT INTO client_reports
                    (id, install_id, account_id, account_name, client_report_id, kind, title,
                     body, severity, status, occurred_at, received_at, character_name,
                     realm_name, pack_name, pack_version, client_build, client_sha256, context_json)
                VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, 'new', %s, %s, %s, %s, %s, %s, %s, %s, %s)
                """,
                (
                    report["id"],
                    report["install_id"],
                    report["account_id"],
                    report["account_name"],
                    report["client_report_id"],
                    report["kind"],
                    report["title"],
                    report["body"],
                    report["severity"],
                    format_dt(report["occurred_at"]),
                    format_dt(report["received_at"]),
                    report.get("character_name"),
                    report.get("realm_name"),
                    report.get("pack_name"),
                    report.get("pack_version"),
                    report.get("client_build"),
                    report.get("client_sha256"),
                    report["context_json"],
                ),
            )
            for item in files:
                cursor.execute(
                    """
                    INSERT INTO client_report_files
                        (id, report_id, filename, content_type, size_bytes, sha256, storage_path)
                    VALUES (%s, %s, %s, %s, %s, %s, %s)
                    """,
                    (
                        item["id"],
                        report["id"],
                        item["filename"],
                        item["content_type"],
                        item["size_bytes"],
                        item["sha256"],
                        item["storage_path"],
                    ),
                )
            conn.commit()
            cursor.close()
        except Exception:
            conn.rollback()
            raise
        finally:
            conn.close()

    def list_reports(self, status=None, kind=None, limit=100):
        clauses, params = [], []
        if status:
            clauses.append("status = %s")
            params.append(status)
        if kind:
            clauses.append("kind = %s")
            params.append(kind)
        where = " WHERE " + " AND ".join(clauses) if clauses else ""
        params.append(max(1, min(int(limit), 500)))
        conn = self._connection()
        try:
            cursor = conn.cursor(dictionary=True)
            cursor.execute(
                "SELECT * FROM client_reports" + where + " ORDER BY received_at DESC LIMIT %s",
                tuple(params),
            )
            rows = cursor.fetchall()
            cursor.close()
            return rows
        finally:
            conn.close()

    def get_report(self, report_id):
        conn = self._connection()
        try:
            cursor = conn.cursor(dictionary=True)
            cursor.execute("SELECT * FROM client_reports WHERE id = %s", (report_id,))
            row = cursor.fetchone()
            if not row:
                cursor.close()
                return None
            cursor.execute(
                "SELECT * FROM client_report_files WHERE report_id = %s ORDER BY filename",
                (report_id,),
            )
            row["files"] = cursor.fetchall()
            cursor.close()
            return row
        finally:
            conn.close()

    def set_status(self, report_id, status):
        conn = self._connection()
        try:
            cursor = conn.cursor()
            cursor.execute("UPDATE client_reports SET status = %s WHERE id = %s", (status, report_id))
            changed = cursor.rowcount
            conn.commit()
            cursor.close()
            return changed > 0
        finally:
            conn.close()

    def get_file(self, report_id, file_id):
        conn = self._connection()
        try:
            cursor = conn.cursor(dictionary=True)
            cursor.execute(
                "SELECT * FROM client_report_files WHERE report_id = %s AND id = %s",
                (report_id, file_id),
            )
            row = cursor.fetchone()
            cursor.close()
            return row
        finally:
            conn.close()


def serialize_install(row):
    return {
        "id": row.get("id"),
        "account_id": row.get("account_id"),
        "account_name": row.get("account_name"),
        "label": row.get("label"),
        "created_at": iso_z(row.get("created_at")),
        "last_seen_at": iso_z(row.get("last_seen_at")),
        "revoked_at": iso_z(row.get("revoked_at")),
    }


def serialize_report(row, include_files=True):
    context = {}
    raw_context = row.get("context_json")
    if raw_context:
        try:
            context = json.loads(raw_context)
        except (TypeError, json.JSONDecodeError):
            context = {}
    result = {
        "id": row.get("id"),
        "client_report_id": row.get("client_report_id"),
        "install_id": row.get("install_id"),
        "account_id": row.get("account_id"),
        "account_name": row.get("account_name"),
        "kind": row.get("kind"),
        "title": row.get("title"),
        "body": row.get("body"),
        "severity": row.get("severity"),
        "status": row.get("status"),
        "occurred_at": iso_z(row.get("occurred_at")),
        "received_at": iso_z(row.get("received_at")),
        "character_name": row.get("character_name"),
        "realm_name": row.get("realm_name"),
        "pack_name": row.get("pack_name"),
        "pack_version": row.get("pack_version"),
        "client_build": row.get("client_build"),
        "client_sha256": row.get("client_sha256"),
        "context": context,
    }
    if include_files:
        result["files"] = [
            {
                "id": item.get("id"),
                "filename": item.get("filename"),
                "content_type": item.get("content_type"),
                "size_bytes": item.get("size_bytes"),
                "sha256": item.get("sha256"),
            }
            for item in row.get("files") or []
        ]
    return result


def _validated_files(files):
    validated = []
    total = 0
    for filename, content, content_type in files or []:
        safe_name = Path(str(filename or "attachment.txt")).name
        if Path(safe_name).suffix.lower() not in ALLOWED_SUFFIXES:
            raise ClientReportError("only .txt, .log, and .json report files are accepted")
        content = bytes(content)
        if len(content) > MAX_FILE_BYTES:
            raise ClientReportError("report file exceeds the per-file size limit: %s" % safe_name)
        total += len(content)
        if total > MAX_TOTAL_FILE_BYTES:
            raise ClientReportError("report files exceed the total size limit")
        text = redact_value(content.decode("utf-8", errors="replace"))
        encoded = text.encode("utf-8")
        validated.append(
            {
                "id": str(uuid.uuid4()),
                "filename": safe_name,
                "content_type": str(content_type or "text/plain")[:120],
                "size_bytes": len(encoded),
                "sha256": hashlib.sha256(encoded).hexdigest(),
                "content": encoded,
            }
        )
    return validated


class ClientReportService:
    def __init__(self, repository, storage_root, now_fn=utc_now):
        self.repository = repository
        self.storage_root = Path(storage_root)
        self.now_fn = now_fn

    def create_pair_code(self, account):
        now = self.now_fn()
        if self.repository.count_active_pair_codes(account["id"], now) >= 5:
            raise ClientReportError("Too many unused pairing codes; use or wait for one to expire", status=429)
        code = generate_pair_code()
        expires = now + timedelta(seconds=PAIR_CODE_TTL_SECONDS)
        self.repository.create_pair_code(account, hash_secret(compact_pair_code(code)), expires, now)
        return {"code": code, "expires_at": iso_z(expires)}

    def exchange_pair_code(self, code, label):
        compact = compact_pair_code(code or "")
        if len(compact) != 12:
            raise ClientReportAuthError("Pairing code is invalid or expired")
        token = secrets.token_urlsafe(32)
        install = {
            "id": str(uuid.uuid4()),
            "token_hash": hash_secret(token),
            "label": str(label or "Gaming PC")[:120],
        }
        account = self.repository.exchange_pair_code(hash_secret(compact), install, self.now_fn())
        if not account:
            raise ClientReportAuthError("Pairing code is invalid or expired")
        return {
            "install_id": install["id"],
            "token": token,
            "account_id": account["account_id"],
            "account_name": account["account_name"],
        }

    def authenticate(self, token):
        if not token:
            raise ClientReportAuthError()
        install = self.repository.get_install_by_token_hash(hash_secret(token))
        if not install:
            raise ClientReportAuthError()
        self.repository.touch_install(install["id"], self.now_fn())
        return install

    def list_installs(self, account_id):
        return [serialize_install(row) for row in self.repository.list_installs(account_id)]

    def revoke_install(self, install_id, account_id, is_admin):
        if not self.repository.revoke_install(install_id, account_id, is_admin, self.now_fn()):
            raise ClientReportNotFound("Client install not found")

    def submit_report(self, install, payload, files):
        validated = validate_report_payload(payload)
        existing_id = self.repository.find_report_id(install["id"], validated["client_report_id"])
        if existing_id:
            return {"id": existing_id, "client_report_id": validated["client_report_id"], "duplicate": True}, False
        validated_files = _validated_files(files)
        report_id = str(uuid.uuid4())
        report_dir = self.storage_root / report_id
        report_dir.mkdir(parents=True, exist_ok=False)
        stored_files = []
        try:
            for item in validated_files:
                relative = "%s-%s" % (item["id"], item["filename"])
                (report_dir / relative).write_bytes(item["content"])
                stored = dict(item)
                stored.pop("content", None)
                stored["storage_path"] = relative
                stored_files.append(stored)
            report = dict(validated)
            report.update(
                {
                    "id": report_id,
                    "install_id": install["id"],
                    "account_id": install["account_id"],
                    "account_name": install["account_name"],
                    "received_at": self.now_fn(),
                }
            )
            self.repository.create_report(report, stored_files)
        except Exception:
            # Do not leave orphan report attachments behind on DB failure.
            for child in report_dir.iterdir():
                child.unlink(missing_ok=True)
            report_dir.rmdir()
            existing_id = self.repository.find_report_id(install["id"], validated["client_report_id"])
            if existing_id:
                return {"id": existing_id, "client_report_id": validated["client_report_id"], "duplicate": True}, False
            raise
        return {"id": report_id, "client_report_id": validated["client_report_id"], "duplicate": False}, True

    def list_reports(self, status=None, kind=None, limit=100):
        if status and status not in ALLOWED_STATUSES:
            raise ClientReportError("invalid report status filter")
        if kind and kind not in ALLOWED_KINDS:
            raise ClientReportError("invalid report kind filter")
        try:
            limit = int(limit)
        except (TypeError, ValueError):
            raise ClientReportError("limit must be an integer") from None
        return [serialize_report(row, include_files=False) for row in self.repository.list_reports(status, kind, limit)]

    def get_report(self, report_id):
        row = self.repository.get_report(report_id)
        if not row:
            raise ClientReportNotFound()
        return serialize_report(row)

    def set_status(self, report_id, status):
        if status not in ALLOWED_STATUSES:
            raise ClientReportError("invalid report status")
        if not self.repository.set_status(report_id, status):
            raise ClientReportNotFound()
        return self.get_report(report_id)

    def file_path(self, report_id, file_id):
        row = self.repository.get_file(report_id, file_id)
        if not row:
            raise ClientReportNotFound("Client report file not found")
        candidate = (self.storage_root / report_id / row["storage_path"]).resolve()
        expected_root = (self.storage_root / report_id).resolve()
        if expected_root not in candidate.parents or not candidate.is_file():
            raise ClientReportNotFound("Client report file not found")
        return candidate, row

    def ping(self, install):
        return {"ok": True, "install_id": install["id"], "account_name": install["account_name"]}
