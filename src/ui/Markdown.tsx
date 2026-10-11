import { useMemo, type ComponentProps } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { remarkWikilinks, type WikilinkTarget } from "./remarkWikilinks";

export type LinkResolver = (target: string) => WikilinkTarget | null;

export interface MarkdownProps {
  children: string;
  resolve: LinkResolver;
  className?: string;
}

const EXTERNAL = /^(?:https?:|mailto:)/i;

function MarkdownLink({ href, children, title, ...rest }: ComponentProps<"a">) {
  if (href && EXTERNAL.test(href)) {
    return (
      <a {...rest} href={href} title={title ?? href} target="_blank" rel="noreferrer noopener">
        {children}
      </a>
    );
  }
  if (href?.startsWith("#")) {
    return (
      <a {...rest} href={href} title={title}>
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

/** Renders KB Markdown (GFM + wikilinks). Raw HTML in the source is not rendered. */
export function Markdown({ children, resolve, className }: MarkdownProps) {
  const remarkPlugins = useMemo(
    () => [remarkGfm, [remarkWikilinks, { resolve }]] as NonNullable<
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
