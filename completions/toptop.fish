# fish completion for toptop
# Install: copy to ~/.config/fish/completions/toptop.fish

complete -c toptop -f
complete -c toptop -s t -l tick -d 'Refresh interval in milliseconds' -r
complete -c toptop -l theme -d 'Color theme' -xa 'gruvbox nord dracula tokyonight matrix'
complete -c toptop -l tree -d 'Start in process-tree view'
complete -c toptop -l no-tree -d 'Start in flat process view'
complete -c toptop -l list-themes -d 'Print available themes and exit'
complete -c toptop -l snapshot -d 'Print a one-shot text snapshot and exit'
complete -c toptop -s h -l help -d 'Show help and exit'
complete -c toptop -s V -l version -d 'Show version and exit'
