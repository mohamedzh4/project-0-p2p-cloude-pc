from __future__ import annotations

from datetime import datetime, timezone
from typing import Annotated
from uuid import UUID

from fastapi import APIRouter, Depends, HTTPException, Response, status
from sqlalchemy.orm import Session

from matchmaker.config import get_settings
from matchmaker.db import get_db
from matchmaker.booking import ProvisionScheduler
from matchmaker.deps import (
    HostBearer,
    adopt_or_check_enrollment_code,
    bind_enrollment_code,
    get_provision_scheduler,
    check_host_secret,
    claim_host_secret,
    get_authorized_host,
    get_headscale_client,
    get_host_provisioner,
    get_host_selector,
    require_host_approver,
    require_host_bootstrap,
    host_secret_header,
)
from matchmaker.inventory import (
    approve_host,
    apply_declared_caps,
    host_response,
    new_pending_host,
    record_heartbeat,
)
from matchmaker.ledger import host_credits_accrued
from matchmaker.headscale import HeadscaleClient, HeadscaleError
from matchmaker.matching import HostSelector
from matchmaker.models import Host
from matchmaker.provision import HostProvisioner
from matchmaker import host_commands
from matchmaker.schemas import (
    HostCommandResponse,
    HostCommandResultRequest,
    HostEarningsResponse,
    HostInventoryRequest,
    HostResponse,
    MeshPreauthResponse,
)
from matchmaker.silence import reclaim_stale_sessions

router = APIRouter()


@router.post("/hosts", response_model=HostResponse, status_code=status.HTTP_201_CREATED)
def register_host(
    payload: HostInventoryRequest,
    db: Annotated[Session, Depends(get_db)],
    bearer: Annotated[HostBearer, Depends(require_host_bootstrap)],
    secret: Annotated[str | None, Depends(host_secret_header)],
) -> HostResponse:
    host = new_pending_host(payload)
    claim_host_secret(host, secret)
    db.add(host)
    db.flush()
    bind_enrollment_code(bearer, host)
    db.commit()
    db.refresh(host)
    return host_response(host)


@router.put("/hosts/{host_id}", response_model=HostResponse)
def update_host(
    host_id: UUID,
    payload: HostInventoryRequest,
    response: Response,
    db: Annotated[Session, Depends(get_db)],
    bearer: Annotated[HostBearer, Depends(require_host_bootstrap)],
    secret: Annotated[str | None, Depends(host_secret_header)],
) -> HostResponse:
    existing = db.get(Host, host_id)
    if existing is None:
        host = new_pending_host(payload, host_id=host_id)
        claim_host_secret(host, secret)
        db.add(host)
        db.flush()
        bind_enrollment_code(bearer, host)
        db.commit()
        db.refresh(host)
        response.status_code = status.HTTP_201_CREATED
        return host_response(host)

    adopt_or_check_enrollment_code(bearer, existing, secret)
    check_host_secret(existing, secret)
    claim_host_secret(existing, secret)
    apply_declared_caps(
        existing,
        payload,
        replace_kasm_url="kasm_url" in payload.model_fields_set,
        replace_agent_url="agent_url" in payload.model_fields_set,
    )
    db.commit()
    db.refresh(existing)
    response.status_code = status.HTTP_200_OK
    return host_response(existing)


@router.post("/hosts/{host_id}/heartbeat", response_model=HostResponse)
def heartbeat_host(
    authorized: Annotated[Host, Depends(get_authorized_host)],
    db: Annotated[Session, Depends(get_db)],
    selector: Annotated[HostSelector, Depends(get_host_selector)],
    provisioner: Annotated[HostProvisioner, Depends(get_host_provisioner)],
    headscale: Annotated[HeadscaleClient, Depends(get_headscale_client)],
    scheduler: Annotated[ProvisionScheduler, Depends(get_provision_scheduler)],
) -> HostResponse:
    host = record_heartbeat(db, authorized.id)
    if host is None:
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="Host not found")
    # Stamp first so a late ping keeps its own session. Then reclaim others.
    reclaim_stale_sessions(
        db,
        selector,
        provisioner,
        credits_per_minute=get_settings().credits_per_minute,
        headscale=headscale,
        scheduler=scheduler,
    )
    db.refresh(host)
    return host_response(host)


