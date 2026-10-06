"""Probe contract without installing or initializing a real framework."""
import sys
from types import SimpleNamespace
import pytest
from ubu_planning_worker.main import framework_environment

@pytest.mark.parametrize("version,broken,expected", [("2.6.0+cpu", False, True), ("2.6.0+cpu", True, False), ("unapproved", False, False)])
def test_probe_checks_version_and_cpu_importability(monkeypatch, version, broken, expected):
    def empty(count, *, device):
        assert count == 0 and device == "cpu"
        if broken:
            raise ImportError("synthetic native import failure")
    monkeypatch.setitem(sys.modules, "torch", SimpleNamespace(__version__=version, empty=empty))
    actual = framework_environment()
    assert actual["importable"] is expected
    assert actual["version"] == (None if broken else version)

def test_absent_framework_is_cleanly_unavailable(monkeypatch):
    monkeypatch.setitem(sys.modules, "torch", None)
    assert framework_environment() == {"importable": False, "version": None}
