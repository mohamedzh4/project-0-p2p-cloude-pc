"""ACL scope for tag:gateway — parsed from the checked-in policy file.

Headscale 0.26 file policy cannot mint pairwise tag:session-<uuid> owners, so
this grant is role-based (every gateway node may reach every guest), same as
tag:user. Session scope is the ingress token, not the ACL.
"""

from __future__ import annotations

import json
from pathlib import Path

_HEADSCALE_DIR = Path(__file__).resolve().parents[2] / "headscale"


def _load_acl_policy() -> dict:
    text = (_HEADSCALE_DIR / "acl.hujson").read_text(encoding="utf-8")
    stripped_lines: list[str] = []
    for line in text.splitlines():
        if "//" in line:
            line = line[: line.index("//")]
        stripped_lines.append(line)
    return json.loads("\n".join(stripped_lines))


def _acl_pairs(policy: dict) -> set[tuple[str, str]]:
    pairs: set[tuple[str, str]] = set()
    for rule in policy["acls"]:
        for src in rule["src"]:
            for dst in rule["dst"]:
                dest = dst.rsplit(":", 1)[0]
                pairs.add((src, dest))
    return pairs


def test_gateway_acl_is_guest_only_and_does_not_reach_hosts() -> None:
    policy = _load_acl_policy()
    pairs = _acl_pairs(policy)
    assert ("tag:gateway", "tag:guest") in pairs
    assert ("tag:guest", "tag:gateway") in pairs
    assert ("tag:gateway", "tag:host") not in pairs
    assert ("tag:host", "tag:gateway") not in pairs
    assert ("tag:gateway", "tag:user") not in pairs
    assert ("tag:gateway", "tag:matchmaker") not in pairs
    assert ("tag:gateway", "tag:exit") not in pairs
    assert ("tag:gateway", "autogroup:internet") not in pairs
    assert policy["tagOwners"]["tag:gateway"] == ["group:operators"]
    for rule in policy["acls"]:
        if "tag:gateway" in rule["src"]:
            assert rule["dst"] == ["tag:guest:*"]
