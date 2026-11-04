# LumenTerm shell entegrasyonu (zsh) — OSC 133 prompt marking.
# ~/.zshrc sonuna ekleyin:
#   [ -n "$LUMENTERM_SHELL_INTEGRATION" ] && source /path/to/zsh.sh

__lumenterm_precmd() {
    local exit_code=$?
    printf '\033]133;D;%s\007' "$exit_code"
    printf '\033]133;A\007'
}
__lumenterm_preexec() {
    printf '\033]133;C\007'
}

autoload -Uz add-zsh-hook
add-zsh-hook precmd __lumenterm_precmd
add-zsh-hook preexec __lumenterm_preexec
