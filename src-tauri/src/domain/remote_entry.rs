use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
}

/// A remote directory entry returned by `sftp_list_dir`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteEntry {
    pub name: String,
    /// Absolute POSIX path (always `/`-separated, regardless of the client OS).
    pub path: String,
    pub kind: EntryKind,
    pub size: u64,
    /// mtime in Unix seconds, when available.
    pub modified: Option<i64>,
    /// POSIX permission bits (e.g. 0o755), when available.
    pub permissions: Option<u32>,
}

/// Orders entries for display: directories first, then files/symlinks, both by
/// name case-insensitively (`COLLATE NOCASE`-like).
pub fn sort_entries(entries: &mut [RemoteEntry]) {
    entries.sort_by(|a, b| {
        let a_dir = a.kind == EntryKind::Dir;
        let b_dir = b.kind == EntryKind::Dir;
        b_dir
            .cmp(&a_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, kind: EntryKind) -> RemoteEntry {
        RemoteEntry {
            name: name.into(),
            path: format!("/home/{name}"),
            kind,
            size: 0,
            modified: None,
            permissions: None,
        }
    }

    #[test]
    fn dirs_before_files_then_case_insensitive_name() {
        let mut entries = vec![
            entry("Zeta.txt", EntryKind::File),
            entry("beta", EntryKind::Dir),
            entry("alpha.txt", EntryKind::File),
            entry("Alpha", EntryKind::Dir),
            entry("link", EntryKind::Symlink),
        ];
        sort_entries(&mut entries);
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "beta", "alpha.txt", "link", "Zeta.txt"]);
    }

    #[test]
    fn serializes_kind_as_snake_case_and_fields_as_camel_case() {
        // The frontend depends on these exact serialized names.
        let e = RemoteEntry {
            name: "notes".into(),
            path: "/home/notes".into(),
            kind: EntryKind::Dir,
            size: 4096,
            modified: Some(1_700_000_000),
            permissions: Some(0o755),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"kind\":\"dir\""), "json={json}");
        assert!(json.contains("\"permissions\":493"), "json={json}");
        assert!(json.contains("\"modified\":1700000000"), "json={json}");
    }
}
