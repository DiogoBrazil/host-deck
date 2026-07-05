// Browser-side adapter between xterm.js and the Tauri SSH commands.
import { Terminal } from "/vendor/xterm/xterm.mjs";
import { FitAddon } from "/vendor/xterm/addon-fit.mjs";

const core = () => window.__TAURI__.core;

function b64ToBytes(b64) {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

/**
 * Creates the terminal, starts the SSH session, and bridges terminal I/O.
 * @param {string} containerId terminal container element id
 * @param {string} connectionId backend connection id
 * @param {(status:string, detail?:string)=>void} onStatus status callback
 * @returns handle with dispose()/disconnect()
 */
export async function startTerminal(containerId, connectionId, onStatus) {
  const container = document.getElementById(containerId);
  if (!container) throw new Error(`container #${containerId} não encontrado`);

  const term = new Terminal({
    fontSize: 13,
    fontFamily: '"JetBrains Mono", "Cascadia Code", Menlo, monospace',
    cursorBlink: true,
    scrollback: 5000,
    theme: { background: "#0f1115", foreground: "#e6e9ef" },
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  term.open(container);
  fit.fit();

  // The frontend owns the session id so it can answer host-key prompts while
  // ssh_connect is still pending.
  const sessionId = crypto.randomUUID();
  let closed = false;

  // Tauri Channel preserves SSH output and lifecycle event ordering.
  const Channel = core().Channel;
  const channel = new Channel();
  channel.onmessage = (msg) => {
    switch (msg.event) {
      case "output":
        term.write(b64ToBytes(msg.data.data));
        break;
      case "hostKeyPrompt":
        onStatus("hostKeyPrompt", JSON.stringify(msg.data));
        break;
      case "connected":
        onStatus("connected");
        break;
      case "closed":
        closed = true;
        onStatus("closed", msg.data.reason);
        break;
      case "error":
        onStatus("error", msg.data.message);
        break;
    }
  };

  const { cols, rows } = term;

  term.onData((data) => {
    if (closed) return;
    core().invoke("ssh_send_data", { sessionId, data });
  });

  let resizeTimer = null;
  const doResize = () => {
    fit.fit();
    if (!closed) {
      core().invoke("ssh_resize", { sessionId, cols: term.cols, rows: term.rows });
    }
  };
  const observer = new ResizeObserver(() => {
    clearTimeout(resizeTimer);
    resizeTimer = setTimeout(doResize, 100);
  });
  observer.observe(container);

  // This promise can stay pending while the backend waits for TOFU confirmation.
  core()
    .invoke("ssh_connect", { sessionId, connectionId, cols, rows, onEvent: channel })
    .then(() => onStatus("connected"))
    .catch((err) =>
      onStatus("error", typeof err === "string" ? err : JSON.stringify(err)),
    );

  term.focus();

  return {
    getSessionId: () => sessionId,
    confirmHostKey: (accept) => {
      core().invoke("confirm_host_key", { sessionId, accept });
    },
    disconnect: () => {
      if (!closed) core().invoke("ssh_disconnect", { sessionId });
    },
    dispose: () => {
      observer.disconnect();
      clearTimeout(resizeTimer);
      if (!closed) core().invoke("ssh_disconnect", { sessionId });
      term.dispose();
    },
  };
}
