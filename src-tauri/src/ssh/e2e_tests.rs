//! Testes E2E contra um sshd real em container. Suba o servidor antes:
//! docker run -d --name hostdeck-ssh-test -p 127.0.0.1:2222:2222 \
//!   -e PASSWORD_ACCESS=true -e USER_NAME=tester -e USER_PASSWORD=senha-teste-123 \
//!   -e PUBLIC_KEY="$(cat test_key.pub)" lscr.io/linuxserver/openssh-server
//! Depois: cargo test ssh_e2e -- --ignored --test-threads 1

use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use tauri::ipc::{Channel, InvokeResponseBody};
use tokio::sync::{mpsc, oneshot};
use zeroize::Zeroizing;

use crate::error::AppError;
use crate::infra::db::Db;
use crate::ssh::client::{AuthSpec, ConnectParams, connect};
use crate::ssh::registry::SessionInput;
use crate::ssh::session::open_shell_and_bridge;

const HOST: &str = "127.0.0.1";
const PORT: u16 = 2222;
const USER: &str = "tester";
const PASSWORD: &str = "senha-teste-123";

/// Channel de teste que acumula os eventos serializados em JSON.
fn test_channel() -> (Channel<super::events::TerminalEvent>, std_mpsc::Receiver<String>) {
    let (tx, rx) = std_mpsc::channel::<String>();
    let channel = Channel::new(move |body| {
        if let InvokeResponseBody::Json(json) = body {
            let _ = tx.send(json);
        }
        Ok(())
    });
    (channel, rx)
}

fn params_password(password: &str) -> ConnectParams {
    ConnectParams {
        host: HOST.into(),
        port: PORT,
        username: USER.into(),
        auth: AuthSpec::Password(Zeroizing::new(password.into())),
    }
}

