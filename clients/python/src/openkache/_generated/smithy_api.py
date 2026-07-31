# Generated from the OpenKache Smithy contract. Do not edit.

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
from typing import Protocol

class SmithySetCondition(str, Enum):
    """Values defined by the Smithy SetCondition shape."""

    IF_ABSENT = "if_absent"
    IF_PRESENT = "if_present"

class SmithySetOutcome(str, Enum):
    """Values defined by the Smithy SetOutcome shape."""

    CREATED = "created"
    REPLACED = "replaced"
    NOT_STORED = "not_stored"

@dataclass(frozen=True, slots=True)
class SmithyDeleteInput:
    """Smithy DeleteInput structure."""

    item_id: bytes

@dataclass(frozen=True, slots=True)
class SmithyDeleteOutput:
    """Smithy DeleteOutput structure."""

    deleted: bool

@dataclass(frozen=True, slots=True)
class SmithyGetInput:
    """Smithy GetInput structure."""

    item_id: bytes

@dataclass(frozen=True, slots=True)
class SmithyGetOutput:
    """Smithy GetOutput structure."""

    value: bytes | None = None

@dataclass(frozen=True, slots=True)
class SmithyPingInput:
    """Smithy PingInput structure."""

    pass

@dataclass(frozen=True, slots=True)
class SmithyPingOutput:
    """Smithy PingOutput structure."""

    pass

@dataclass(frozen=True, slots=True)
class SmithySetInput:
    """Smithy SetInput structure."""

    item_id: bytes
    value: bytes
    condition: SmithySetCondition | None = None
    ttl_milliseconds: int | None = None

@dataclass(frozen=True, slots=True)
class SmithySetOutput:
    """Smithy SetOutput structure."""

    outcome: SmithySetOutcome

@dataclass(frozen=True, slots=True)
class SmithyStatsInput:
    """Smithy StatsInput structure."""

    pass

@dataclass(frozen=True, slots=True)
class SmithyStatsOutput:
    """Smithy StatsOutput structure."""

    json: str

@dataclass(frozen=True, slots=True)
class SmithySyncInput:
    """Smithy SyncInput structure."""

    pass

@dataclass(frozen=True, slots=True)
class SmithySyncOutput:
    """Smithy SyncOutput structure."""

    pass


class SmithyOpenKacheApi(Protocol):
    """Async operations defined by the OpenKache Smithy service."""

    async def ping(
        self, input: SmithyPingInput
    ) -> SmithyPingOutput: ...
    async def get(
        self, input: SmithyGetInput
    ) -> SmithyGetOutput: ...
    async def set(
        self, input: SmithySetInput
    ) -> SmithySetOutput: ...
    async def delete(
        self, input: SmithyDeleteInput
    ) -> SmithyDeleteOutput: ...
    async def stats(
        self, input: SmithyStatsInput
    ) -> SmithyStatsOutput: ...
    async def sync(
        self, input: SmithySyncInput
    ) -> SmithySyncOutput: ...
