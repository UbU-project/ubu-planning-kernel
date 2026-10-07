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
    assert framework_environment() == {"importable": False, "version": None, "import_warning_count": 0}

@pytest.mark.parametrize("broken", [False, True])
def test_worker_import_path_counts_warnings_even_on_fallback(monkeypatch, broken):
    import warnings
    def empty(count, *, device):
        warnings.warn("synthetic worker import warning", UserWarning)
        if broken:
            raise ImportError("synthetic native failure")
    monkeypatch.setitem(sys.modules, "torch", SimpleNamespace(__version__="2.6.0+cpu", empty=empty))
    actual = framework_environment()
    assert actual["import_warning_count"] == 1
    assert actual["importable"] is not broken

def test_warning_during_the_framework_import_is_counted(monkeypatch):
    import builtins
    import warnings
    original = builtins.__import__
    def importing(name, *args, **kwargs):
        if name == "torch":
            warnings.warn("synthetic NumPy initialization warning", UserWarning)
            return SimpleNamespace(__version__="2.6.0+cpu", empty=lambda *a, **k: None)
        return original(name, *args, **kwargs)
    monkeypatch.setattr(builtins, "__import__", importing)
    assert framework_environment()["import_warning_count"] == 1
