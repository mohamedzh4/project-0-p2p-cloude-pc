import { describe, expect, it } from "vitest";

import {
  hostnameIsPlaceholder,
  isLiveConnectionUrl,
  isLiveKasmUrl,
  sessionDesktopLink,
  sessionLocator,
  sessionMoonlightClient,
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
    expect(isLiveKasmUrl("https://provision.invalid/qemu-kvm")).toBe(false);
    expect(isLiveConnectionUrl("https://example.invalid/kasm")).toBe(false);
  });

  it("treats a missing locator as not live", () => {
    expect(isLiveConnectionUrl(null)).toBe(false);
    expect(isLiveConnectionUrl(undefined)).toBe(false);
  });

  it("accepts a public gateway locator (https origin + /s/<token>/)", () => {
    expect(
      isLiveConnectionUrl(
        "https://gateway.test.example/s/abcdefghijklmnopqrstuvwxyz012345/",
      ),
    ).toBe(true);
  });

  it("accepts phase-31 Kasm mesh locators (https host, path /)", () => {
    expect(isLiveConnectionUrl("https://dcl-guest-aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa/")).toBe(
      true,
    );
    expect(isLiveConnectionUrl("https://dcl-guest.test.example/")).toBe(true);
    expect(isLiveConnectionUrl("https://100.64.0.12/")).toBe(true);
  });

  it("accepts phase-31 Sunshine locators on port 47990", () => {
    expect(
      isLiveConnectionUrl("https://dcl-guest-aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa:47990/"),
    ).toBe(true);
    expect(isLiveConnectionUrl("https://dcl-guest.test.example:47990/")).toBe(true);
    expect(isLiveConnectionUrl("https://127.0.0.1:47990/")).toBe(true);
  });
});

describe("hostnameIsPlaceholder", () => {
  it("treats .invalid and empty as placeholders", () => {
    expect(hostnameIsPlaceholder("provision.invalid")).toBe(true);
    expect(hostnameIsPlaceholder("example.invalid.")).toBe(true);
    expect(hostnameIsPlaceholder("")).toBe(true);
    expect(hostnameIsPlaceholder("dcl-guest.test.example")).toBe(false);
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

describe("sessionDesktopLink", () => {
  it("returns a clickable Open desktop href for a live Standard Kasm locator", () => {
    const current = session({
      connection: {
        streamer: "kasm",
        kasmUrl: "https://dcl-guest.test.example/",
        kasmToken: "token",
      },
    });
    expect(sessionDesktopLink(current)).toEqual({
      href: "https://dcl-guest.test.example/",
      label: "Open desktop",
      kind: "kasm",
    });
  });

  it("hides the Open desktop link for .invalid Standard stubs", () => {
    const current = session({
      connection: {
        streamer: "kasm",
        kasmUrl: "https://example.invalid/kasm",
        kasmToken: "seed",
      },
    });
    expect(sessionDesktopLink(current)).toBeNull();
  });

  it("returns a clickable Sunshine web UI href for a live Pro locator", () => {
    const current = session({
      labTier: "pro",
      connection: {
        streamer: "sunshine",
        kasmUrl: null,
        sunshineUrl: "https://dcl-guest.test.example:47990/",
        moonlight: { host: "dcl-guest.test.example", app: "Desktop" },
        gpuExclusive: true,
      },
    });
    expect(sessionDesktopLink(current)).toEqual({
      href: "https://dcl-guest.test.example:47990/",
      label: "Open Sunshine web UI",
      kind: "sunshine",
    });
  });

  it("hides the Sunshine link for .invalid Pro stubs", () => {
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
    expect(sessionDesktopLink(current)).toBeNull();
  });
});

describe("sessionMoonlightClient", () => {
  it("shows Moonlight host/app when the Pro locator is live", () => {
    const current = session({
      labTier: "pro",
      connection: {
        streamer: "sunshine",
        sunshineUrl: "https://dcl-guest.test.example:47990/",
        moonlight: { host: "dcl-guest.test.example", app: "Desktop" },
        gpuExclusive: true,
      },
    });
    expect(sessionMoonlightClient(current)).toEqual({
      host: "dcl-guest.test.example",
      app: "Desktop",
    });
  });

  it("does not present a placeholder Moonlight host as a real client", () => {
    const current = session({
      labTier: "pro",
      connection: {
        streamer: "sunshine",
        sunshineUrl: "https://provision.invalid/sunshine",
        moonlight: { host: "provision.invalid", app: "Desktop" },
        gpuExclusive: true,
      },
    });
    expect(sessionMoonlightClient(current)).toBeNull();
  });

  it("is absent for Standard Kasm sessions", () => {
    expect(sessionMoonlightClient(session())).toBeNull();
  });
});
