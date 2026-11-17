"""Booking a host without making the renter wait on the boot.

`open_session` books the host (a `provisioning` session row occupies it)
and returns a job. The job is the slow part: it asks the Host Agent to
boot the guest, which can take minutes. It runs on a `ProvisionScheduler`
so `POST /v1/sessions` returns at once and the browser polls.

The clock for credits starts when the desktop is ready, not at booking.
"""

from __future__ import annotations

import logging
from collections.abc import Callable
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from typing import Protocol
from uuid import UUID, uuid4

from sqlalchemy import select
from sqlalchemy.exc import IntegrityError
from sqlalchemy.orm import Session, sessionmaker

from matchmaker.config import get_settings
from matchmaker.guest_bootstrap import ensure_guest_bootstrap, expire_session_bootstrap
from matchmaker.headscale import HeadscaleClient, HeadscaleError
from matchmaker.ingress import expire_session_ingress
from matchmaker.mesh import (
    GuestMeshHint,
    expire_session_mesh,
    guest_mesh_hint,
    mint_session_mesh,
)
from matchmaker.models import OCCUPYING_STATUSES, Host, LabSession, SessionStatus
from matchmaker.provision import (
    HostProvisioner,
    accept_provision_result,
    teardown_booked_host,
)
from matchmaker.storage import GuestStorageLocators

logger = logging.getLogger(__name__)

FAILURE_HOST_PROVISION = "The host could not start the desktop."
FAILURE_TIMED_OUT = "The host took too long to start the desktop."


class ProvisionScheduler(Protocol):
    def submit(self, job: Callable[[], None]) -> None:
        """Run `job` soon. It must not raise."""


class InlineScheduler:
    """Runs the job before returning. Tests, and callers with no pool."""

    def submit(self, job: Callable[[], None]) -> None:
        job()


class ThreadScheduler:
    """Runs jobs on a small pool so a slow host does not block requests."""

    def __init__(self, max_workers: int = 16) -> None:
        self._pool = ThreadPoolExecutor(max_workers=max_workers, thread_name_prefix="provision")

    def submit(self, job: Callable[[], None]) -> None:
        self._pool.submit(job)


@dataclass(frozen=True)
class ProvisionJob:
    """What the slow half needs. `guest_mesh` holds the guest bootstrap token."""

    session_id: UUID
    host_id: UUID
    lab_tier: str
    storage: GuestStorageLocators | None
    guest_mesh: GuestMeshHint | None


def occupying_session_for_user(db: Session, user_id: UUID) -> LabSession | None:
    return db.scalars(
        select(LabSession).where(
            LabSession.user_id == user_id,
            LabSession.status.in_(OCCUPYING_STATUSES),
        )
    ).first()


def open_session(
    db: Session,
    *,
    user_id: UUID,
    host: Host,
    lab_tier: str,
    headscale: HeadscaleClient | None,
    storage: GuestStorageLocators | None,
) -> tuple[LabSession, ProvisionJob] | None:
    """Book `host` for the user. None when the host or user was just taken.

    Raises HeadscaleError when the guest mesh key cannot be minted; nothing
    is booked in that case.
    """
    settings = get_settings()
    session_id = uuid4()
    mesh = None
    if headscale is not None:
        mesh = mint_session_mesh(
            headscale,
            session_id=session_id,
            login_server=settings.headscale_login_server,
            exit_node=settings.headscale_exit_node,
        )
    session = LabSession(
        id=session_id,
        user_id=user_id,
        host_id=host.id,
        status=SessionStatus.PROVISIONING,
        started_at=datetime.now(timezone.utc),
    )
    db.add(session)
    try:
        db.commit()
    except IntegrityError:
        db.rollback()
        return None
    db.refresh(session)

    guest_mesh = None
    if mesh is not None and storage is not None:
        guest_mesh = ensure_guest_bootstrap(
            db,
            session_id=session_id,
            grant=mesh,
            storage=storage,
            ttl_seconds=settings.gateway_token_ttl_seconds,
            public_origin=settings.matchmaker_public_origin,
        )
    elif mesh is not None:
        guest_mesh = guest_mesh_hint(mesh)
    job = ProvisionJob(
        session_id=session_id,
        host_id=host.id,
        lab_tier=lab_tier,
        storage=storage,
        guest_mesh=guest_mesh,
    )
    return session, job


