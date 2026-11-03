# azulejo (Python client)

Python client for Azulejo's socket API, sync and asyncio. Every API method is a
method here, generated from `schema/api.v1.json`, and every reply, event and
parameter has a `TypedDict` in `azulejo.types`.

```python
from azulejo import Client

with Client() as c:
    c.layout_split("right", content={"kind": "browser", "url": "http://localhost:3000"})
    with c.subscribe(["tile.", "agent.state_changed"]) as events:
        for event in events:
            print(event["event"], event["data"])
```

```python
import asyncio
from azulejo import AsyncClient

async def main():
    async with await AsyncClient.connect() as c:
        await asyncio.gather(*(c.layout_split_auto() for _ in range(3)))
        async with await c.subscribe(["focus.changed"]) as events:
            async for event in events:
                print(event["data"])

asyncio.run(main())
```

- **Socket:** `AZULEJO_SOCKET`, else `$TMPDIR/azulejo/azulejo.sock` (macOS) or
  `$XDG_RUNTIME_DIR/azulejo/azulejo.sock` (Linux), the same as `azj`.
- **Caller:** run inside an Azulejo terminal tile, methods act on that tile,
  its tab and its workspace unless you pass them, the same as `azj`. That uses
  `AZULEJO_TILE_ID`, and falls back to `HERDR_PANE_ID` when the pane belongs
  to the core's herdr session. `Client(caller=False)` acts on whatever is
  focused.
- **Arguments:** required params are positional, the rest keyword-only.
  `None` means "leave it out". A param named after a Python keyword gets a
  trailing underscore (`layout_swap(with_=...)`).
- **Errors:** `ApiError` (`.code` is the protocol's error code, such as
  `not_found`), `NotRunning`, `ProtocolError`.
- **Events:** `subscribe(filter, workspace=, tile=)` opens a connection of its
  own. Each item is `{"event", "data", "subscription"}`. A reader that falls
  behind gets `{"event": "events.lagged", "data": {"dropped": n}}` and should
  re-read whatever state it tracks.

## Examples

`examples/` covers the PRD's core-only use cases:

| Script | Use case |
| --- | --- |
| `scripted_layout.py` | Open a project's layout in one command: editor, tests, dev server, browser |
| `drive_terminal.py` | An agent runs a command in another terminal and reads the output |
| `plan_review.py` | An agent writes `plan.md`, previews it, and watches each revision land (asyncio) |

## Development

```sh
just python-codegen   # regenerate after a schema change (`just schema` does it too)
just python-test      # freshness check + pytest against a headless azulejod
```

The tests start their own `azulejod` on throwaway sockets. Terminal tests use
a throwaway herdr session (`azj-py-*`), never your own.
