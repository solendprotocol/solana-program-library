let SessionLoad = 1
let s:so_save = &g:so | let s:siso_save = &g:siso | setg so=0 siso=0 | setl so=-1 siso=-1
let v:this_session=expand("<sfile>:p")
silent only
silent tabonly
cd ~/Projects/solend/solana-program-library
if expand('%') == '' && !&modified && line('$') <= 1 && getline(1) == ''
  let s:wipebuf = bufnr('%')
endif
let s:shortmess_save = &shortmess
if &shortmess =~ 'A'
  set shortmess=aoOA
else
  set shortmess=aoO
endif
badd +11 ~/.zshrc
badd +2502 token-lending/program/src/processor.rs
badd +608 term://~/Projects/solend/solana-program-library//69736:/bin/zsh
badd +79 token-lending/program/tests/helpers/mod.rs
badd +171 ~/.cargo/registry/src/github.com-1ecc6299db9ec823/solana-banks-client-1.8.14/src/lib.rs
badd +205 ~/.config/nvim/init.vim
badd +343 token-lending/program/src/state/reserve.rs
badd +170 token-lending/program/src/error.rs
badd +77 term://~/Projects/solend/solana-program-library//92423:/bin/zsh
badd +1 ci/solana-version.sh
badd +8 token-lending/program/Cargo.toml
badd +1 Cargo.lock
badd +4 token-lending/program/tests/flash_borrow_repay.rs
badd +3 token-lending/program/tests/init_reserve.rs
badd +156 token-lending/program/tests/flash_loan.rs
badd +39 term://~/Projects/solend/solana-program-library//59649:/bin/zsh
badd +842 ~/.cargo/registry/src/github.com-1ecc6299db9ec823/solana-program-1.9.18/src/instruction.rs
badd +194 ~/.cargo/registry/src/github.com-1ecc6299db9ec823/solana-program-1.9.18/src/program_stubs.rs
badd +470 ~/.cargo/registry/src/github.com-1ecc6299db9ec823/solana-program-test-1.9.18/src/lib.rs
badd +383 ~/.cargo/registry/src/github.com-1ecc6299db9ec823/solana-program-runtime-1.9.18/src/invoke_context.rs
badd +1 token-lending/program/src/instruction.rs
badd +10 token-lending/program/src/state/last_update.rs
badd +23 token-lending/program/src/state/obligation.rs
badd +0 token-lending/program/token-lending/program/src/instruction.rs
badd +5 term://~/Projects/solend/solana-program-library/token-lending/program//24775:/bin/zsh
badd +64 ~/.config/nvim/syntax/move.vim
argglobal
%argdel
edit token-lending/program/src/instruction.rs
let s:save_splitbelow = &splitbelow
let s:save_splitright = &splitright
set splitbelow splitright
wincmd _ | wincmd |
vsplit
1wincmd h
wincmd w
let &splitbelow = s:save_splitbelow
let &splitright = s:save_splitright
wincmd t
let s:save_winminheight = &winminheight
let s:save_winminwidth = &winminwidth
set winminheight=0
set winheight=1
set winminwidth=0
set winwidth=1
exe 'vert 1resize ' . ((&columns * 155 + 89) / 179)
exe 'vert 2resize ' . ((&columns * 23 + 89) / 179)
argglobal
balt token-lending/program/src/state/obligation.rs
setlocal fdm=manual
setlocal fde=0
setlocal fmr={{{,}}}
setlocal fdi=#
setlocal fdl=0
setlocal fml=1
setlocal fdn=20
setlocal nofen
silent! normal! zE
let &fdl = &fdl
let s:l = 457 - ((54 * winheight(0) + 38) / 77)
if s:l < 1 | let s:l = 1 | endif
keepjumps exe s:l
normal! zt
keepjumps 457
normal! 09|
lcd ~/Projects/solend/solana-program-library/token-lending/program
wincmd w
argglobal
if bufexists(fnamemodify("~/Projects/solend/solana-program-library/token-lending/program/src/state/reserve.rs", ":p")) | buffer ~/Projects/solend/solana-program-library/token-lending/program/src/state/reserve.rs | else | edit ~/Projects/solend/solana-program-library/token-lending/program/src/state/reserve.rs | endif
if &buftype ==# 'terminal'
  silent file ~/Projects/solend/solana-program-library/token-lending/program/src/state/reserve.rs
endif
balt ~/Projects/solend/solana-program-library/token-lending/program/src/state/last_update.rs
setlocal fdm=manual
setlocal fde=0
setlocal fmr={{{,}}}
setlocal fdi=#
setlocal fdl=0
setlocal fml=1
setlocal fdn=20
setlocal nofen
silent! normal! zE
let &fdl = &fdl
let s:l = 21 - ((20 * winheight(0) + 38) / 77)
if s:l < 1 | let s:l = 1 | endif
keepjumps exe s:l
normal! zt
keepjumps 21
normal! 011|
lcd ~/Projects/solend/solana-program-library/token-lending/program
wincmd w
exe 'vert 1resize ' . ((&columns * 155 + 89) / 179)
exe 'vert 2resize ' . ((&columns * 23 + 89) / 179)
tabnext 1
if exists('s:wipebuf') && len(win_findbuf(s:wipebuf)) == 0 && getbufvar(s:wipebuf, '&buftype') isnot# 'terminal'
  silent exe 'bwipe ' . s:wipebuf
endif
unlet! s:wipebuf
set winheight=1 winwidth=20
let &shortmess = s:shortmess_save
let &winminheight = s:save_winminheight
let &winminwidth = s:save_winminwidth
let s:sx = expand("<sfile>:p:r")."x.vim"
if filereadable(s:sx)
  exe "source " . fnameescape(s:sx)
endif
let &g:so = s:so_save | let &g:siso = s:siso_save
set hlsearch
nohlsearch
let g:this_session = v:this_session
let g:this_obsession = v:this_session
doautoall SessionLoadPost
unlet SessionLoad
" vim: set ft=vim :
