#!/usr/bin/env python3
"""Plan review (PRD use case): an agent writes `plan.md`, opens it in a
preview tile, and the human follows along as it changes. Uses the asyncio
client and an event stream to confirm each revision reached the tile.

    uv run examples/plan_review.py plan.md
"""

import asyncio
import sys
from pathlib import Path

from azulejo import AsyncClient

STEPS = ["Read the code", "Write the failing test", "Fix the bug", "Ship it"]


async def main() -> None:
    plan = Path(sys.argv[1] if len(sys.argv) > 1 else "plan.md").resolve()
    plan.write_text("# Plan\n\n_Thinking…_\n")
    async with await AsyncClient.connect() as c:
        # Subscribe first, so the tile's first render can't slip by.
        async with await c.subscribe(["preview.updated"]) as updates:
            tile = (await c.layout_split("right", content={"kind": "preview", "path": str(plan)}))["tile"]

            async def next_render() -> dict:
                async for event in updates:
                    if event["event"] == "preview.updated" and event["data"]["tile"] == tile:
                        return event["data"]
                raise RuntimeError("Azulejo went away")

            first = await asyncio.wait_for(next_render(), 10)
            print(f"preview open: {first['path']} ({first['kind']})")
            for done in range(1, len(STEPS) + 1):
                lines = [f"- [{'x' if i < done else ' '}] {s}" for i, s in enumerate(STEPS)]
                plan.write_text(f"# Plan\n\n{done} of {len(STEPS)} done\n\n" + "\n".join(lines) + "\n")
                render = await asyncio.wait_for(next_render(), 10)
                print(f"revision {render['revision']}: {done}/{len(STEPS)} done")


if __name__ == "__main__":
    asyncio.run(main())
