pub mod anthropic;
pub mod openai;

/// Constrói o client HTTP compartilhado pelos adapters.
///
/// Sem timeout total: um turno com streaming pode legitimamente durar
/// minutos. `connect_timeout` cobre o caso de endpoint inalcançável.
pub(super) fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("configuração estática do reqwest não pode falhar")
}

/// Extrai a mensagem de erro do corpo de uma resposta não-2xx.
///
/// Ambos os dialetos usam `{"error": {"message": ...}}`; qualquer outra
/// coisa vira o corpo bruto truncado.
pub(super) fn api_error_message(body: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(msg) = v
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
        {
            return msg.to_string();
        }
    }
    let mut msg: String = body.chars().take(300).collect();
    if msg.is_empty() {
        msg = "sem detalhes".into();
    }
    msg
}
