## jzombie portfolio cluster

How these 15 repos connect — ones that work together sit close together. 21 links between them (+2 from docs alone); layers 4 foundation · 5 middleware · 7 application · 1 isolated. Busiest right now: rust-sec-fetcher; most reused: rust-line-ending (5 use it).

Rebuilt nightly from GitHub data — last run 2026-11-14 23:03 UTC. Every label links to its repo.

<sub>Diagram: repos that work together sit close together. Screen readers: the block below uses braille characters for lines and is noisy, so it is hidden; the same edges are machine-readable in `data/portfolio-graph.json`.</sub>

<pre aria-hidden="true">
                                 [<a href="https://github.com/jzombie/windows-headless">windows-headless</a>]
        [<a href="https://github.com/jzombie/rust-sec-fetcher">sec-fetcher</a>]             [<a href="https://github.com/jzombie/rust-cache-manager">cache-manager</a>]⠉⠉[<a href="https://github.com/jzombie/lab-qemu-portable">lab-qemu-portable</a>]
               ⡇⠈⠑⠢⢄⡀              ⣀⠤⠔⠊⠉⡰⠁
               ⠸⡀   ⠈⠑⠢⢄⡀     ⣀⠤⠔⠊⠉    ⡜
                ⢇  [<a href="https://github.com/jzombie/rust-reqwest-drive">reqwest-drive</a>]   ⢀⠎
                ⠘⡄      ⢀⠜⢸⠘⢄       ⢠⠃
                 ⢣     ⡠⠃ ⠸⡀ ⠑⡄    ⡰⠁
                 ⠘⡄  ⢀⠜    ⡇  ⠈⠢⡀ ⡔⠁
       [<a href="https://github.com/jzombie/rust-llkv">llkv</a>]⡀    ⢱ ⡠⠃     ⡇[<a href="https://github.com/jzombie/rust-triplets">triplets</a>]
            [<a href="https://github.com/jzombie/rust-simd-r-drive">simd-r-drive</a>]⠉⡏     ⢸
                  ⠨⡂       ⢇      ⡇
                  ⠨⡂       ⢸      ⠸⡀
                  ⠨⡂[<a href="https://github.com/jzombie/jzombie-graph">jzombie-graph</a>]⢇
                  ⠨⡂        ⠱⡀     ⠸⡀
                  ⠨⡂         ⠑⡄     ⢣
                   ⠈⠢⡀        ⠘⡄    ⠘⡄
                     ⠈⠢⡀    [<a href="https://github.com/jzombie/rust-oxdock">oxdock</a>]⡀⢱
                     [<a href="https://github.com/jzombie/rust-muxio">muxio</a>]⠭⠥⠤⠤⠤⠬⠵⠤⡬⠭⡯[<a href="https://github.com/jzombie/term-wm">term-wm</a>]⢄⢄
                                [<a href="https://github.com/jzombie/rust-line-ending">line-ending</a>]⡇    ⠑⢄
                                    ⢸[<a href="https://github.com/jzombie/term-wm-vt100">term-wm-vt100</a>]⡇
                                     ⠉⠑⢄          ⡠⠊
                                        ⠑⢄      ⡠⠊
                                          ⠑⢄  ⡠⠊
                                            ⠑⢎
                                          [<a href="https://github.com/jzombie/rust-argtuner">argtuner</a>]

</pre>
Repos that work together pull close; unconnected ones drift apart. Forks show up only after diverging or being renamed; repos quiet for over a year are left out unless something current uses them.
