// Browser-side adapter for the SFTP session lifecycle and native file dialogs.
// Directory listing and file operations are invoked directly from WASM
// (see src/sftp_api.rs); this glue owns the event Channel and the dialogs.

const core = () => window.__TAURI__.core;

function errText(err) {
  if (typeof err === "string") return err;
  if (err && typeof err.data === "string") return err.data;
  try {
    return JSON.stringify(err);
  } catch {
    return "erro desconhecido";
  }
}

/**
 * Starts an SFTP session and bridges its events.
 *
 * The invoke promise can stay pending during the TOFU host-key prompt, so the
 * handle is returned immediately; connection outcome arrives via onEvent.
 *
 * @param {string} connectionId backend connection id
 * @param {(event:string, detail?:string)=>void} onEvent event callback
 * @returns handle with getSessionId()/confirmHostKey()/disconnect()/dispose()
 */
export async function startSftp(connectionId, onEvent) {
  // The frontend owns the session id so it can answer host-key prompts while
  // sftp_connect is still pending.
  const sessionId = crypto.randomUUID();
  let closed = false;

  const Channel = core().Channel;
  const channel = new Channel();
  channel.onmessage = (msg) => {
    switch (msg.event) {
      case "connected":
        onEvent("connected");
        break;
      case "hostKeyPrompt":
        onEvent("hostKeyPrompt", JSON.stringify(msg.data));
        break;
      case "progress":
        onEvent("progress", JSON.stringify(msg.data));
        break;
      case "transferDone":
        onEvent("transferDone", JSON.stringify(msg.data));
        break;
      case "error":
        onEvent("error", msg.data.message);
        break;
      case "closed":
        closed = true;
        onEvent("closed", msg.data.reason);
        break;
    }
  };

  core()
    .invoke("sftp_connect", { sessionId, connectionId, onEvent: channel })
    .catch((err) => onEvent("error", errText(err)));

  return {
    getSessionId: () => sessionId,
    confirmHostKey: (accept) => {
      core().invoke("confirm_host_key", { sessionId, accept });
    },
    disconnect: () => {
      if (!closed) core().invoke("sftp_disconnect", { sessionId });
    },
    dispose: () => {
      if (!closed) core().invoke("sftp_disconnect", { sessionId });
    },
  };
}

// The dialog plugin's JS API is not exposed on window.__TAURI__ (no npm
// bundler here), so we invoke its commands directly. Both return the chosen
// path as a plain string, or null when the user cancels.

/** Opens the native "save file" dialog; returns the chosen path or null. */
export async function pickSavePath(defaultName) {
  const path = await core().invoke("plugin:dialog|save", {
    options: { defaultPath: defaultName },
  });
  return typeof path === "string" ? path : null;
}

/** Opens the native "open file" dialog; returns the chosen path or null. */
export async function pickOpenPath() {
  const path = await core().invoke("plugin:dialog|open", {
    options: { multiple: false, directory: false },
  });
  return typeof path === "string" ? path : null;
}
