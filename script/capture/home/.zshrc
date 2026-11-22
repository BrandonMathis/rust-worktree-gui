# Shell used inside the screenshot tmux panes. Fixed prompt and colors so
# screenshots stay comparable between runs.
export LANG=en_US.UTF-8 CLICOLOR=1 LSCOLORS=ExGxFxdxCxDxDxhbadExEx
export SHELL_SESSIONS_DISABLE=1
autoload -Uz vcs_info
precmd() { vcs_info }
zstyle ':vcs_info:git:*' formats ' %F{cyan}%b%f'
setopt prompt_subst
PROMPT='%F{magenta}%1~%f${vcs_info_msg_0_} %F{green}❯%f '
# Skip the reverse-video % that marks output without a trailing newline.
unsetopt prompt_sp
