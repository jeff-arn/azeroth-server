"""Shared test setup for the dashboard test suite.

Two things happen here at collection time:

1. The dashboard module reads DB_PASSWORD from the environment at import
   time and raises KeyError if it's missing. Set a dummy value so tests
   can import the module without requiring real DB credentials.

2. The dashboard/ directory and dashboard.py share a name, so the
   directory shadows the file when pytest runs from the repo root.
   Load dashboard.py directly via importlib and register it as
   `dashboard_app` so tests can `from dashboard_app import ...`.
"""

import importlib.util
import os
import sys
from pathlib import Path

os.environ.setdefault("DB_PASSWORD", "test-only-dummy")

if "dashboard_app" not in sys.modules:
    _src = Path(__file__).resolve().parent.parent / "dashboard.py"
    _spec = importlib.util.spec_from_file_location("dashboard_app", _src)
    _module = importlib.util.module_from_spec(_spec)
    sys.modules["dashboard_app"] = _module
    _spec.loader.exec_module(_module)
