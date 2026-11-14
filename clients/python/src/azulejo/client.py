"""Connections to Azulejo's socket API: `Client` (blocking) and `AsyncClient`
(asyncio).

Every API method is a method here too, generated from the schema
(`layout.split` → `client.layout_split(...)`). Each returns the reply as a
dict, or raises `ApiError`.

Run inside an Azulejo terminal tile, methods act on that tile, its tab and its
workspace unless told otherwise, like `azj`. Pass `caller=False` to act on
whatever is focused instead.
"""

from __future__ import annotations

import asyncio
import itertools
import json
import os
import socket
import sys
import tempfile
from collections import deque
from collections.abc import Iterator
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from ._generated import CALLER_PARAMS, AsyncMethods, Methods

SOCKET_ENV = "AZULEJO_SOCKET"


def socket_path() -> Path:
    """`AZULEJO_SOCKET` if set; otherwise `$TMPDIR/azulejo/azulejo.sock` on
    macOS and `$XDG_RUNTIME_DIR/azulejo/azulejo.sock` on Linux."""
    if path := os.environ.get(SOCKET_ENV):
        return Path(path)
    var = "TMPDIR" if sys.platform == "darwin" else "XDG_RUNTIME_DIR"
    base = os.environ.get(var) or tempfile.gettempdir()
    return Path(base) / "azulejo" / "azulejo.sock"


class AzulejoError(Exception):
    """Anything that went wrong talking to Azulejo."""


class NotRunning(AzulejoError):
    """Nothing is listening at the socket."""

    def __init__(self, path: Path):
        super().__init__(f"Azulejo isn't running (nothing listening at {path})")
        self.path = path


class ApiError(AzulejoError):
    """The core refused a request.

    `code` is the protocol's error code (`not_found`, `invalid_params`, …);
    `rpc_code` the JSON-RPC number."""

    def __init__(self, code: str, message: str, rpc_code: int):
        super().__init__(f"{code}: {message}")
        self.code = code
        self.message = message
        self.rpc_code = rpc_code


class ProtocolError(AzulejoError):
    """The server sent something unexpected."""


# ----- caller resolution (shared by both clients) -----


@dataclass
class _Caller:
    """What the environment says about the terminal tile we run in."""

    tile: str | None = None
    workspace: str | None = None
    herdr_pane: str | None = None
    herdr_workspace: str | None = None
    herdr_socket: str | None = None
    # Looked up once, then cached.
    resolved: dict[str, str] | None = field(default=None, repr=False)

    @classmethod
    def from_env(cls) -> _Caller:
        get = lambda name: os.environ.get(name) or None  # noqa: E731
        return cls(
            tile=get("AZULEJO_TILE_ID"),
            workspace=get("AZULEJO_WORKSPACE_ID"),
            herdr_pane=get("HERDR_PANE_ID"),
            herdr_workspace=get("HERDR_WORKSPACE_ID"),
            herdr_socket=get("HERDR_SOCKET_PATH"),
        )

    def known(self) -> bool:
        return self.tile is not None or self.herdr_pane is not None


def _wants(method: str, params: dict[str, Any]) -> list[str]:
    return [n for n in CALLER_PARAMS.get(method, ()) if params.get(n) is None]


def _from_tile(found: dict[str, Any], caller: _Caller) -> dict[str, str]:
    info = {k: found[k] for k in ("tab", "window", "workspace") if isinstance(found.get(k), str)}
    info["tile"] = caller.tile or found.get("id", "")
    if caller.workspace:
        info["workspace"] = caller.workspace
    return info


def _same_herdr_session(caller: _Caller, version: dict[str, Any]) -> bool:
    theirs = version.get("herdr_socket")
    if caller.herdr_socket is None or not isinstance(theirs, str):
        return True
    return os.path.realpath(caller.herdr_socket) == os.path.realpath(theirs)


def _find_herdr_pane(session: dict[str, Any], pane: str, herdr_ws: str | None) -> dict[str, str] | None:
    for window in session.get("windows", []):
        for ws in window.get("workspaces", []):
            bound = ws.get("herdr_workspace")
            if herdr_ws and bound and herdr_ws != bound:
                continue
            for tab in ws.get("tabs", []):
                for tile in tab.get("tiles", []):
                    if (tile.get("herdr") or {}).get("pane_id") == pane:
                        return {"window": window["id"], "workspace": ws["id"], "tab": tab["id"], "tile": tile["id"]}
    return None


def _clean(params: dict[str, Any]) -> dict[str, Any] | None:
    params = {k: v for k, v in params.items() if v is not None}
    return params or None


def _request(id_: int, method: str, params: dict[str, Any] | None) -> bytes:
    request: dict[str, Any] = {"jsonrpc": "2.0", "id": id_, "method": method}
    if params:
        request["params"] = params
    return (json.dumps(request) + "\n").encode()


def _result(response: dict[str, Any]) -> Any:
    if (err := response.get("error")) is not None:
        data = err.get("data") or {}
        raise ApiError(str(data.get("code", "error")), str(err.get("message", "")), int(err.get("code", 0)))
    if "result" not in response:
        raise ProtocolError(f"response has no result: {response}")
    return response["result"]


