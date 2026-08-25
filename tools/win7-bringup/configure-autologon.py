"""Configure an installed Win7 guest image for an unattended console logon.

The bring-up image was installed with an unattend that asked for AutoLogon as
Administrator and skipped user OOBE, so no ordinary account was ever created.
Only Administrator and Guest exist and both are disabled, which leaves LogonUI
with nothing to show: it exits immediately and winlogon respawns it forever. The
symptom is a logon screen that flickers and never settles.

This applies, offline, what the unattend was supposed to apply:

  * SAM      - enable Administrator and clear its password
  * SOFTWARE - AutoAdminLogon / DefaultUserName / DefaultPassword
  * SYSTEM   - allow a blank password to be used for logon

Point it at a mounted guest volume's config directory. The hives must not be in
use; mount the image read-write and unmount it afterwards so the changes are
flushed before the guest boots.

    python3 configure-autologon.py --config-dir /mnt/win7/Windows/System32/config

See the wiki: the Windows 7 bring-up, for why this was needed rather than a
scaffolding poke at runtime.
"""

import argparse
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from hive import Hive, dump  # noqa: E402

ACB_DISABLED = 0x0001
ACB_PASSWD_NOTREQD = 0x0020
NT_HASH_ENTRY = 14
ADMINISTRATOR_RID = "000001F4"  # RID 500


def utf16z(s):
    return (s + "\0").encode("utf-16-le")


def fix_sam(config):
    h = Hive(str(config / "SAM"), mutable=True)
    nk = h.find(f"SAM\\Domains\\Account\\Users\\{ADMINISTRATOR_RID}")
    if nk is None:
        raise RuntimeError("Administrator (RID 500) not found in SAM")

    vk = h.value(nk, "F")
    if vk is None:
        raise RuntimeError("Administrator has no F record — is this a SAM hive?")
    f = bytearray(h.value_raw(vk))
    acb = struct.unpack_from("<H", f, 0x38)[0]
    new_acb = (acb & ~ACB_DISABLED) | ACB_PASSWD_NOTREQD
    struct.pack_into("<H", f, 0x38, new_acb)
    h.set_value_data(vk, bytes(f), h.value_type(vk))
    print(f"  SAM  F   ACB 0x{acb:04x} -> 0x{new_acb:04x} (enabled, password not required)")

    # Entry 14 of the V record is the NT hash. Setting its length to 4 leaves the
    # header but no hash, which is how Windows represents a blank password.
    vk = h.value(nk, "V")
    if vk is None:
        raise RuntimeError("Administrator has no V record — is this a SAM hive?")
    v = bytearray(h.value_raw(vk))
    _, size, _ = struct.unpack_from("<III", v, NT_HASH_ENTRY * 12)
    struct.pack_into("<I", v, NT_HASH_ENTRY * 12 + 4, 4)
    h.set_value_data(vk, bytes(v), h.value_type(vk))
    print(f"  SAM  V   NT hash size {size} -> 4 (blank password)")

    h.save()


def fix_software(config):
    h = Hive(str(config / "SOFTWARE"), mutable=True)
    nk = h.find("Microsoft\\Windows NT\\CurrentVersion\\Winlogon")
    if nk is None:
        raise RuntimeError("Winlogon key not found in SOFTWARE")

    for name, value in (
        ("AutoAdminLogon", "1"),
        ("DefaultUserName", "Administrator"),
        ("DefaultPassword", ""),
        ("DefaultDomainName", "."),
    ):
        existing = h.value(nk, name)
        if existing is not None:
            h.set_value_data(existing, utf16z(value), 1)
            print(f"  SOFTWARE {name} = {value!r} (updated)")
        else:
            h.add_value(nk, name, 1, utf16z(value))
            print(f"  SOFTWARE {name} = {value!r} (added)")

    # AutoLogonCount counts down and deletes AutoAdminLogon when it reaches zero.
    # Autologon should stay on for every boot of a bring-up image.
    count = h.value(nk, "AutoLogonCount")
    if count is not None:
        h.set_value_data(count, struct.pack("<I", 0xFFFFFFFF), 4)
        print("  SOFTWARE AutoLogonCount = 0xffffffff (never expires)")

    h.save()


def fix_system(config):
    h = Hive(str(config / "SYSTEM"), mutable=True)
    touched = 0
    for cs in ("ControlSet001", "ControlSet002"):
        nk = h.find(cs + "\\Control\\Lsa")
        if nk is None:
            continue
        vk = h.value(nk, "LimitBlankPasswordUse")
        if vk is not None:
            h.set_value_data(vk, struct.pack("<I", 0), 4)
            print(f"  SYSTEM {cs} LimitBlankPasswordUse = 0 (updated)")
        else:
            # Absent means the restrictive default applies, so it has to be added
            # rather than skipped — skipping silently leaves blank-password logon
            # blocked, which is the whole point of this script.
            h.add_value(nk, "LimitBlankPasswordUse", 4, struct.pack("<I", 0))
            print(f"  SYSTEM {cs} LimitBlankPasswordUse = 0 (added)")
        touched += 1
    if touched == 0:
        raise RuntimeError("no ControlSet*\\Control\\Lsa key found in SYSTEM")
    h.save()


def verify(config):
    print("\nverification (re-read from disk):")
    h = Hive(str(config / "SOFTWARE"))
    dump(h, "Microsoft\\Windows NT\\CurrentVersion\\Winlogon")

    s = Hive(str(config / "SAM"))
    nk = s.find(f"SAM\\Domains\\Account\\Users\\{ADMINISTRATOR_RID}")
    f = s.value_raw(s.value(nk, "F"))
    acb = struct.unpack_from("<H", f, 0x38)[0]
    v = s.value_raw(s.value(nk, "V"))
    _, ntsize, _ = struct.unpack_from("<III", v, NT_HASH_ENTRY * 12)
    print(
        f"  SAM Administrator ACB=0x{acb:04x} "
        f"{'ENABLED' if not acb & ACB_DISABLED else 'DISABLED'} nt_hash_size={ntsize}"
    )


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument(
        "--config-dir",
        required=True,
        type=Path,
        help="the guest's Windows/System32/config directory, on a mounted image",
    )
    args = ap.parse_args()

    config = args.config_dir
    for hive in ("SAM", "SOFTWARE", "SYSTEM"):
        if not (config / hive).is_file():
            ap.error(f"{config / hive} not found — is the guest volume mounted?")

    # A hive whose sequence numbers disagree gets its transaction log replayed on
    # the next boot, which would silently undo everything below. Say so before
    # spending the edit.
    problems = []
    for name in ("SAM", "SOFTWARE", "SYSTEM"):
        h = Hive(str(config / name))
        if h.dirty():
            problems.append(f"{name} is dirty (seq {h.u32(4)} != {h.u32(8)})")
        logs = h.log_files()
        if logs:
            problems.append(f"{name} has transaction logs: {', '.join(Path(x).name for x in logs)}")
    if problems:
        print("WARNING: these edits may be reverted on the next guest boot:")
        for pr in problems:
            print(f"  - {pr}")
        print("  Boot the guest cleanly and shut it down, or remove the logs, then re-run.")

    print("applying offline logon configuration:")
    fix_sam(config)
    fix_software(config)
    fix_system(config)
    verify(config)


if __name__ == "__main__":
    main()
