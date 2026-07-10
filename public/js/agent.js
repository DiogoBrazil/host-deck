// Bridge between the Leptos agent panel and the agent_send Tauri command.
// Mirrors terminal.js: the Channel lives here; each event is forwarded to the
// WASM side as a JSON string.

const core = () => window.__TAURI__.core;

/**
 * Starts an agent turn. Resolves when the command is accepted (the turn keeps
 * running in the backend); rejects with the backend AppError on refusal.
 * @param {string} sessionId SSH session id
 * @param {string} providerId provider record id
 * @param {string|null} model model override (null uses the provider default)
 * @param {string} message user message
 * @param {number|null} temperature only when the model announces support
 * @param {boolean} thinking extended reasoning, capability-gated by the UI
 * @param {(eventJson:string)=>void} onEvent receives each AgentEvent as JSON
 */
export function agentSend(
  sessionId,
  providerId,
  model,
  message,
  temperature,
  thinking,
  onEvent,
) {
  const channel = new (core().Channel)();
  channel.onmessage = (msg) => onEvent(JSON.stringify(msg));
  return core().invoke("agent_send", {
    sessionId,
    providerId,
    model,
    message,
    temperature,
    thinking,
    onEvent: channel,
  });
}
