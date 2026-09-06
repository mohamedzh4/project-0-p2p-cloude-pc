import { describe, expect, it } from "vitest";

import {
  isLiveConnectionUrl,
  isLiveKasmUrl,
  sessionLocator,
  sessionStreamer,
} from "./connection";
import type { SessionResponse } from "./types";

function session(overrides: Partial<SessionResponse> = {}): SessionResponse {
  return {
    sessionId: "11111111-1111-1111-1111-111111111111",
    hostId: "22222222-2222-2222-2222-222222222222",
    labTier: "standard",
    startedAt: "2026-09-05T12:00:00.000Z",
    connection: {
      streamer: "kasm",
      kasmUrl: "https://kasm.test.example/session",
      kasmToken: "test-kasm-token",
    },
    ...overrides,
  };
}

describe("isLiveConnectionUrl", () => {
  it("rejects RFC 2606 .invalid Sunshine stubs the same as Kasm", () => {
    expect(isLiveConnectionUrl("https://provision.invalid/sunshine")).toBe(false);
    expect(isLiveConnectionUrl("https://provision.invalid/qemu-vfio")).toBe(false);
    expect(isLiveKasmUrl("https://provision.invalid/firecracker")).toBe(false);
  });

  it("treats a missing locator as not live", () => {
    expect(isLiveConnectionUrl(null)).toBe(false);
    expect(isLiveConnectionUrl(undefined)).toBe(false);
  });
});

describe("sessionStreamer / sessionLocator", () => {
  it("uses kasmUrl for Standard", () => {
    const current = session();
    expect(sessionStreamer(current)).toBe("kasm");
    expect(sessionLocator(current)).toBe("https://kasm.test.example/session");
  });

  it("uses sunshineUrl for Pro, not kasmUrl", () => {
    const current = session({
      labTier: "pro",
      connection: {
        streamer: "sunshine",
        kasmUrl: null,
        sunshineUrl: "https://provision.invalid/sunshine",
        moonlight: { host: "provision.invalid", app: "Desktop" },
        gpuExclusive: true,
      },
    });
    expect(sessionStreamer(current)).toBe("sunshine");
    expect(sessionLocator(current)).toBe("https://provision.invalid/sunshine");
  });
});
