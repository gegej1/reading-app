import { useMemo, type ComponentProps, type MouseEvent } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { findAnchorTarget } from "./anchors";
import { cx } from "./components";
import { remarkHeadingIds } from "./remarkHeadingIds";
import { remarkWikilinks, type WikilinkTarget } from "./remarkWikilinks";

export type LinkResolver = (target: string) => WikilinkTarget | null;

export interface MarkdownProps {
  children: string;
  resolve: LinkResolver;
  className?: string;
}

const EXTERNAL = /^(?:https?:|mailto:)/i;
const HEADINGS = "h1[id], h2[id], h3[id], h4[id], h5[id], h6[id]";

/**
 * Scrolls to the heading an in-page link (`#小节`) points at. The default
 * action is always cancelled: under the hash router, `location.hash` is the
 * app's route, so following `#小节` would open a page that does not exist.
 */
function scrollToAnchor(event: MouseEvent<HTMLAnchorElement>, fragment: string) {
  event.preventDefault();
  const scope = event.currentTarget.closest(".prose") ?? document;
  const target =
    findAnchorTarget(scope.querySelectorAll<HTMLElement>(HEADINGS), fragment) ??
    findAnchorTarget(document.querySelectorAll<HTMLElement>(HEADINGS), fragment);
  target?.scrollIntoView({ block: "start", behavior: "smooth" });
}

function MarkdownLink({ href, children, title, className, ...rest }: ComponentProps<"a">) {
  if (href && EXTERNAL.test(href)) {
    return (
      <a {...rest} className={className} href={href} title={title ?? href} target="_blank" rel="noreferrer noopener">
        {children}
      </a>
    );
  }
  // `#/…` is an app route (resolved wikilinks); the hash router follows it.
  if (href?.startsWith("#/")) {
    return (
      <a {...rest} className={className} href={href} title={title}>
        {children}
      </a>
    );
  }
  if (href?.startsWith("#")) {
    return (
      <a
        {...rest}
        className={cx(className, "anchor-link")}
        href={href}
        title={title}
        onClick={(event) => scrollToAnchor(event, href.slice(1))}
      >
        {children}
      </a>
    );
  }
  // Relative file links would navigate the webview away from the app.
  return (
    <span className="link--local" title={href}>
      {children}
    </span>
  );
}

const components: Components = {
  a: ({ node: _node, ...props }) => <MarkdownLink {...props} />,
  table: ({ node: _node, ...props }) => (
    <div className="table-wrap">
      <table {...props} />
    </div>
  ),
  input: ({ node: _node, type, checked, ...props }) =>
    type === "checkbox" ? (
      <input
        {...props}
        type="checkbox"
        checked={Boolean(checked)}
        disabled
        readOnly
        className="task-box"
        aria-label={checked ? "已勾选" : "未勾选"}
      />
    ) : (
      <input {...props} type={type} checked={checked} />
    ),
};

/**
 * Renders KB Markdown (GFM + wikilinks). Raw HTML in the source is not
 * rendered. Headings get stable ids so `[文字](#小节)` links can scroll to them.
 */
export function Markdown({ children, resolve, className }: MarkdownProps) {
  const remarkPlugins = useMemo(
    () => [remarkGfm, [remarkWikilinks, { resolve }], remarkHeadingIds] as NonNullable<
      ComponentProps<typeof ReactMarkdown>["remarkPlugins"]
    >,
    [resolve],
  );
  return (
    <div className={className ? `prose ${className}` : "prose"}>
      <ReactMarkdown remarkPlugins={remarkPlugins} components={components}>
        {children}
      </ReactMarkdown>
    </div>
  );
}
