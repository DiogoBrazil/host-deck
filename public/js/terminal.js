// Glue entre xterm.js e o backend Tauri.
// Toda a lógica de negócio vive no Rust; aqui só adaptamos APIs do browser:
// - instancia o Terminal + FitAddon
// - repassa entrada/resize do usuário via commands Tauri
// - recebe a saída da sessão SSH por um tauri Channel e escreve no terminal
import { Terminal } from "/vendor/xterm/xterm.mjs";
import { FitAddon } from "/vendor/xterm/addon-fit.mjs";

const core = () => window.__TAURI__.core;

// base64 → Uint8Array (a saída SSH trafega em base64 para preservar bytes brutos)
function b64ToBytes(b64) {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

/**
 * Cria o terminal, conecta via SSH e faz a ponte de I/O.
 * @param {string} containerId  id do <div> onde o terminal é montado
 * @param {string} connectionId id da conexão (backend)
 * @param {(status:string, detail?:string)=>void} onStatus callback de estado
 * @returns handle com dispose()/disconnect()
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

  // O session_id é gerado AQUI e passado ao backend. Assim a UI já o conhece
  // durante o prompt de host key (a chamada ssh_connect fica pendente
  // aguardando a confirmação, então não podemos depender do seu retorno).
  const sessionId = crypto.randomUUID();
  let closed = false;

  // Canal Tauri: recebe eventos ordenados do backend (saída, prompt, close).
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

  // Entrada do usuário → backend
  term.onData((data) => {
    if (closed) return;
    core().invoke("ssh_send_data", { sessionId, data });
  });

  // Resize (debounced) → FitAddon + backend
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

  // Dispara a conexão. A promise fica pendente durante o prompt de host key
  // (o backend aguarda confirm_host_key), por isso o sessionId já foi gerado
  // acima e não depende deste retorno.
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
