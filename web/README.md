# Lab console (Matchmaker harness)

A small Vite + React + TypeScript page whose only job is to exercise the
Phase 1 Matchmaker API against a Phase 0 Kasm session that is already
running on the host PC. It is **not** the product: no intent prompt, no
billing, no Kasm iframe (the desktop opens in a new tab).

## File structure

```
web/
├── .env.example          VITE_MATCHMAKER_URL
├── index.html
├── package.json
├── src/
│   ├── api.ts            fetch helpers; no hardcoded host
│   ├── api.test.ts
│   ├── sessionRequest.ts 201 / 200 / 409 mapping for the panel
│   ├── App.tsx           JWT in React state only (not localStorage)
│   ├── types.ts
│   ├── styles.css
│   └── components/
│       ├── AuthForm.tsx
│       └── SessionPanel.tsx
└── vite.config.ts        port 5173 (already on the Matchmaker CORS list)
```

## UI states

1. **Auth** — signup or login. Token lives in memory.
2. **Ready** — logged in, no session; "Request session".
3. **Held** — 201 (new) or 200 (you already have one). **Open desktop** if
   `kasmUrl` is a live `http(s)` URL (not `example.invalid`). Token is shown,
   not stuffed into the URL.
4. **Occupied** — 409; another user holds the host. Specific copy, not a
   generic error.
5. **Ended** — after `POST /v1/sessions/{id}/end`, back to Ready.

## Manual steps

The Matchmaker must already be running. From `matchmaker/` (see that
README): Docker Postgres, `.env`, `alembic upgrade head`,
`python -m matchmaker.seed`, then:

```bash
uvicorn matchmaker.main:app --reload --app-dir src
```

Then this app:

```bash
cd web
cp .env.example .env   # points at http://127.0.0.1:8000
npm install
npm run dev
```

Open http://127.0.0.1:5173 — CORS allows that origin and `localhost:5173`.
Change `VITE_MATCHMAKER_URL` and restart Vite when the API is not local.
The app will not start against a missing env var; there is no localhost
fallback in code.

## Tests

```bash
npm test
npm run typecheck
```

Vitest covers `isLiveKasmUrl`, occupied 409 vs existing-session 200, and
Matchmaker error mapping (`detail` string, validation array, network failure).
These are unit tests with a mocked `fetch`; they do not start Postgres.
