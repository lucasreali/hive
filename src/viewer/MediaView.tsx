import type { FileMedia } from "../protocol";

/** The service's type of an HTML page (`hive_protocol::HTML_TYPE`, 24.4). */
const HTML = "text/html; charset=utf-8";

/** An HTML page (24.4): shown as its text, with a Preview, not as media. */
export const isPage = (media: FileMedia | null | undefined) => media?.mime === HTML;

/**
 * An image or a PDF of a worktree (24.2, 24.3), from the URL the service's `media` gives: an
 * image (an SVG too, so its scripts never run) as an `<img>` fitted to the view, a PDF in a
 * plain iframe, drawn by the WebView's own viewer (it draws none in a sandboxed one, 24.1).
 */
export function MediaView({ media, path }: { media: FileMedia; path: string }) {
  if (media.mime === "application/pdf") {
    return <iframe className="media-view" title={path} src={media.url} />;
  }
  return (
    <div className="media-view hive-scroll">
      <img alt={path} src={media.url} />
    </div>
  );
}

/**
 * An HTML page's Preview (24.4): the file on disk with its relative files, through the scheme,
 * in an iframe sandboxed without `allow-same-origin` (origin `null`): its scripts run but never
 * reach the app, and the scheme's CSP keeps them off the network (no `fetch`, even of the
 * worktree's files).
 */
export function PagePreview({ media, path }: { media: FileMedia; path: string }) {
  return (
    <iframe
      className="media-view page-frame"
      sandbox="allow-scripts"
      title={path}
      src={media.url}
    />
  );
}
