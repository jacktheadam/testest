/*
 * aero_trace — a QEMU TCG plugin that captures ground-truth execution streams
 * for forward first-divergence diffing against the Aero emulator.
 *
 * Produces two independent text streams (any subset, opt-in via plugin args).
 * Each is line-oriented and hex-formatted to diff directly against the
 * corresponding Aero-side capture:
 *
 *   exec=<file>    per-instruction linear address (cs.base + rip), one per line
 *                  as `0x...`. Matches Aero's AERO_TRACE_COMPACT output exactly,
 *                  enabling a line-by-line control-flow divergence diff. This is
 *                  the strongest first-divergence signal: the first RIP where
 *                  Aero and QEMU part company is the bug (wrong branch, or a
 *                  device read that returned a different value).
 *
 *   writes=<file>  every store to guest RAM: `rip paddr size` (hex). Matches the
 *                  (rip,addr,len) prefix of Aero's AERO_WRITE_STREAM. Aero
 *                  additionally records the first <=8 value bytes; the QEMU mem
 *                  callback does not expose the store value, so value-level
 *                  divergence is detected via periodic RAM-snapshot diffs
 *                  instead (see README.md).
 *
 * Optional controls:
 *   from=<n>       first retired-instruction index to log (default 0)
 *   to=<n>         last index (inclusive; default unlimited)
 *   maxexec=<n>    cap on exec records (default 200_000_000)
 *   maxwrites=<n>  cap on write records (default 50_000_000)
 *
 * Usage:
 *   qemu-system-x86_64 -nographic ... \
 *       -plugin ./aero_trace.so,exec=exec.txt,writes=writes.txt
 *
 * Implementation note: this plugin deliberately uses only the robust TCG plugin
 * APIs (insn_exec_cb, mem_cb, insn_vaddr/size). It does NOT call
 * qemu_plugin_insn_data(), which segfaults inside translator_st() for some TB
 * configurations in QEMU 10.x (QEMU's own example plugins avoid it too).
 * Consequently CPUID/MSR ground truth is derived from QEMU source + SDM analysis
 * rather than captured here (see tools/qemu-trace-plugin/README.md).
 *
 * Build: see Makefile. No libglib2.0-dev required.
 */
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <pthread.h>
#include "qemu-plugin.h"

QEMU_PLUGIN_EXPORT int qemu_plugin_version = QEMU_PLUGIN_VERSION;

typedef struct {
    FILE          *fp;
    pthread_mutex_t lock;
    const char    *path;
    uint64_t       logged;
    uint64_t       max;
    int            enabled;
} Stream;

static Stream s_exec   = { .lock = PTHREAD_MUTEX_INITIALIZER };
static Stream s_writes = { .lock = PTHREAD_MUTEX_INITIALIZER };

static uint64_t g_from = 0;
static uint64_t g_to   = UINT64_MAX;
static uint64_t g_retired = 0;

static void stream_open(Stream *s) {
    if (s->fp) return;
    s->fp = fopen(s->path, "w");
    if (!s->fp) {
        fprintf(stderr, "aero_trace: cannot open %s: %s\n", s->path, strerror(errno));
        s->enabled = 0;
    }
}

static void emit(Stream *s, const char *fmt, ...) {
    uint64_t n = __atomic_fetch_add(&s->logged, 1, __ATOMIC_RELAXED);
    if (n >= s->max) return;
    if (!s->fp) {
        pthread_mutex_lock(&s->lock);
        stream_open(s);
        pthread_mutex_unlock(&s->lock);
    }
    if (!s->fp) return;
    va_list ap;
    va_start(ap, fmt);
    pthread_mutex_lock(&s->lock);
    vfprintf(s->fp, fmt, ap);
    fputc('\n', s->fp);
    /* Periodic flush so captures survive SIGTERM/SIGKILL and long runs. */
    if ((n & 0x3FFF) == 0) fflush(s->fp);
    pthread_mutex_unlock(&s->lock);
    va_end(ap);
}

static inline int in_window(uint64_t idx) {
    return idx >= g_from && idx <= g_to;
}

static void on_insn_exec(unsigned int vcpu_index, void *userdata) {
    (void)vcpu_index;
    uint64_t idx = __atomic_fetch_add(&g_retired, 1, __ATOMIC_RELAXED);
    if (s_exec.enabled && in_window(idx)) {
        uint64_t rip = (uint64_t)(uintptr_t)userdata;
        emit(&s_exec, "%#" PRIx64, rip);
    }
}

