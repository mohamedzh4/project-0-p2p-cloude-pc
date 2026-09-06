from __future__ import annotations

from typing import Annotated

from fastapi import APIRouter, Depends, HTTPException, status
from sqlalchemy import select
from sqlalchemy.exc import IntegrityError
from sqlalchemy.orm import Session

from matchmaker.auth import create_access_token, hash_password, verify_password
from matchmaker.db import get_db
from matchmaker.deps import get_current_user
from matchmaker.models import User
from matchmaker.schemas import LoginRequest, SignupRequest, TokenResponse, UserPublic

router = APIRouter()


def _token_response(user: User) -> TokenResponse:
    return TokenResponse(
        access_token=create_access_token(user.id),
        user=UserPublic(id=user.id, email=user.email),
    )


@router.post("/signup", response_model=TokenResponse, status_code=status.HTTP_201_CREATED)
def signup(
    payload: SignupRequest,
    db: Annotated[Session, Depends(get_db)],
) -> TokenResponse:
    email = str(payload.email).lower()
    existing = db.scalars(select(User).where(User.email == email)).first()
    if existing is not None:
        raise HTTPException(
            status_code=status.HTTP_409_CONFLICT,
            detail="Email already registered",
        )

    user = User(email=email, password_hash=hash_password(payload.password))
    db.add(user)
    try:
        db.commit()
    except IntegrityError:
        db.rollback()
        raise HTTPException(
            status_code=status.HTTP_409_CONFLICT,
            detail="Email already registered",
        ) from None
    db.refresh(user)
    return _token_response(user)


@router.post("/login", response_model=TokenResponse)
def login(
    payload: LoginRequest,
    db: Annotated[Session, Depends(get_db)],
) -> TokenResponse:
    email = str(payload.email).lower()
    user = db.scalars(select(User).where(User.email == email)).first()
    if user is None or not verify_password(payload.password, user.password_hash):
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="Invalid email or password",
        )
    return _token_response(user)


@router.get("/me", response_model=UserPublic)
def me(user: Annotated[User, Depends(get_current_user)]) -> UserPublic:
    return UserPublic(id=user.id, email=user.email)
