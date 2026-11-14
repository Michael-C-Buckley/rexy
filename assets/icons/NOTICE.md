# Program icons

This folder is for the tiles of Superlogical's Rex icon set, the icons the official Rex clients show for the program running in a terminal. The artwork is Superlogical's, and the logos it draws are trademarks of their owners. Superlogical has published no terms for the set, so **the icons are not kept in this repository** and no licence here covers them.

`scripts/icons` fetches them from [rex-rune](https://github.com/shuv1337/rex-rune), snapshot `db0f86c`, whose `scripts/program-icons.py` converted the set to plain SVG: each tile, a program's mark on a ground of its colour, on a square canvas. Whichever are here when Rex Lite is built go into it; a tab whose program has no icon here shows a prompt. Before passing on a build that has them, ask Superlogical.

`src/program.rs` lists the icons, with the names each goes by.