@router.post("/hosts/{host_id}/approve", response_model=HostResponse)
def approve_registered_host(
    host_id: UUID,
    db: Annotated[Session, Depends(get_db)],
    _: Annotated[None, Depends(require_host_approver)],
) -> HostResponse:
    host = approve_host(db, host_id)
    if host is None:
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="Host not found")
    return host_response(host)


@router.get("/hosts/{host_id}/earnings", response_model=HostEarningsResponse)
def host_earnings(
    host: Annotated[Host, Depends(get_authorized_host)],
    db: Annotated[Session, Depends(get_db)],
) -> HostEarningsResponse:
    return HostEarningsResponse(
        host_id=host.id,
        credits_accrued=host_credits_accrued(db, host.id),
    )


@router.post("/hosts/{host_id}/mesh/preauth", response_model=MeshPreauthResponse)
def mint_host_mesh_preauth(
    _host: Annotated[Host, Depends(get_authorized_host)],
    headscale: Annotated[HeadscaleClient, Depends(get_headscale_client)],
) -> MeshPreauthResponse:
    """Mint a Host Agent preauth key via Headscale, tagged `tag:host`.

    Default-deny policy lives in `headscale/acl.hujson`. Guest/user identities
    are minted on `POST /v1/sessions`, not here. This join never sets
    `--exit-node` or `--advertise-exit-node`.
    """
    try:
        grant = headscale.mint_host_preauth()
    except HeadscaleError as exc:
        raise HTTPException(
            status_code=status.HTTP_502_BAD_GATEWAY,
            detail="Headscale preauth mint failed",
        ) from exc
    return MeshPreauthResponse(
        login_server=grant.login_server,
        preauth_key=grant.key,
        user=grant.user,
        reusable=grant.reusable,
        ephemeral=grant.ephemeral,
        expires_at=grant.expires_at,
    )


@router.post(
    "/hosts/{host_id}/commands/poll",
    response_model=HostCommandResponse,
    responses={204: {"description": "Nothing to do"}},
)
def poll_host_command(
    response: Response,
    host: Annotated[Host, Depends(get_authorized_host)],
    db: Annotated[Session, Depends(get_db)],
):
    """The agent asks for its next command. 204 when there is none.

    Only a host with its own secret may poll: a command can hold a guest
    bootstrap token, which the shared bootstrap token must not unlock.
    """
    if host.secret_hash is None or host.command_token is None:
        raise HTTPException(
            status_code=status.HTTP_403_FORBIDDEN,
            detail="Register a host secret before polling for commands",
        )
    host.pull_mode = True
    host.last_polled_at = datetime.now(timezone.utc)
    db.commit()
    db.refresh(host)
    command_token = host.command_token
    claimed = host_commands.claim_next(db, host.id)
    if claimed is None:
        return Response(status_code=status.HTTP_204_NO_CONTENT)
    return HostCommandResponse(
        command_id=claimed.id,
        kind=claimed.kind,
        command_token=command_token,
        body=claimed.payload,
    )


@router.post(
    "/hosts/{host_id}/commands/{command_id}/result",
    status_code=status.HTTP_204_NO_CONTENT,
)
def report_host_command_result(
    command_id: UUID,
    payload: HostCommandResultRequest,
    host: Annotated[Host, Depends(get_authorized_host)],
    db: Annotated[Session, Depends(get_db)],
) -> Response:
    """The agent reports what happened. Only for a command it has taken."""
    if not host_commands.complete(
        db, host.id, command_id, status=payload.status, body=payload.body
    ):
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="Command not found")
    return Response(status_code=status.HTTP_204_NO_CONTENT)
