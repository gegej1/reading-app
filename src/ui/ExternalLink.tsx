import type { ComponentProps } from "react";
import { openExternalLink } from "../app/desktop";

/** True for the links that leave the app: http and https only. */
export function isExternalUrl(href: string): boolean {
  return /^https?:\/\//i.test(href);
}

/**
 * A link that leaves the app. The web build opens it in a new tab; the
 * desktop app hands it to the default browser.
 */
export function ExternalLink({ href, title, children, ...rest }: ComponentProps<"a"> & { href: string }) {
  return (
    <a
      {...rest}
      href={href}
      title={title ?? href}
      target="_blank"
      rel="noopener noreferrer"
      onClick={(event) => openExternalLink(event, href)}
    >
      {children}
    </a>
  );
}
