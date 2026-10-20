import { XIcon } from "@phosphor-icons/react";
import { type InputHTMLAttributes, type ReactNode, useRef } from "react";
import { ICON } from "./icons";

type Props = Omit<InputHTMLAttributes<HTMLInputElement>, "value" | "onChange" | "type"> & {
  value: string;
  onChange: (value: string) => void;
  /** Drawn before the input, e.g. the Files panel's magnifying glass. */
  children?: ReactNode;
};

/**
 * A search box with the app's own clear button (14.5): the WebView's native one is hidden in
 * styles.css. The button shows only with text; it empties the field and leaves the focus in
 * it. Esc clears too: Chromium's `type="search"` does it natively, WebKit (macOS) does not
 * (17.3); the key still goes on, as it does natively.
 */
export function SearchField({ value, onChange, children, ...input }: Props) {
  const field = useRef<HTMLInputElement>(null);
  return (
    <label className="files-search-field">
      {children}
      <input
        {...input}
        ref={field}
        type="search"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") onChange("");
          input.onKeyDown?.(e);
        }}
      />
      {value && (
        <button
          type="button"
          className="search-clear"
          aria-label="Clear search"
          title="Clear search"
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => {
            onChange("");
            field.current?.focus();
          }}
        >
          <XIcon {...ICON} />
        </button>
      )}
    </label>
  );
}
