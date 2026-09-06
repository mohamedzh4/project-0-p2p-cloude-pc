"""Per-session Headscale guest/user grants (phase 17).

Headscale 0.26.1 can mint tagged, ephemeral preauth keys without Tailscale
SaaS. Matchmaker uses that for `tag:guest` (the VM) and `tag:user` (the
renter client) on each session. Host Agent join stays `tag:host` and never
receives these keys.

Leftover: `acl.hujson` is still role-based. Headscale 0.26 has no policy API
to insert pairwise `tag:session-<uuid>` owners, so every `tag:user` may still
reach every `tag:guest`. Isolation beyond that is hostname + ephemeral keys,
not a unique ACL tag per booking.
"""

from __future__ import annotations

from dataclasses import dataclass
from uuid import UUID

from matchmaker.headscale import GUEST_ACL_TAGS, USER_ACL_TAGS, HeadscaleClient, PreauthGrant
from matchmaker.schemas import MeshPeerInfo, SessionMeshInfo

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
    """Guest + user join plans for one Lab session."""

    login_server: str
    exit_node: str | None
    guest: SessionPeerGrant
    user: SessionPeerGrant

    @property
    def exit_node_applied(self) -> bool:
        if not self.exit_node:
            return False
        needle = f"--exit-node={self.exit_node}"
        return needle in self.guest.tailscale_up


@dataclass(frozen=True)
class GuestMeshHint:
    """Provision-port hint for a guest image. No preauth key (not for the volunteer OS)."""

    login_server: str
    hostname: str
    exit_node: str | None
    acl_tags: tuple[str, ...]


def guest_hostname(session_id: UUID) -> str:
    return f"dcl-guest-{session_id}"


def user_hostname(session_id: UUID) -> str:
    return f"dcl-user-{session_id}"


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
        "--accept-dns=false",
        "--accept-routes=false",
    ]
    if exit_node:
        args.append(f"--exit-node={exit_node}")
        args.append("--exit-node-allow-lan-access=false")
    _reject_forbidden(args)
    return tuple(args)


def user_tailscale_up_args(
    *,
    login_server: str,
    auth_key: str,
    hostname: str,
) -> tuple[str, ...]:
    """Renter-client join. No `--exit-node` — only the guest VM must hairpin egress."""
    args = [
        "up",
        "--login-server",
        login_server,
        "--authkey",
        auth_key,
        "--hostname",
        hostname,
        "--accept-dns=false",
        "--accept-routes=false",
    ]
    _reject_forbidden(args)
    if any(item == "--exit-node" or item.startswith("--exit-node=") for item in args):
        raise ValueError("user mesh join must not set --exit-node")
    return tuple(args)


def mint_session_mesh(
    client: HeadscaleClient,
    *,
    session_id: UUID,
    login_server: str,
    exit_node: str | None,
) -> SessionMeshGrant:
    """Mint ephemeral guest+user keys. Does not join a node and does not call Tailscale SaaS."""
    guest_grant = client.mint_tagged_preauth(acl_tags=GUEST_ACL_TAGS, ephemeral=True)
    user_grant = client.mint_tagged_preauth(acl_tags=USER_ACL_TAGS, ephemeral=True)
    guest_host = guest_hostname(session_id)
    user_host = user_hostname(session_id)
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
        user=_peer(
            grant=user_grant,
            hostname=user_host,
            acl_tags=USER_ACL_TAGS,
            tailscale_up=user_tailscale_up_args(
                login_server=login_server,
                auth_key=user_grant.key,
                hostname=user_host,
            ),
        ),
    )


def expire_session_mesh(client: HeadscaleClient, *, session_id: UUID) -> int:
    """Best-effort expire of nodes that joined with this session's hostnames."""
    return client.expire_nodes_named((guest_hostname(session_id), user_hostname(session_id)))


def guest_mesh_hint(grant: SessionMeshGrant) -> GuestMeshHint:
    return GuestMeshHint(
        login_server=grant.login_server,
        hostname=grant.guest.hostname,
        exit_node=grant.exit_node,
        acl_tags=grant.guest.acl_tags,
    )


def mesh_info_from_grant(grant: SessionMeshGrant) -> SessionMeshInfo:
    return SessionMeshInfo(
        login_server=grant.login_server,
        exit_node=grant.exit_node,
        exit_node_applied=grant.exit_node_applied,
        guest=_peer_info(grant.guest),
        user=_peer_info(grant.user),
    )


def _peer_info(peer: SessionPeerGrant) -> MeshPeerInfo:
    return MeshPeerInfo(
        hostname=peer.hostname,
        acl_tags=list(peer.acl_tags),
        preauth_key=peer.preauth_key,
        ephemeral=peer.ephemeral,
        reusable=peer.reusable,
        expires_at=peer.expires_at,
        tailscale_up=list(peer.tailscale_up),
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
