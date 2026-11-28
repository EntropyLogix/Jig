#!/bin/sh
set -eu

SRC=$(cd "$(dirname "$0")" && pwd)
CLAUDE_SKILL="${CLAUDE_CONFIG_DIR:-$HOME/.claude}/skills/blt-crew"
CODEX_SKILL="$HOME/.agents/skills/blt-crew"

usage() {
    echo "usage: $0 [--claude | --codex] [--uninstall]"
    echo "Installs the blt-crew skill for Claude Code and Codex (both by default)."
    exit 1
}

display() {
    case $1 in
        "$HOME"/*) printf '~%s' "${1#"$HOME"}" ;;
        *) printf '%s' "$1" ;;
    esac
}

escape() {
    printf '%s' "$1" | sed 's/[\\|&]/\\&/g'
}

remove_skill() {
    if [ -L "$1" ]; then
        rm "$1"
    elif [ -d "$1" ]; then
        if [ "$(cd "$1" && pwd -P)" = "$(cd "$SRC" && pwd -P)" ]; then
            echo "refusing to remove $1: it is the source checkout"
            exit 1
        fi
        if [ ! -f "$1/bin/crew" ]; then
            echo "refusing to remove $1: it is not a blt-crew skill"
            exit 1
        fi
        rm -rf "$1"
    fi
}

install_skill() {
    remove_skill "$1"
    mkdir -p "$1/bin" "$1/roles" "$1/templates" "$1/agents"
    install -m 0755 "$SRC/bin/crew" "$1/bin/crew"
    for file in "$SRC"/roles/*.md; do
        install -m 0644 "$file" "$1/roles/"
    done
    for file in "$SRC"/templates/*.md; do
        install -m 0644 "$file" "$1/templates/"
    done
    install -m 0644 "$SRC/agents/openai.yaml" "$1/agents/openai.yaml"
    install -m 0644 "$SRC/README.md" "$1/README.md"
    sed -e "s|~/.claude/skills/blt-crew|$(escape "$(display "$CLAUDE_SKILL")")|g" \
        -e "s|~/.agents/skills/blt-crew|$(escape "$(display "$CODEX_SKILL")")|g" \
        "$SRC/SKILL.md" >"$1/SKILL.md"
    echo "installed blt-crew for $2: $1"
}

claude=yes
codex=yes
mode=install
for arg in "$@"; do
    case $arg in
        --claude) codex=no ;;
        --codex) claude=no ;;
        --uninstall) mode=uninstall ;;
        *) usage ;;
    esac
done
[ "$claude" = no ] && [ "$codex" = no ] && usage

if [ "$mode" = uninstall ]; then
    [ "$claude" = yes ] && remove_skill "$CLAUDE_SKILL" && echo "removed blt-crew for Claude Code: $CLAUDE_SKILL"
    [ "$codex" = yes ] && remove_skill "$CODEX_SKILL" && echo "removed blt-crew for Codex: $CODEX_SKILL"
    exit 0
fi

missing=
for tool in blt jq git; do
    command -v "$tool" >/dev/null 2>&1 || missing="$missing $tool"
done
if [ -n "$missing" ]; then
    echo "blt-crew needs on PATH:$missing"
    for tool in $missing; do
        case $tool in
            blt) echo "  blt: cargo install --locked --git https://github.com/EntropyLogix/Billet --tag v1.0.0" ;;
            jq) echo "  jq: recent macOS ships /usr/bin/jq; otherwise brew install jq or apt install jq" ;;
            git) echo "  git: install it with your system's package manager" ;;
        esac
    done
    exit 1
fi

[ "$claude" = yes ] && install_skill "$CLAUDE_SKILL" "Claude Code"
[ "$codex" = yes ] && install_skill "$CODEX_SKILL" "Codex"
echo "start a role in a new session: /blt-crew ... (Claude Code) or \$blt-crew ... (Codex)"
