# HostDeck

Cliente desktop de gerenciamento de conexões SSH com terminal embutido.
Construído com **Tauri v2 + Rust + Leptos (WASM)**, `russh` para SSH nativo,
`rusqlite` para persistência local e o keyring do sistema para segredos.

## Recursos (MVP)

- CRUD de conexões SSH (senha ou chave privada, com passphrase opcional).
- Lista lateral com busca (nome/host/grupo) e agrupamento.
- Senhas e passphrases **nunca** são gravadas no SQLite — vão para o keyring do
  sistema (Secret Service / Keychain / Credential Manager). O banco guarda só a
  referência.
- Conexão com um clique abre um terminal interativo (xterm.js) com PTY remoto.
- Verificação de host key **TOFU**: primeira conexão pede confirmação do
  fingerprint; conexões seguintes bloqueiam se a chave mudar (proteção MITM).

## Arquitetura

```
Leptos (WASM) ── glue JS (xterm.js) ── IPC Tauri (invoke + Channel) ── Rust ── russh ── servidor
```

- Frontend: `src/` (crate `host-deck-ui`, alvo `wasm32-unknown-unknown`).
- Backend: `src-tauri/` (crate `host-deck`).
- Saída do terminal trafega por `tauri::ipc::Channel` (ordenado/streaming);
  entrada e resize via commands `invoke`.

## Desenvolvimento

Pré-requisitos: Rust, `wasm32-unknown-unknown`, `trunk`, `tauri-cli`, e as libs
de sistema do Tauri (WebKitGTK 4.1).

```bash
cargo tauri dev      # roda o app (Trunk + WebKitGTK)
cargo tauri build    # gera o binário/instalador
```

## Testes

```bash
# Unitários (validação, repository, credenciais com mock):
cd src-tauri && cargo test

# Keyring real do sistema:
cargo test real_keyring -- --ignored

# SSH E2E contra um sshd em container:
docker run -d --name hostdeck-ssh-test -p 127.0.0.1:2222:2222 \
  -e PASSWORD_ACCESS=true -e USER_NAME=tester -e USER_PASSWORD=senha-teste-123 \
  -e PUBLIC_KEY="$(cat test_key.pub)" lscr.io/linuxserver/openssh-server
HOSTDECK_TEST_KEY=./test_key cargo test ssh_e2e -- --ignored --test-threads 1
```

## Segurança

- Segredos nunca em texto puro no SQLite, em logs ou em mensagens de erro;
  `zeroize` limpa buffers de senha após o uso.
- `russh` fala o protocolo diretamente — sem shell local nem concatenação de
  comandos. Entrada do usuário vai como dados do canal PTY.
- CSP restritiva e capabilities mínimas (`core:default`).
- Excluir uma conexão remove suas credenciais do keyring.
