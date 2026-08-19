export const BYTES_PER_GIB = 1024 ** 3;

export function toGib(bytes: number): number {
  return bytes / BYTES_PER_GIB;
}

export function formatGib(bytes: number): string {
  const gib = toGib(bytes);
  const decimals = gib > 0 && gib < 10 ? 1 : 0;

  return `${gib.toFixed(decimals)} GiB`;
}

export function formatCores(count: number): string {
  return `${count} ${count === 1 ? "core" : "cores"}`;
}
