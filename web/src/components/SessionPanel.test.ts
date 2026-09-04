import { describe, expect, it } from "vitest";

import { isLiveKasmUrl } from "./SessionPanel";

describe("isLiveKasmUrl", () => {
  it("rejects RFC 2606 .invalid placeholders (seed and provision stubs)", () => {
    expect(isLiveKasmUrl("https://example.invalid/kasm")).toBe(false);
    expect(isLiveKasmUrl("http://example.invalid/desktop")).toBe(false);
    expect(isLiveKasmUrl("https://provision.invalid/firecracker")).toBe(false);
    expect(isLiveKasmUrl("https://provision.invalid/qemu-whpx")).toBe(false);
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
