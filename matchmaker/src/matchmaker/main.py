from __future__ import annotations

import asyncio
import logging
from contextlib import asynccontextmanager
from typing import AsyncIterator

from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware

from matchmaker.config import get_settings
from matchmaker.db import get_session_factory
from matchmaker.deps import get_headscale_client, get_host_provisioner, get_host_selector
from matchmaker.routers import auth, hosts, images, sessions, storage
from matchmaker.silence import reclaim_stale_sessions

logger = logging.getLogger(__name__)


def _sweep_stale_sessions_once() -> None:
    settings = get_settings()
    db = get_session_factory()()
    try:
        reclaim_stale_sessions(
            db,
            get_host_selector(),
            get_host_provisioner(),
            credits_per_minute=settings.credits_per_minute,
            headscale=get_headscale_client(),
        )
    except Exception:
        db.rollback()
        logger.exception("stale session sweep failed")
    finally:
        db.close()


@asynccontextmanager
async def lifespan(_app: FastAPI) -> AsyncIterator[None]:
    interval = get_settings().stale_session_sweep_interval_seconds
    stop = asyncio.Event()
    task: asyncio.Task[None] | None = None

    async def loop() -> None:
        while True:
            try:
                await asyncio.wait_for(stop.wait(), timeout=interval)
                return
            except asyncio.TimeoutError:
                await asyncio.to_thread(_sweep_stale_sessions_once)

    if interval > 0:
        task = asyncio.create_task(loop())
    yield
    stop.set()
    if task is not None:
        await task


def create_app() -> FastAPI:
    get_settings()
    app = FastAPI(title="Matchmaker", version="0.1.0", lifespan=lifespan)
    app.add_middleware(
        CORSMiddleware,
        allow_origins=[
            "http://localhost:5173",
            "http://127.0.0.1:5173",
            "http://localhost:3000",
            "http://127.0.0.1:3000",
        ],
        allow_credentials=True,
        allow_methods=["*"],
        allow_headers=["*"],
    )
    app.include_router(auth.router, prefix="/v1/auth", tags=["auth"])
    app.include_router(hosts.router, prefix="/v1", tags=["hosts"])
    app.include_router(images.router, prefix="/v1", tags=["images"])
    app.include_router(sessions.router, prefix="/v1", tags=["sessions"])
    app.include_router(storage.router, prefix="/v1", tags=["storage"])
    return app


app = create_app()
