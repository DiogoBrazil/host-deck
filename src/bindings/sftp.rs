use wasm_bindgen::prelude::*;

// Mirrors bindings/terminal.rs: wasm-bindgen packages this JS module as a
// snippet whose absolute imports resolve against the copied public assets.
#[wasm_bindgen(module = "/public/js/sftp.js")]
extern "C" {
    #[wasm_bindgen(js_name = startSftp, catch)]
    pub async fn start_sftp(connection_id: &str, on_event: &JsValue) -> Result<JsValue, JsValue>;

    /// Handle returned by startSftp.
    pub type SftpHandle;

    #[wasm_bindgen(method, js_name = getSessionId)]
    pub fn get_session_id(this: &SftpHandle) -> String;

    #[wasm_bindgen(method, js_name = confirmHostKey)]
    pub fn confirm_host_key(this: &SftpHandle, accept: bool);

    #[wasm_bindgen(method)]
    pub fn disconnect(this: &SftpHandle);

    #[wasm_bindgen(method)]
    pub fn dispose(this: &SftpHandle);

    /// Native "save file" dialog; resolves to the chosen path or null.
    #[wasm_bindgen(js_name = pickSavePath, catch)]
    pub async fn pick_save_path(default_name: &str) -> Result<JsValue, JsValue>;

    /// Native "open file" dialog; resolves to the chosen path or null.
    #[wasm_bindgen(js_name = pickOpenPath, catch)]
    pub async fn pick_open_path() -> Result<JsValue, JsValue>;
}
