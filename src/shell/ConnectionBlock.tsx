import type { ReactNode } from "react";
import { connect } from "../connect";
import { useHive } from "../store";
import { transport } from "../transport";
import { isMac } from "../window";

// The installed app brings its own service (4.18), so a mismatch is an old service still
// waiting for an app: a refused handshake does not stop it. Only development builds (`bundled`
// false, decided by the app's Rust side) use `cargo install`; installed users never need it (12.4).
const INSTALL = "cargo install --path crates/hive";
const STOP = "pkill -f 'hive daemon'";
/** The same on Windows itself (12.5.4): every `hive.exe`, the bridge and the old service. */
const STOP_WINDOWS = "taskkill /F /IM hive.exe";

function reconnect() {
  useHive.setState({ connection: { status: "connecting" } });
  void connect();
}

function Dialog({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="connection-block">
      <div role="alertdialog" aria-modal="true" aria-labelledby="connection-title">
        <h2 id="connection-title">{title}</h2>
        {children}
        <div className="actions">
          <button
            type="button"
            className="primary"
            onClick={reconnect}
            // Focus leaves the inert workspace for the one available action.
            ref={(button) => button?.focus()}
          >
            Reconnect
          </button>
        </div>
      </div>
    </div>
  );
}

/**
 * The first run on Windows with WSL (12.5.4): where the service runs. Nothing starts until the
 * user picks; the settings switch it later.
 */
function ModeChoice() {
  return (
    <div className="connection-block">
      <div role="alertdialog" aria-modal="true" aria-labelledby="connection-title">
        <h2 id="connection-title">Where should Hive run?</h2>
        <p>
          Hive's service runs your terminals and agents, in WSL or on Windows itself. Each keeps its
          own projects, spaces and settings. You can switch later in Settings.
        </p>
        <div className="actions">
          <button
            type="button"
            className="secondary"
            onClick={() => void transport.setMode("native")}
          >
            Windows
          </button>
          <button
            type="button"
            className="primary"
            onClick={() => void transport.setMode("wsl")}
            ref={(button) => button?.focus()}
          >
            WSL
          </button>
        </div>
      </div>
    </div>
  );
}

/** Covers the workspace while there is no usable service connection (#29). */
export function ConnectionBlock() {
  const c = useHive((s) => s.connection);
  const mode = useHive((s) => s.appMode?.mode);
  if (mode === null) return <ModeChoice />;
  // On macOS, or in native mode on Windows (12.5.4), the service does not run in WSL.
  const where = isMac() || mode === "native" ? "" : " in WSL";
  const stop = mode === "native" ? STOP_WINDOWS : STOP;
  if (c.status === "version_mismatch") {
    return (
      <Dialog title="The app and the hive service versions differ">
        <dl>
          <dt>App</dt>
          <dd>
            {c.app_version} (protocol {c.app_protocol})
          </dd>
          <dt>Service</dt>
          <dd>
            {c.version} (protocol {c.protocol})
          </dd>
        </dl>
        <p>Hive brings its own service. Stop the old one{where}:</p>
        <pre>{stop}</pre>
        <p>Then reconnect, or restart Hive.</p>
        {c.bundled ? (
          <p>If it keeps happening, reinstall Hive.</p>
        ) : (
          <p>
            A development build runs the service from <code>{INSTALL}</code>: build both from the
            same commit.
          </p>
        )}
      </Dialog>
    );
  }
  if (c.status === "disconnected") {
    return (
      <Dialog title="Lost the connection to the hive service">
        <pre>{c.reason}</pre>
        {c.bundled ? (
          <p>Reconnect, or restart Hive. If it keeps failing, reinstall Hive.</p>
        ) : (
          <p>
            Reconnect, or restart Hive. If it keeps failing, check that <code>hive</code> is
            installed{where}: <code>{INSTALL}</code>
          </p>
        )}
      </Dialog>
    );
  }
  return null;
}
