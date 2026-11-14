"""The client against a headless core (no herdr)."""

import asyncio
from pathlib import Path

import pytest

from azulejo import ApiError, AsyncClient, Client, NotRunning


def tiles(c: Client) -> list[dict]:
    return c.tile_list()["tiles"]


def test_calls_and_replies(daemon):
    with Client() as c:
        assert c.core_ping()["type"] == "pong"
        assert c.call("core.version")["api_version"] == 1
        created = c.layout_split("right")
        assert created["type"] == "created"
        assert created["tile"].startswith("tile_")
        assert len(tiles(c)) == 2


def test_errors_carry_the_protocol_code(daemon):
    with Client() as c:
        with pytest.raises(ApiError) as e:
            c.tile_get(tile="tile_00000000000000000000000000")
        assert e.value.code == "not_found"
        assert e.value.rpc_code == -32001


def test_not_running(tmp_short):
    with pytest.raises(NotRunning):
        Client(tmp_short / "nothing.sock")


def test_none_means_leave_it_out(daemon):
    with Client() as c:
        # `content=None` isn't sent, so the core's default (a terminal) applies.
        tile = c.layout_split("down", content=None)["tile"]
        assert c.tile_get(tile=tile)["kind"] == "terminal"


def test_caller_tile_from_the_environment(daemon, monkeypatch):
    with Client() as c:
        first = c.tile_list()["tiles"][0]["id"]
        c.layout_split("right")  # focus moves to the new tile
    monkeypatch.setenv("AZULEJO_TILE_ID", first)
    with Client() as inside:
        inside.tile_rename(title="mine")
    with Client(caller=False) as outside:
        outside.tile_rename(title="focused")
        by_id = {t["id"]: t.get("title") for t in tiles(outside)}
    assert by_id[first] == "mine"
    assert sorted(by_id.values()) == ["focused", "mine"]


def test_subscribe_streams_events_in_order(daemon):
    with Client() as c, c.subscribe(["tile.created"]) as events:
        made = [c.layout_split("right")["tile"] for _ in range(3)]
        seen = [events.next(timeout=5)["data"]["tile"]["id"] for _ in made]
        assert seen == made
        assert events.next(timeout=0.2) is None


def test_subscribe_scopes_to_a_tile(daemon):
    with Client() as c:
        a = c.tile_list()["tiles"][0]["id"]
        b = c.layout_split("right")["tile"]
        with c.subscribe(["tile.updated"], tile=a) as events:
            c.tile_rename(tile=b, title="not this one")
            c.tile_rename(tile=a, title="this one")
            event = events.next(timeout=5)
            assert event["data"]["tile"]["id"] == a
            assert events.next(timeout=0.2) is None


async def test_async_calls_run_concurrently(daemon):
    async with await AsyncClient.connect() as c:
        replies = await asyncio.gather(*(c.layout_split_auto() for _ in range(8)))
        assert len({r["tile"] for r in replies}) == 8
        assert len((await c.tile_list())["tiles"]) == 9


async def test_async_subscribe(daemon):
    async with await AsyncClient.connect() as c:
        async with await c.subscribe(["layout.changed"]) as events:
            await c.layout_split("down")
            event = await events.next(timeout=5)
            assert event["event"] == "layout.changed"
            assert event["subscription"] == events.id
            assert await events.next(timeout=0.2) is None


def test_examples_without_herdr(daemon, tmp_short):
    import subprocess
    import sys

    examples = Path(__file__).resolve().parents[1] / "examples"
    r = subprocess.run(
        [sys.executable, str(examples / "scripted_layout.py"), str(tmp_short), "--tests", "true"],
        capture_output=True, text=True, timeout=30,
    )
    assert r.returncode == 0, r.stderr
    assert "4 tiles" in r.stdout
    r = subprocess.run(
        [sys.executable, str(examples / "plan_review.py"), str(tmp_short / "plan.md")],
        capture_output=True, text=True, timeout=60,
    )
    assert r.returncode == 0, r.stderr
    assert "preview open:" in r.stdout
    assert "4/4 done" in r.stdout
