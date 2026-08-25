# The logon wall was a guest account state, not an emulator bug (2026-08-25)

Investigation record for the `LogonUI.exe` recycle that blocked the installed
Win7 image from ever reaching a shell. Normative resume state lives in
`wiki/state/repo-state-and-structure.md`.

## The symptom

After the install completed and OOBE marked the image `IMAGE_STATE_COMPLETE`,
the second boot reached `winlogon.exe` and then never progressed:

- a new `LogonUI.exe` appeared roughly every 250 M instructions, each one
  exiting with status `0`;
- `winlogon` was seen issuing `NtTerminateProcess` followed immediately by
  `NtCreateUserProcess` — it was deliberately recycling the process, not
  catching a crash;
- the framebuffer was black plus a cursor, and when `LogonUI` was prevented
  from being torn down it painted the **Harmony logon wallpaper** but never any
  credential chrome;
- no `userinit.exe`, no `explorer.exe`, ever.

Long chains of diagnosis had attributed this to the missing WDDM path (no
`dwm.exe` under XPDM `vga.sys`) and to an unapplied unattend AutoLogon.

## The actual cause

Reading the installed hives offline settles it. The image was installed from an
unattend containing `SkipUserOOBE` / `SkipMachineOOBE` plus an `AutoLogon` block
for `Administrator`. `SkipUserOOBE` means the "create your user account" screen
never runs, so **no ordinary account was ever created**, and the AutoLogon half
of the answer file only partially applied.

`SAM\Domains\Account\Users` contains exactly two accounts, and the `F` record
account-control bits show both are disabled:

| RID | Account | ACB | State |
|--|--|--|--|
| `0x1F4` | Administrator | `0x0211` | `ACCOUNTDISABLE` set |
| `0x1F5` | Guest | `0x0215` | `ACCOUNTDISABLE` set |

`HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon` had
`AutoLogonCount = 980` but **no `AutoAdminLogon`, no `DefaultUserName` and no
`DefaultPassword`**.

So Windows was behaving correctly: autologon was not armed, and the interactive
logon UI had **zero enabled accounts to offer**. `LogonUI` had nothing to draw a
tile for, exited cleanly, and `winlogon` restarted it forever. The wallpaper
painting proves the whole display and session path already worked; the missing
credential tiles were the tell.

This is guest configuration, not a device-model or CPU defect. None of the
display, input, storage or CPU work was implicated.

## The fix

Applied offline to the materialised install image, which is what the unattend
was supposed to have applied:

- `SAM` — Administrator `F` account-control `0x0211` → `0x0230`: clear
  `ACCOUNTDISABLE`, set `PASSWD_NOTREQD`.
- `SAM` — Administrator `V` record, NT hash entry (index 14) length `20` → `4`,
  i.e. header only and no hash, which is a blank password.
- `SOFTWARE` — add `AutoAdminLogon = "1"`, `DefaultUserName = "Administrator"`,
  `DefaultPassword = ""`, `DefaultDomainName = "."`, and set `AutoLogonCount` so
  it does not expire.
- `SYSTEM` — `Control\Lsa\LimitBlankPasswordUse = 0` in both control sets, so a
  blank password is usable.

Both `SAM` and `SOFTWARE` had matching primary/secondary sequence numbers, so no
transaction-log replay could overwrite the edits.

Adding the three `Winlogon` values needed real cell allocation rather than the
in-place value renames used previously against live guest RAM: the hive has
roughly 969 KB of free cells, so the new `vk` records, their data and a widened
value list are carved from that free space with the remainder left free.

## Materialising the image

The install lived as a 14.4 GB AeroSparse overlay over the 32 GiB base. The
format is a 64-byte header plus a flat `u64` block table where `0` means "not
allocated, read the base", so flattening it to a single raw image is a header
parse and 13 758 block copies. That raw image is what gets mounted, edited and
booted, which also makes the install reproducible without carrying the overlay.

## The second wall: injected platform time ran the guest clock ~480x fast

With the account enabled and autologon armed, a cold boot still recycled
`LogonUI`: `AERO_LOG_SYSCALL=aa,29` on winlogon's CR3 shows
`NtTerminateProcess(handle, 0)` immediately followed by `NtCreateUserProcess`,
about every 250 M instructions, and the framebuffer stays black plus a cursor
because `LogonUI` is killed before it paints.

