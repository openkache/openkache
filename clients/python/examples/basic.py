"""Minimal OpenKache client example.

Set ``OPENKACHE_ADDRESS`` to the server's ``host:port`` endpoint and
``OPENKACHE_CA_CERT`` to the trusted CA certificate before running:

    python examples/basic.py
"""

from __future__ import annotations

import asyncio
import os
from pathlib import Path

from openkache import Client, SetOptions


async def main() -> None:
    address = os.environ.get("OPENKACHE_ADDRESS", "127.0.0.1:4433")
    certificate = os.environ.get("OPENKACHE_CA_CERT")
    if certificate is None:
        raise SystemExit(
            "Set OPENKACHE_CA_CERT to the PEM/DER CA certificate trusted by "
            "the OpenKache server."
        )

    async with await Client.connect(
        address,
        certificate=Path(certificate),
    ) as client:
        outcome = await client.set(
            "example:profile",
            {"name": "OpenKache", "visits": 1},
            SetOptions(condition="if_absent", ttl_ms=300_000),
        )
        profile = await client.get("example:profile")
        print(f"SET outcome: {outcome.value}")
        print(f"GET value: {profile!r}")


if __name__ == "__main__":
    asyncio.run(main())
