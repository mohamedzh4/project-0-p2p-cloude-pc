import { describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";

import { isLiveKasmUrl } from "../connection";
import type { SessionResponse } from "../types";
import { SessionPanel } from "./SessionPanel";

function session(overrides: Partial<SessionResponse> = {}): SessionResponse {
  return {
    sessionId: "11111111-1111-1111-1111-111111111111",
    hostId: "22222222-2222-2222-2222-222222222222",
    labTier: "standard",
    startedAt: "2026-09-07T12:00:00.000Z",
    connection: {
      streamer: "kasm",
      kasmUrl: "https://example.invalid/kasm",
      kasmToken: "seed",
    },
    ...overrides,
  };
}

const panelBase = {
  user: { id: "user-1", email: "user@test.example" },
  notice: null,
  busy: false,
  intent: "Write code and browse the web",
  onIntent: vi.fn(),
  onRequest: vi.fn(),
  onEnd: vi.fn(),
  onSignOut: vi.fn(),
};

function renderPanel(current: SessionResponse | null): string {
  return renderToStaticMarkup(
    <SessionPanel {...panelBase} session={current} />,
  );
}

describe("isLiveKasmUrl", () => {
  it("rejects RFC 2606 .invalid placeholders (seed and provision stubs)", () => {
    expect(isLiveKasmUrl("https://example.invalid/kasm")).toBe(false);
    expect(isLiveKasmUrl("http://example.invalid/desktop")).toBe(false);
    expect(isLiveKasmUrl("https://provision.invalid/firecracker")).toBe(false);
    expect(isLiveKasmUrl("https://provision.invalid/qemu-whpx")).toBe(false);
    expect(isLiveKasmUrl("https://provision.invalid/sunshine")).toBe(false);
    expect(isLiveKasmUrl("https://provision.invalid/qemu-vfio")).toBe(false);
    expect(isLiveKasmUrl("https://provision.invalid/hyperv-gpu")).toBe(false);
  });

  it("accepts http(s) URLs that are not the placeholder", () => {
    expect(isLiveKasmUrl("https://kasm.lab.example/desktop")).toBe(true);
    expect(isLiveKasmUrl("http://172.16.0.2/")).toBe(true);
    expect(isLiveKasmUrl("https://192.168.1.20:8443/#/")).toBe(true);
  });

  it("rejects non-http schemes and unparseable strings", () => {
    expect(isLiveKasmUrl("ftp://example.com/kasm")).toBe(false);
    expect(isLiveKasmUrl("javascript:alert(1)")).toBe(false);
    expect(isLiveKasmUrl("not a url")).toBe(false);
    expect(isLiveKasmUrl("")).toBe(false);
  });

  it("does not treat a lookalike hostname as the placeholder", () => {
    expect(isLiveKasmUrl("https://example.invalid.evil.example/kasm")).toBe(true);
  });
});

describe("SessionPanel live locator", () => {
  it("hides Open desktop when the locator is .invalid", () => {
    const html = renderPanel(session());
    expect(html).not.toContain('class="open-link"');
    expect(html).not.toContain("Open Sunshine web UI");
    expect(html).not.toContain("<iframe");
    expect(html).toContain("data-live=\"false\"");
  });

  it("renders a clickable Open desktop link for a live Kasm mesh URL", () => {
    const html = renderPanel(
      session({
        connection: {
          streamer: "kasm",
          kasmUrl: "https://dcl-guest.test.example/",
          kasmToken: "token",
        },
      }),
    );
    expect(html).toContain("Open desktop");
    expect(html).toContain('href="https://dcl-guest.test.example/"');
    expect(html).toContain("data-live=\"true\"");
    expect(html).not.toContain("<iframe");
  });

  it("renders a clickable Sunshine web UI link and Moonlight host for a live Pro locator", () => {
    const html = renderPanel(
      session({
        labTier: "pro",
        connection: {
          streamer: "sunshine",
          kasmUrl: null,
          sunshineUrl: "https://dcl-guest.test.example:47990/",
          moonlight: { host: "dcl-guest.test.example", app: "Desktop" },
          gpuExclusive: true,
        },
      }),
    );
    expect(html).toContain("Open Sunshine web UI");
    expect(html).toContain('href="https://dcl-guest.test.example:47990/"');
    expect(html).toContain("Moonlight client");
    expect(html).toContain("dcl-guest.test.example");
    expect(html).not.toContain("Open desktop");
    expect(html).not.toContain("<iframe");
  });

  it("does not iframe or link a .invalid Pro Sunshine stub", () => {
    const html = renderPanel(
      session({
        labTier: "pro",
        connection: {
          streamer: "sunshine",
          sunshineUrl: "https://provision.invalid/sunshine",
          moonlight: { host: "provision.invalid", app: "Desktop" },
          gpuExclusive: true,
        },
      }),
    );
    expect(html).not.toContain("Open Sunshine web UI");
    expect(html).not.toContain('class="open-link"');
    expect(html).not.toContain("<iframe");
    expect(html).not.toContain("Moonlight client");
  });

  it("keeps automatic match — no host picker on the request form", () => {
    const html = renderPanel(null);
    expect(html).toContain("Request session");
    expect(html).toContain("auto-matches");
    expect(html).not.toContain("<select");
    expect(html).not.toContain("Browse hosts");
    expect(html).not.toContain("Pick a host");
  });

  it("shows saved-as-of from session storage or the guest-flush copy", () => {
    const pending = renderPanel(
      session({
        storage: {
          provider: "renterd",
          endpoint: "http://renterd.test.invalid",
          region: "us-east-1",
          bucket: "dcl-example",
          prefix: "files/",
          pathStyle: true,
          mountInsideGuest: true,
          rclone: "rclone mount",
          objects: [],
        },
      }),
    );
    expect(pending).toContain("updates inside the guest after rclone flush");
    const stamped = renderPanel(
      session({
        storage: {
          provider: "renterd",
          endpoint: "http://renterd.test.invalid",
          region: "us-east-1",
          bucket: "dcl-example",
          prefix: "files/",
          pathStyle: true,
          mountInsideGuest: true,
          rclone: "rclone mount",
          objects: [],
          savedAsOf: "2026-09-11T21:00:00.000Z",
        },
      }),
    );
    expect(stamped).toContain("2026-09-11T21:00:00.000Z");
    expect(stamped).not.toContain("updates inside the guest after rclone flush");
  });
});
