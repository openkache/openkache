"""Development-only OpenKache example.

This example deliberately uses TLS 1.3 with server verification disabled,
because it targets a local development server.  Do not copy this trust policy
to production deployments.

Run from this package directory:

    OPENKACHE_ADDRESS=127.0.0.1:4433 python examples/basic.py
"""

from __future__ import annotations

import os

from openkache import Client, DeleteOutcome, Found, Missing


def main() -> None:
    address = os.environ.get("OPENKACHE_ADDRESS", "127.0.0.1:4433")
    client = Client.connect(address)
    try:
        outcome = client.set("example:profile", {"from": "python"})
        result = client.get("example:profile")
        print(f"SET outcome: {outcome.value}")
        if isinstance(result, Found):
            print(f"GET value: {result.value!r}")
        elif isinstance(result, Missing):
            print("GET value: missing")
        delete = client.delete("example:profile")
        print(f"DELETE outcome: {delete.value}")
        if delete is not DeleteOutcome.DELETED:
            raise RuntimeError(f"unexpected DELETE outcome: {delete!r}")
        if isinstance(client.get("example:profile"), Missing):
            print("GET after DELETE: missing")
    finally:
        client.close()


if __name__ == "__main__":
    main()
