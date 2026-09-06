import { describe, expect, it } from "vitest";

import {
  INTENT_PRESETS,
  canRequestSession,
  gpuExclusivityVisible,
  mapIntentToLabTier,
  normalizeIntent,
} from "./intent";

describe("mapIntentToLabTier", () => {
  it("maps empty and whitespace to standard (POST default)", () => {
    expect(mapIntentToLabTier("")).toBe("standard");
    expect(mapIntentToLabTier("   \n\t  ")).toBe("standard");
  });

  it("maps office / coding / browsing copy to standard", () => {
    expect(mapIntentToLabTier("Write code and browse the web")).toBe("standard");
    expect(mapIntentToLabTier("Office work and documents")).toBe("standard");
    expect(mapIntentToLabTier("I want to program in Python")).toBe("standard");
    expect(mapIntentToLabTier("Light computing on a laptop")).toBe("standard");
  });

  it("maps GPU, game, 3D, and training copy to pro", () => {
    expect(mapIntentToLabTier("Play a game with a dedicated GPU")).toBe("pro");
    expect(mapIntentToLabTier("Train a model on a GPU")).toBe("pro");
    expect(mapIntentToLabTier("gaming tonight")).toBe("pro");
    expect(mapIntentToLabTier("Blender 3D rendering")).toBe("pro");
    expect(mapIntentToLabTier("CUDA / PyTorch training")).toBe("pro");
    expect(mapIntentToLabTier("video editing")).toBe("pro");
    expect(mapIntentToLabTier("Pro lab")).toBe("pro");
    expect(mapIntentToLabTier("Need exclusive GPU")).toBe("pro");
  });

  it("does not treat lookalike words as Pro keywords", () => {
    expect(mapIntentToLabTier("python scripts")).toBe("standard");
    expect(mapIntentToLabTier("community notes")).toBe("standard");
    expect(mapIntentToLabTier("cascade a task")).toBe("standard");
  });

  it("maps every preset text to the tier its label implies", () => {
    const byLabel = Object.fromEntries(INTENT_PRESETS.map((preset) => [preset.label, preset.text]));
    expect(mapIntentToLabTier(byLabel["Office / coding"] ?? "")).toBe("standard");
    expect(mapIntentToLabTier(byLabel["Game (dedicated GPU)"] ?? "")).toBe("pro");
    expect(mapIntentToLabTier(byLabel["GPU model training"] ?? "")).toBe("pro");
  });
});

describe("canRequestSession", () => {
  it("requires non-empty intent before booking", () => {
    expect(canRequestSession("")).toBe(false);
    expect(canRequestSession("  ")).toBe(false);
    expect(canRequestSession("code")).toBe(true);
  });
});

describe("gpuExclusivityVisible", () => {
  it("is only true for Pro", () => {
    expect(gpuExclusivityVisible("standard")).toBe(false);
    expect(gpuExclusivityVisible("pro")).toBe(true);
  });
});

describe("normalizeIntent", () => {
  it("trims and collapses whitespace", () => {
    expect(normalizeIntent("  play   a   game  ")).toBe("play a game");
  });
});
