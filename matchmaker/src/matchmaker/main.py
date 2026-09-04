from __future__ import annotations

from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware

from matchmaker.config import get_settings
from matchmaker.routers import auth, hosts, sessions


def create_app() -> FastAPI:
    get_settings()
    app = FastAPI(title="Matchmaker", version="0.1.0")
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
    app.include_router(sessions.router, prefix="/v1", tags=["sessions"])
    return app


app = create_app()