/// Confirma automaticamente o fingerprint quando o prompt TOFU chegar.
fn auto_accept(rx_events: &std_mpsc::Receiver<String>, confirm_tx: oneshot::Sender<bool>) {
    let mut confirm = Some(confirm_tx);
    // O prompt chega de forma assíncrona durante o connect; aguardamos aqui.
    for _ in 0..100 {
        match rx_events.recv_timeout(Duration::from_millis(200)) {
            Ok(json) if json.contains("hostKeyPrompt") => {
                if let Some(tx) = confirm.take() {
                    let _ = tx.send(true);
                }
                return;
            }
            Ok(_) => continue,
            Err(_) => continue,
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn ssh_e2e_password_shell_roundtrip() {
    let db = Db::open_in_memory().unwrap();
    let (events, rx_events) = test_channel();
    let (confirm_tx, confirm_rx) = oneshot::channel();

    // TOFU: aceita o fingerprint em thread separada (como o usuário faria).
    let accept_thread = std::thread::spawn({
        let (events2, rx2) = (events.clone(), rx_events);
        move || {
            auto_accept(&rx2, confirm_tx);
            drop(events2);
            rx2
        }
    });

    let handle = connect(db.handle(), params_password(PASSWORD), events.clone(), confirm_rx)
        .await
        .expect("conexão e autenticação devem funcionar");

    let rx_events = accept_thread.join().unwrap();

    // Host salvo após aceitar (TOFU)
    let count: u32 = {
        let conn = db.0.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM known_hosts", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(count, 1, "host key deve ter sido salvo");

    // Abre shell e faz roundtrip de um comando
    let (input_tx, input_rx) = mpsc::channel(16);
    open_shell_and_bridge(handle, 80, 24, events.clone(), input_rx, || {})
        .await
        .expect("PTY + shell devem abrir");

    tokio::time::sleep(Duration::from_millis(1500)).await;
    input_tx
        .send(SessionInput::Data(b"echo hostdeck-e2e-ok\n".to_vec()))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // resize não deve falhar
    input_tx
        .send(SessionInput::Resize { cols: 120, rows: 40 })
        .await
        .unwrap();

    input_tx.send(SessionInput::Close).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let mut output = Vec::new();
    while let Ok(json) = rx_events.try_recv() {
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        if value["event"] == "output" {
            let bytes = B64.decode(value["data"]["data"].as_str().unwrap()).unwrap();
            output.extend_from_slice(&bytes);
        }
    }
    let text = String::from_utf8_lossy(&output);
    assert!(
        text.contains("hostdeck-e2e-ok"),
        "saída do shell deve conter o echo; obtido: {text:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn ssh_e2e_second_connection_skips_prompt() {
    let db = Db::open_in_memory().unwrap();

    // 1ª conexão: aceita o prompt
    {
        let (events, rx_events) = test_channel();
        let (confirm_tx, confirm_rx) = oneshot::channel();
        let t = std::thread::spawn(move || auto_accept(&rx_events, confirm_tx));
        connect(db.handle(), params_password(PASSWORD), events, confirm_rx)
            .await
            .expect("primeira conexão");
        t.join().unwrap();
    }

    // 2ª conexão: NÃO deve haver prompt (host já conhecido) — o oneshot
    // é descartado sem resposta; se o prompt fosse emitido, check_server_key
    // recusaria e a conexão falharia.
    {
        let (events, rx_events) = test_channel();
        let (_confirm_tx, confirm_rx) = oneshot::channel::<bool>();
        drop(_confirm_tx);
        connect(db.handle(), params_password(PASSWORD), events, confirm_rx)
            .await
            .expect("segunda conexão deve pular o prompt TOFU");
        let prompted = rx_events
            .try_iter()
            .any(|json| json.contains("hostKeyPrompt"));
        assert!(!prompted, "não deve emitir prompt para host conhecido");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn ssh_e2e_wrong_password_fails_with_friendly_error() {
    let db = Db::open_in_memory().unwrap();
    let (events, rx_events) = test_channel();
    let (confirm_tx, confirm_rx) = oneshot::channel();
    let t = std::thread::spawn(move || auto_accept(&rx_events, confirm_tx));

    let result = connect(db.handle(), params_password("senha-errada"), events, confirm_rx).await;
    t.join().unwrap();

    match result {
        Err(AppError::Ssh(msg)) => {
            assert!(
                msg.contains("Autenticação recusada"),
                "mensagem amigável esperada, obtida: {msg}"
            );
            assert!(!msg.contains("senha-errada"), "não deve vazar a senha");
        }
        Err(other) => panic!("esperava erro de autenticação amigável, obtive: {other:?}"),
        Ok(_) => panic!("senha errada não deveria autenticar"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn ssh_e2e_changed_host_key_is_blocked() {
    let db = Db::open_in_memory().unwrap();

    // Simula um host key registrado anteriormente com fingerprint diferente.
    {
        let conn = db.0.lock().unwrap();
        for key_type in ["ssh-ed25519", "rsa-sha2-512", "ecdsa-sha2-nistp256"] {
            conn.execute(
                "INSERT INTO known_hosts (id, host, port, key_type, public_key, fingerprint, added_at) \
                 VALUES (?1, ?2, ?3, ?4, 'chave-antiga', 'SHA256:fingerprint-antigo', ?5)",
                rusqlite::params![
                    uuid::Uuid::new_v4().to_string(),
                    HOST,
                    PORT,
                    key_type,
                    chrono::Utc::now().to_rfc3339(),
                ],
            )
            .unwrap();
        }
    }

    let (events, _rx_events) = test_channel();
    let (_confirm_tx, confirm_rx) = oneshot::channel::<bool>();

    let result = connect(db.handle(), params_password(PASSWORD), events, confirm_rx).await;
    match result {
        Err(AppError::Ssh(msg)) => assert!(
            msg.contains("ALERTA DE SEGURANÇA") || msg.contains("chave do servidor"),
            "esperava alerta de MITM, obtido: {msg}"
        ),
        Err(other) => panic!("esperava alerta de MITM, obtive: {other:?}"),
        Ok(_) => panic!("conexão com host key divergente deveria ser bloqueada"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn ssh_e2e_private_key_with_passphrase() {
    let key_path = std::env::var("HOSTDECK_TEST_KEY")
        .unwrap_or_else(|_| "/tmp/hostdeck-test-key".to_string());
    if !std::path::Path::new(&key_path).exists() {
        panic!("defina HOSTDECK_TEST_KEY apontando para a chave de teste");
    }

    let db = Db::open_in_memory().unwrap();
    let (events, rx_events) = test_channel();
    let (confirm_tx, confirm_rx) = oneshot::channel();
    let t = std::thread::spawn(move || auto_accept(&rx_events, confirm_tx));

    let params = ConnectParams {
        host: HOST.into(),
        port: PORT,
        username: USER.into(),
        auth: AuthSpec::PrivateKey {
            path: key_path,
            passphrase: Some(Zeroizing::new("frase-teste".into())),
        },
    };

    connect(db.handle(), params, events, confirm_rx)
        .await
        .expect("autenticação por chave com passphrase deve funcionar");
    t.join().unwrap();
}