That cadence is an emulator artifact. `Machine::maybe_busy_tick_starved_waiters`
injects a 16 ms platform-timer period after any full long-mode slice, to stop
compute-bound guest threads from starving `DelayExecution` waiters. It was
billed **per slice**, and `aero-machine-cli` drives the guest in 100 k-instruction
slices — so guest wall-clock advanced 16 ms for every 100 k instructions, about
**160 guest-seconds per billion retired**. The guest's own TSC advances one cycle
per instruction and HAL calibrated it at ~3 GHz, so tick-based time ran roughly
480x ahead of TSC-based time.

Windows' watchdogs are wall-clock. Winlogon gives `LogonUI` a bounded time to
report itself ready; under that dilation the deadline expired after ~190 M
instructions, which is the observed ~250 M recycle. `LogonUI` never had enough
instruction budget to finish starting, on any lineage — which also explains why
the older poke-based sessions only ever saw the logon *wallpaper* (reached once
the termination was NOP'd out) and never credential chrome.

The fix bills the tick against retired instructions instead of slice boundaries,
so both tick paths share the one rate the mid-slice path already documented
(`busy_tick_mid_period()`, default 1 M instructions, `AERO_BUSY_TICK_PERIOD`).
That is a 10x reduction in dilation. Regression test:
`busy_waiter_tick_rate_does_not_depend_on_how_execution_is_sliced` runs the same
2 M instructions as one slice and as twenty 100 k slices and requires the
injected platform time to match.

The general rule this encodes: **anything that injects virtual time must be
billed against retired guest instructions, never against however the embedder
chose to slice execution.** Otherwise the guest's sense of elapsed time depends
on the host's scheduling granularity, and every guest timeout becomes a function
of an implementation detail.

## Consequence: honest time makes the `drvinst` Windows Update wait expensive

Correcting the dilation cuts both ways. First-logon PnP spawns four
`drvinst.exe` workers that block in `drvinst!GetSearchOrder`'s Windows Update
search with a **300 s** default before giving up with `ERROR_TIMEOUT` (`0x5B4`)
and respawning. Under the old 160-guest-seconds-per-billion dilation that wait
burned ~1.9 B instructions; billed honestly it costs ~19 B, and the four workers
starve `explorer.exe` of CPU while Active Setup's "Personalized Settings" dialog
sits on screen.

There is a real guest-side fix for an offline VM, and it is the same shape as
the logon one — configure the guest instead of patching the emulator: set
`HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\DriverSearching\SearchOrderConfig`
to `0` (do not search Windows Update). `GetSearchOrder` already treats the
missing value as `0`/local-only, but the search still runs, so the value has to
be present. Apply it offline with `tools/win7-bringup/configure-autologon.py` before
the boot that will log on.

For a lineage already past logon the cheap escape is the knob: the LogonUI
watchdog is behind us, so `AERO_BUSY_TICK_PERIOD=10000` temporarily runs guest
wall-clock fast again and expires the outstanding 300 s waits in ~190 M
instructions instead of ~19 B. Put it back to the default afterwards.

## What the shell is waiting on after logon

`AERO_DUMP_MODULES=svchost.exe` on the logged-on session identifies the hot
process by its DLL set. The sampled RIPs land in `repdrvfs.dll` and the `msvcrt`
string routines it calls, inside the 104-module netsvcs `svchost` that hosts
`winmgmt` (`wmisvc` / `wbemcore` / `FastProx` / `esscli` / `repdrvfs`) alongside
`gpsvc`, `profsvc` and `themeservice`. That is the **WMI CIM repository walk** —
the same workload that gated Setup earlier in this project's history, now
running again at first logon while Active Setup's "Personalized Settings"
dialog waits.

So the remaining wait is genuine guest computation, not a wedge: the census
keeps changing, `drvinst` workers reach `ERROR_TIMEOUT` and exit, and the
kernel-side samples are a ~1.4 KB driver routine on a System thread rather than
a tight spin. It is also the natural place for the next fuse, since `repdrvfs`
UTF-16 comparison through `msvcrt` is exactly the shape the existing
KERNELBASE NLS fuse targets.

## Status

- [verified] Root cause 1: both accounts disabled, autologon not armed.
- [verified] Offline hive edits applied and read back from disk.
- [verified] The reconfigured image cold-boots through `smss` → session 0
  (`csrss` / `wininit` / `services` / `lsass` / `lsm`) → `winlogon.exe`.
- [verified] Root cause 2: the starved-waiter busy tick was billed per slice, so
  guest wall-clock ran ~480x ahead of the guest TSC and winlogon's `LogonUI`
  readiness deadline expired every ~250 M instructions.
  `AERO_LOG_SYSCALL=aa,29` on winlogon's CR3 caught the
  `NtTerminateProcess(handle, 0)` → `NtCreateUserProcess` pair directly.
- [verified] **The Win7 logon screen paints naturally.** With both fixes, an
  800 M-instruction continuation from a cold boot spawns **no** replacement
  `LogonUI`, `non_zero` goes 144 → 480 000 and `lfb_wr` → 710 661, and the
  framebuffer is the real "Windows 7 Professional" logon screen with the
  **Administrator** credential tile, accessibility button and power button.
  This is the first credential chrome this project has produced; every earlier
  lineage stopped at black-plus-cursor or, with the termination poked out, a
  bare wallpaper. Capture: `<work>/probe-v5.png`.
- [verified] **The logon-screen checkpoint is reproducible.** Reloading
  `<work>/logon-screen.bin` and running 40 M instructions produces a
  framebuffer that is byte-identical (SHA/MD5 `b8666632a5064d10a18275187dca3d87`)
  to both the original 800 M continuation and the independent grind slice that
  first reached it.
- [verified] **The logon completes.** One `AERO_INJECT_KEY=Enter` with
  `AERO_INJECT_KEY_ONCE=1` on the credential tile (blank password) takes the
  guest through to the Windows 7 "Welcome" splash with **`userinit.exe`,
  `explorer.exe` and `dwm.exe` all alive** — no pokes, no forced resume, no
  token forgery, and `dwm.exe` running at all is new (earlier sessions
  concluded XPDM `vga.sys` meant DWM would never start). Checkpoint
  `<work>/desktop/desktop-10.bin`, capture `desktop-10.png`.
- [verified] **The Windows 7 desktop shell paints.** After Active Setup's
  "Personalized Settings" dialog clears, `explorer.exe` builds the shell and the
  framebuffer shows the **taskbar**: Start orb, Internet Explorer, Windows
  Explorer and Media Player pinned buttons, the show-hidden-icons chevron and
  the system-tray clock. `non_zero` settles at 32 144 — an 800x40 taskbar is
  32 000 pixels — with `lfb_wr` climbing past 295 000, and the census is 34
  processes including `explorer.exe`, `dwm.exe`, `SearchIndexer`, `sppsvc`,
  `WmiPrvSE` and `WMIADAP`. **A live `Shell_TrayWnd` is what every earlier
  lineage failed to produce**; the poke-based sessions got as far as `Progman`,
  `SHELLDLL_DefView` and `WorkerW` but never the tray, and reached those only by
  forcing `CTray::InitWindow`, planting a vtable and calling `SetWindowPos` by
  hand. This one is unforced: no `AERO_POKE_*`, no `FORCE_RESUME`, no `--jit`.
  Proof: [`wiki/assets/win7-taskbar-first-paint.png`](../assets/win7-taskbar-first-paint.png),
  checkpoint `win7-desktop.bin` in the bring-up working directory.
- [verified] **The full desktop paints.** One slice later the whole 800x600
  surface is drawn: the Windows 7 Harmony wallpaper, the Windows logo, the
  **Recycle Bin** desktop icon and the taskbar. `non_zero=479996` of 480 000
  and `lfb_wr` past 1 048 000. Proof:
  [`wiki/assets/win7-desktop.png`](../assets/win7-desktop.png); checkpoint
  `win7-desktop.bin` + `win7-desktop.aerospar` in the bring-up working directory.
- [verified] **The desktop checkpoint is reproducible.** Reloading it and
  running 60 M instructions redraws a byte-identical frame
  (`cc07fcd926a9979eb0a25bf26f47c42f`) with 36 live processes — and it was
  reloaded on a *different* interpreter build than the one that produced it, so
  that also demonstrates the memoised-dispatch change is behaviour-preserving on
  a real Windows desktop workload, not just under the unit suites.
