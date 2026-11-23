import { describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";

import { kitFilename } from "../api";
import { SharePcPanel } from "./SharePcPanel";

describe("SharePcPanel", () => {
  it("offers only the kits the server can make", () => {
    const html = renderToStaticMarkup(
      <SharePcPanel
        kits={[
          { os: "windows", available: true },
          { os: "linux", available: false },
        ]}
        busy={null}
        message={null}
        onDownload={vi.fn()}
      />,
    );
    expect(html).toContain("Download for Windows");
    expect(html).not.toContain("Download for Linux");
    expect(html).toContain("Run anyway");
  });

  it("says when no download is available", () => {
    const html = renderToStaticMarkup(
      <SharePcPanel kits={[]} busy={null} message={null} onDownload={vi.fn()} />,
    );
    expect(html).toContain("not available yet");
    expect(html).not.toContain("<button");
  });

  it("shows progress and errors", () => {
    const html = renderToStaticMarkup(
      <SharePcPanel
        kits={[{ os: "linux", available: true }]}
        busy="linux"
        message="Download failed (503)"
        onDownload={vi.fn()}
      />,
    );
    expect(html).toContain("Preparing");
    expect(html).toContain("Download failed (503)");
  });
});

describe("kitFilename", () => {
  it("reads the server's file name, with a fallback", () => {
    const named = new Response("", {
      headers: { "Content-Disposition": 'attachment; filename="dcl-host-setup-windows.zip"' },
    });
    expect(kitFilename(named, "windows")).toBe("dcl-host-setup-windows.zip");
    expect(kitFilename(new Response(""), "linux")).toBe("dcl-host-setup-linux.zip");
  });
});