static void on_mem_write(unsigned int vcpu_index, qemu_plugin_meminfo_t info,
                         uint64_t vaddr, void *userdata) {
    (void)vcpu_index;
    (void)vaddr;
    if (!qemu_plugin_mem_is_store(info)) return;
    struct qemu_plugin_hwaddr *h = qemu_plugin_get_hwaddr(info, vaddr);
    if (!h || qemu_plugin_hwaddr_is_io(h)) return;   /* only guest RAM */
    uint64_t paddr = qemu_plugin_hwaddr_phys_addr(h);
    unsigned int sz = 1u << qemu_plugin_mem_size_shift(info);
    uint64_t rip = (uint64_t)(uintptr_t)userdata;
    emit(&s_writes, "%#" PRIx64 " %#" PRIx64 " %u", rip, paddr, sz);
}

static void on_tb_trans(qemu_plugin_id_t id, struct qemu_plugin_tb *tb) {
    (void)id;
    size_t n = qemu_plugin_tb_n_insns(tb);
    for (size_t i = 0; i < n; i++) {
        struct qemu_plugin_insn *insn = qemu_plugin_tb_get_insn(tb, i);
        if (!insn) continue;
        uint64_t vaddr = qemu_plugin_insn_vaddr(insn);
        void *ud = (void *)(uintptr_t)vaddr;
        if (s_exec.enabled) {
            qemu_plugin_register_vcpu_insn_exec_cb(
                insn, on_insn_exec, QEMU_PLUGIN_CB_NO_REGS, ud);
        }
        if (s_writes.enabled) {
            qemu_plugin_register_vcpu_mem_cb(
                insn, on_mem_write, QEMU_PLUGIN_CB_NO_REGS,
                QEMU_PLUGIN_MEM_W, ud);
        }
    }
}

static void parse_u64(const char *s, uint64_t *out) {
    if (s) *out = strtoull(s, NULL, 0);
}

static void atexit_flush(qemu_plugin_id_t id, void *userdata) {
    (void)id; (void)userdata;
    Stream *all[] = { &s_exec, &s_writes };
    for (size_t i = 0; i < 2; i++) {
        if (all[i]->fp) {
            pthread_mutex_lock(&all[i]->lock);
            fflush(all[i]->fp);
            fclose(all[i]->fp);
            all[i]->fp = NULL;
            pthread_mutex_unlock(&all[i]->lock);
        }
    }
    fprintf(stderr, "aero_trace: retired=%llu exec=%llu writes=%llu\n",
            (unsigned long long)__atomic_load_n(&g_retired,   __ATOMIC_RELAXED),
            (unsigned long long)__atomic_load_n(&s_exec.logged,   __ATOMIC_RELAXED),
            (unsigned long long)__atomic_load_n(&s_writes.logged, __ATOMIC_RELAXED));
}

QEMU_PLUGIN_EXPORT int qemu_plugin_install(qemu_plugin_id_t id,
                                           const qemu_info_t *info,
                                           int argc, char **argv) {
    (void)info;
    s_exec.max = 200000000ULL;
    s_writes.max = 50000000ULL;

    for (int i = 0; i < argc; i++) {
        const char *a = argv[i];
        if      (!strncmp(a, "exec=", 5))      { s_exec.path = a + 5;   s_exec.enabled = 1; }
        else if (!strncmp(a, "writes=", 7))    { s_writes.path = a + 7; s_writes.enabled = 1; }
        else if (!strncmp(a, "from=", 5))      parse_u64(a + 5, &g_from);
        else if (!strncmp(a, "to=", 3))        parse_u64(a + 3, &g_to);
        else if (!strncmp(a, "maxexec=", 8))   parse_u64(a + 8, &s_exec.max);
        else if (!strncmp(a, "maxwrites=", 10))parse_u64(a + 10, &s_writes.max);
        else fprintf(stderr, "aero_trace: unknown arg '%s'\n", a);
    }

    qemu_plugin_register_vcpu_tb_trans_cb(id, on_tb_trans);
    qemu_plugin_register_atexit_cb(id, atexit_flush, NULL);
    return 0;
}
