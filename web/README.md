# Lab console (Matchmaker harness)

A small Vite + React + TypeScript page whose only job is to exercise the
Matchmaker API (auth, FR2 intent → `labTier`, session book/end, session
history + credits, locators-only storage, mesh hostname/tags/exit-node). It
is **not** the product: no payment processor, no Kasm iframe (a live desktop
would open in a new tab). Seed and mock-provision URLs use the `.invalid`
TLD. GATE-FC / GATE-WHPX / GATE-PRO are operator-declared closed (2026-09-05);
Open desktop stays hidden until a live URL exists (`SEED_HOST_KASM_URL`
remains `https://example.invalid/kasm`). Preauth keys are not shown.

## File structure

```
web/
├── .env.example          VITE_MATCHMAKER_URL
├── index.html
├── package.json
├── src/
│   ├── api.ts            fetch helpers; no hardcoded host
│   ├── api.test.ts
│   ├── intent.ts         FR2: free-text / presets → standard | pro
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
2. **Intent** — logged in, no session. Prompt: "What do you want to do in
   your lab today?" Maps to `labTier` (`standard` / `pro`). Pro shows GPU
   exclusivity copy before booking. Intent text is **not** sent to the API.
3. **Held** — 201 (new) or 200 (you already have one). **Open desktop** if
   `kasmUrl` is a live `http(s)` URL whose host is not RFC 2606 `.invalid`
   (seed `example.invalid` and stub `provision.invalid` are placeholders).
   Token is shown, not stuffed into the URL. Pro sessions keep the exclusivity
   note. Held sessions also list guest/user mesh hostnames, ACL tags, and
   whether `--exit-node` was applied — not the preauth secret.
4. **History** — `GET /v1/sessions` under the booking panel: past sessions and
   lifetime credits on the internal ledger (not a card charge). `status` is
   `active`, `ended`, or `interrupted` (host went silent ~45s).
4. **Occupied** — 409 on Standard; another user holds the host.
5. **Unavailable** — 409 on Pro; no approved live Pro host (or GPU already
   assigned). Expected while inventory is Standard-only.
6. **Ended** — after `POST /v1/sessions/{id}/end`, back to Intent.

## Manual steps

The Matchmaker must already be running (host inventory, heartbeats, mock
provision). From `matchmaker/` (see that README): Docker Postgres, `.env`
with host tokens, `alembic upgrade head`, `python -m matchmaker.seed`, then:

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

Vitest covers intent → `labTier` mapping, live vs `.invalid` locators
(Kasm and Sunshine stubs), occupied 409 vs existing-session 200 vs Pro 409, and
Matchmaker error mapping (`detail` string, validation array, network
failure). These are unit tests with a mocked `fetch`; they do not start
Postgres.
