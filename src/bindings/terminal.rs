use wasm_bindgen::prelude::*;

// wasm-bindgen packages this JS module as a snippet; its absolute imports
// resolve against the copied public assets at runtime.
#[wasm_bindgen(module = "/public/js/terminal.js")]
extern "C" {
    #[wasm_bindgen(js_name = startTerminal, catch)]
    pub async fn start_terminal(
        container_id: &str,
        connection_id: &str,
        on_status: &JsValue,
    ) -> Result<JsValue, JsValue>;

    /// Handle returned by startTerminal.
    pub type TerminalHandle;

    #[wasm_bindgen(method, js_name = getSessionId)]
    pub fn get_session_id(this: &TerminalHandle) -> String;

    #[wasm_bindgen(method, js_name = confirmHostKey)]
    pub fn confirm_host_key(this: &TerminalHandle, accept: bool);

    #[wasm_bindgen(method)]
    pub fn disconnect(this: &TerminalHandle);

    #[wasm_bindgen(method)]
    pub fn dispose(this: &TerminalHandle);
}
