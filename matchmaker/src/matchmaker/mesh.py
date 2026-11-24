"""Per-session Headscale guest grant.

Each session mints one ephemeral `tag:guest` preauth key for the VM. The
key is stored with the guest bootstrap row and reaches only the guest; it
is never in session JSON and never sent to the Host Agent.

Renters do not join the mesh. The public gateway is the only way to a
guest desktop, and its per-session token is what scopes access. (Before,
every renter got a `tag:user` key that could reach every guest.)
"""

from __future__ import annotations

from dataclasses import dataclass
from uuid import UUID

from matchmaker.headscale import GUEST_ACL_TAGS, HeadscaleClient, PreauthGrant
from matchmaker.schemas import SessionMeshInfo

_FORBIDDEN_JOIN_FLAGS = ("--advertise-exit-node", "--advertise-tags")


@dataclass(frozen=True)
class SessionPeerGrant:
    """One mesh identity. `preauth_key` is a secret — do not log it."""

    hostname: str
    acl_tags: tuple[str, ...]
    preauth_key: str
    ephemeral: bool
    reusable: bool
    expires_at: str
    tailscale_up: tuple[str, ...]


@dataclass(frozen=True)
class SessionMeshGrant:
    """Guest join plan for one Lab session."""

    login_server: str
    exit_node: str | None
    guest: SessionPeerGrant


@dataclass(frozen=True)
class GuestMeshHint:
    """Provision-port hint for a guest image. No preauth key (not for the volunteer OS).

    `bootstrap_token` is a Matchmaker capability the guest redeems. It is not a
    Headscale preauth and it is not an S3 key. Omit `bootstrap_origin` when the
    operator has not set MATCHMAKER_PUBLIC_ORIGIN — then the guest cannot join.
    """

    login_server: str
    hostname: str
    exit_node: str | None
    acl_tags: tuple[str, ...]
    bootstrap_origin: str | None = None
    bootstrap_token: str | None = None


def guest_hostname(session_id: UUID) -> str:
    return f"dcl-guest-{session_id}"


def guest_tailscale_up_args(
    *,
    login_server: str,
    auth_key: str,
    hostname: str,
    exit_node: str | None,
) -> tuple[str, ...]:
    """Guest `tailscale up`. Always `--exit-node` when an exit node is configured.

    Never `--advertise-exit-node` (volunteers must not become exits) and never
    `--advertise-tags` (tags come from the preauth key).
    """
    args = [
        "up",
        "--login-server",
        login_server,
        "--authkey",
        auth_key,
        "--hostname",
        hostname,
        "--accept-dns=true",
        "--accept-routes=false",
    ]
    if exit_node:
        args.append(f"--exit-node={exit_node}")
        args.append("--exit-node-allow-lan-access=false")
    _reject_forbidden(args)
    return tuple(args)


def mint_session_mesh(
    client: HeadscaleClient,
    *,
    session_id: UUID,
    login_server: str,
    exit_node: str | None,
) -> SessionMeshGrant:
    """Mint the ephemeral guest key. Does not join a node."""
    guest_grant = client.mint_tagged_preauth(acl_tags=GUEST_ACL_TAGS, ephemeral=True)
    guest_host = guest_hostname(session_id)
    return SessionMeshGrant(
        login_server=login_server,
        exit_node=exit_node,
        guest=_peer(
            grant=guest_grant,
            hostname=guest_host,
            acl_tags=GUEST_ACL_TAGS,
            tailscale_up=guest_tailscale_up_args(
                login_server=login_server,
                auth_key=guest_grant.key,
                hostname=guest_host,
                exit_node=exit_node,
            ),
        ),
    )


def expire_session_mesh(client: HeadscaleClient, *, session_id: UUID) -> int:
    """Best-effort expire of the guest node that joined for this session."""
    return client.expire_nodes_named((guest_hostname(session_id),))


def guest_mesh_hint(
    grant: SessionMeshGrant,
    *,
    bootstrap_origin: str | None = None,
    bootstrap_token: str | None = None,
) -> GuestMeshHint:
    return GuestMeshHint(
        login_server=grant.login_server,
        hostname=grant.guest.hostname,
        exit_node=grant.exit_node,
        acl_tags=grant.guest.acl_tags,
        bootstrap_origin=bootstrap_origin,
        bootstrap_token=bootstrap_token,
    )


def session_mesh_info(
    session_id: UUID, *, login_server: str, exit_node: str | None
) -> SessionMeshInfo:
    """What a renter may know about the guest's mesh identity. No keys."""
    return SessionMeshInfo(
        login_server=login_server,
        exit_node=exit_node,
        exit_node_applied=bool(exit_node),
        guest_hostname=guest_hostname(session_id),
    )


def _peer(
    *,
    grant: PreauthGrant,
    hostname: str,
    acl_tags: tuple[str, ...],
    tailscale_up: tuple[str, ...],
) -> SessionPeerGrant:
    return SessionPeerGrant(
        hostname=hostname,
        acl_tags=acl_tags,
        preauth_key=grant.key,
        ephemeral=grant.ephemeral,
        reusable=grant.reusable,
        expires_at=grant.expires_at,
        tailscale_up=tailscale_up,
    )


def _reject_forbidden(args: list[str]) -> None:
    joined = " ".join(args)
    for flag in _FORBIDDEN_JOIN_FLAGS:
        if flag in joined:
            raise ValueError(f"mesh join must not pass {flag}")
