# bash completion for toptop
# Install: source this file, or copy to /etc/bash_completion.d/toptop
_toptop() {
    local cur prev opts themes
    COMPREPLY=()
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev="${COMP_WORDS[COMP_CWORD-1]}"
    opts="-t --tick --theme --tree --no-tree --list-themes --snapshot -h --help -V --version"
    themes="gruvbox nord dracula tokyonight matrix"

    case "$prev" in
        --theme)
            COMPREPLY=( $(compgen -W "$themes" -- "$cur") )
            return 0
            ;;
        -t|--tick)
            return 0
            ;;
    esac

    COMPREPLY=( $(compgen -W "$opts" -- "$cur") )
    return 0
}
complete -F _toptop toptop
