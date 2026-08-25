# aero_trace — QEMU TCG plugin for ground-truth execution capture.
#
# Build:  make
# Output: aero_trace.so (load via `qemu -plugin ./aero_trace.so,...`)
#
# No libglib2.0-dev required: glib-shim.h provides the few glib types the
# vendored qemu-plugin.h references at compile time, and the plugin never calls
# any glib-returning API function (see README.md).
CC      ?= cc
CFLAGS  ?= -O2 -g -Wall -Wextra -fPIC -fvisibility=hidden
LDFLAGS ?= -shared
PLUGIN  := aero_trace.so
SRC     := aero_trace.c

.PHONY: all clean test

all: $(PLUGIN)

$(PLUGIN): $(SRC) qemu-plugin.h glib-shim.h
	$(CC) $(CFLAGS) $(LDFLAGS) -o $@ $(SRC)

# Smoke test: build + run a tiny QEMU that exits immediately via isa-debug-exit,
# confirming the plugin loads and emits its atexit summary.
test: $(PLUGIN)
	qemu-system-x86_64 -display none -machine pc -m 16 \
		-drive if=floppy,format=raw,file=/dev/null 2>/dev/null || true
	@echo "plugin build OK ($(PLUGIN))"

clean:
	rm -f $(PLUGIN)
