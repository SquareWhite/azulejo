"""The generated module matches the schema."""

import inspect
import json
import subprocess
import sys

from azulejo import AsyncClient, Client, types

from conftest import ROOT


def test_generated_code_is_fresh():
    r = subprocess.run([sys.executable, str(ROOT / "scripts" / "codegen-python"), "--check"], capture_output=True, text=True)
    assert r.returncode == 0, r.stderr


def test_every_method_has_a_sync_and_async_method():
    schema = json.loads((ROOT / "schema" / "api.v1.json").read_text())
    assert list(types.METHODS) == schema["methods"]
    for method in schema["methods"]:
        name = method.replace(".", "_")
        assert callable(getattr(Client, name)), method
        assert inspect.iscoroutinefunction(getattr(AsyncClient, name)), method


def test_signatures_follow_the_schema():
    split = inspect.signature(Client.layout_split)
    assert split.parameters["direction"].kind is inspect.Parameter.POSITIONAL_OR_KEYWORD
    assert split.parameters["tile"].kind is inspect.Parameter.KEYWORD_ONLY
    # `with` is a Python keyword.
    assert "with_" in inspect.signature(Client.layout_swap).parameters


def test_caller_params_skip_filters():
    assert types.CALLER_PARAMS["layout.split"] == ("tile",)
    assert "events.subscribe" not in types.CALLER_PARAMS
    assert "tile.list" not in types.CALLER_PARAMS
