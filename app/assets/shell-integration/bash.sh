# LumenTerm shell entegrasyonu (bash) — OSC 133 prompt marking.
# Kuruluma eklemek için ~/.bashrc sonuna şunu ekleyin:
#   [ -n "$LUMENTERM_SHELL_INTEGRATION" ] && source /path/to/bash.sh

__lumenterm_osc133_prompt_start() { printf '\033]133;A\007'; }
__lumenterm_osc133_input_start()  { printf '\033]133;B\007'; }
__lumenterm_osc133_cmd_start()    { printf '\033]133;C\007'; }
__lumenterm_osc133_cmd_end() {
    local exit_code=$?
    printf '\033]133;D;%s\007' "$exit_code"
    return $exit_code
}

PROMPT_COMMAND='__lumenterm_osc133_cmd_end; __lumenterm_osc133_prompt_start'
PS1="\[\$(__lumenterm_osc133_input_start)\]$PS1\[\$(__lumenterm_osc133_cmd_start)\]"
