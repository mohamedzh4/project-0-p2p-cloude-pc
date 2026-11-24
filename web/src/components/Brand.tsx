import { routeHref } from "../router";
import { PixelIcon } from "./PixelIcon";

export function Brand() {
  return (
    <a className="brand" href={routeHref("home")}>
      <PixelIcon name="cloud" size={22} />
      <span>Blue Sky</span>
    </a>
  );
}
