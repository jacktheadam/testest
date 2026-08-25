# Windows 7 bring-up tools

Host-side tools that were written during the Windows 7 bring-up and are worth
keeping. They are in the repository because the wiki cites them and because they
were previously reachable only as loose files in one machine's working
directory — which is exactly how the earlier bring-up scripts were lost and had
to be rewritten from memory.

| Tool | What it does |
|---|---|
| `hive.py` | Minimal read/patch access to Windows registry hive files. Walks keys, reads values, edits a value in place when the replacement fits the existing cell, and allocates a new cell when it does not. Enough for offline guest configuration; not a general hive library. |
| `configure-autologon.py` | Applies, offline, the unattended-logon settings the install's answer file was supposed to apply: enables Administrator with a blank password, arms autologon, and permits blank-password logon. |
| `bench-insns.sh` | Measures marginal host cost per guest instruction using retired-instruction counters, by differencing two guest-instruction budgets. The only throughput metric that is stable on a shared machine. |

## Why the autologon tool exists

The bring-up image was installed with an answer file that requested autologon as
Administrator and skipped the out-of-box user setup — so no ordinary account was
ever created, and both built-in accounts were left disabled. That leaves the
logon UI with nothing to display: it exits immediately, `winlogon` respawns it,
and the screen flickers forever.

It is worth being precise that this is a **guest configuration** fix and not
emulator scaffolding. Nothing here patches guest memory at runtime or stubs a
kernel routine; it edits the installed image's registry the way the installer
would have. The distinction matters, and the wiki keeps it — see
[the Windows 7 bring-up](../../wiki/history/windows-7-bring-up.md) for what was
scaffolded and what was not.

## Registry hive editing, briefly

Hives are made of cells inside `hbin` blocks. Editing a value in place — same
cell, same size — leaves every offset in the file untouched, so no allocator or
parent-pointer bookkeeping is involved and the change is safe. Adding a value,
or growing one past its cell, needs a real allocation: `hive.py` walks the
`hbin` list for a free cell of sufficient size, splits it, and fixes up the
parent's value list. The base block carries an XOR checksum over its first 508
bytes, recomputed on save; get that wrong and Windows rejects the hive.

The hives must not be in use. Mount the guest volume read-write, run the tool,
and unmount so the writes are flushed before the guest boots.
