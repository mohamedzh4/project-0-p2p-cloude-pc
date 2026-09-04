export type LabTier = "standard" | "pro";

/**
 * Preset answers for the FR2 prompt. Labels are short; `text` is what we map.
 * Architecture §3.1 Standard = office/browse/code; §3.2 Pro = GPU work.
 */
export const INTENT_PRESETS: { label: string; text: string }[] = [
  { label: "Office / coding", text: "Write code and browse the web" },
  { label: "Game (dedicated GPU)", text: "Play a game with a dedicated GPU" },
  { label: "GPU model training", text: "Train a model on a GPU" },
];

/** Visible whenever the mapped tier is Pro — not a concurrent CPU/RAM slice. */
export const PRO_GPU_EXCLUSIVITY =
  "Pro assigns one GPU exclusively to the guest. The host loses that GPU until the session ends — not a concurrent CPU/RAM slice.";

export const STANDARD_SLICE_HINT =
  "Standard is a CPU/RAM slice. The host keeps using the rest of the machine, including their GPU.";

/**
 * GPU / game / 3D / training phrases → pro. Everything else, including empty
 * input, → standard (Matchmaker's POST default). Word boundaries avoid
 * "python"→pytorch and "community"→unity.
 */
const PRO_PATTERNS: RegExp[] = [
  /\bgpus?\b/i,
  /\bcuda\b/i,
  /\bnvidia\b/i,
  /\bvram\b/i,
  /\bgamer\b/i,
  /\bgaming\b/i,
  /\bgames?\b/i,
  /\bblender\b/i,
  /\b3d\b/i,
  /\brender(?:ing)?\b/i,
  /\bpytorch\b/i,
  /\btensorflow\b/i,
  /\bllms?\b/i,
  /\bmachine learning\b/i,
  /\bdeep learning\b/i,
  /\bstable diffusion\b/i,
  /\bcomfyui\b/i,
  /\bcad\b/i,
  /\bunreal\b/i,
  /\bunity\b/i,
  /\bpassthrough\b/i,
  /\bvfio\b/i,
  /\bvideo edit(?:ing)?\b/i,
  /\bvideo encod/i,
  /\bpro lab\b/i,
  /\bgamer lab\b/i,
  /\bexclusive gpu\b/i,
  /\bdedicated gpu\b/i,
];

export function normalizeIntent(intent: string): string {
  return intent.trim().replace(/\s+/g, " ");
}

export function canRequestSession(intent: string): boolean {
  return normalizeIntent(intent).length > 0;
}

export function mapIntentToLabTier(intent: string): LabTier {
  const text = normalizeIntent(intent).toLowerCase();
  if (!text) {
    return "standard";
  }
  return PRO_PATTERNS.some((pattern) => pattern.test(text)) ? "pro" : "standard";
}

export function gpuExclusivityVisible(tier: LabTier): boolean {
  return tier === "pro";
}
