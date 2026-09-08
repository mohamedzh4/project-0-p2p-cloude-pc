"""Fail-closed rate limit for wrong/expired token attempts."""

from __future__ import annotations

from dataclasses import dataclass, field
import time


@dataclass
class FailClosedLimiter:
    limit: int
    window_seconds: float
    _failures: dict[str, list[float]] = field(default_factory=dict)

    def blocked(self, key: str, *, now: float | None = None) -> bool:
        moment = time.time() if now is None else now
        self._prune(key, moment)
        return len(self._failures.get(key, [])) >= self.limit

    def record_failure(self, key: str, *, now: float | None = None) -> None:
        moment = time.time() if now is None else now
        self._prune(key, moment)
        self._failures.setdefault(key, []).append(moment)

    def _prune(self, key: str, moment: float) -> None:
        cutoff = moment - self.window_seconds
        bucket = self._failures.get(key, [])
        kept = [stamp for stamp in bucket if stamp > cutoff]
        if kept:
            self._failures[key] = kept
        else:
            self._failures.pop(key, None)
