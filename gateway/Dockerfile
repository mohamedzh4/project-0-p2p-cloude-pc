FROM python:3.12-slim

WORKDIR /app
COPY pyproject.toml /app/pyproject.toml
COPY src /app/src
RUN pip install --no-cache-dir .

# Loopback only. Caddy on the same Linux VPS terminates public TLS.
EXPOSE 7843
CMD ["uvicorn", "dcl_gateway.main:app", "--host", "127.0.0.1", "--port", "7843"]
