# HostDeck

HostDeck é um cliente desktop para gerenciar conexões SSH, abrir terminais
interativos embutidos e transferir arquivos por SFTP. O projeto foi construído
com **Tauri v2 + Rust + Leptos (WASM)**, usa `russh` para falar SSH nativamente,
`rusqlite` para persistência local e o keyring do sistema para armazenar
segredos.

O objetivo é oferecer uma alternativa simples e segura para organizar hosts,
credenciais e sessões de terminal sem depender de shell local, concatenação de
comandos ou armazenamento de senha em texto puro.

## Download

Os instaladores prontos ficam na página de **Releases**:

**➡️ [github.com/DiogoBrazil/host-deck/releases/latest](https://github.com/DiogoBrazil/host-deck/releases/latest)**

Escolha o arquivo conforme o seu sistema:

| Sistema | Arquivo | Observação |
|---|---|---|
| **Windows** | `HostDeck_x.y.z_x64_en-US.msi` | Instalador padrão (recomendado) |
| **Windows** | `HostDeck_x.y.z_x64-setup.exe` | Instalador alternativo (NSIS) |
| **Linux** | `host-deck_x.y.z_amd64.AppImage` | Portátil — dê permissão de execução e rode |
| **Linux** | `host-deck_x.y.z_amd64.deb` | Debian / Ubuntu |
| **Linux** | `host-deck-x.y.z-1.x86_64.rpm` | Fedora / openSUSE |

> Cada Release é gerada automaticamente pelo GitHub Actions
> (`.github/workflows/build.yml`) ao publicar uma tag `v*`. Builds de
> desenvolvimento (sem tag) ficam como *artifacts* na aba
> [Actions](https://github.com/DiogoBrazil/host-deck/actions).

## Funcionalidades

- Cadastro, edição, remoção e listagem de conexões SSH.
- Autenticação por senha ou chave privada, com passphrase opcional.
- Lista lateral agrupada por grupo, com busca por nome, host ou grupo.
- Botões para salvar, salvar e conectar, editar, remover e conectar.
- Terminal interativo embutido com xterm.js e PTY remoto.
- Redimensionamento do terminal sincronizado com a sessão SSH.
- Verificação de host key com modelo TOFU.
- Bloqueio de conexão quando a host key conhecida muda.
- Armazenamento seguro de senha/passphrase no keyring do sistema.
- SQLite local contendo apenas metadados e referências para os segredos.

### Em desenvolvimento

- Transferência de arquivos por **SFTP**, reaproveitando a mesma conexão SSH,
  o TOFU de host key e as credenciais já cadastradas. Navegador de arquivos
  remoto com listagem, navegação, upload, download, criar pasta, renomear e
  remover. O desenho completo está em
  [docs/SFTP_SPEC.md](docs/SFTP_SPEC.md).

## Stack Técnica

- **Tauri v2**: shell desktop, IPC, empacotamento e integração com o sistema.
- **Rust**: backend, domínio, persistência, keyring e cliente SSH.
- **Leptos 0.8 CSR**: frontend em WebAssembly.
- **Trunk**: build/dev server do frontend.
- **xterm.js**: terminal no WebView.
- **russh**: implementação SSH nativa em Rust.
- **russh-sftp** *(planejado)*: cliente SFTP sobre o channel SSH do russh.
- **tauri-plugin-dialog** *(planejado)*: seleção de caminho local para
  upload/download.
- **rusqlite**: banco local SQLite com SQLite bundled.
- **keyring**: Secret Service, Keychain ou Credential Manager.
- **tokio**: tarefas assíncronas, canais e rede.
- **zeroize**: limpeza de buffers sensíveis em memória.

## Arquitetura

```text
Leptos (WASM)
  -> wasm-bindgen
  -> public/js/terminal.js + xterm.js
  -> Tauri IPC (invoke + Channel)
  -> Rust backend
  -> russh
  -> servidor SSH
```

O frontend vive em `src/` e compila para WASM. Ele chama comandos Tauri por
`window.__TAURI__.core.invoke`, encapsulado em `src/bindings/tauri.rs`.

O terminal é montado pelo glue JavaScript em `public/js/terminal.js`. Esse glue
instancia xterm.js, cria um `tauri::ipc::Channel`, envia input/resize para o
backend e escreve a saída recebida no terminal.

O backend vive em `src-tauri/src/`. Ele registra o SQLite, o keyring e o
registry de sessões como estado Tauri, expõe comandos IPC e mantém as sessões
SSH ativas.

## Estrutura do Repositório

```text
.
├── docs/
│   └── SFTP_SPEC.md             # Especificação da transferência via SFTP
├── src/                         # Frontend Leptos/WASM
│   ├── api.rs                   # Wrapper das chamadas IPC de CRUD
│   ├── bindings/                # Bindings Tauri e terminal JS
│   ├── components/              # Layout, lista, formulário e terminal
│   └── models.rs                # Tipos espelhados do backend
├── public/
│   ├── js/terminal.js           # Integração xterm.js <-> Tauri
│   ├── styles.css               # Estilos da UI
│   └── vendor/xterm/            # Assets vendorizados do xterm.js
├── src-tauri/
│   ├── migrations/              # Migrations SQLite
│   ├── src/
│   │   ├── commands/            # Commands Tauri
│   │   ├── domain/              # Tipos e validação de domínio
│   │   ├── infra/               # SQLite e keyring
│   │   ├── ssh/                 # Cliente SSH, TOFU, sessões e eventos
│   │   ├── sftp/                # Cliente SFTP e transferências (planejado)
│   │   ├── error.rs             # Erros serializados para o frontend
│   │   ├── lib.rs               # Setup Tauri e registro de commands
│   │   └── state.rs             # Estado compartilhado
│   └── tauri.conf.json          # Configuração Tauri
├── Cargo.toml                   # Crate frontend
├── Trunk.toml                   # Configuração Trunk
└── README.md
```

## Fluxos Principais

### Cadastro e edição de conexão

O formulário monta um `ConnectionInput` no frontend e chama `create_connection`
ou `update_connection`. No backend, o input é validado e normalizado antes de ir
para o SQLite.

Campos vazios recebem defaults quando aplicável:

- porta vazia vira `22`;
- nome vazio vira `usuario@host`;
- grupo vazio vira `Geral`;
- observações vazias viram `NULL`.

Em criação com senha, a senha é obrigatória. Em edição, senha vazia significa
manter a credencial atual.

### Armazenamento de segredos

Senhas e passphrases não são persistidas no SQLite. Elas transitam no submit,
são gravadas no keyring do sistema e depois os campos são limpos na UI.

O SQLite guarda apenas referências como:

- `ssh-password:<connection_id>`;
- `key-passphrase:<connection_id>`.

Ao remover uma conexão, as credenciais correspondentes também são removidas do
keyring. A remoção no keyring é idempotente.

### Conexão SSH

Ao clicar em conectar, o frontend gera um `session_id` e chama `ssh_connect`.
Esse id é gerado no frontend para permitir que a UI responda a prompts de host
key enquanto a chamada de conexão ainda está pendente.

O backend:

1. carrega os dados da conexão no SQLite;
2. busca a senha/passphrase no keyring quando necessário;
3. registra a sessão no `SessionRegistry`;
4. abre conexão TCP e handshake SSH via `russh`;
5. verifica host key com TOFU;
6. autentica;
7. abre PTY e shell remoto;
8. inicia a ponte entre SSH e frontend.

Entrada do usuário e resize entram por commands Tauri (`ssh_send_data`,
`ssh_resize`). Saída do servidor é transmitida por `tauri::ipc::Channel`.

### Terminal embutido

O backend envia a saída do terminal como base64 para preservar bytes brutos e
sequências ANSI que podem cruzar fronteiras de chunk. O JavaScript converte de
base64 para `Uint8Array` e escreve no xterm.js.

A sessão envia eventos para o frontend:

- `connected`: terminal pronto;
- `output`: bytes do servidor em base64;
- `hostKeyPrompt`: primeira conexão para um host desconhecido;
- `closed`: sessão encerrada;
- `error`: erro assíncrono após iniciar a sessão.

### TOFU e host keys

HostDeck usa TOFU (trust on first use):

- primeira conexão para um host/porta/tipo de chave pede confirmação do
  fingerprint;
- ao aceitar, a chave é salva em `known_hosts`;
- conexões futuras para a mesma chave seguem sem prompt;
- se a chave mudar, a conexão é bloqueada com alerta de segurança.

Esse comportamento protege contra mudanças inesperadas de host key, incluindo
possíveis ataques man-in-the-middle.

## Modelo de Dados

As migrations ficam em `src-tauri/migrations/` e são aplicadas por
`PRAGMA user_version`. Novas migrations devem ser adicionadas ao final da lista
em `src-tauri/src/infra/db.rs`; migrations antigas não devem ser removidas nem
reordenadas.

### `ssh_connections`

Guarda os metadados das conexões:

- `id`;
- `name`;
- `host`;
- `port`;
- `username`;
- `auth_method`;
- `identity_file`;
- `group_name`;
- `notes`;
- `password_secret_key`;
- `key_passphrase_secret_key`;
- `last_connected_at`;
- `created_at`;
- `updated_at`.

### `known_hosts`

Guarda as host keys confiadas por TOFU:

- `id`;
- `host`;
- `port`;
- `key_type`;
- `public_key`;
- `fingerprint`;
- `added_at`.

A combinação `(host, port, key_type)` é única.

## Commands Tauri

### Conexões

- `list_connections`: lista todas as conexões ordenadas por grupo e nome.
- `get_connection`: busca uma conexão por id.
- `create_connection`: valida, persiste metadados e salva segredos no keyring.
- `update_connection`: atualiza metadados e credenciais conforme necessário.
- `delete_connection`: remove credenciais e depois remove o registro.

### Terminal/SSH

- `ssh_connect`: conecta usando credenciais salvas.
- `ssh_connect_with_password`: fallback backend com senha em memória.
- `ssh_send_data`: envia input do terminal para a sessão SSH.
- `ssh_resize`: redimensiona o PTY remoto.
- `ssh_disconnect`: encerra a sessão.
- `confirm_host_key`: responde ao prompt TOFU.

### SFTP *(planejado)*

Conjunto de commands para o navegador de arquivos, reaproveitando a conexão SSH
e o TOFU. Detalhes e assinaturas em [docs/SFTP_SPEC.md](docs/SFTP_SPEC.md).

- `sftp_connect` / `sftp_connect_with_password`: abre o subsistema SFTP.
- `sftp_realpath`: resolve o diretório home e caminhos canônicos.
- `sftp_list_dir`: lista um diretório remoto.
- `sftp_download` / `sftp_upload`: transferências com progresso via evento.
- `sftp_mkdir`, `sftp_rename`, `sftp_remove_file`, `sftp_remove_dir`: gerência.
- `sftp_disconnect`: encerra a sessão SFTP.

## Segurança

- Segredos não são gravados no SQLite.
- Senhas e passphrases não devem aparecer em logs ou mensagens de erro.
- Buffers sensíveis usam `zeroize` quando aplicável.
- A entrada do usuário é enviada como dados de canal PTY, não concatenada em
  comandos locais.
- O app usa CSP restritiva em `tauri.conf.json`.
- A capability padrão usa apenas `core:default`.
- Mudança de host key bloqueia a conexão.
- Excluir conexão remove referências correspondentes no keyring.

## Desenvolvimento Local

Pré-requisitos:

- Rust;
- target `wasm32-unknown-unknown`;
- Trunk;
- Tauri CLI;
- dependências de sistema do Tauri, incluindo WebKitGTK 4.1 no Linux.

Instalações comuns:

```bash
rustup target add wasm32-unknown-unknown
cargo install trunk
cargo install tauri-cli
```

Rodar em desenvolvimento:

```bash
cargo tauri dev
```

O Tauri executa `trunk serve` como `beforeDevCommand` e espera o frontend em
`http://localhost:1420`.

Gerar build:

```bash
cargo tauri build
```

## Testes

Frontend:

```bash
cargo check
```

Backend:

```bash
cd src-tauri
cargo test
```

Teste real de keyring do sistema:

```bash
cd src-tauri
cargo test real_keyring -- --ignored
```

Testes E2E SSH contra container:

```bash
docker run -d --name hostdeck-ssh-test -p 127.0.0.1:2222:2222 \
  -e PASSWORD_ACCESS=true -e USER_NAME=tester -e USER_PASSWORD=hostdeck-test-password \
  -e PUBLIC_KEY="$(cat test_key.pub)" lscr.io/linuxserver/openssh-server

cd src-tauri
HOSTDECK_TEST_KEY=./test_key cargo test ssh_e2e -- --ignored --test-threads 1
```

Os testes E2E são ignorados por padrão porque dependem de um `sshd` externo em
execução.

## Build e Empacotamento

Configuração principal:

- produto: `HostDeck`;
- identificador: `com.hostdeck.app`;
- janela inicial: `1100x720`;
- tamanho mínimo: `800x500`;
- frontend de produção: `../dist`;
- dev server: `http://localhost:1420`;
- bundle ativo para todos os targets suportados pelo Tauri.

Ícones e configuração de bundle ficam em `src-tauri/icons/` e
`src-tauri/tauri.conf.json`.

## Publicando uma nova versão (Release)

Os instaladores para download são gerados pelo GitHub Actions
(`.github/workflows/build.yml`). O workflow **não roda em todo push** — ele só é
acionado ao publicar uma **tag `v*`** (ou manualmente pelo botão "Run workflow"
na aba Actions).

Para publicar, use o script — ele **calcula a próxima versão sozinho** a partir
do registro `scripts/releases.json`. No Windows, rode pelo Git Bash:

```bash
bash scripts/release.sh          # incrementa o patch (0.1.0 -> 0.1.1)
bash scripts/release.sh minor    # 0.1.3 -> 0.2.0
bash scripts/release.sh major    # 0.4.2 -> 1.0.0
bash scripts/release.sh 1.2.3    # versão explícita
bash scripts/release.sh show     # só mostra as versões, sem publicar
```

O script, em um único commit `release: vX.Y.Z`:

1. lê a última versão em `scripts/releases.json` e calcula a próxima;
2. atualiza a versão em `Cargo.toml` (raiz), `src-tauri/Cargo.toml` e
   `src-tauri/tauri.conf.json`;
3. registra a nova versão em `scripts/releases.json` (histórico, mais novo no topo);
4. cria a tag e faz push da branch + tag.

Alguns minutos depois, os instaladores aparecem em
[Releases](https://github.com/DiogoBrazil/host-deck/releases) (Windows `.msi`/
`.exe` e Linux `.deb`/`.rpm`/`.AppImage`).

O arquivo `scripts/releases.json` é a fonte da verdade do versionamento — o
script sempre parte da última entrada dele para o próximo número.

## Troubleshooting

### `runtime do Tauri indisponível`

A página foi aberta fora do app Tauri. Rode com:

```bash
cargo tauri dev
```

### Keyring indisponível

No Linux, confirme se Secret Service está disponível e se há um daemon de
keyring ativo. Em ambientes headless, o keyring pode não estar acessível.

### Senha não encontrada

Edite a conexão e salve a senha novamente. O SQLite pode conter a referência,
mas o item correspondente pode ter sido removido do keyring do sistema.

### Host key mudou

O app bloqueia a conexão. Isso pode indicar reinstalação legítima do servidor ou
ataque man-in-the-middle. Só remova o host conhecido após verificar o
fingerprint por outro canal confiável.

### Terminal não abre

Verifique:

- se o host e a porta estão corretos;
- se o usuário existe no servidor;
- se a senha/chave está correta;
- se o arquivo de chave privada existe no caminho informado;
- se o servidor aceita o método de autenticação escolhido.

## Limitações Atuais

- O MVP mantém uma sessão de terminal ativa por vez.
- `ssh_connect_with_password` existe no backend como fallback, mas ainda não há
  fluxo de UI dedicado para pedir senha avulsa quando o keyring falha.
- Não há tela própria para gerenciar/remover entradas de `known_hosts`.
- Não há importação/exportação de conexões.
- Não há suporte explícito a jump host, agent forwarding ou múltiplas abas de
  terminal.
- A transferência de arquivos por SFTP está especificada
  ([docs/SFTP_SPEC.md](docs/SFTP_SPEC.md)) mas ainda não implementada.

## Convenções de Manutenção

- Não gravar segredos no SQLite.
- Não incluir senhas/passphrases em logs, erros ou testes de snapshot.
- Não reordenar migrations existentes.
- Manter nomes serializados de eventos e erros compatíveis com o frontend.
- Preferir mudanças pequenas e testáveis nos limites atuais de domínio, infra,
  commands e UI.
