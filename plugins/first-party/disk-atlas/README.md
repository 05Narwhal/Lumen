# Disk Atlas for Lumen

Lets [Disk Atlas](https://github.com/05Narwhal/disk-atlas) answer Lumen's file and folder dialogs
(*Choose folder*, *Open*, *Save*) and *Show in folder*, instead of the system's.

It needs Disk Atlas installed (a version with `--pick` and `--select`). Lumen finds it by itself;
if it can't, Settings → Plugins → Disk Atlas has *Locate…*. If Disk Atlas can't be started, or exits
with an error, Lumen opens the system dialog instead. Closing the dialog without choosing cancels,
with no second dialog.

It has no code (no `plugin.wasm`) and asks for no permissions: it only names a program Lumen may start
and the arguments to give it. Each of the two switches turns one use off.