def _stream_item(message: dict[str, Any]) -> dict[str, Any]:
    """A notification as an event dict: `{"event", "data", "subscription"}`.
    A lag notice becomes `{"event": "events.lagged", "data": {"dropped": n}}`."""
    params = dict(message.get("params") or {})
    if message.get("method") == "events.lagged":
        return {
            "event": "events.lagged",
            "data": {"dropped": params.get("dropped", 0)},
            "subscription": params.get("subscription"),
        }
    return params


def _connect_error(path: Path, e: OSError) -> AzulejoError:
    if isinstance(e, (FileNotFoundError, ConnectionRefusedError)):
        return NotRunning(path)
    return AzulejoError(f"can't connect to {path}: {e}")


# ----- blocking client -----


class Client(Methods):
    """A blocking connection. Not thread-safe: one call at a time.

    >>> with Client() as c:
    ...     c.layout_split("right", content={"kind": "browser", "url": "http://localhost:3000"})
    """

    def __init__(self, path: str | os.PathLike[str] | None = None, *, caller: bool = True, timeout: float = 30.0):
        self.path = Path(path) if path is not None else socket_path()
        self.timeout = timeout
        self._caller = _Caller.from_env() if caller else _Caller()
        self._ids = itertools.count(1)
        self._pending: deque[dict[str, Any]] = deque()
        self._sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self._sock.settimeout(timeout)
        try:
            self._sock.connect(str(self.path))
        except OSError as e:
            self._sock.close()
            raise _connect_error(self.path, e) from None
        self._reader = self._sock.makefile("rb")

    def close(self) -> None:
        self._reader.close()
        self._sock.close()

    def __enter__(self) -> Client:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    def call(self, method: str, params: dict[str, Any] | None = None) -> Any:
        """Calls `method` as is: no caller filling, no None-stripping."""
        id_ = next(self._ids)
        self._sock.sendall(_request(id_, method, params))
        while True:
            message = self._read()
            if "id" not in message and "method" in message:
                self._pending.append(message)
                continue
            if message.get("id") != id_:
                raise ProtocolError(f"expected a response to request {id_}, got {message}")
            return _result(message)

    def _call(self, method: str, params: dict[str, Any]) -> Any:
        params = dict(params)
        if wants := _wants(method, params):
            info = self._resolve_caller()
            for name in wants:
                if name in info:
                    params[name] = info[name]
        return self.call(method, _clean(params))

    def _resolve_caller(self) -> dict[str, str]:
        c = self._caller
        if not c.known():
            return {}
        if c.resolved is None:
            if c.tile:
                c.resolved = _from_tile(self.call("tile.get", {"tile": c.tile}), c)
            elif c.herdr_pane and _same_herdr_session(c, self.call("core.version")):
                found = _find_herdr_pane(self.call("session.get")["session"], c.herdr_pane, c.herdr_workspace)
                c.resolved = found or {}
            else:
                c.resolved = {}
        return c.resolved

    def _read(self, timeout: float | None = None) -> dict[str, Any]:
        self._sock.settimeout(timeout if timeout is not None else self.timeout)
        try:
            line = self._reader.readline()
        finally:
            self._sock.settimeout(self.timeout)
        if not line:
            raise ProtocolError("server closed the connection")
        return json.loads(line)

    def subscribe(
        self,
        filter: list[str] | None = None,
        *,
        workspace: str | None = None,
        tile: str | None = None,
    ) -> Subscription:
        """Streams events on a new connection of its own. Iterate it; close it
        (or leave the `with` block) to stop.

        `filter` takes event names or prefixes (`"tile."`, `"agent.state_changed"`).
        A reader that falls behind gets `{"event": "events.lagged", ...}` and
        should re-read whatever state it tracks."""
        return Subscription(self.path, _clean({"filter": filter, "workspace": workspace, "tile": tile}))


class Subscription:
    """An event stream. Iterating blocks until the next event."""

    def __init__(self, path: Path, params: dict[str, Any] | None):
        self._client = Client(path, caller=False)
        try:
            reply = self._client.call("events.subscribe", params)
        except BaseException:
            self._client.close()
            raise
        self.id: int = reply["subscription"]

    def next(self, timeout: float | None = None) -> dict[str, Any] | None:
        """The next event, or None after `timeout` seconds (None waits forever)."""
        if self._client._pending:
            return _stream_item(self._client._pending.popleft())
        try:
            message = self._client._read(timeout=timeout)
        except (TimeoutError, socket.timeout):
            return None
        return _stream_item(message)

    def __iter__(self) -> Iterator[dict[str, Any]]:
        while True:
            item = self.next(timeout=None)
            if item is not None:
                yield item

    def close(self) -> None:
        self._client.close()

    def __enter__(self) -> Subscription:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()


# ----- asyncio client -----


