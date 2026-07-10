//! Ferramentas que o agente executa contra a sessão SSH.
//!
//! Três canais deliberadamente distintos:
//! - `run_command`: canal `exec` novo na mesma conexão — terminal limpo;
//! - `read_remote_file`: SFTP (subsistema próprio, reaproveita `sftp/client`);
//! - `type_into_terminal`: digita no shell interativo do usuário (o `cd`
//!   dele, as variáveis dele, o `sudo` já autenticado) — sempre confirmado.

use std::time::Duration;

use async_trait::async_trait;
use russh::ChannelMsg;
use serde::Deserialize;
use tokio::io::AsyncReadExt;
use tokio::sync::oneshot;

use super::domain::{ToolCall, ToolResult, ToolSpec};
use super::events::{AgentEvent, SharedEventSink};
use super::policy::{self, CommandPolicy};
use super::registry::AgentRegistry;
use crate::sftp::client::{map_sftp_err, open_sftp};
use crate::ssh::registry::{SessionInput, SessionRegistry};

/// Teto de saída devolvida ao modelo por `run_command`.
const RUN_OUTPUT_CAP: usize = 64 * 1024;
/// Teto de leitura de `read_remote_file`.
const READ_FILE_CAP: usize = 128 * 1024;
/// Comandos `exec` que não terminam sozinhos (ex.: `tail -f`) não podem
/// segurar o turno para sempre; o usuário ainda pode cancelar antes disso.
const EXEC_TIMEOUT: Duration = Duration::from_secs(60);

/// Fronteira entre o laço agêntico e a execução real; o laço só conhece
/// esta trait, então os testes usam um executor roteirizado.
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    fn specs(&self) -> Vec<ToolSpec>;
    async fn execute(&self, call: &ToolCall) -> ToolResult;
}

/// Especificações expostas ao modelo. Descrições em inglês: é texto
/// dirigido ao modelo, não ao usuário.
pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "run_command".into(),
            description: "Run a shell command on the remote server over a dedicated SSH exec \
                          channel, outside the user's interactive shell. Returns the exit code \
                          and combined stdout/stderr (possibly truncated). Read-only commands \
                          run immediately; anything else waits for the user's confirmation and \
                          may be declined. Prefer this tool for inspecting the system."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Shell command to execute, e.g. \"df -h\"."
                    }
                },
                "required": ["command"]
            }),
        },
        ToolSpec {
            name: "read_remote_file".into(),
            description: "Read a file from the remote server via SFTP. Returns the file as \
                          UTF-8 text (invalid bytes replaced, long files truncated). Use \
                          absolute paths."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path of the file to read."
                    }
                },
                "required": ["path"]
            }),
        },
        ToolSpec {
            name: "type_into_terminal".into(),
            description: "Type text directly into the user's interactive terminal, exactly as \
                          if typed on the keyboard (include \\n to press Enter). Runs inside \
                          the user's context: their current directory, environment and sudo \
                          state. Always requires the user's confirmation. Use only when that \
                          interactive context matters; otherwise prefer run_command."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "Raw text to type into the terminal."
                    }
                },
                "required": ["text"]
            }),
        },
    ]
}

#[derive(Deserialize)]
struct RunCommandArgs {
    command: String,
}

#[derive(Deserialize)]
struct ReadFileArgs {
    path: String,
}

#[derive(Deserialize)]
struct TypeArgs {
    text: String,
}

/// Executor real, amarrado a uma sessão SSH ativa.
pub struct SessionToolbox {
    sessions: SessionRegistry,
    agents: AgentRegistry,
    session_id: String,
    events: SharedEventSink,
}

impl SessionToolbox {
    pub fn new(
        sessions: SessionRegistry,
        agents: AgentRegistry,
        session_id: String,
        events: SharedEventSink,
    ) -> Self {
        Self {
            sessions,
            agents,
            session_id,
            events,
        }
    }

    /// Emite `CommandPrompt` e espera a decisão do usuário. Sender derrubado
    /// (cancelamento, fim do turno) conta como recusa.
    async fn confirm(&self, call_id: &str, tool: &str, command: &str) -> bool {
        let (tx, rx) = oneshot::channel::<bool>();
        self.agents
            .register_confirmation(&self.session_id, call_id, tx);
        (self.events)(AgentEvent::CommandPrompt {
            call_id: call_id.to_string(),
            tool: tool.to_string(),
            command: command.to_string(),
        });
        rx.await.unwrap_or(false)
    }

