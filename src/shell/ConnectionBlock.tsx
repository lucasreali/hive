import type { ReactNode } from "react";
import { connect } from "../connect";
import { useHive } from "../store";

// How the service is installed (#29). `pkill` ends an old service still waiting for an app:
// a refused handshake does not stop it.
const INSTALL = "cargo install --path crates/hive";
const STOP = "pkill -f 'hive daemon'";

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

/** Covers the workspace while there is no usable service connection (#29). */
export function ConnectionBlock() {
  const c = useHive((s) => s.connection);
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
        <p>
          Build both from the same commit of the Hive repository. To reinstall the service, run in
          WSL from the repository:
        </p>
        <pre>{`${INSTALL}\n${STOP}`}</pre>
        <p>Then reconnect, or restart Hive.</p>
      </Dialog>
    );
  }
  if (c.status === "disconnected") {
    return (
      <Dialog title="Lost the connection to the hive service">
        <pre>{c.reason}</pre>
        <p>
          Reconnect, or restart Hive. If it keeps failing, check that <code>hive</code> is installed
          in WSL: <code>{INSTALL}</code>
        </p>
      </Dialog>
    );
  }
  return null;
}
