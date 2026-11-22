" Vim settings for the screenshot tmux panes.
set nocompatible
syntax on
filetype plugin indent on
set number nowrap cursorline laststatus=2 noswapfile shortmess+=I
set fillchars+=vert:│
" habamax ships with Vim 9; older Vims keep their default scheme.
silent! colorscheme habamax
