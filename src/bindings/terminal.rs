use wasm_bindgen::prelude::*;

// Liga ao glue em public/js/terminal.js. wasm-bindgen empacota o arquivo como
// snippet; os imports absolutos (/vendor/xterm/*) resolvem em runtime.
#[wasm_bindgen(module = "/public/js/terminal.js")]
extern "C" {
    #[wasm_bindgen(js_name = startTerminal, catch)]
    pub async fn start_terminal(
        container_id: &str,
        connection_id: &str,
        on_status: &JsValue,
    ) -> Result<JsValue, JsValue>;

    /// Handle retornado por startTerminal.
    pub type TerminalHandle;

    #[wasm_bindgen(method, js_name = confirmHostKey)]
    pub fn confirm_host_key(this: &TerminalHandle, accept: bool);

    #[wasm_bindgen(method)]
    pub fn disconnect(this: &TerminalHandle);

    #[wasm_bindgen(method)]
    pub fn dispose(this: &TerminalHandle);
}
