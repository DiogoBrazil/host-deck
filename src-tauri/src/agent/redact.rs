//! Redação de segredos no contexto do terminal (Fase 4).
//!
//! O scrollback contém tudo que passou pela tela: tokens colados, conteúdo de
//! `.env`, saída de `env`. Antes de qualquer byte sair da máquina, os padrões
//! óbvios de segredo são substituídos por `[REDACTED]`. É melhor-esforço por
//! definição — um filtro não prova ausência de segredos —, por isso o envio
//! também exige consentimento e o preview mostra exatamente o texto filtrado.

use std::sync::LazyLock;

use regex_lite::Regex;

/// Marcador que substitui o trecho sigiloso; visível no preview ao usuário.
pub const MARKER: &str = "[REDACTED]";

/// Padrões aplicados em ordem; os mais estruturados (blocos PEM) vêm antes
/// para que os genéricos não quebrem o casamento deles.
static PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        // Blocos PEM de chave privada, mesmo truncados pelo corte do buffer.
        r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[A-Za-z0-9+/=\s]*(?:-----END [A-Z0-9 ]*PRIVATE KEY-----)?",
        // Cabeçalhos Authorization ecoados por curl -v, http traces etc.
        r"(?i)authorization\s*:\s*(?:bearer|basic|token)\s+\S+",
        // Atribuições nome=valor / nome: valor com nomes tipicamente sigilosos
        // (`.env`, `export`, YAML/JSON de config, saída de `env`).
        r#"(?i)\b[A-Z0-9_.-]*(?:password|passwd|pwd|secret|token|api[_-]?key|apikey|access[_-]?key|private[_-]?key|client[_-]?secret|passphrase|credential)[A-Z0-9_.-]*["']?\s*[=:]\s*["']?[^\s"']+"#,
        // Formatos de token reconhecíveis pelo prefixo, soltos no texto:
        // Anthropic/OpenAI/OpenRouter (sk-*), GitHub (ghp_ etc.), GitLab,
        // Slack, AWS (chave de acesso), Google (AIza) e JWTs.
        r"\bsk-[A-Za-z0-9_-]{16,}",
        r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{20,}",
        r"\bgithub_pat_[A-Za-z0-9_]{20,}",
        r"\bglpat-[A-Za-z0-9_-]{16,}",
        r"\bxox[abprs]-[A-Za-z0-9-]{10,}",
        r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b",
        r"\bAIza[0-9A-Za-z_-]{30,}",
        r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}",
    ]
    .iter()
    .map(|p| Regex::new(p).expect("padrão de redação inválido"))
    .collect()
});

/// Substitui os padrões conhecidos de segredo por [`MARKER`].
///
/// Nas atribuições (`DB_PASSWORD=...`) o nome da variável se perde junto —
/// simplifica os padrões e nada de útil é removido: o modelo não precisa
/// saber qual segredo existia, só que algo foi omitido.
pub fn redact(input: &str) -> String {
    let mut out = input.to_string();
    for pattern in PATTERNS.iter() {
        if pattern.is_match(&out) {
            out = pattern.replace_all(&out, MARKER).into_owned();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_pem_private_key_blocks_even_truncated() {
        let text = "antes\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXk=\n-----END OPENSSH PRIVATE KEY-----\ndepois";
        let out = redact(text);
        assert!(out.contains("antes"));
        assert!(out.contains("depois"));
        assert!(!out.contains("b3BlbnNzaC1rZXk"), "{out}");

        // O corte do ring buffer pode comer o rodapé do bloco.
        let truncated = "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA";
        assert!(!redact(truncated).contains("MIIEpAIBAAKCAQEA"));
    }

    #[test]
    fn redacts_env_style_assignments_and_keeps_the_rest() {
        let text = "$ cat .env\nDB_PASSWORD=hunter2\nDB_HOST=localhost\nAPI_KEY: \"abc123\"\n";
        let out = redact(text);
        assert!(!out.contains("hunter2"), "{out}");
        assert!(!out.contains("abc123"), "{out}");
        assert!(out.contains("DB_HOST=localhost"), "{out}");
    }

    #[test]
    fn redacts_known_token_shapes_in_loose_text() {
        let cases = [
            "sk-ant-api03-abcdefghijklmnopqrstuv",
            "sk-or-v1-0123456789abcdef0123456789abcdef",
            "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ012345",
            "github_pat_11ABCDEFG0123456789_abcdef",
            "glpat-abcDEF123456_78901234",
            "xoxb-1234567890-ABCdefGHIjkl",
            "AKIAIOSFODNN7EXAMPLE",
            "AIzaSyA-1234567890abcdefghijklmnopqrstu",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r_wW1gFWFOEjXk",
        ];
        for secret in cases {
            let out = redact(&format!("echo {secret} fim"));
            assert!(!out.contains(secret), "não redigiu: {secret}");
            assert!(out.contains(MARKER), "sem marcador para: {secret}");
            assert!(out.contains("fim"), "comeu texto vizinho: {secret}");
        }
    }

    #[test]
    fn redacts_authorization_headers() {
        let text = "> Authorization: Bearer abc.def.ghi\n< HTTP/1.1 200 OK";
        let out = redact(text);
        assert!(!out.contains("abc.def.ghi"), "{out}");
        assert!(out.contains("200 OK"), "{out}");
    }

    #[test]
    fn leaves_ordinary_terminal_output_untouched() {
        let text = "$ df -h\nFilesystem Size Used Avail Use%\n/dev/sda1 40G 12G 28G 30%\n\
                    $ systemctl status nginx\nActive: active (running)\n";
        assert_eq!(redact(text), text);
    }
}
