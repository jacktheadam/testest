# Wiki assets

Images that are evidence for a claim made elsewhere in the wiki. They live in
the repository rather than in a working directory on one machine, because a
claim whose proof can evaporate is not a claim anyone can check later — see the
rule on evidence durability in
[../meta/working-agreements.md](../meta/working-agreements.md).

Keep this directory small and every file referenced from a page. An image nobody
links to is not evidence, it is weight.

| File | What it shows |
|---|---|
| `win7-desktop.png` | The Windows 7 desktop in the Aero emulator: Harmony wallpaper, Recycle Bin, and the taskbar with Start orb, Internet Explorer, Windows Explorer, Media Player and the notification-area clock. 800×600×32 from the native runner, with 479,996 of 480,000 pixels non-black. |
| `win7-taskbar-first-paint.png` | The same session earlier: the taskbar has painted but the desktop is still black — 32,144 non-black pixels, which is essentially the 800×40 taskbar strip alone. Kept because "the taskbar exists" and "the desktop is rendered" are different milestones, and conflating them is how a bring-up flatters itself. (An earlier draft of the state page did exactly that, quoting the taskbar figure for the desktop.) |

The guest clock reads `1/1/2000` in both because guest wall-clock time is not
set from the host — a detail of the time-injection work, not a fault.
