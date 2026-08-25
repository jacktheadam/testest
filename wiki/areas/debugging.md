# Debugging and introspection

> The instruments Aero exposes for understanding a running guest — CPU tracing,
> memory watchpoints, device and register dumps, GPU command capture and
> replay, and the browser-side debug surfaces — and the discipline for using
> them.
>
> The bring-up loop these tools serve is described in
> [../history/windows-7-bring-up.md](../history/windows-7-bring-up.md).

This document describes Aero's developer-focused debugging surface area: serial console capture, VM state inspection, breakpoints/stepping, and trace export.

---

## The loop itself is a debugging tool

The cost of answering one question dominates bring-up. A full cycle — rebuild,
boot to the failure point, look — was measured at around half an hour, and yields
a single framebuffer: roughly one bit of information. So the loop, not any
individual bug, is what decides how fast a causal chain can be walked. Backward-chaining a crash costs effort
proportional to chain length, and one root cause in
[../history/windows-7-bring-up.md](../history/windows-7-bring-up.md) needed six hops.

Three techniques change the cost of every investigation:

1. **Kernel debug output over COM1.** Windows will describe its own boot if the
   guest BCD sets `/DEBUG /DEBUGPORT=COM1 /BAUDRATE=115200`, and `bcdedit
   /bootdebug on` extends that back through bootmgr and winload. The payoff is
   concrete: on a bugcheck you get the stop code, its four parameters and a
   stack, instead of a screen. Aero already
   captures COM1 via `--serial-out`. Before this was understood, every serial log
   from a Win7 run was zero bytes — not because the UART was broken, but because
   nothing had been asked to talk. Producing such media is still blocked; see
   [Kernel debugging over COM1](#kernel-debugging-over-com1-blocked-on-the-bcd-edit).
2. **Snapshot-first iteration.** Boot once to just before the wall, save, and
   resume from there; a multi-minute boot becomes seconds. Snapshots are tied to
   the CPU state layout, so expect to regenerate them when that changes, and
   prefer a loud failure to a silent mismatch.
3. **First-divergence against QEMU.** The first place two emulators disagree
   while running the same guest *is* the bug, found in one query rather than one
   per hop, which makes chain length irrelevant.

### The bring-up loop scripts

These exist so that the flags which make a run explain itself are not optional —
the reason instrumentation kept being left off was that a boot was an eight-flag
command line typed by hand.

| Command | What it does |
| --- | --- |
| `scripts/win7.sh snapshot [budget_ms]` | boot cold, save the machine just before the wall |
| `scripts/win7.sh resume [budget_ms]` | resume from that snapshot — the fast loop |
| `scripts/win7.sh cold [budget_ms]` | boot cold without saving |
| `scripts/win7.sh ladder [run_dir]` | report the furthest milestone reached |
| `scripts/win7-diverge.sh [max_insts] [mode]` | first divergence against QEMU; modes `raw`, `first-visit`, `page` |
| `scripts/win7-debug-media.sh` | build install media with kernel debugging enabled over COM1 |

The `justfile` wraps the first five as `just boot-snapshot`, `boot-resume`,
`boot-cold`, `boot-ladder` and `boot-diverge`, with the same budgets as defaults
— use whichever you prefer; they are the same thing.

Every `win7.sh` mode writes a run directory containing `run.log` (progress
samples), `serial.log`, `debugcon.log` and `fb.png`.

`tools/tracediff/tracediff.py` is the divergence engine underneath
`win7-diverge.sh`. It diffs an Aero execution trace (`AERO_TRACE_COMPACT`)
against a QEMU one (`qemu-system-x86_64 -d exec -D <file>`). QEMU can also expose
a GDB stub (`-s -S`) for hardware breakpoints and watchpoints against the
reference side, which needs no QEMU rebuild — **Aero has no GDB stub**, so this
is a way to interrogate the reference, not to single-step the two in lockstep. It lives in the repository
deliberately: it previously existed only as `/tmp/flowdiff.py`,
`/tmp/firstvisit.py` and `/tmp/ordercmp.py` with a note to promote it "once the
shape settles", `/tmp` was cleared, all three were lost, and the technique had to
be rebuilt from memory every time it was wanted. That is most of the reason the
strategic tool was only ever used ad hoc.

### Kernel debugging over COM1: blocked on the BCD edit

`scripts/win7-debug-media.sh` produces media with `/DEBUG /DEBUGPORT=COM1
/BAUDRATE=115200` set, `bcd_patch` understands the elements that mean it, and
`bcd_patch::inplace` edits a hive in place rather than rebuilding it. None of it
boots yet.

Rebuilding the ISO with `xorriso` fails at `CDBOOT: Couldn't find BOOTMGR` —
Win7's El Torito loader depends on the original ISO9660 layout. Splicing the
patched hive into a copy at its original byte offset avoids that; the store is
located by scanning for the `REGF` signature on 2048-byte sector boundaries, and
the one at offset 6354944 is confirmed to be `boot/bcd` by an exact hash match
against a `7z`-extracted copy.

Every spliced image nonetheless faults early, at `rip=0x03ea` in protected mode
executing interrupt-vector-table content. That held for the rebuild-based
patcher, for a rebuild with every flag off (a pure read-then-write), for the
in-place editor which preserves file size exactly and changes 0.66% of bytes, and
for splicing only `boot/bcd` while leaving the second store untouched.

Two controls locate the fault:

- A byte-identical `cp` of the ISO boots normally, so the copy and splice
  machinery are sound.
- **QEMU fails on the modified ISO in the same way** — booted headless to the
  same wall-clock point, the unmodified ISO paints 479,439 non-black pixels and
  the modified one paints zero.

A second, independent emulator rejecting the same image means the fault is in the
BCD edit, not in Aero and not in the rebuild-versus-in-place distinction. Nothing
here casts doubt on the driver-signing path.

**Where to pick it up.** Both code paths share `element_key_name`,
`bcd_encode_boolean` and the object selection in `select_target_objects`, and all
of them add element *keys* absent from this media (`16000048`, `16000049` and the
debug set are all missing from a stock `boot/bcd`). The next question is which of
those three is wrong, and the cheap way to ask it is a differential: patch a
store, mount both under a working Windows or a second `REGF` reader, and diff the
object trees against what `bcdedit` produces for the same change. Reaching for
the emulator first is what cost the time here.


## The bring-up instruments

These are the instruments the Windows 7 bring-up was actually done with. The
rule they serve is that bring-up is **never open-loop**: make the machine tell
you what happened, then confirm it against QEMU before changing anything. They
are grouped here by the question each one answers.

### "What state was the machine in when it faulted?"

Nothing to enable — on any fault the CLI prints RIP, the CS base, the linear
address, the mode, the instruction bytes ±24, the stack, the bytes below SP, the
GDTR and the full GDT (`exception_debug_context` in
`crates/aero-machine-cli/src/main.rs`). Most faults are diagnosable from this
alone, so read it before reaching for anything below.

### "What did it execute?"

`AERO_TRACE_FROM=<n>` and `AERO_TRACE_TO=<m>` print each instruction — mode,
`cs_base:ip`, raw bytes, general-purpose registers and flags — within a global
instruction-count window. `AERO_TRACE_COMPACT=1` switches to an address-only
form fast enough to capture a whole run.

> **Build the binary, not the library.** `cargo build -p aero-machine` refreshes
> the *library* and leaves `target/release/aero-machine` untouched, so the next
> run silently executes the previous build. Use
> `cargo build -p aero-machine-cli --release --locked`. This has cost real time
> more than once — a billion-instruction slice was run against a stale binary
> before anyone checked the inode.

> **Hook hazard.** This lives in the Tier-0 batch loop
> (`run_batch_cpu_core_with_assists` in
> `crates/aero-cpu-core/src/interp/tier0/exec.rs`). The machine drives *that*
> loop, **not** `step_with_config` — a trace hooked into the latter will
> silently never fire.

### "Who wrote this value?"

`AERO_WATCH_WRITE=<addr>` logs any write covering a linear address;
`AERO_WATCH_VALUE=<val>` logs any write whose little-endian value matches. Block
copies are scanned too, and both print the writing RIP. They are hooked at the
single choke point in `crates/aero-cpu-core/src/linear_mem.rs`
(`write_u*_wrapped` / `write_bytes_wrapped`), so they catch ALU and MOV stores,
push and pop, string operations, and assist-layer writes alike.

`AERO_WATCH_READ=<addr>[,<addr>…]` is the read counterpart, covering the 16/32/64/128-bit
paths, block copies, and the 8-bit data path. It maintains a hit counter
(`linear_mem::WATCH_READ_HITS`) so a test can assert that it fired.

**`AERO_WATCH_PHYS_WRITE` watches a *physical* address**, and the distinction is
not academic. A linear watchpoint only sees writes through the mapping you named;
a page-table entry is routinely written through a *different* mapping, so a linear
watch on it sees nothing at all. The same-value page-table-reload defect could
only be pinned — down to four instructions — with a physical watch on the entry
itself. When you are watching guest paging structures, watch them physically.

`AERO_WATCH_EXEC=<addr>[,…]` is the third kind: it fires when execution reaches an
address, printing the full register set with `cr3` and the segment base, which is
how you prove a specific function ran at all. `AERO_WATCH_EXEC_DUMP` adds a memory
dump at the hit. `AERO_LOG_PF` logs page faults, capped so it cannot flood.

That counter matters more than it looks. **Proving a read never happened is a
result**, and often a better one than proving a value was wrong: it was a read
watchpoint with zero hits that established winload never reads the ACPI tables
before rejecting the firmware, which moved the whole search upstream. Before
theorising about what a guest validation computes, prove the validation runs at
all.

### "Where do the two emulators first disagree?"

`AERO_WRITE_STREAM=<path>` appends every write as `rip addr len value` in
execution order — around 191,600 records for a two-million-instruction boot.

This is Aero's half of **forward first-divergence diffing**, and it is the
strategically important instrument here. Backward-chaining from a crash costs
effort proportional to the chain length, and the worst chain in this bring-up was
six hops. Diffing forward makes chain length irrelevant: run both emulators to
the same budget, compare the streams, and the first differing record is at or
adjacent to the bug.

The reference half is less finished. QEMU needs a write stream of its own, and
the clean way is the TCG `mem` plugin — but no plugin shared object ships in the
Debian package, so it needs a QEMU source build. Until that exists, the working
substitutes are the KVM plus GDB hardware watchpoint for a specific address, and
a cheaper set-difference of the basic blocks QEMU executes but Aero does not
(`-d exec` against `AERO_TRACE_COMPACT`), which lands directly on skipped code
paths — the "the writer never ran" class of divergence.

### How these instruments mislead

Every one of the following cost real time during the bring-up. They are
properties of the tools, not of any particular bug, so they will recur.

**A watch-value hit tells you the value *moved*, not that it is *wrong*.** The
worst red herring of the bring-up was a `AERO_WATCH_VALUE` hit that looked like
a corrupted pointer table being built. It was a callee spilling an argument
pointer to its own stack frame — entirely mundane. Days went into "who built the
table wrong" when the table was never the problem. Correlate a watch hit against
the full trace before theorising about it.

**Raw addresses are not comparable between Aero and QEMU.** QEMU's winload runs
at different physical addresses because different BIOS and e820 details shift the
boot allocations. Comparing "QEMU writes `0x0158a023` here, we write `0x194c78`"
is comparing two legitimately different layouts, and one recorded hypothesis
— a supposed "load-address divergence" — was an artefact of exactly this. Only
*semantic* steps are comparable across the two emulators.

**Control flow matching is itself a result.** If a page-level first-visit
comparison shows the two emulators executing the same path, the divergence is in
data, not in flow — which eliminates most of the search space in one query. Run
that comparison early.

**An instrument can lie.** The fault-context byte dumper once read the faulting
linear address as though it were physical, which in long mode with paging yields
open-bus `0xFF` bytes. That sent an investigation toward a page-table problem
that did not exist; the real cause was an unhandled MSR. It is fixed and the fix
carries a comment saying why, but the general form stands: when an instrument
reports something impossible, suspect the instrument before rebuilding your model
of the guest.

**A silent zero is the dangerous failure mode.** `AERO_WATCH_WRITE` once parsed
its value as a single integer, so passing two comma-separated addresses did not
watch two addresses — it silently disabled the watch and reported zero hits. Zero
hits is exactly the shape of a *positive* finding here ("that memory is never
written"), so the tool failing produced a confident, wrong conclusion rather than
an obvious error. It accepts a list now. Before relying on a zero-hit result,
arm the watch on something you know *is* touched and confirm it fires.

**Configuration is latched once per process.** The memory-debug hooks read the
environment behind a `OnceLock`, because the alternative is a `getenv` on every
guest memory access. In a test binary that means the first test to touch memory
decides the setting for every test after it, which is why the check is
re-evaluated per call under `cfg(test)`.

**Match guest memory against the kernel setup actually booted.** An early pass
compared guest RAM against `ntoskrnl.exe` from `install.wim`, inferred an image
base from it, and concluded that a paged section was unmapped. Windows Setup
boots the kernel from **`boot.wim` index 2**, which is a different binary with a
different hash and a different base. Everything downstream of the wrong PE was
noise.

**Debug scratch must not outlive the session that made it.** Two leftover decode
probes in the test directory broke the suite build against a since-changed
signature, in a way that had nothing to do with the work in flight.

### Your own scaffolding is a suspect

This is the most expensive lesson in the bring-up, and it recurred in several
forms. Before diagnosing anything, **list the overrides that are active and run
the control without them.**

A sticky `AERO_FORCE_CR8=0`, added to get past one wall, silently manufactured a
different bugcheck billions of instructions later — `ATTEMPTED_SWITCH_FROM_DPC`
— which was then investigated at length as a guest defect. The mechanism is
exact: the guest raises CR8 to run at deferred-procedure-call level, the override
forces it back to zero, the interrupt poll mirrors CR8 to the task-priority
register, and interrupts then land inside code that is not allowed to be
preempted. CR8 *is* the guest's interrupt-level contract; forcing it breaks the
invariant Windows depends on. It is now a documented prohibition with a warning
in the CLI.

The same shape appeared again later: a stubbed `KeWaitForSingleObject` returning
a synthetic failure produced a `MUTANT_NOT_OWNED` bugcheck inside GDI, which was
initially attributed to the emulator's graphics locking. **A synthetic error
return from a synchronisation primitive is never a safe stub** — waits on
ownership-transferring objects like mutants and resources carry a paired-release
invariant, so the illegal operation surfaces at some later, unrelated release.

And once a processor has bugchecked, every subsequent observation of it is
meaningless — "the machine is spinning" says nothing after that point.

### A lead is not proof

**A string in memory proves only that something containing that string is
loaded.** This was got wrong at least three times: a `framebuf.pdb` debug string
was read as the display driver being resident (its code signature was absent, and
the page holding the string had no mapping in any live address space); a
`winpeshl.exe` path in the registry was read as the process existing; and
`Shell_TrayWnd` string hits were read as a live taskbar when every hit was a
constant in a loaded module's string table. Liveness needs a live object — a
process entry, a mapped code signature, an executed `CreateWindowExW`.

The disciplined version costs little: reverse-map the physical page through every
live address space, and search for the module's actual code bytes rather than its
name.

**A zero-hit watchpoint, by contrast, can be decisive** — but only when the
reference binary makes the access *unconditional*. If the real driver always
probes the framebuffer after mapping it, then no probe means execution never got
there. That is a proof; "the value looked wrong" is not.

### Validate the debugger before you doubt the emulator

Two wrong structure offsets in Aero's own thread dumper produced a completely
coherent and completely wrong diagnosis: that a process's initial thread was
never made runnable. The thread environment block was being read from the wrong
offset, and the field being printed as "thread state" was not the state field on
this kernel build — it read zero for every thread, including ones visibly
running. The real situation was mundane priority starvation.

**A wrong offset does not produce an obviously wrong answer. It produces a
plausible one.** The tell was available the whole time: a field that reads
identically for every thread, including one you know is running, is not the field
you think it is. Cross-check every offset against a thread or process whose state
you already know.

The same caution applies to symbol files. Public symbols for a system library did
not match the copy of that library inside the boot image, and using them would
have produced confident nonsense — verify a symbol file against the actual bytes.

### Never attribute a fault to whatever is mapped there now

A crash appeared to be inside a cryptographic primitives library. It was not:
another module had been loaded at that address, had armed a callback, and had
then been unloaded — and a *different* library was later mapped over the same
range. The callback fired into whatever now occupied the address.

Reconstruct the module map **as it was when the pointer was created**, not as it
is when the pointer is used. What made the aliasing visible was checking an
earlier snapshot, where the address was not mapped at all. Symbol-plus-offset
matches are seductive and can be pure coincidence of layout.

### A hot loop is not a hang

Twice a run was written off as stuck when it was working. One was a bulk memory
copy whose call sites kept changing under timer preemption; the other was the
boot image's LZX decompression, identified by resolving the hot offsets in the
exact `wimfsf.sys` binary to its decoder functions. Sample the guest's own hot
addresses and resolve them against the real module before concluding anything.

Relatedly: **"slow" is a hypothesis, not an observation.** A run that appeared to
crawl at a third of the usual rate turned out to be sitting on the firmware's
`Press any key to boot from CD or DVD` prompt. Capture the screen first.

The reliable way to tell them apart is **not** the instruction pointer. Use the
guest's own progress counters: per-thread context-switch counts and the kernel's
tick count. Several walls in this bring-up — a plug-and-play initialisation, a
management-instrumentation repository walk, font rasterisation, a kernel name
hash — were all finite work misread as hangs, and in each case some thread's
context-switch count was climbing steadily the whole time. A histogram of unique
instruction pointers helps too: several thousand distinct addresses in a sample
is not a spin.

The shape of a wait is also informative. A thread waiting on a local-procedure
reply and a thread waiting on a user request are blocked on opposite sides of a
call, which tells you which end to investigate.

### Scaffolding that quietly does nothing

Worse than scaffolding that breaks things is scaffolding that appears to work.

**A patch must be byte-length exact.** A seven-byte instruction planted over an
eight-byte one left a trailing byte that corrupted the following instruction, and
the resulting downstream behaviour was bizarre enough to be investigated as a
guest problem. Pad to the original length.

**A poke only matters if the consumer re-reads the value.** Registry values were
rewritten in memory to arm an automatic logon, and nothing changed — a syscall
watch showed the consumer never queried those values again on the path in
question. Verify the effect with a watch, not by assuming.

**Re-test anything a poke appears to have unblocked.** A patched branch in the
window manager was credited with letting a frame complete. Running the identical
slice *without* it did the same thing: the guest had simply needed more time. The
patch was also actively harmful, leaving visible corruption.

**Hook one-shot, not every call.** Hooking a frequently-called routine to inject
a single action turns into a denial of service against the very thread you are
trying to help — repeating a desktop switch on every input poll stalled the
dialog it was meant to reveal. If a call must run on a specific thread with a
specific context, hook something that thread already calls, do the work once, and
restore the hook.

### Do not let a cleanup script eat the lineage

A snapshot lineage representing billions of instructions of progress was
destroyed by a pruning script whose keep-list was held in a single scalar
variable. **zsh does not word-split unquoted parameter expansions the way bash
does**, so the keep-list was one long string, matched nothing, and everything was
deleted. Use arrays, and prefer moving to a holding directory over deleting.

Note also that a snapshot capturing a *partially completed* guest operation is
not a valid restart point for that operation — a half-applied installation was
correctly rejected by the guest as corrupt on the next attempt. Track which
snapshots hold mid-transaction state.

### A late change cannot test an early decision

This invalidated two separate experiments, and it is easy to repeat. Applying a
change to a *restored checkpoint* cannot test a rule that fires *once, earlier*.
Plug-and-play makes a single attempt to claim a device; if that attempt already
happened under the old configuration, flipping the configuration afterwards
changes nothing and the null result means nothing. Likewise, rewriting a firmware
table in memory after the kernel has already parsed it tests nothing.

Topology and firmware changes must be tested **from a cold power-on**, on a fresh
disk overlay.

### Snapshot lineage hygiene

Snapshots carry stale device state that quietly invalidates experiments — a
cached BIOS video mode, a stale virtual scanline width, a PCI command word, a
firmware pointer in reserved memory. Several confident results turned out to be
artefacts of resuming the wrong lineage.

The working rules: keep a pristine, hashed disk overlay per investigation so a
later lineage can start from an untouched disk; label snapshots taken during
diagnostic continuations as contaminated, because they are; and when running an
A/B, start both branches from **byte-identical** overlays and compare the
outputs by hash.

**Publish negatives as negatives.** Identical output hashes across a change are
the honest way to say "this did nothing", and the log does this repeatedly.
Notably, one such A/B showed input being consumed by the guest keyboard
controller while the rendered frame stayed byte-for-byte identical — delivery
worked, there was simply nothing on screen to respond to it. That is a real
result and worth recording as one.

### Beware of internal consistency

The interrupt-routing defect recorded in
[the bring-up history](../history/windows-7-bring-up.md) is the cautionary case:
the firmware tables,
the device configuration, the runtime router and the polarity model all agreed
with each other, and all four were wrong, because every one of them derived from
the same incorrect constant. Local consistency tests cannot catch that class of
error. Only comparison against external ground truth can — and the structural fix
is to give the mapping exactly one home that every consumer derives from.

### "What is in memory right now?"

`AERO_DUMP_MEM=<addr>:<len>[,<addr>:<len>…]` hexdumps physical regions, capped at
`0x400` bytes each. It fires on the exception context *and* on clean stops —
error screens, halts, budget reached. The clean-stop case is the entire point: it
is how the RSDP and ACPI tables were read out of a guest parked on an error
screen.

`AERO_DUMP_LINEAR` is the same idea through the guest page tables, which is what
you want for anything above the kernel's high-half split.

### Changing the guest to test a hypothesis

`AERO_POKE_LINEAR=<va>:<hexbytes>[,…]` writes through the guest page tables, and
is **re-applied every host slice** rather than once. It has to be: a one-shot
write applied at restore lands under the boot loader's address space and
evaporates the moment the kernel remaps that address.

Treat every result obtained under a poke as **scaffolded, not natural**. A poke
answers "if this value were right, would the guest proceed?" — which is a real
and useful question — but it is not evidence that the guest can reach that state
by itself, and the distinction is easy to lose several hours into a session.
Poking is also easy to get half-right in a way that produces confident nonsense:
writing the service-descriptor table's limit without also converting the table to
its packed form simply moved the fault, because the dispatcher then read the
untouched absolute addresses as packed offsets. When a poke is used, record what
it was, and record separately what the guest later did without it.

Note also that a poke is applied under a particular address space. One written
while the boot loader's page tables are active silently evaporates when the
kernel remaps the address — so "the poke did not take" can be a statement about
*when* it was applied rather than about the value. And kernel addresses move
between boots, so absolute ones are not portable across lineages.

#### Overrides that are retired or hazardous

These still exist in the CLI and should not be reached for casually.

| Override | Status |
|---|---|
| `AERO_FORCE_CR8` | **Prohibited once a multi-threaded session is up**, and the CLI warns. Forcing the interrupt level to zero breaks the invariant that guarantees no thread switch at deferred-procedure-call level, and it manufactured a bugcheck billions of instructions after it was set. |
| `AERO_RDTSC_QUANTUM` | Its original behaviour — broadly fast-forwarding the timestamp counter — is **banned**: a controlled comparison showed it suppresses plug-and-play root initialisation. The name survives for recipe compatibility and now only yields tight timestamp-poll loops to the scheduler, never altering the architectural counter. Use `AERO_PM_TIMER_POLL_QUANTUM` for coherent acceleration. |
| `AERO_FORCE_IF` | A bring-up wedge for the firmware clock wait. The wait's two real root causes have both been fixed, and neither was ever addressed by this flag. |
| `AERO_STDVGA_ENABLE_IO` | A compatibility wedge for checkpoints predating the rule that enables legacy I/O decode for VGA-class functions at POST. Not needed on a cold boot. |
| `AERO_FORCE_RESUME` | Resumes execution at a chosen context. The spec is **keyed, not positional**: `cr3=<hex>,rip=<hex>,rsp=<hex>[,rax=][,teb=]`. Resuming user code without also switching the dispatcher's current thread runs it on whatever thread the emulator is actually on — right registers, wrong identity — which corrupts everything downstream. |
| `AERO_BUSY_TICK_PERIOD` | How often waiter ticks fire inside a long slice (default one million retired instructions). Stretching it is occasionally needed so a clock interrupt does not land mid-syscall. |
| `AERO_IGNORE_RESET` | Keeps the run going instead of calling `Machine::reset()`. This one works around a **real open defect**: an in-process reset livelocks in firmware POST — a guest-initiated reboot was observed grinding for 700 million instructions in an 720×400 POST loop and had to be killed. Until that is fixed, take a guest reboot by cold-booting the overlay instead of resuming through the reset. |

### "Is it making progress?" — the flag everything else is built on

`--progress-interval <ms>` prints a line to stderr every N milliseconds of host
time:

```
[progress] t= inst= (N Minst/s) mode= rip= cr3= cr8= if= halted= display=WxH non_zero= lfb_wr=
```

This is the single most useful flag in the runner and it is easy to forget,
which is exactly why `scripts/win7.sh` always passes it. Two things depend on it
directly:

- **The boot ladder.** Every rung in
  [testing.md](./testing.md#the-boot-ladder-as-a-metric) is a grep over these
  lines — `inst=`, `mode=Protected`, `display=1024x768`, `non_zero=`, `cr8`.
  Without this flag there is no `run.log` and no ladder.
- **Telling a slow boot from a hung one**, from the guest's side rather than the
  host's: `inst=` climbing with `rip=` spread across varied addresses is forward
  progress, and it answers the question before you reach for `perf`.

`lfb_wr` counts writes the guest made to the linear framebuffer, and it is
enabled by `AERO_COUNT_LFB_WRITES` (`crates/aero-devices-gpu/src/pci.rs`). It is
the number that distinguishes *the guest painted this* from *something was
already in the buffer* — the desktop result is evidenced by natural `lfb_wr`
counts, not by a screenshot alone.

### "What is on screen, over time?"

`--vga-png <path> --vga-png-interval <ms>` re-dumps the framebuffer periodically,
so a long boot can be watched as it happens rather than only at the end. Dumps
that fail before a mode is set are ignored; the final dump still reports errors.

`AERO_SCAN_SURFOBJ` walks the guest's GDI surface objects, which is how you tell
a device surface with a real framebuffer pointer from a memory bitmap that will
never reach the screen. `AERO_SCAN_ASCII` and `AERO_SCAN_UTF16` scan physical
memory for a string — useful for finding a module or a path, but see the warning
about strings above: a hit is a lead, not proof.

### "What does the guest think its hardware is?"

These dump guest-side state on a clean stop, so a run can *report* what Windows
concluded instead of leaving you to infer it from the screen:

| Variable | What it reports |
|---|---|
| `AERO_DUMP_DEVNODES` | The plug-and-play device tree: which devices exist, which driver bound, and the problem code if one did not. `AERO_DUMP_DEVNODE_RESOURCES` adds the resource lists. |
| `AERO_DUMP_PROCS` | Live processes, walked out of the kernel's process list. `AERO_LOG_NEW_PROCS` reports them as they appear. |
| `AERO_DUMP_THREADS` | Per-thread state. Validate its struct offsets before trusting them — see above. |
| `AERO_DUMP_LAPIC` / `AERO_DUMP_IOAPIC` | Interrupt controller state, including which lines are masked. |
| `AERO_I8042_TRACE` | Keyboard-controller port traffic, which is how you prove the guest actually *read* an injected keystroke rather than assuming delivery. |
| `AERO_IDE_TRACE` / `AERO_ATA_TRACE` | Storage command traces. Turn these up before believing a high-level error code: a generic invalid-argument failure from Setup turned out to be writes being aborted at the device, invisible until the trace was full. |
| `AERO_DUMP_VGA` | Extended display state — `dispi=WxHxB virtual=WxH+X+Y pitch=N`. This is what makes a pitch mismatch explicit rather than something inferred from banding in a screenshot. |
| `AERO_DUMP_PCI` | Configuration space as the guest sees it. Worth remembering that the device model and the platform bus can hold *different* copies; this dumps the one that matters. |
| `AERO_DUMP_MODULES` / `AERO_DUMP_CPU` / `AERO_DUMP_CR3` | Loaded modules, processor state, and a linear dump under another process's page tables. |

Replacing inference with direct observation is the repeated move in this
project's hardest investigations: a mouse that would not move was diagnosed not
from pixels but from a device tree showing the driver had never started.

### Injecting input

`AERO_INJECT_KEY=<key>` presses and releases repeatedly; `AERO_INJECT_KEY_ONCE=1`
does it exactly once, and `AERO_INJECT_MOUSE_ONCE=<x>,<y>,<button>` moves and
clicks once. Prefer the one-shot forms: on a snapshot that halts and wakes
frequently, the repeating form re-injects every few milliseconds and
spawn-storms the guest.

Key injection deliberately goes to **two** places — the emulated keyboard
controller *and* the firmware's INT 16h queue — because the firmware's own
"press any key to boot" prompt reads the BIOS queue and would never see a
controller event. That is why injection works before the kernel is loaded.

### "What is the firmware being asked for?"

`AERO_PCI_BIOS_TRACE=1` logs INT 1Ah AH=0xB1 PCI BIOS calls with full registers
(`crates/firmware/src/bios/interrupts.rs`). It is worth copying as a pattern
rather than for itself: an environment-gated `eprintln!` at a handler is the
cheapest possible firmware probe.

For the guest-side fault and process instruments — `AERO_LOG_UD`, `AERO_LOG_GP`,
`AERO_LOG_SYSCALL` and `AERO_LOG_NEW_PROCS` — see
[the native CLI runner](#native-cli-runner-aero-machine) above.

### "Can I get back here without re-walking the boot?"

`--snapshot-save <path>` writes a checkpoint when the run ends — **including on a
fault**, where the saved state sits at the faulting instruction. That converts
the loop from "re-walk from boot" into: run to the wall with `--snapshot-load
prev.bin --snapshot-save cur.bin`, diagnose from the printed context, fix and add
the regression test, then resume from `cur.bin`, where the corrected CPU
re-executes the previously faulting instruction. Each probe costs seconds to
minutes instead of tens of minutes, and that ratio is what makes deep bring-up
tractable at all.

### "Is it stuck, or just slow?"

A screen that has not changed in ten minutes looks hung and often is not —
winload's file decompression is genuinely expensive at interpreter speed. Do not
guess, and do not infer it from the framebuffer. Probe the *host* process:

```bash
perf record -F 999 -p <pid> -- sleep 4
```

Interpreter hot-loop symbols at the top (`PhysicalMemoryBus::read_physical`,
`SparseMemory::read_into`, `run_batch_cpu_core_with_assists`) mean the guest is
executing CPU- and memory-bound code, so it is progressing; a storage wait or an
HLT stub has a visibly different profile. Note that `/proc/<pid>/io` is useless
for this — the ISO is page-cached and the working directory is tmpfs, so the byte
counters stay at zero.

The emulator is deterministic, which gives one more piece of leverage: a
guest-side hang cannot spontaneously recover. When genuinely unsure, let the run
reach its budget and inspect the end state.

## Serial Console (16550 UART)

The emulator models a classic 16550-compatible UART to support BIOS/bootloader logging via COM ports:

| Port | Base I/O | IRQ |
| ---- | -------- | --- |
| COM1 | `0x3F8`  | 4   |
| COM2 | `0x2F8`  | 3   |
| COM3 | `0x3E8`  | 4   |
| COM4 | `0x2E8`  | 3   |

### Host capture

When the guest writes to the THR register (offset `+0`, DLAB=0), the UART invokes its transmit callback with the written bytes. The
owning worker forwards those bytes to the main thread via the binary IPC event:

- `vmRuntime="legacy"`: I/O worker owns the UART device model.
- `vmRuntime="machine"`: machine CPU worker (`api.Machine`) owns the UART device model.

- `aero_ipc::protocol::Event::SerialOutput { port, data }` (tag `0x1500`)

---

## DebugCon (I/O port `0xE9`)

Many emulators (Bochs/QEMU) provide a "debug console" byte sink at I/O port `0xE9`. This is a
convenient early-boot logging channel because it does not require UART initialization.

Guest code can print a byte with a single `OUT`:

```text
mov al, 'A'
out 0xE9, al
```

In `aero_machine::Machine`, port `0xE9` is available when `MachineConfig::enable_debugcon=true`
(default) and bytes written to it are captured in a host-visible buffer:

- `Machine::take_debugcon_output() -> Vec<u8>`
- `Machine::debugcon_output_len() -> u64`

To disable the device (and leave the port unmapped), set `MachineConfig::enable_debugcon=false`.

In the browser runtime (`crates/aero-wasm`), the JS-facing `Machine` wrapper exposes the same log:

- `Machine.debugcon_output() -> Uint8Array`
- `Machine.debugcon_output_len() -> number`

---

## Native CLI runner (`aero-machine`)

For quick boot/integration debugging without the browser runtime, the repo includes a small native CLI tool that runs the canonical [`aero_machine::Machine`] directly.

Build + run (recommended via `scripts/safe-run.sh` to apply time/memory limits when running untrusted disk images):

```bash
# Boot a tiny fixture disk image and print COM1 output to stdout.
bash ./scripts/safe-run.sh \
  cargo run -p aero-machine-cli -- \
    --disk tests/fixtures/boot/boot_vga_serial_8s.img \
    --ram 64 \
    --max-insts 100000 \
    --serial-out stdout \
    --debugcon-out stdout
```

**`--serial-out` and `--debugcon-out` do not take the same values.**
`--debugcon-out` understands the keyword `none`; `--serial-out` does not, and
treats anything that is not `stdout` as a filename — so `--serial-out none`
silently creates a file called `none` in the working directory and looks like it
suppressed output. If a run seems to have produced no serial data, check for that
file before concluding the guest was silent.

Bring-up key injection: `AERO_INJECT_KEY=Enter` (or `Tab` / `Escape` /
`Space`) presses and releases once at start, then again every ~50 host
slices. On an interruptible-HLT snapshot that re-injects every ~50 ms and
will spawn-storm Setup. Use `AERO_INJECT_KEY_ONCE=1` for a single
press+release. `AERO_INJECT_CLICK_ONCE=1` left-clicks once at the
current PS/2 cursor (for owner-drawn buttons that ignore Enter).

`AERO_LOG_SYSCALL=<hex,hex>` logs matching `eax` values at the
architectural `SYSCALL` assist (shared by JIT and Tier-0). Win7
`NtCreateUserProcess` is `0xAA`; `NtTerminateProcess` is `0x29`.
`AERO_LOG_GP=1` logs the first 64 **user-mode** `#GP`
faults (`cpl=3`) with RIP/CS/error-code/bytes — use it
when LogonUI recycles without an `AERO_LOG_UD` hit
(aligned `movaps`, etc.). Kernel `#GP` is suppressed.

`AERO_LOG_NEW_PROCS[=<N>]` walks `EPROCESS.ActiveProcessLinks` from
`gs:[0x188]` and prints newly seen `(pid, ImageFileName)` every N
retired instructions (default 10M). Use this to catch short-lived
COM local servers (`vdsldr.exe`, `vds.exe`) that a late `DUMP_PROCS`
scan will miss.

The CLI requires at least one of:

- `--disk` (primary HDD image)
- `--install-iso` (ATAPI CD-ROM install/recovery media)

When `--disk` is used, the CLI opens the disk image as a native file-backed disk. By default it is **writable** (guest writes can modify the image). Use `--disk-ro` or a copy if you want a read-only base.

To keep a base image immutable while still allowing guest writes, use a copy-on-write overlay:

```bash
bash ./scripts/safe-run.sh \
  cargo run -p aero-machine-cli -- \
    --disk /path/to/base.img \
    --disk-overlay /tmp/overlay.aerospar \
    --max-insts 100000 \
    --serial-out stdout
```

Install media (ATAPI CD-ROM) and boot policy:

```bash
# Attach a Win7 install ISO and boot from CD once, then allow the guest to reboot into the HDD.
# (The CLI automatically disables the firmware CD-first policy after the first guest reset if the
# active boot device was the CD-ROM, to avoid setup looping back into the installer.)
bash ./scripts/safe-run.sh \
  cargo run -p aero-machine-cli -- \
    --disk /path/to/win7.img \
    --install-iso /path/to/win7.iso \
    --boot cd-first \
    --max-ms 60000 \
    --serial-out stdout

# Force boot from CD every time (even after guest resets).
bash ./scripts/safe-run.sh \
  cargo run -p aero-machine-cli -- \
    --disk /path/to/win7.img \
    --install-iso /path/to/win7.iso \
    --boot cdrom \
    --max-ms 60000

# ISO-only boot (no HDD attached).
bash ./scripts/safe-run.sh \
  cargo run -p aero-machine-cli -- \
    --install-iso /path/to/recovery.iso \
    --boot cdrom \
    --max-ms 60000
```

Optional outputs:

```bash
# Save a snapshot and dump the last VGA framebuffer to a PNG on exit.
bash ./scripts/safe-run.sh \
  cargo run -p aero-machine-cli -- \
    --disk tests/fixtures/boot/boot_vga_serial_8s.img \
    --ram 64 \
    --max-insts 100000 \
    --serial-out stdout \
    --snapshot-save /tmp/aero.snap \
    --vga-png /tmp/aero.png

# Restore a snapshot (disk bytes are still provided externally via --disk).
bash ./scripts/safe-run.sh \
  cargo run -p aero-machine-cli -- \
    --disk tests/fixtures/boot/boot_vga_serial_8s.img \
    --ram 64 \
    --max-insts 100000 \
    --serial-out stdout \
    --snapshot-load /tmp/aero.snap
```

Two opt-in virtual-time controls keep long Windows calibration/stall loops
practical in the native runner:

- `AERO_RDTSC_QUANTUM=<cycles>` adds the given TSC cycles to each
  `RDTSC`/`RDTSCP` assist and yields so platform timers can catch up. The CPU
  core default is zero; current Win7 bring-up uses `2000`.
- `AERO_PM_TIMER_POLL_QUANTUM=<cycles>` advances the TSC after a PM_TMR port
  read, yields, and advances every shared platform timer from that same TSC
  delta before the next poll. Current Win7 bring-up uses `300000`.

The second control is deliberately a machine scheduling policy, not a PM-timer
device shortcut: `PM_TMR` reads themselves remain pure samples of shared
deterministic time. See the shared-clock rule in `wiki/history/windows-7-bring-up.md`.

Useful display/PCI bring-up probes:

- `AERO_LOG_VBE=1` logs INT 10h VBE calls plus DISPI index/data reads and
  writes; unsupported DISPI access widths are explicitly reported rather than
  disappearing silently.
- `AERO_FIND_PHYS=<pa>` reverse-walks System and current page tables for a
  physical page. Add comma-separated `AERO_FIND_PHYS_CR3=<cr3,...>` to check
  specific process DTBs; this is essential before calling a raw physical
  string hit a resident module.
- `AERO_STDVGA_ENABLE_IO=1` is a narrow diagnostic compatibility wedge for
  restored standard-VGA checkpoints captured before PCI POST enabled fixed
  legacy I/O decode on class `03/00` controllers. It validates the device
  identity/class and changes only PCI command bit 0. Do not use it as
  canonical fixed-firmware proof. Its exact pre-framebuf +2B replay was
  unchanged: no VBE or `0xaa55aa55` hit and a byte-identical final PNG.
  This is not a cold negative unless the override is proven to precede
  `vgapnp!VgaFindAdapter`.
- `AERO_LOG_STDVGA_PCI=1` logs guest configuration-mechanism-1 reads/writes
  for the standard-VGA-compatible function at 00:07.0, including offset,
  width, and value. Use it to prove when videoprt reads command/class.
- `AERO_STDVGA_REVISION=<u8>` is the corresponding restored-checkpoint-only
  PCI revision override. It validates exact `1234:1111` and class `03/00`
  first; use it without the I/O override to isolate QEMU's revision 2.

---

## Debug IPC

The core IPC queue format and the stable binary message tags are documented in [`../specs/worker-ipc-protocol.md`](../specs/worker-ipc-protocol.md).

### Runtime log events

Each worker can emit structured `log` events on its runtime event ring. The coordinator prints
these with a `[role]` prefix in the browser console and records `WARN`/`ERROR` logs in the nonfatal
event stream.

Example (L2 tunnel forwarder telemetry from the network worker):

```
[net] l2: open tx=... rx=... drop+{...} pending=...
```

Tunnel transport failures are surfaced as `ERROR` logs (e.g. `l2: error: ...`).

---

## Breakpoints / Stepping

The `aero-debug` crate provides a `Debugger` helper that the CPU execution loop can consult:

- `check_before_exec(rip)` for breakpoints / paused state
- `check_after_exec()` for single-step completion
- `check_watchpoint(addr, len, access)` for optional memory watchpoints

The CPU worker can surface pause/breakpoint state to the host UI either through shared memory state blocks or through IPC events (TBD).

---

## Tracing

The `aero-debug::Tracer` collects structured `TraceEvent`s with:

- configurable type filters (`TraceFilter`)
- simple sampling (`sample_rate`)
- bounded in-memory buffering (`max_events`)
- JSON export (`export_json()`)

The I/O bus can log port reads/writes, while the CPU core can log instructions and interrupts.

### Network traffic capture (PCAPNG)

For packet-level networking debugging (guest↔tunnel Ethernet frames, exportable to Wireshark),
see:

- [`07-networking.md`](networking.md#network-tracing-pcappcapng-export) – **Network Tracing (PCAP/PCAPNG Export)**
  - Includes the browser runtime UI panel and the `window.aero.netTrace` automation API.

---

## Browser Automation Debug API (`window.aero.debug`)

The browser runtime installs a small set of automation-friendly helpers under `window.aero.debug`.
This is intended for tests/harnesses that need a stable interface for introspecting VM runtime
state without reaching into private coordinator fields.

See:

- `apps/web/src/runtime/boot_device_backend.ts` (implementation)
- `shared/aero_api.ts` (`AeroDebugApi` types)

### Boot disk selection vs active boot device

Aero distinguishes between:

- **Selected boot policy** – what the host requested for the next reset (e.g. "boot CD first").
- **Active boot device** – what firmware actually booted from in the current boot session (CD vs
  HDD), which can differ when fallback policies are enabled.

The relevant helpers are:

- `window.aero.debug.getBootDisks() -> { mounts: {hddId?, cdId?}, bootDevice? } | null`
  - Returns the current boot disk selection snapshot from the main-thread coordinator.
  - `mounts.*Id` are DiskManager mount IDs (opaque strings).
  - `bootDevice` is the requested policy (`"hdd"` / `"cdrom"`), which may differ from the active
    boot source when firmware falls back.
- `window.aero.debug.getMachineCpuActiveBootDevice() -> "hdd" | "cdrom" | null`
  - Returns the active boot device reported by the machine CPU worker (requires
    `vmRuntime="machine"`).
  - `null` means unknown/unavailable (e.g. workers not running, older builds without reporting, or
    a reboot/disk-reattach transition where the next boot session has not reported yet).
- `window.aero.debug.getMachineCpuBootConfig() -> { bootDrive: number, cdBootDrive: number, bootFromCdIfPresent: boolean } | null`
  - Returns the machine CPU worker's firmware boot configuration snapshot:
    - `bootDrive` – BIOS boot drive number (`DL`), typically `0x80` (HDD0) or `0xE0` (CD0).
    - `cdBootDrive` – BIOS CD-ROM drive number used by the "CD-first when present" policy (`0xE0..=0xEF`).
    - `bootFromCdIfPresent` – whether the firmware CD-first fallback policy is enabled.
  - `null` means unknown/unavailable (e.g. older WASM builds without boot-config exports, or a
    reboot/disk-reattach transition where the CPU worker has not re-reported yet).

---

## Web Debug UI

`apps/web/debug.html` is a lightweight debug UI page intended for development. It expects the host to:

1. send events to the UI by calling `window.aeroDebug.onEvent(event)` (or `postMessage`)
2. listen for `aero-debug-command` DOM events and forward them to the emulator

The serial console pane supports copy/save/clear actions and auto-scrolls on new output.
