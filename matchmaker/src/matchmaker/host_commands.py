"""Commands a Host Agent fetches itself ("pull mode").

A volunteer's PC sits behind a home router, so the Matchmaker usually
cannot dial it. Instead the agent asks every couple of seconds whether
there is something to do:

    POST /v1/hosts/{id}/commands/poll              -> 204, or one command
    POST /v1/hosts/{id}/commands/{cmd}/result      -> the outcome

Both calls carry the host's own secret, so only that host sees its
commands. The command also carries the host's command token, which the
agent checks exactly as it would on its own listener.

A host that has ever polled is in pull mode for good: the Matchmaker
queues for it and waits, and never falls back to dialling it or to an old
desktop link. Hosts that give an `agentUrl` are still dialled directly
(LAN and development setups).

A provision command holds the guest bootstrap token, so its payload is
erased the moment the agent takes it.
"""

from __future__ import annotations

import json
import time
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from uuid import UUID

from sqlalchemy import select, update
from sqlalchemy.orm import Session, sessionmaker

from matchmaker.mesh import GuestMeshHint
from matchmaker.models import Host, HostCommand, HostCommandStatus
from matchmaker.provision import (
    AGENT_FAILED,
    HttpHostProvisioner,
    ProvisionResult,
    agent_origin,
    result_from_agent_body,
)
from matchmaker.storage import GuestStorageLocators

KIND_PROVISION = "provision"
KIND_TEARDOWN = "teardown"

# An agent polls every 2 s. One that has not taken a provision command in
# this long is not there; the booking fails instead of hanging.
PICKUP_TIMEOUT_SECONDS = 45.0
# Same budget the direct call gives a cold boot.
RESULT_TIMEOUT_SECONDS = 330.0
# A teardown for a host that stayed away this long is no longer of use.
TEARDOWN_MAX_AGE = timedelta(hours=6)
MAX_RESULT_BODY_CHARS = 64 * 1024
_POLL_INTERVAL_SECONDS = 0.5


@dataclass(frozen=True)
class ClaimedCommand:
    id: UUID
    kind: str
    payload: dict


def _now(now: datetime | None = None) -> datetime:
    moment = now if now is not None else datetime.now(timezone.utc)
    if moment.tzinfo is None:
        return moment.replace(tzinfo=timezone.utc)
    return moment


def enqueue(db: Session, host_id: UUID, kind: str, payload: dict | None = None) -> UUID:
    row = HostCommand(
        host_id=host_id,
        kind=kind,
        payload=json.dumps(payload) if payload is not None else None,
        status=HostCommandStatus.PENDING,
    )
    db.add(row)
    db.commit()
    return row.id


def _expire_stale(db: Session, host_id: UUID, moment: datetime) -> None:
    """Never hand an agent a command nobody is waiting for any more."""
    for kind, max_age in (
        (KIND_PROVISION, timedelta(seconds=PICKUP_TIMEOUT_SECONDS)),
        (KIND_TEARDOWN, TEARDOWN_MAX_AGE),
    ):
        db.execute(
            update(HostCommand)
            .where(
                HostCommand.host_id == host_id,
                HostCommand.kind == kind,
                HostCommand.status == HostCommandStatus.PENDING,
                HostCommand.created_at < moment - max_age,
            )
            .values(status=HostCommandStatus.EXPIRED, payload=None, done_at=moment)
        )
    db.commit()


def claim_next(db: Session, host_id: UUID, now: datetime | None = None) -> ClaimedCommand | None:
    """Give the host its oldest waiting command, once. Erases the payload."""
    moment = _now(now)
    _expire_stale(db, host_id, moment)
    row = db.scalars(
        select(HostCommand)
        .where(
            HostCommand.host_id == host_id,
            HostCommand.status == HostCommandStatus.PENDING,
        )
        .order_by(HostCommand.created_at.asc(), HostCommand.id.asc())
        .limit(1)
        .with_for_update(skip_locked=True)
    ).first()
    if row is None:
        db.rollback()
        return None
    try:
        payload = json.loads(row.payload) if row.payload else {}
    except ValueError:
        payload = {}
    claimed = ClaimedCommand(id=row.id, kind=row.kind, payload=payload)
    row.status = HostCommandStatus.TAKEN
    row.taken_at = moment
    row.payload = None
    db.commit()
    return claimed