    async fn run_command(&self, call: &ToolCall) -> Result<String, String> {
        let args: RunCommandArgs = parse_args(&call.arguments)?;

        match policy::classify(&args.command) {
            CommandPolicy::ReadOnly => {}
            CommandPolicy::NeedsConfirmation => {
                if !self.confirm(&call.id, "run_command", &args.command).await {
                    return Err("The user declined to run this command.".into());
                }
            }
            CommandPolicy::Denied => return Err("Empty command.".into()),
        }

        let handle = self
            .sessions
            .ssh_handle(&self.session_id)
            .ok_or("The SSH session is no longer active.")?;
        let mut channel = handle
            .channel_open_session()
            .await
            .map_err(|e| format!("Failed to open an exec channel: {e}."))?;
        channel
            .exec(true, args.command.as_str())
            .await
            .map_err(|e| format!("Failed to start the command: {e}."))?;

        let mut output: Vec<u8> = Vec::new();
        let mut truncated = false;
        let mut exit_code: Option<u32> = None;

        let collect = async {
            while let Some(msg) = channel.wait().await {
                match msg {
                    ChannelMsg::Data { ref data } => {
                        append_capped(&mut output, data, RUN_OUTPUT_CAP, &mut truncated);
                    }
                    ChannelMsg::ExtendedData { ref data, .. } => {
                        append_capped(&mut output, data, RUN_OUTPUT_CAP, &mut truncated);
                    }
                    ChannelMsg::ExitStatus { exit_status } => exit_code = Some(exit_status),
                    // `Eof` costuma chegar antes do `ExitStatus`; só `Close`
                    // (ou o fim do stream) encerra a coleta.
                    ChannelMsg::Close => break,
                    _ => {}
                }
            }
        };
        let timed_out = tokio::time::timeout(EXEC_TIMEOUT, collect).await.is_err();

        let text = String::from_utf8_lossy(&output);
        let mut content = match exit_code {
            Some(code) => format!("exit code: {code}\n"),
            None => "exit code: unknown\n".to_string(),
        };
        if text.trim().is_empty() {
            content.push_str("(no output)");
        } else {
            content.push_str(text.trim_end());
        }
        if truncated {
            content.push_str("\n[output truncated]");
        }
        if timed_out {
            content.push_str(&format!(
                "\n[command still running after {}s; showing output so far, channel closed]",
                EXEC_TIMEOUT.as_secs()
            ));
        }
        Ok(content)
    }

    async fn read_remote_file(&self, call: &ToolCall) -> Result<String, String> {
        let args: ReadFileArgs = parse_args(&call.arguments)?;

        let handle = self
            .sessions
            .ssh_handle(&self.session_id)
            .ok_or("The SSH session is no longer active.")?;
        let sftp = open_sftp(&handle).await.map_err(|e| e.to_string())?;
        let mut file = sftp
            .open(&args.path)
            .await
            .map_err(|e| map_sftp_err(e).to_string())?;

        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = vec![0u8; 32 * 1024];
        let mut truncated = false;
        loop {
            let n = file
                .read(&mut chunk)
                .await
                .map_err(|e| format!("Failed to read the file: {e}."))?;
            if n == 0 {
                break;
            }
            append_capped(&mut buf, &chunk[..n], READ_FILE_CAP, &mut truncated);
            if truncated {
                break;
            }
        }

        let mut content = String::from_utf8_lossy(&buf).into_owned();
        if content.is_empty() {
            content.push_str("(empty file)");
        }
        if truncated {
            content.push_str("\n[file truncated]");
        }
        Ok(content)
    }

    async fn type_into_terminal(&self, call: &ToolCall) -> Result<String, String> {
        let args: TypeArgs = parse_args(&call.arguments)?;

        if !self.confirm(&call.id, "type_into_terminal", &args.text).await {
            return Err("The user declined to type this into the terminal.".into());
        }

        let sender = self
            .sessions
            .input_sender(&self.session_id)
            .ok_or("The terminal session is no longer active.")?;
        sender
            .send(SessionInput::Data(args.text.into_bytes()))
            .await
            .map_err(|_| "The terminal session closed before the text was typed.".to_string())?;
        Ok("The text was typed into the user's terminal.".into())
    }
}

#[async_trait]
impl ToolExecutor for SessionToolbox {
    fn specs(&self) -> Vec<ToolSpec> {
        tool_specs()
    }

    async fn execute(&self, call: &ToolCall) -> ToolResult {
        let outcome = match call.name.as_str() {
            "run_command" => self.run_command(call).await,
            "read_remote_file" => self.read_remote_file(call).await,
            "type_into_terminal" => self.type_into_terminal(call).await,
            other => Err(format!("Unknown tool: {other}.")),
        };
        match outcome {
            Ok(content) => ToolResult {
                call_id: call.id.clone(),
                content,
                is_error: false,
            },
            Err(message) => ToolResult {
                call_id: call.id.clone(),
                content: message,
                is_error: true,
            },
        }
    }
}

fn parse_args<T: serde::de::DeserializeOwned>(arguments: &serde_json::Value) -> Result<T, String> {
    serde_json::from_value(arguments.clone()).map_err(|e| format!("Invalid tool arguments: {e}."))
}

