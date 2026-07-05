use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::prelude::*;

use crate::models::AppError;

#[wasm_bindgen]
extern "C" {
    /// `window.__TAURI__.core.invoke` (disponível via `withGlobalTauri`).
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke, catch)]
    async fn invoke_raw(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
}

/// O runtime IPC do Tauri está presente? (falso em browser comum)
fn has_tauri_runtime() -> bool {
    web_sys::window()
        .map(|win| js_sys::Reflect::has(&win, &JsValue::from_str("__TAURI__")).unwrap_or(false))
        .unwrap_or(false)
}

/// Chama um command Tauri serializando args e desserializando o retorno.
pub async fn invoke<T, A>(cmd: &str, args: &A) -> Result<T, AppError>
where
    T: DeserializeOwned,
    A: Serialize,
{
    if !has_tauri_runtime() {
        return Err(AppError::internal(
            "runtime do Tauri indisponível (página aberta fora do app)".into(),
        ));
    }

    let args = serde_wasm_bindgen::to_value(args)
        .map_err(|e| AppError::internal(format!("serializando args: {e}")))?;

    match invoke_raw(cmd, args).await {
        Ok(value) => serde_wasm_bindgen::from_value(value)
            .map_err(|e| AppError::internal(format!("desserializando resposta: {e}"))),
        Err(err) => Err(AppError::from_js(err)),
    }
}

/// Command sem argumentos.
pub async fn invoke_no_args<T: DeserializeOwned>(cmd: &str) -> Result<T, AppError> {
    invoke(cmd, &serde_json::Map::new()).await
}