def provision_job_runner(
    job: ProvisionJob,
    *,
    provisioner: HostProvisioner,
    headscale: HeadscaleClient | None,
    session_factory: sessionmaker[Session],
) -> Callable[[], None]:
    """Bind a job to its collaborators. The result never raises."""

    def run() -> None:
        db = session_factory()
        try:
            run_provision_job(db, job, provisioner=provisioner, headscale=headscale)
        except Exception:
            db.rollback()
            logger.exception("provision job crashed for session %s", job.session_id)
            try:
                fail_session(
                    db, job.session_id, FAILURE_HOST_PROVISION, headscale=headscale
                )
            except Exception:
                db.rollback()
                logger.exception("could not mark session %s failed", job.session_id)
        finally:
            db.close()

    return run


def run_provision_job(
    db: Session,
    job: ProvisionJob,
    *,
    provisioner: HostProvisioner,
    headscale: HeadscaleClient | None,
) -> None:
    session = db.get(LabSession, job.session_id)
    host = db.get(Host, job.host_id)
    if session is None or host is None or session.status != SessionStatus.PROVISIONING:
        return
    # The agent call can take minutes. Do not hold a transaction open for it.
    db.refresh(host)
    db.expunge(host)
    db.rollback()

    result = provisioner.provision(
        host,
        lab_tier=job.lab_tier,
        storage=job.storage,
        guest_mesh=job.guest_mesh,
    )

    host = db.get(Host, job.host_id)
    session = db.get(LabSession, job.session_id, with_for_update=True, populate_existing=True)
    if session is None or host is None or session.status != SessionStatus.PROVISIONING:
        # Ended, timed out or deleted while the guest booted: stop the guest.
        db.rollback()
        teardown_booked_host(provisioner, host)
        return
    if not accept_provision_result(db, host, result):
        db.rollback()
        fail_session(db, job.session_id, FAILURE_HOST_PROVISION, headscale=headscale)
        teardown_booked_host(provisioner, host)
        return
    session = db.get(LabSession, job.session_id, with_for_update=True, populate_existing=True)
    if session is None or session.status != SessionStatus.PROVISIONING:
        db.rollback()
        teardown_booked_host(provisioner, host)
        return
    session.status = SessionStatus.ACTIVE
    session.started_at = datetime.now(timezone.utc)
    db.commit()


def fail_session(
    db: Session,
    session_id: UUID,
    reason: str,
    *,
    headscale: HeadscaleClient | None,
) -> bool:
    """Mark a provisioning session failed and release everything it held."""
    session = db.get(LabSession, session_id, with_for_update=True, populate_existing=True)
    if session is None or session.status != SessionStatus.PROVISIONING:
        db.rollback()
        return False
    session.status = SessionStatus.FAILED
    session.ended_at = datetime.now(timezone.utc)
    session.failure_reason = reason
    db.commit()
    release_session_grants(db, session_id, headscale=headscale)
    return True


def release_session_grants(
    db: Session, session_id: UUID, *, headscale: HeadscaleClient | None
) -> None:
    """Drop the gateway token, the guest bootstrap row and the guest mesh node."""
    expire_session_ingress(db, session_id=session_id)
    expire_session_bootstrap(db, session_id=session_id)
    if headscale is not None:
        try:
            expire_session_mesh(headscale, session_id=session_id)
        except HeadscaleError:
            # Expected when the guest never joined.
            pass


def fail_stuck_provisioning(
    db: Session,
    *,
    timeout_seconds: int,
    headscale: HeadscaleClient | None,
    provisioner: HostProvisioner | None = None,
    now: datetime | None = None,
) -> list[UUID]:
    """Fail sessions left in `provisioning` (for example after a restart)."""
    moment = now if now is not None else datetime.now(timezone.utc)
    cutoff = moment - timedelta(seconds=timeout_seconds)
    stuck = db.scalars(
        select(LabSession).where(
            LabSession.status == SessionStatus.PROVISIONING,
            LabSession.started_at < cutoff,
        )
    ).all()
    failed: list[UUID] = []
    for row in stuck:
        session_id, host_id = row.id, row.host_id
        if fail_session(db, session_id, FAILURE_TIMED_OUT, headscale=headscale):
            failed.append(session_id)
            if provisioner is not None:
                teardown_booked_host(provisioner, db.get(Host, host_id))
    return failed
