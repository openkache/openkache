"""Development-only OpenKache example.

This example deliberately uses TLS 1.3 with server verification disabled,
because it targets a local development server.  Do not copy this trust policy
to production deployments.

Run from this package directory:

    OPENKACHE_ADDRESS=127.0.0.1:4433 python examples/basic.py
"""

from __future__ import annotations

import os

from openkache import Client, Found, Missing


def main() -> None:
    address = os.environ.get("OPENKACHE_ADDRESS", "127.0.0.1:4433")
    client = Client.connect(address)
    try:
        outcome = client.set(
            "example:profile",
            {"name": "OpenKache", "visits": 1, "development_only": True},
        )
        result = client.get("example:profile")
        print(f"SET outcome: {outcome.value}")
        if isinstance(result, Found):
            print(f"GET value: {result.value!r}")
        elif isinstance(result, Missing):
            print("GET value: missing")
    finally:
        client.close()


if __name__ == "__main__":
    main()