class AsyncClient(AsyncMethods):
    """An asyncio connection. Calls may run concurrently.

    >>> async with await AsyncClient.connect() as c:
    ...     await c.layout_split("down")
    """

    def __init__(self, path: Path, reader: asyncio.StreamReader, writer: asyncio.StreamWriter, caller: bool):
        self.path = path
        self._reader = reader
        self._writer = writer
        self._caller = _Caller.from_env() if caller else _Caller()
        self._ids = itertools.count(1)
        self._waiting: dict[int, asyncio.Future[dict[str, Any]]] = {}
        self._notifications: asyncio.Queue[dict[str, Any] | None] = asyncio.Queue()
        self._caller_lock = asyncio.Lock()
        self._read_task = asyncio.create_task(self._read_loop())

    @classmethod
    async def connect(cls, path: str | os.PathLike[str] | None = None, *, caller: bool = True) -> AsyncClient:
        path = Path(path) if path is not None else socket_path()
        try:
            reader, writer = await asyncio.open_unix_connection(str(path), limit=16 * 1024 * 1024)
        except OSError as e:
            raise _connect_error(path, e) from None
        return cls(path, reader, writer, caller)

    async def close(self) -> None:
        self._read_task.cancel()
        self._writer.close()
        try:
            await self._writer.wait_closed()
        except (ConnectionError, OSError):
            pass

    async def __aenter__(self) -> AsyncClient:
        return self

    async def __aexit__(self, *_: object) -> None:
        await self.close()

    async def _read_loop(self) -> None:
        error: BaseException = ProtocolError("server closed the connection")
        try:
            while line := await self._reader.readline():
                message = json.loads(line)
                if "id" not in message and "method" in message:
                    self._notifications.put_nowait(message)
                    continue
                future = self._waiting.pop(message.get("id"), None)
                if future is not None and not future.done():
                    future.set_result(message)
        except asyncio.CancelledError:
            error = AzulejoError("connection closed")
            raise
        except Exception as e:  # noqa: BLE001
            error = e
        finally:
            for future in self._waiting.values():
                if not future.done():
                    future.set_exception(error)
            self._waiting.clear()
            self._notifications.put_nowait(None)

    async def call(self, method: str, params: dict[str, Any] | None = None) -> Any:
        """Calls `method` as is: no caller filling, no None-stripping."""
        id_ = next(self._ids)
        future: asyncio.Future[dict[str, Any]] = asyncio.get_running_loop().create_future()
        self._waiting[id_] = future
        self._writer.write(_request(id_, method, params))
        await self._writer.drain()
        return _result(await future)

    async def _call(self, method: str, params: dict[str, Any]) -> Any:
        params = dict(params)
        if wants := _wants(method, params):
            info = await self._resolve_caller()
            for name in wants:
                if name in info:
                    params[name] = info[name]
        return await self.call(method, _clean(params))

    async def _resolve_caller(self) -> dict[str, str]:
        c = self._caller
        if not c.known():
            return {}
        async with self._caller_lock:
            if c.resolved is None:
                if c.tile:
                    c.resolved = _from_tile(await self.call("tile.get", {"tile": c.tile}), c)
                elif c.herdr_pane and _same_herdr_session(c, await self.call("core.version")):
                    session = (await self.call("session.get"))["session"]
                    c.resolved = _find_herdr_pane(session, c.herdr_pane, c.herdr_workspace) or {}
                else:
                    c.resolved = {}
        return c.resolved

    async def subscribe(
        self,
        filter: list[str] | None = None,
        *,
        workspace: str | None = None,
        tile: str | None = None,
    ) -> AsyncSubscription:
        """Streams events on a new connection of its own. The subscription is
        live once this returns, so nothing that happens next is missed:

            async with await client.subscribe(["tile.", "agent."]) as events:
                async for event in events:
                    ...

        See `Client.subscribe` for the items."""
        stream = await AsyncClient.connect(self.path, caller=False)
        try:
            params = _clean({"filter": filter, "workspace": workspace, "tile": tile})
            reply = await stream.call("events.subscribe", params)
        except BaseException:
            await stream.close()
            raise
        return AsyncSubscription(stream, reply["subscription"])


class AsyncSubscription:
    """An event stream for asyncio. Iterate with `async for`."""

    def __init__(self, stream: AsyncClient, id_: int):
        self._stream = stream
        self.id = id_

    async def next(self, timeout: float | None = None) -> dict[str, Any] | None:
        """The next event, or None after `timeout` seconds (None waits forever).
        Raises `ProtocolError` once the server closes the stream."""
        try:
            message = await asyncio.wait_for(self._stream._notifications.get(), timeout)
        except TimeoutError:
            return None
        if message is None:
            self._stream._notifications.put_nowait(None)
            raise ProtocolError("server closed the connection")
        return _stream_item(message)

    def __aiter__(self) -> AsyncSubscription:
        return self

    async def __anext__(self) -> dict[str, Any]:
        try:
            item = await self.next()
        except ProtocolError:
            raise StopAsyncIteration from None
        assert item is not None
        return item

    async def close(self) -> None:
        await self._stream.close()

    async def __aenter__(self) -> AsyncSubscription:
        return self

    async def __aexit__(self, *_: object) -> None:
        await self.close()