def complete(
    db: Session,
    host_id: UUID,
    command_id: UUID,
    *,
    status: int,
    body: str,
    now: datetime | None = None,
) -> bool:
    """Record an agent's outcome. Only for a command that host has taken."""
    row = db.get(HostCommand, command_id, with_for_update=True, populate_existing=True)
    if row is None or row.host_id != host_id or row.status != HostCommandStatus.TAKEN:
        db.rollback()
        return False
    row.status = HostCommandStatus.DONE
    row.result_status = status
    row.result_body = body[:MAX_RESULT_BODY_CHARS]
    row.done_at = _now(now)
    db.commit()
    return True


def expire_if_pending(db: Session, command_id: UUID, now: datetime | None = None) -> bool:
    """Withdraw a command the agent has not taken. False if it was taken."""
    changed = db.execute(
        update(HostCommand)
        .where(
            HostCommand.id == command_id,
            HostCommand.status == HostCommandStatus.PENDING,
        )
        .values(status=HostCommandStatus.EXPIRED, payload=None, done_at=_now(now))
    ).rowcount
    db.commit()
    return bool(changed)


@dataclass(frozen=True)
class CommandOutcome:
    status: str
    result_status: int | None
    result_body: str | None


def outcome(db: Session, command_id: UUID) -> CommandOutcome | None:
    row = db.get(HostCommand, command_id, populate_existing=True)
    if row is None:
        return None
    found = CommandOutcome(row.status, row.result_status, row.result_body)
    db.rollback()
    return found


class RoutingHostProvisioner:
    """Reach a host the way it can be reached.

    - It gave an `agentUrl` (or a fallback is configured): dial it.
    - It polls for commands: queue the command and wait for its answer.
    - Neither: there is no agent to ask (`None`, a host row with a fixed
      desktop URL).
    """

    def __init__(
        self,
        http: HttpHostProvisioner,
        session_factory: sessionmaker[Session],
        *,
        pickup_timeout_seconds: float = PICKUP_TIMEOUT_SECONDS,
        result_timeout_seconds: float = RESULT_TIMEOUT_SECONDS,
        poll_interval_seconds: float = _POLL_INTERVAL_SECONDS,
    ) -> None:
        self._http = http
        self._session_factory = session_factory
        self._pickup_timeout = pickup_timeout_seconds
        self._result_timeout = result_timeout_seconds
        self._poll_interval = poll_interval_seconds

    def _dials(self, host: Host) -> bool:
        return agent_origin(host.agent_url) is not None

    def provision(
        self,
        host: Host,
        *,
        lab_tier: str,
        storage: GuestStorageLocators | None = None,
        guest_mesh: GuestMeshHint | None = None,
    ) -> ProvisionResult | None:
        if self._dials(host) or not host.pull_mode:
            return self._http.provision(
                host, lab_tier=lab_tier, storage=storage, guest_mesh=guest_mesh
            )
        payload = self._http.payload_for(
            host, lab_tier=lab_tier, storage=storage, guest_mesh=guest_mesh
        )
        host_id = host.id
        with self._session_factory() as db:
            command_id = enqueue(db, host_id, KIND_PROVISION, payload)
        found = self._wait(command_id)
        if found is None or found.result_status != 200:
            if found is None:
                # Taken but never answered, or never taken: make sure a guest
                # that may still come up is stopped.
                self.teardown(host)
            return AGENT_FAILED
        try:
            body = json.loads(found.result_body or "")
        except ValueError:
            return AGENT_FAILED
        return result_from_agent_body(lab_tier, body)

    def _wait(self, command_id: UUID) -> CommandOutcome | None:
        """The finished command, or None when the agent never delivered."""
        started = time.monotonic()
        taken_at: float | None = None
        while True:
            with self._session_factory() as db:
                found = outcome(db, command_id)
                if found is None or found.status == HostCommandStatus.EXPIRED:
                    return None
                if found.status == HostCommandStatus.DONE:
                    return found
                now = time.monotonic()
                if found.status == HostCommandStatus.PENDING:
                    if now - started >= self._pickup_timeout and expire_if_pending(
                        db, command_id
                    ):
                        return None
                else:
                    taken_at = taken_at if taken_at is not None else now
                    if now - taken_at >= self._result_timeout:
                        return None
            time.sleep(self._poll_interval)

    def teardown(self, host: Host) -> None:
        if self._dials(host) or not host.pull_mode:
            self._http.teardown(host)
            return
        with self._session_factory() as db:
            enqueue(db, host.id, KIND_TEARDOWN, None)
