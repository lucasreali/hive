/** Makes the WebView look like macOS' (WKWebView's user agent) until the test ends (setup.ts). */
export function asMac(): void {
  Object.defineProperty(navigator, "userAgent", {
    value:
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)",
    configurable: true,
  });
}
