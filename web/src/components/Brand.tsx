import { routeHref } from "../router";

/** A screen with a second, smaller screen inside it: a computer you open from yours. */
export function BrandMark({ size = 22 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <rect x="2.5" y="3.5" width="19" height="13" rx="2.5" stroke="currentColor" strokeWidth="1.6" />
      <rect x="11" y="8" width="7" height="5" rx="1" fill="var(--share)" />
      <path d="M8 20.5h8" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
    </svg>
  );
}

export function Brand() {
  return (
    <a className="brand" href={routeHref("home")}>
      <BrandMark />
      <span>Blue Sky</span>
    </a>
  );
}
