/*
 * Minimal glib type shim for compiling QEMU TCG plugins without libglib2.0-dev.
 *
 * The official <qemu-plugin.h> includes <glib.h> because some plugin API
 * functions return/take glib containers (GArray, GByteArray). This plugin never
 * calls those glib-returning functions, so we only need the *types* to satisfy
 * the C compiler. At runtime the plugin resolves qemu_plugin_* symbols from the
 * QEMU process; the only functions we invoke use standard C ABI (uint64_t,
 * size_t, void*, bool). Keeping this shim tiny avoids a system-wide
 * libglib2.0-dev dependency for a build artifact.
 *
 * If a future revision of this plugin needs qemu_plugin_get_registers() /
 * qemu_plugin_read_register(), install libglib2.0-dev and drop this shim.
 */
#ifndef AERO_GLIB_SHIM_H
#define AERO_GLIB_SHIM_H

#include <stdint.h>
#include <stddef.h>

typedef int gboolean;
typedef char gchar;
typedef unsigned char guint8;
typedef unsigned int guint;
typedef unsigned long guint32_val_unused; /* keep types reserved */
typedef size_t gsize;
typedef void* gpointer;
typedef const void* gconstpointer;

/* Opaque glib container types — only referenced as pointers in prototypes. */
typedef struct _GArray GArray;
typedef struct _GByteArray GByteArray;
typedef struct _GString GString;
typedef struct _GHashTable GHashTable;
typedef struct _GSList GSList;
typedef struct _GList GList;
typedef struct _GPtrArray GPtrArray;

/* Freely-defined here so the linker never needs real glib. We never call them,
 * but defining them silences any stray references and keeps the .so clean. */
static inline void g_free(gpointer p) { (void)p; }
static inline gpointer g_malloc(gsize n) { (void)n; return 0; }
static inline gpointer g_malloc0(gsize n) { (void)n; return 0; }
static inline gpointer g_realloc(gpointer p, gsize n) { (void)p; (void)n; return 0; }
static inline gpointer g_try_malloc(gsize n) { (void)n; return 0; }
static inline gpointer g_try_malloc0(gsize n) { (void)n; return 0; }

#define TRUE 1
#define FALSE 0

#endif /* AERO_GLIB_SHIM_H */