fn append_capped(buf: &mut Vec<u8>, data: &[u8], cap: usize, truncated: &mut bool) {
    let remaining = cap.saturating_sub(buf.len());
    if remaining == 0 {
        *truncated = true;
        return;
    }
    let take = remaining.min(data.len());
    buf.extend_from_slice(&data[..take]);
    if take < data.len() {
        *truncated = true;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tokio::sync::mpsc;

    use super::*;
    use crate::ssh::registry::SessionHandle;
    use crate::ssh::scrollback::Scrollback;

    fn session_with_terminal(
        sessions: &SessionRegistry,
        session_id: &str,
    ) -> mpsc::Receiver<SessionInput> {
        let (input_tx, input_rx) = mpsc::channel::<SessionInput>(4);
        sessions.insert(
            session_id.to_string(),
            SessionHandle {
                input_tx,
                host_key_tx: None,
                ssh: None,
                scrollback: Arc::new(Mutex::new(Scrollback::default())),
            },
        );
        input_rx
    }

    /// Sink que responde ao `CommandPrompt` na hora, como faria o
    /// `confirm_agent_command` disparado pela UI.
    fn auto_confirm_sink(
        agents: AgentRegistry,
        session_id: &str,
        accept: bool,
        prompts: Arc<Mutex<Vec<String>>>,
    ) -> SharedEventSink {
        let session_id = session_id.to_string();
        Arc::new(move |ev: AgentEvent| {
            if let AgentEvent::CommandPrompt {
                call_id, command, ..
            } = ev
            {
                prompts.lock().unwrap().push(command);
                if let Some(tx) = agents.take_confirmation(&session_id, &call_id) {
                    let _ = tx.send(accept);
                }
            }
        })
    }

    fn call(name: &str, arguments: serde_json::Value) -> ToolCall {
        ToolCall {
            id: "c1".into(),
            name: name.into(),
            arguments,
        }
    }

    #[test]
    fn specs_expose_the_three_tools() {
        let specs = tool_specs();
        let names: Vec<_> = specs.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["run_command", "read_remote_file", "type_into_terminal"]
        );
        for spec in &specs {
            assert_eq!(spec.input_schema["type"], "object", "{}", spec.name);
            assert!(spec.input_schema["required"].is_array(), "{}", spec.name);
        }
    }

    #[tokio::test]
    async fn type_into_terminal_confirmed_sends_input_to_shell() {
        let sessions = SessionRegistry::default();
        let agents = AgentRegistry::default();
        let mut input_rx = session_with_terminal(&sessions, "s1");
        let prompts = Arc::new(Mutex::new(Vec::new()));

        let toolbox = SessionToolbox::new(
            sessions,
            agents.clone(),
            "s1".into(),
            auto_confirm_sink(agents, "s1", true, prompts.clone()),
        );

        let result = toolbox
            .execute(&call("type_into_terminal", serde_json::json!({"text": "ls\n"})))
            .await;

        assert!(!result.is_error, "{}", result.content);
        assert_eq!(*prompts.lock().unwrap(), vec!["ls\n".to_string()]);
        match input_rx.recv().await {
            Some(SessionInput::Data(bytes)) => assert_eq!(bytes, b"ls\n"),
            other => panic!("esperava Data, veio {:?}", other.is_some()),
        }
    }

    #[tokio::test]
    async fn type_into_terminal_declined_reports_error_and_sends_nothing() {
        let sessions = SessionRegistry::default();
        let agents = AgentRegistry::default();
        let mut input_rx = session_with_terminal(&sessions, "s1");
        let prompts = Arc::new(Mutex::new(Vec::new()));

        let toolbox = SessionToolbox::new(
            sessions,
            agents.clone(),
            "s1".into(),
            auto_confirm_sink(agents, "s1", false, prompts.clone()),
        );

        let result = toolbox
            .execute(&call("type_into_terminal", serde_json::json!({"text": "rm x\n"})))
            .await;

        assert!(result.is_error);
        assert!(result.content.contains("declined"), "{}", result.content);
        assert!(input_rx.try_recv().is_err(), "nada deve chegar ao shell");
    }

    #[tokio::test]
    async fn run_command_without_ssh_handle_fails_cleanly() {
        let sessions = SessionRegistry::default();
        let agents = AgentRegistry::default();
        // Sessão registrada mas sem handle SSH (autenticação nunca concluiu).
        let _input_rx = session_with_terminal(&sessions, "s1");

        let toolbox = SessionToolbox::new(
            sessions,
            agents.clone(),
            "s1".into(),
            Arc::new(|_| {}),
        );

        // `uptime` é ReadOnly: não deve pedir confirmação, deve falhar no handle.
        let result = toolbox
            .execute(&call("run_command", serde_json::json!({"command": "uptime"})))
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("no longer active"), "{}", result.content);
    }

    #[tokio::test]
    async fn unknown_tool_and_bad_arguments_are_errors() {
        let sessions = SessionRegistry::default();
        let agents = AgentRegistry::default();
        let toolbox =
            SessionToolbox::new(sessions, agents, "s1".into(), Arc::new(|_| {}));

        let result = toolbox.execute(&call("format_disk", serde_json::json!({}))).await;
        assert!(result.is_error);
        assert!(result.content.contains("Unknown tool"), "{}", result.content);

        let result = toolbox
            .execute(&call("run_command", serde_json::json!({"cmd": "ls"})))
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("Invalid tool arguments"), "{}", result.content);
    }
}
