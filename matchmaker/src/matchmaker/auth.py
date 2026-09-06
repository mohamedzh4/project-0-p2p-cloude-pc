from __future__ import annotations

from datetime import datetime, timedelta, timezone
from uuid import UUID

import jwt
from argon2 import PasswordHasher
from argon2.exceptions import InvalidHashError, VerificationError, VerifyMismatchError

from matchmaker.config import get_settings

_hasher = PasswordHasher()
ALGORITHM = "HS256"


def hash_password(password: str) -> str:
    return _hasher.hash(password)


def verify_password(password: str, password_hash: str) -> bool:
    try:
        return _hasher.verify(password_hash, password)
    except (VerifyMismatchError, VerificationError, InvalidHashError):
        return False


def create_access_token(user_id: UUID) -> str:
    settings = get_settings()
    expire = datetime.now(timezone.utc) + timedelta(minutes=settings.jwt_expire_minutes)
    payload = {"sub": str(user_id), "exp": expire}
    return jwt.encode(payload, settings.jwt_secret, algorithm=ALGORITHM)


def decode_access_token(token: str) -> UUID:
    settings = get_settings()
    try:
        payload = jwt.decode(token, settings.jwt_secret, algorithms=[ALGORITHM])
    except jwt.PyJWTError as exc:
        raise ValueError("invalid token") from exc

    subject = payload.get("sub")
    if not subject or not isinstance(subject, str):
        raise ValueError("invalid token")
    try:
        return UUID(subject)
    except ValueError as exc:
        raise ValueError("invalid token") from exc
