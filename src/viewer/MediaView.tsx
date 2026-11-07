import type { FileMedia } from "../protocol";

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
