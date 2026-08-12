# bash completion for toptop
# Install: source this file, or copy to /etc/bash_completion.d/toptop
_toptop() {
    local cur prev opts themes
    COMPREPLY=()
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev="${COMP_WORDS[COMP_CWORD-1]}"
    opts="-t --tick --theme --tree --no-tree --ai --remote --remote-cmd --config --no-save --list-themes --snapshot --export --serve-metrics --alert-vram --alert-kv --alert-queue -h --help -V --version"
    themes="gruvbox nord dracula tokyonight matrix cyberpunk paper"

    case "$prev" in
        --theme)
            COMPREPLY=( $(compgen -W "$themes" -- "$cur") )
            return 0
            ;;
        --export)
            COMPREPLY=( $(compgen -W "json csv prometheus" -- "$cur") )
            return 0
            ;;
        --config)
            COMPREPLY=( $(compgen -f -- "$cur") )
            return 0
            ;;
        -t|--tick|--remote|--remote-cmd|--serve-metrics|--alert-vram|--alert-kv|--alert-queue)
            return 0
            ;;
    esac

    COMPREPLY=( $(compgen -W "$opts" -- "$cur") )
    return 0
}
complete -F _toptop toptop
