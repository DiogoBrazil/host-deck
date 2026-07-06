#!/usr/bin/env bash
#
# Publica uma nova versão do HostDeck.
#
# Lê a última versão do registro (scripts/releases.json), calcula a próxima,
# atualiza o número da versão nos arquivos do projeto (Cargo.toml da raiz,
# src-tauri/Cargo.toml e src-tauri/tauri.conf.json), grava a nova versão no
# registro, commita, cria a tag vX.Y.Z e faz push da branch + tag. O GitHub
# Actions detecta a tag, compila para Linux e Windows e cria a Release.
#
# Uso:
#   ./scripts/release.sh            # incrementa o patch (0.1.0 -> 0.1.1)
#   ./scripts/release.sh patch      # idem
#   ./scripts/release.sh minor      # 0.1.3 -> 0.2.0
#   ./scripts/release.sh major      # 0.4.2 -> 1.0.0
#   ./scripts/release.sh 1.2.3      # versão explícita
#   ./scripts/release.sh show       # só mostra as versões, sem publicar

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(dirname "$SCRIPT_DIR")"
cd "$ROOT"

CARGO_ROOT="Cargo.toml"
CARGO_TAURI="src-tauri/Cargo.toml"
CONF_TAURI="src-tauri/tauri.conf.json"
RELEASES_JSON="scripts/releases.json"

# Versão atual do app (fonte da verdade: tauri.conf.json).
app_version="$(perl -0777 -ne 'print $1 if /"version"\s*:\s*"([^"]+)"/' "$CONF_TAURI")"

# Última versão registrada; cai para a do app se o registro não existir/estiver vazio.
latest=""
if [ -f "$RELEASES_JSON" ]; then
  latest="$(perl -0777 -ne 'print $1 if /"latest"\s*:\s*"([^"]+)"/' "$RELEASES_JSON")"
fi
[ -n "$latest" ] || latest="$app_version"

# Calcula X.Y.Z a partir de "latest" + tipo de bump.
bump() {
  local base="$1" kind="$2" MA MI PA
  IFS=. read -r MA MI PA <<< "$base"
  case "$kind" in
    major) MA=$((MA + 1)); MI=0; PA=0 ;;
    minor) MI=$((MI + 1)); PA=0 ;;
    patch) PA=$((PA + 1)) ;;
  esac
  echo "$MA.$MI.$PA"
}

arg="${1:-patch}"

if [ "$arg" = "show" ]; then
  echo "Versão do app:      $app_version"
  echo "Última no registro: $latest"
  echo "Próxima (patch):    $(bump "$latest" patch)"
  echo "Próxima (minor):    $(bump "$latest" minor)"
  echo "Próxima (major):    $(bump "$latest" major)"
  exit 0
fi

case "$arg" in
  patch|minor|major) version="$(bump "$latest" "$arg")" ;;
  *)
    if [[ "$arg" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
      version="$arg"
    else
      echo "Argumento inválido: '$arg'. Use: patch | minor | major | X.Y.Z | show." >&2
      exit 1
    fi
    ;;
esac

tag="v$version"
branch="$(git rev-parse --abbrev-ref HEAD)"

echo "Última versão:  $latest"
echo "Nova versão:    $version  (tag $tag)"
echo "Branch:         $branch"
echo

# Aborta se houver mudanças em arquivos RASTREADOS (ignora untracked como docs/).
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  echo "Há mudanças não commitadas em arquivos rastreados. Commit ou stash antes de publicar." >&2
  exit 1
fi

# Impede recriar uma tag que já existe.
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  echo "A tag $tag já existe. Escolha outra versão." >&2
  exit 1
fi

read -r -p "Confirmar release $tag e push para origin/$branch? (s/N) " ans
case "$ans" in
  [sS]) ;;
  *) echo "Cancelado."; exit 0 ;;
esac

# --- Atualiza a versão nos arquivos ---

# Cargo.toml: só a linha `version` DENTRO da seção [package] (evita rust-version
# e versões de dependências). /ms: ^ casa início de linha, . casa newline.
for f in "$CARGO_ROOT" "$CARGO_TAURI"; do
  V="$version" perl -0777 -pi -e \
    's/(\[package\].*?^version\s*=\s*")[^"]*(")/$1$ENV{V}$2/ms' "$f"
done

# tauri.conf.json: primeira ocorrência de "version".
V="$version" perl -0777 -pi -e \
  's/("version"\s*:\s*")[^"]*(")/$1$ENV{V}$2/' "$CONF_TAURI"

# Registro: seta "latest" e insere a nova entrada no topo de "releases".
date_utc="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
perl - "$version" "$tag" "$date_utc" "$RELEASES_JSON" <<'PERL'
my ($ver, $tag, $date, $file) = @ARGV;
my @entries;
if (open my $fh, '<', $file) {
  local $/; my $c = <$fh>; close $fh;
  while ($c =~ /"version"\s*:\s*"([^"]+)"\s*,\s*"tag"\s*:\s*"([^"]+)"\s*,\s*"date"\s*:\s*"([^"]+)"/g) {
    push @entries, [$1, $2, $3];
  }
}
unshift @entries, [$ver, $tag, $date];
my $out = "{\n  \"latest\": \"$ver\",\n  \"releases\": [\n";
$out .= join(",\n", map { "    { \"version\": \"$_->[0]\", \"tag\": \"$_->[1]\", \"date\": \"$_->[2]\" }" } @entries);
$out .= "\n  ]\n}\n";
open my $out_fh, '>', $file or die "não gravou $file: $!";
print $out_fh $out;
close $out_fh;
PERL

# --- Commit, tag e push ---
git add "$CARGO_ROOT" "$CARGO_TAURI" "$CONF_TAURI" "$RELEASES_JSON"
git commit -m "release: $tag"
git tag "$tag"
git push origin "$branch"
git push origin "$tag"

echo
echo "Release $tag disparada."
echo "Acompanhe o build:  https://github.com/DiogoBrazil/host-deck/actions"
echo "Downloads (quando terminar):  https://github.com/DiogoBrazil/host-deck/releases"
