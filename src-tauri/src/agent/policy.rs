//! Classifica comandos do `run_command` em executar, confirmar ou negar.
//!
//! Decidir automaticamente se um comando arbitrário é seguro é impossível
//! (`rm -rf $DIR` depende do valor de `$DIR`). O desenho aqui é o mais
//! conservador que ainda é útil: uma allowlist de comandos de leitura roda
//! sozinha; **qualquer** outra coisa cai em confirmação do usuário. Nenhuma
//! tentativa de detectar "comandos perigosos" — o usuário é quem decide.

/// Veredito da política para um comando.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandPolicy {
    /// Comprovadamente só lê; executa sem perguntar.
    ReadOnly,
    /// Não dá para provar que é inofensivo; o usuário confirma.
    NeedsConfirmation,
    /// Não executável (hoje, apenas comando vazio).
    Denied,
}

/// Comandos cuja invocação simples (sem metacaracteres de shell) não muda
/// estado no servidor. Deliberadamente curto: ficam de fora comandos com
/// flags mutantes (`date -s`, `dmesg -C`, `find -delete`, `journalctl
/// --vacuum-*`, `sort -o`, `uniq saida`), os que executam terceiros (`env`,
/// `xargs`, `watch`) e os interativos (`top`, `less`). Revisado em
/// 2026-07-10: mantido o desenho allowlist + metacaracteres; acrescentados
/// apenas inspecionadores de rede/hardware sem flag de escrita.
const READ_ONLY: &[&str] = &[
    "ls", "cat", "head", "tail", "wc", "grep", "stat", "file", "pwd", "whoami", "id", "uname",
    "hostname", "uptime", "df", "du", "free", "ps", "w", "last", "which", "whereis", "lsblk",
    "lscpu", "echo", "ss", "lsof", "printenv", "nproc", "lsmod", "findmnt", "lspci", "lsusb",
    "md5sum", "sha256sum",
];

/// Classifica um comando de shell.
///
/// Metacaracteres (pipes, redirecionamentos, encadeamento, substituição)
/// tornam a análise do primeiro token insuficiente, então qualquer ocorrência
/// derruba o comando para confirmação — mesmo que as partes sejam de leitura.
pub fn classify(command: &str) -> CommandPolicy {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return CommandPolicy::Denied;
    }

    let has_metachar = trimmed
        .chars()
        .any(|c| matches!(c, '|' | ';' | '&' | '>' | '<' | '`' | '\n'))
        || trimmed.contains("$(");
    if has_metachar {
        return CommandPolicy::NeedsConfirmation;
    }

    // Primeiro token, ignorando o caminho (`/usr/bin/ls` conta como `ls`).
    let first = trimmed.split_whitespace().next().unwrap_or_default();
    let name = first.rsplit('/').next().unwrap_or(first);

    if READ_ONLY.contains(&name) {
        CommandPolicy::ReadOnly
    } else {
        CommandPolicy::NeedsConfirmation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_commands_run_without_confirmation() {
        assert_eq!(classify("uptime"), CommandPolicy::ReadOnly);
        assert_eq!(classify("  df -h  "), CommandPolicy::ReadOnly);
        assert_eq!(classify("/usr/bin/ls -la /etc"), CommandPolicy::ReadOnly);
        assert_eq!(classify("grep -r TODO src"), CommandPolicy::ReadOnly);
        assert_eq!(classify("ss -tlnp"), CommandPolicy::ReadOnly);
        assert_eq!(classify("lsof -i :443"), CommandPolicy::ReadOnly);
        assert_eq!(classify("sha256sum /etc/passwd"), CommandPolicy::ReadOnly);
    }

    #[test]
    fn unknown_commands_need_confirmation() {
        assert_eq!(classify("rm -rf build"), CommandPolicy::NeedsConfirmation);
        assert_eq!(classify("systemctl status nginx"), CommandPolicy::NeedsConfirmation);
        assert_eq!(classify("apt install htop"), CommandPolicy::NeedsConfirmation);
        // Mutantes disfarçados de leitura ficam fora da allowlist.
        assert_eq!(classify("date -s 2026-01-01"), CommandPolicy::NeedsConfirmation);
        assert_eq!(classify("find / -delete"), CommandPolicy::NeedsConfirmation);
        assert_eq!(classify("env rm -rf /"), CommandPolicy::NeedsConfirmation);
    }

    #[test]
    fn shell_metacharacters_force_confirmation() {
        assert_eq!(classify("cat a > b"), CommandPolicy::NeedsConfirmation);
        assert_eq!(classify("ls | wc -l"), CommandPolicy::NeedsConfirmation);
        assert_eq!(classify("uptime; rm -rf /"), CommandPolicy::NeedsConfirmation);
        assert_eq!(classify("echo `id`"), CommandPolicy::NeedsConfirmation);
        assert_eq!(classify("echo $(id)"), CommandPolicy::NeedsConfirmation);
        assert_eq!(classify("ls &&\nrm x"), CommandPolicy::NeedsConfirmation);
    }

    #[test]
    fn empty_command_is_denied() {
        assert_eq!(classify(""), CommandPolicy::Denied);
        assert_eq!(classify("   "), CommandPolicy::Denied);
    }
}
