import { afterEach, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";
import { MediaView } from "./MediaView";

afterEach(cleanup);

const media = (mime: string) => ({
  mime,
  token: "t",
  version: "1-2",
  url: "hive-file://localhost/t/a?v=1-2",
});

test("an image shows as an img, an SVG too, so no script of it runs", () => {
  for (const mime of ["image/png", "image/svg+xml"]) {
    const { container, unmount } = render(<MediaView media={media(mime)} path="art/a" />);
    const img = container.querySelector(".media-view img");
    expect(img?.getAttribute("src")).toBe("hive-file://localhost/t/a?v=1-2");
    expect(img?.getAttribute("alt")).toBe("art/a");
    expect(container.querySelector("iframe")).toBeNull();
    unmount();
  }
});

test("a PDF shows in a plain iframe, which the WebView draws it in", () => {
  const { container } = render(<MediaView media={media("application/pdf")} path="doc.pdf" />);
  const frame = container.querySelector("iframe.media-view");
  expect(frame?.getAttribute("src")).toBe("hive-file://localhost/t/a?v=1-2");
  expect(frame?.getAttribute("title")).toBe("doc.pdf");
  expect(frame?.hasAttribute("sandbox")).toBe(false);
  expect(container.querySelector("img")).toBeNull();
});
