"""Development-only OpenKache example.

This example deliberately uses TLS 1.3 with server verification disabled,
because it targets a local development server.  Do not copy this trust policy
to production deployments.

Run from this package directory:

    OPENKACHE_ADDRESS=127.0.0.1:4433 python examples/basic.py
"""

from __future__ import annotations

import os

from openkache import Client


def main() -> None:
    address = os.environ.get("OPENKACHE_ADDRESS", "127.0.0.1:4433")
    client = Client.connect(address)
    try:
        outcome = client.set("example:profile", {"from": "python"})
        result = client.get("example:profile")
        print(f"SET outcome: {outcome.value}")
        print(f"GET value: {result!r}")
        delete = client.delete("example:profile")
        print(f"DELETE removed: {delete}")
        if not delete:
            raise RuntimeError("unexpected DELETE result: key was missing")
        if client.get("example:profile") is None:
            print("GET after DELETE: missing")
    finally:
        client.close()


if __name__ == "__main__":
    main()
