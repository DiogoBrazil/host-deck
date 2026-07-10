use wasm_bindgen::prelude::*;

// Mirrors bindings/terminal.rs: wasm-bindgen packages this JS module as a
// snippet whose absolute imports resolve against the copied public assets.
#[wasm_bindgen(module = "/public/js/agent.js")]
extern "C" {
    /// Starts an agent turn; each `AgentEvent` reaches `on_event` as a JSON
    /// string. The returned promise settles when the backend accepts (or
    /// refuses) the turn, not when the turn finishes.
    #[wasm_bindgen(js_name = agentSend, catch)]
    pub async fn agent_send(
        session_id: &str,
        provider_id: &str,
        model: Option<String>,
        message: &str,
        on_event: &JsValue,
    ) -> Result<JsValue, JsValue>;
}
