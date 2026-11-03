"""Python client for Azulejo's socket API (F23).

    from azulejo import Client

    with Client() as c:
        c.layout_split_auto()
        with c.subscribe(["tile."]) as events:
            for event in events:
                print(event["event"], event["data"])

`AsyncClient` is the asyncio twin. Types for every reply, event and
parameter live in `azulejo.types` (generated from the API schema).
"""

from . import _generated as types
from ._generated import API_VERSION, METHODS
from .client import (
    ApiError,
    AsyncClient,
    AsyncSubscription,
    AzulejoError,
    Client,
    NotRunning,
    ProtocolError,
    Subscription,
    socket_path,
)

__all__ = [
    "API_VERSION",
    "METHODS",
    "ApiError",
    "AsyncClient",
    "AsyncSubscription",
    "AzulejoError",
    "Client",
    "NotRunning",
    "ProtocolError",
    "Subscription",
    "socket_path",
    "types",
]
