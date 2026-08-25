use super::{
    BIOS_SIZE, DEFAULT_INT_STUB_OFFSET, DISKETTE_PARAM_TABLE_OFFSET, EBDA_BASE,
    FIXED_DISK_PARAM_TABLE_OFFSET, INT10_STUB_OFFSET, INT13_STUB_OFFSET, INT15_STUB_OFFSET,
    INT16_STUB_OFFSET, INT1A_STUB_OFFSET, VGA_FONT_8X16_OFFSET, VGA_FONT_8X8_OFFSET,
    VIDEO_PARAM_TABLE_OFFSET,
};
use crate::video::vbe::VbeDevice;

use font8x8::{UnicodeFonts, BASIC_FONTS};
use std::collections::HashMap;

const VBE_ROM_CODE_OFFSET: u16 = 0xA000;
const VBE_ROM_OEM_STRING_OFFSET: u16 = 0xA700;
const VBE_ROM_VENDOR_STRING_OFFSET: u16 = 0xA720;
const VBE_ROM_PRODUCT_STRING_OFFSET: u16 = 0xA740;
const VBE_ROM_REV_STRING_OFFSET: u16 = 0xA760;
const VBE_ROM_MODE_LIST_OFFSET: u16 = 0xA780;
const VBE_ROM_MODE_INFO_OFFSET: u16 = 0xA800;
const VBE_ROM_EDID_OFFSET: u16 = 0xB800;
const VBE_ROM_DATA_END: u16 = VGA_FONT_8X8_OFFSET;
const BIOS_BUILD_DATE_OFFSET: usize = 0xFFF5;
const BIOS_BUILD_DATE: &[u8; 8] = b"06/23/99";

// The interpreter-safe ROM handler needs a small writable state block. Keep it
// in an explicitly reserved part of Aero's EBDA, after the INT 15h AH=C0h
// system-configuration table (0x20..0x2f) and before the ACPI RSDP (0x100).
//
// PhysBasePtr cannot live only in the immutable ROM mode-info blocks: the
// canonical machine learns the final VGA/AeroGPU PCI BAR address during POST,
// after the ROM mapping has been installed. The machine updates this EBDA
// value after PCI enumeration, before any guest code executes.
const VBE_EBDA_LFB_BASE_OFFSET: u16 = 0x0040;
const VBE_EBDA_CURRENT_MODE_OFFSET: u16 = 0x0044;
const VBE_EBDA_WIDTH_OFFSET: u16 = 0x0046;
const VBE_EBDA_PITCH_OFFSET: u16 = 0x0048;
const VBE_EBDA_BPP_OFFSET: u16 = 0x004A;
const VBE_EBDA_VIRTUAL_WIDTH_OFFSET: u16 = 0x004C;
const VBE_EBDA_X_OFFSET: u16 = 0x004E;
const VBE_EBDA_Y_OFFSET: u16 = 0x0050;
const VBE_EBDA_BANK_OFFSET: u16 = 0x0052;
const VBE_EBDA_DAC_WIDTH_OFFSET: u16 = 0x0054;
const VBE_EBDA_SEGMENT: u16 = (EBDA_BASE / 16) as u16;

#[derive(Clone, Copy)]
enum RelativeFixup {
    Near16,
}

struct RomAssembler {
    origin: u16,
    bytes: Vec<u8>,
    labels: HashMap<String, usize>,
    fixups: Vec<(usize, String, RelativeFixup)>,
}

impl RomAssembler {
    fn new(origin: u16) -> Self {
        Self {
            origin,
            bytes: Vec::new(),
            labels: HashMap::new(),
            fixups: Vec::new(),
        }
    }

    fn emit(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    fn label(&mut self, name: impl Into<String>) {
        let old = self.labels.insert(name.into(), self.bytes.len());
        assert!(old.is_none(), "duplicate real-mode ROM label");
    }

    fn near_jump(&mut self, opcode: u8, target: impl Into<String>) {
        self.emit(&[opcode, 0, 0]);
        self.fixups
            .push((self.bytes.len() - 2, target.into(), RelativeFixup::Near16));
    }

    fn jump(&mut self, target: impl Into<String>) {
        self.near_jump(0xE9, target);
    }

    fn jump_if_equal(&mut self, target: impl Into<String>) {
        // JNE +3; JMP rel16. This works on an 8086, whose conditional branches
        // only have an 8-bit displacement, without constraining handler layout.
        self.emit(&[0x75, 0x03]);
        self.near_jump(0xE9, target);
    }

    fn jump_if_not_equal(&mut self, target: impl Into<String>) {
        // JE +3; JMP rel16.
        self.emit(&[0x74, 0x03]);
        self.near_jump(0xE9, target);
    }

    fn jump_if_above_or_equal(&mut self, target: impl Into<String>) {
        // JB +3; JMP rel16.
        self.emit(&[0x72, 0x03]);
        self.near_jump(0xE9, target);
    }

    fn jump_if_below_or_equal(&mut self, target: impl Into<String>) {
        // JA +3; JMP rel16.
        self.emit(&[0x77, 0x03]);
        self.near_jump(0xE9, target);
    }

    fn finish(mut self) -> Vec<u8> {
        for (pos, target, kind) in self.fixups {
            let target_pos = *self
                .labels
                .get(&target)
                .unwrap_or_else(|| panic!("missing real-mode ROM label {target}"));
            match kind {
                RelativeFixup::Near16 => {
                    let next = pos + 2;
                    let displacement = target_pos as isize - next as isize;
                    let displacement = i16::try_from(displacement)
                        .expect("real-mode ROM near jump displacement must fit i16");
                    self.bytes[pos..pos + 2].copy_from_slice(&displacement.to_le_bytes());
                }
            }
        }
        assert!(
            usize::from(self.origin) + self.bytes.len() <= usize::from(VBE_ROM_OEM_STRING_OFFSET),
            "real-mode VBE handler overlaps its ROM data"
        );
        self.bytes
    }
}

/// Build the 64KiB BIOS ROM image.
pub fn build_bios_rom() -> Vec<u8> {
    build_bios_rom_with_vbe(
        VbeDevice::LFB_BASE_DEFAULT,
        VbeDevice::new().total_memory_64kb_blocks,
    )
}

pub(super) fn build_bios_rom_with_vbe(lfb_base: u32, total_memory_64kb_blocks: u16) -> Vec<u8> {
    let mut rom = vec![0xFFu8; BIOS_SIZE];

    // Install a conventional x86 reset vector at F000:FFF0.
    //
    // Aero performs POST in host code, but guests and tooling may still expect
    // the reset vector to contain a FAR JMP instruction.
    //
    // Encoding: JMP FAR ptr16:16 => EA iw (offset) iw (segment)
    // Target: F000:E000.
    let reset_off = 0xFFF0usize;
    rom[reset_off] = 0xEA;
    rom[reset_off + 1] = 0x00; // offset low
    rom[reset_off + 2] = 0xE0; // offset high (0xE000)
    rom[reset_off + 3] = 0x00; // segment low
    rom[reset_off + 4] = 0xF0; // segment high (0xF000)

    // IBM-compatible firmware publishes an MM/DD/YY build date immediately
    // after the reset vector. Operating systems probe F000:FFF5 directly
    // before falling back to scanning the entire system BIOS.
    rom[BIOS_BUILD_DATE_OFFSET..BIOS_BUILD_DATE_OFFSET + BIOS_BUILD_DATE.len()]
        .copy_from_slice(BIOS_BUILD_DATE);

    // Safe fallback at F000:E000: `cli; hlt; jmp $-2`.
    //
    // In a full-system integration this address is never reached because POST
    // is performed in host code, but it provides deterministic behavior if it is.
    let stub_off = 0xE000usize;
    rom[stub_off] = 0xFA;
    rom[stub_off + 1] = 0xF4;
    rom[stub_off + 2] = 0xEB;
    rom[stub_off + 3] = 0xFE;

    let stub = [0xF4u8, 0xCFu8]; // HLT; IRET
    write_stub(&mut rom, DEFAULT_INT_STUB_OFFSET, &stub);
    write_stub(&mut rom, INT13_STUB_OFFSET, &stub);
    write_stub(&mut rom, INT15_STUB_OFFSET, &stub);
    write_stub(&mut rom, INT16_STUB_OFFSET, &stub);
    write_stub(&mut rom, INT1A_STUB_OFFSET, &stub);

    install_vbe_rom(
        &mut rom,
        lfb_base,
        total_memory_64kb_blocks,
        INT10_STUB_OFFSET,
    );

    // Diskette Parameter Table (IVT vector 0x1E).
    //
    // This is an 11-byte table traditionally used by DOS-era software to probe or patch floppy
    // timing/geometry parameters. Our floppy implementation is fully emulated in software, but
    // providing a reasonable table improves compatibility with guests that expect it to exist.
    //
    // Values below match common 1.44MiB defaults:
    // - 512 bytes/sector, 18 sectors/track.
    let diskette_param_table: [u8; 11] = [
        0xAF, 0x02, 0x25, 0x02, 0x12, 0x1B, 0xFF, 0x6C, 0xF6, 0x0F, 0x08,
    ];
    write_stub(&mut rom, DISKETTE_PARAM_TABLE_OFFSET, &diskette_param_table);

    // Fixed Disk Parameter Table (IVT vectors 0x41/0x46).
    //
    // Older software reads these vectors to obtain CHS geometry for BIOS disk services.
    // We provide a table that matches our "fixed disk" INT 13h geometry (1024/16/63).
    //
    // Table format is 16 bytes (IBM PC/AT):
    // - word: cylinders
    // - byte: heads
    // - word: write precomp cylinder
    // - byte: control
    // - word: landing zone
    // - byte: sectors/track
    // - remaining bytes reserved (0)
    let fixed_disk_param_table: [u8; 16] = [
        0x00, 0x04, // cylinders = 1024
        0x10, // heads = 16
        0x00, 0x00, // write precomp = 0
        0x00, // control
        0x00, 0x00, // landing zone = 0
        0x3F, // sectors/track = 63
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // reserved
    ];
    write_stub(
        &mut rom,
        FIXED_DISK_PARAM_TABLE_OFFSET,
        &fixed_disk_param_table,
    );

    // Video parameter table (IVT vector 0x1D).
    //
    // Many DOS-era programs read this pointer directly for timing/CRTC defaults. We provide a
    // small, VGA-compatible table for text mode.
    //
    // This is not a complete hardware model; the INT 10h VGA implementation is the source of truth.
    let video_param_table: [u8; 16] = [
        0x5F, 0x4F, 0x50, 0x82, 0x55, 0x81, 0xBF, 0x1F, 0x00, 0x4F, 0x0D, 0x0E, 0x00, 0x00, 0x00,
        0x00,
    ];
    write_stub(&mut rom, VIDEO_PARAM_TABLE_OFFSET, &video_param_table);

    // VGA font table (INT 10h AH=11h AL=30h).
    //
    // DOS-era software commonly retrieves the ROM font and expects the CP437 "line graphics" range
    // (0xB3..=0xDF) to be populated with box drawing/block glyphs. Our VGA text renderer has a
    // CP437-compatible built-in font; the BIOS ROM needs to expose similar glyphs via INT 10h so
    // guests that query the font table can render boot-time UIs without blank characters.
    let font_8x16 = build_font8x16_cp437();
    write_stub(&mut rom, VGA_FONT_8X16_OFFSET, &font_8x16);

    // IVT vector 0x1F also historically points at an 8x8 graphics character table.
    let font_8x8 = build_font8x8_cp437();
    write_stub(&mut rom, VGA_FONT_8X8_OFFSET, &font_8x8);

    // Optional ROM signature (harmless and convenient for identification).
    rom[BIOS_SIZE - 2] = 0x55;
    rom[BIOS_SIZE - 1] = 0xAA;

    rom
}

fn install_vbe_rom(
    rom: &mut [u8],
    lfb_base: u32,
    total_memory_64kb_blocks: u16,
    int10_stub_offset: u16,
) {
    let mut vbe = VbeDevice::new();
    vbe.lfb_base = lfb_base;
    vbe.total_memory_64kb_blocks = total_memory_64kb_blocks;
    let modes = vbe.supported_modes().collect::<Vec<_>>();

    let mut asm = RomAssembler::new(VBE_ROM_CODE_OFFSET);

    for (function, label) in [
        (0x4F00u16, "controller_info"),
        (0x4F01, "mode_info"),
        (0x4F02, "set_mode"),
        (0x4F03, "get_mode"),
        (0x4F05, "window_control"),
        (0x4F06, "scan_line"),
        (0x4F07, "display_start"),
        (0x4F08, "dac_format"),
        (0x4F09, "palette_data"),
        (0x4F10, "dpms"),
        (0x4F15, "ddc"),
    ] {
        asm.emit(&[0x3D]);
        asm.emit(&function.to_le_bytes()); // cmp ax, imm16
        asm.jump_if_equal(label);
    }
    asm.jump("failure");

    asm.label("controller_info");
    asm.emit(&[0x53, 0x51, 0x52, 0x56, 0x57, 0x1E, 0x06]); // push bx,cx,dx,si,di,ds,es
    asm.emit(&[0x57, 0x31, 0xC0, 0xB9, 0x00, 0x01, 0xFC, 0xF3, 0xAB, 0x5F]);
    emit_store_es_di_word(&mut asm, 0, 0x4556); // "VE"
    emit_store_es_di_word(&mut asm, 2, 0x4153); // "SA"
    emit_store_es_di_word(&mut asm, 4, 0x0300);
    emit_store_es_di_far_ptr(&mut asm, 6, VBE_ROM_OEM_STRING_OFFSET);
    emit_store_es_di_word(&mut asm, 10, 1);
    emit_store_es_di_word(&mut asm, 12, 0);
    emit_store_es_di_far_ptr(&mut asm, 14, VBE_ROM_MODE_LIST_OFFSET);
    emit_store_es_di_word(&mut asm, 18, total_memory_64kb_blocks);
    emit_store_es_di_word(&mut asm, 20, 1);
    emit_store_es_di_far_ptr(&mut asm, 22, VBE_ROM_VENDOR_STRING_OFFSET);
    emit_store_es_di_far_ptr(&mut asm, 26, VBE_ROM_PRODUCT_STRING_OFFSET);
    emit_store_es_di_far_ptr(&mut asm, 30, VBE_ROM_REV_STRING_OFFSET);
    asm.emit(&[0x07, 0x1F, 0x5F, 0x5E, 0x5A, 0x59, 0x5B]); // pop es,ds,di,si,dx,cx,bx
    asm.jump("success");

    asm.label("mode_info");
    asm.emit(&[0x53, 0x51, 0x52, 0x56, 0x57, 0x1E]); // push bx,cx,dx,si,di,ds
    asm.emit(&[0x89, 0xCB, 0x81, 0xE3, 0xFF, 0x3F]); // mov bx,cx; and bx,3fffh
    for (index, mode) in modes.iter().enumerate() {
        asm.emit(&[0x81, 0xFB]); // cmp bx, imm16
        asm.emit(&mode.mode.to_le_bytes());
        asm.jump_if_equal(format!("copy_mode_{index}"));
    }
    asm.emit(&[0x1F, 0x5F, 0x5E, 0x5A, 0x59, 0x5B]);
    asm.jump("failure");
    for (index, _) in modes.iter().enumerate() {
        asm.label(format!("copy_mode_{index}"));
        let offset = VBE_ROM_MODE_INFO_OFFSET
            .checked_add(u16::try_from(index * 0x100).expect("VBE mode ROM offset"))
            .expect("VBE mode ROM offset");
        asm.emit(&[0xBE]); // mov si, imm16
        asm.emit(&offset.to_le_bytes());
        asm.jump("copy_mode");
    }
    asm.label("copy_mode");
    // push cs; pop ds; mov cx,128; cld; rep movsw
    asm.emit(&[0x0E, 0x1F, 0xB9, 0x80, 0x00, 0xFC, 0xF3, 0xA5]);
    // REP MOVSW advanced DI by 256 bytes. Rewind it to PhysBasePtr
    // (mode-info offset 40) and replace the immutable template value with the
    // final, post-PCI LFB address from Aero's EBDA state block.
    asm.emit(&[0xB8]);
    asm.emit(&VBE_EBDA_SEGMENT.to_le_bytes()); // mov ax, VBE_EBDA_SEGMENT
    asm.emit(&[0x8E, 0xD8]); // mov ds,ax
    asm.emit(&[0x81, 0xEF, 0xD8, 0x00]); // sub di,216
    asm.emit(&[0xA1]);
    asm.emit(&VBE_EBDA_LFB_BASE_OFFSET.to_le_bytes()); // mov ax,[lfb]
    asm.emit(&[0x26, 0x89, 0x05]); // mov es:[di],ax
    asm.emit(&[0xA1]);
    asm.emit(&(VBE_EBDA_LFB_BASE_OFFSET + 2).to_le_bytes()); // mov ax,[lfb+2]
    asm.emit(&[0x26, 0x89, 0x45, 0x02]); // mov es:[di+2],ax
    asm.emit(&[0x1F, 0x5F, 0x5E, 0x5A, 0x59, 0x5B]);
    asm.jump("success");

    asm.label("set_mode");
    asm.emit(&[0x53, 0x51, 0x52, 0x56, 0x57, 0x55, 0x1E]);
    // Preserve VBE mode flag bit 15 until the mode has been validated and all
    // resolution registers have been consumed. DISPI expresses the same
    // contract as bit 7 of ENABLE (NOCLEARMEM).
    asm.emit(&[0x89, 0xD8, 0x25, 0x00, 0x80, 0x50]); // mov ax,bx; and ax,8000h; push ax
    asm.emit(&[0x89, 0xDF, 0x81, 0xE7, 0xFF, 0x3F]); // mov di,bx; and di,3fffh
    for (index, mode) in modes.iter().enumerate() {
        asm.emit(&[0x81, 0xFF]); // cmp di, imm16
        asm.emit(&mode.mode.to_le_bytes());
        asm.jump_if_equal(format!("set_mode_{index}"));
    }
    asm.emit(&[0x58, 0x1F, 0x5D, 0x5F, 0x5E, 0x5A, 0x59, 0x5B]);
    asm.jump("failure");
    for (index, mode) in modes.iter().enumerate() {
        asm.label(format!("set_mode_{index}"));
        asm.emit(&[0xBE]); // mov si, width
        asm.emit(&mode.width.to_le_bytes());
        asm.emit(&[0xBD]); // mov bp, height
        asm.emit(&mode.height.to_le_bytes());
        asm.emit(&[0xB1, mode.bpp]); // mov cl, bpp
        asm.emit(&[0xBB]); // mov bx, pitch
        asm.emit(&mode.bytes_per_scan_line().to_le_bytes());
        asm.jump("program_mode");
    }
    asm.label("program_mode");
    emit_dispi_mode_programming(&mut asm);
    asm.emit(&[0xB8]);
    asm.emit(&VBE_EBDA_SEGMENT.to_le_bytes());
    asm.emit(&[0x8E, 0xD8]); // mov ds,VBE_EBDA_SEGMENT
    asm.emit(&[0x81, 0xCF, 0x00, 0x40]); // or di,4000h
    emit_store_ds_word_from_reg(&mut asm, VBE_EBDA_CURRENT_MODE_OFFSET, 0x3E); // di
    emit_store_ds_word_from_reg(&mut asm, VBE_EBDA_WIDTH_OFFSET, 0x36); // si
    emit_store_ds_word_from_reg(&mut asm, VBE_EBDA_PITCH_OFFSET, 0x1E); // bx
    emit_store_ds_word_from_reg(&mut asm, VBE_EBDA_VIRTUAL_WIDTH_OFFSET, 0x36); // si
    asm.emit(&[0x31, 0xC0, 0x88, 0xC8, 0xA3]); // xor ax,ax; mov al,cl; mov [bpp],ax
    asm.emit(&VBE_EBDA_BPP_OFFSET.to_le_bytes());
    asm.emit(&[0xB8, 0x06, 0x00, 0xA3]); // mov ax,6; mov [dac_width],ax
    asm.emit(&VBE_EBDA_DAC_WIDTH_OFFSET.to_le_bytes());
    asm.emit(&[0x31, 0xC0, 0xA3]); // xor ax,ax; mov [x_offset],ax
    asm.emit(&VBE_EBDA_X_OFFSET.to_le_bytes());
    asm.emit(&[0xA3]); // mov [y_offset],ax
    asm.emit(&VBE_EBDA_Y_OFFSET.to_le_bytes());
    asm.emit(&[0xA3]); // mov [bank],ax
    asm.emit(&VBE_EBDA_BANK_OFFSET.to_le_bytes());
    asm.emit(&[0xB8, 0x40, 0x00, 0x8E, 0xD8]); // mov ax,40h; mov ds,ax
    asm.emit(&[0xC6, 0x06, 0x49, 0x00, 0x6F]); // BDA video mode = VESA active
    asm.emit(&[0x1F, 0x5D, 0x5F, 0x5E, 0x5A, 0x59, 0x5B]);
    asm.jump("success");

    asm.label("get_mode");
    asm.emit(&[0x1E, 0xB8]);
    asm.emit(&VBE_EBDA_SEGMENT.to_le_bytes());
    asm.emit(&[0x8E, 0xD8, 0x8B, 0x1E]); // mov ds,ax; mov bx,[current_mode]
    asm.emit(&VBE_EBDA_CURRENT_MODE_OFFSET.to_le_bytes());
    asm.emit(&[0x1F]);
    asm.jump("success");

    asm.label("window_control");
    asm.emit(&[0x56, 0x1E]); // push si,ds
    asm.emit(&[0x80, 0xFF, 0x00]); // cmp bh,0 (only window A exists)
    asm.jump_if_equal("window_control_decode");
    asm.jump("window_control_failure");

    asm.label("window_control_decode");
    asm.emit(&[0x89, 0xDE, 0x81, 0xE6, 0xFF, 0x00]); // mov si,bx; and si,00ffh
    asm.emit(&[0xB8]);
    asm.emit(&VBE_EBDA_SEGMENT.to_le_bytes());
    asm.emit(&[0x8E, 0xD8]); // mov ds,ax
    asm.emit(&[0x81, 0xFE, 0x00, 0x00]); // cmp si,0 (set)
    asm.jump_if_equal("window_control_set");
    asm.emit(&[0x81, 0xFE, 0x01, 0x00]); // cmp si,1 (get)
    asm.jump_if_equal("window_control_get");
    asm.jump("window_control_failure_with_ds");

    asm.label("window_control_set");
    emit_store_ds_word_from_reg(&mut asm, VBE_EBDA_BANK_OFFSET, 0x16); // dx
    asm.emit(&[0x89, 0xD6]); // mov si,dx
                             // VBE_DISPI_INDEX_BANK = si.
    asm.emit(&[
        0xBA, 0xCE, 0x01, 0xB8, 0x05, 0x00, 0xEF, 0x42, 0x89, 0xF0, 0xEF,
    ]);
    asm.jump("window_control_success");

    asm.label("window_control_get");
    asm.emit(&[0x8B, 0x16]); // mov dx,[bank]
    asm.emit(&VBE_EBDA_BANK_OFFSET.to_le_bytes());

    asm.label("window_control_success");
    asm.emit(&[0x1F, 0x5E]); // pop ds,si
    asm.jump("success");

    asm.label("window_control_failure_with_ds");
    asm.emit(&[0x1F]); // pop ds
    asm.emit(&[0x5E]); // pop si
    asm.jump("failure");

    asm.label("window_control_failure");
    asm.emit(&[0x1F, 0x5E]); // pop ds,si
    asm.jump("failure");

    asm.label("scan_line");
    asm.emit(&[0x56, 0x57, 0x1E]); // push si,di,ds
    asm.emit(&[0x89, 0xDE, 0x81, 0xE6, 0xFF, 0x00]); // mov si,bx; and si,00ffh
    asm.emit(&[0xB8]);
    asm.emit(&VBE_EBDA_SEGMENT.to_le_bytes());
    asm.emit(&[0x8E, 0xD8]); // mov ds,ax
    asm.emit(&[0x8B, 0x3E]); // mov di,[bpp]
    asm.emit(&VBE_EBDA_BPP_OFFSET.to_le_bytes());
    // Convert bits per pixel to bytes per pixel. All advertised direct-color modes are byte
    // aligned, so three 8086-compatible single-bit shifts avoid relying on an 80186 immediate
    // shift instruction in guest ROM interpreters.
    asm.emit(&[0xD1, 0xEF, 0xD1, 0xEF, 0xD1, 0xEF]); // shr di,1 (x3)
    asm.emit(&[0x09, 0xFF]); // or di,di
    asm.jump_if_equal("scan_line_failure");

    asm.emit(&[0x81, 0xFE, 0x01, 0x00]); // cmp si,1 (get)
    asm.jump_if_equal("scan_line_report");
    asm.emit(&[0x81, 0xFE, 0x03, 0x00]); // cmp si,3 (get maximum)
    asm.jump_if_equal("scan_line_report");
    asm.emit(&[0x81, 0xFE, 0x00, 0x00]); // cmp si,0 (set pixels)
    asm.jump_if_equal("scan_line_set_pixels");
    asm.emit(&[0x81, 0xFE, 0x02, 0x00]); // cmp si,2 (set bytes)
    asm.jump_if_equal("scan_line_set_bytes");
    asm.jump("scan_line_failure");

    asm.label("scan_line_set_pixels");
    asm.emit(&[0x89, 0xCE]); // mov si,cx
                             // Clamp the requested pixel count so `pixels * bytes_per_pixel` remains representable in the
                             // VBE 16-bit BytesPerScanLine result.
    asm.emit(&[0xB8, 0xFF, 0xFF, 0x31, 0xD2, 0xF7, 0xF7]); // ax=ffffh; xor dx,dx; div di
    asm.emit(&[0x39, 0xC6]); // cmp si,ax
    asm.jump_if_below_or_equal("scan_line_clamp_minimum");
    asm.emit(&[0x89, 0xC6]); // mov si,ax
    asm.jump("scan_line_clamp_minimum");

    asm.label("scan_line_set_bytes");
    // Bochs/QEMU represents line length as VIRT_WIDTH pixels. Match SeaBIOS by rounding a
    // byte-granular request down to the nearest whole pixel.
    asm.emit(&[0x89, 0xC8, 0x31, 0xD2, 0xF7, 0xF7, 0x89, 0xC6]); // ax=cx; dx=0; div di; si=ax

    asm.label("scan_line_clamp_minimum");
    asm.emit(&[0x3B, 0x36]); // cmp si,[physical_width]
    asm.emit(&VBE_EBDA_WIDTH_OFFSET.to_le_bytes());
    asm.jump_if_above_or_equal("scan_line_program");
    asm.emit(&[0x8B, 0x36]); // mov si,[physical_width]
    asm.emit(&VBE_EBDA_WIDTH_OFFSET.to_le_bytes());

    asm.label("scan_line_program");
    asm.emit(&[0x89, 0xF0, 0xF7, 0xE7]); // mov ax,si; mul di
    asm.emit(&[0xA3]); // mov [pitch],ax
    asm.emit(&VBE_EBDA_PITCH_OFFSET.to_le_bytes());
    emit_store_ds_word_from_reg(&mut asm, VBE_EBDA_VIRTUAL_WIDTH_OFFSET, 0x36); // si

    // VBE_DISPI_INDEX_VIRT_WIDTH = si.
    asm.emit(&[
        0xBA, 0xCE, 0x01, 0xB8, 0x06, 0x00, 0xEF, 0x42, 0x89, 0xF0, 0xEF,
    ]);

    asm.label("scan_line_report");
    asm.emit(&[0x8B, 0x1E]); // mov bx,[pitch]
    asm.emit(&VBE_EBDA_PITCH_OFFSET.to_le_bytes());
    asm.emit(&[0x8B, 0x0E]); // mov cx,[virtual_width]
    asm.emit(&VBE_EBDA_VIRTUAL_WIDTH_OFFSET.to_le_bytes());
    asm.emit(&[0xBA, 0xFF, 0xFF]); // conservative maximum scan-line count
    asm.emit(&[0x1F, 0x5F, 0x5E]);
    asm.jump("success");

    asm.label("scan_line_failure");
    asm.emit(&[0x1F, 0x5F, 0x5E]);
    asm.jump("failure");

    asm.label("display_start");
    asm.emit(&[0x56, 0x57, 0x1E]); // push si,di,ds
    asm.emit(&[0x89, 0xDE, 0x81, 0xE6, 0x7F, 0x00]); // mov si,bx; and si,007fh
    asm.emit(&[0xB8]);
    asm.emit(&VBE_EBDA_SEGMENT.to_le_bytes());
    asm.emit(&[0x8E, 0xD8]); // mov ds,ax
    asm.emit(&[0x81, 0xFE, 0x00, 0x00]); // cmp si,0 (set)
    asm.jump_if_equal("display_start_set");
    asm.emit(&[0x81, 0xFE, 0x01, 0x00]); // cmp si,1 (get)
    asm.jump_if_equal("display_start_get");
    asm.jump("display_start_failure");

    asm.label("display_start_set");
    asm.emit(&[0x89, 0xCE, 0x89, 0xD7]); // mov si,cx; mov di,dx
    emit_store_ds_word_from_reg(&mut asm, VBE_EBDA_X_OFFSET, 0x36); // si
    emit_store_ds_word_from_reg(&mut asm, VBE_EBDA_Y_OFFSET, 0x3E); // di
                                                                    // VBE_DISPI_INDEX_X_OFFSET = si.
    asm.emit(&[
        0xBA, 0xCE, 0x01, 0xB8, 0x08, 0x00, 0xEF, 0x42, 0x89, 0xF0, 0xEF,
    ]);
    // VBE_DISPI_INDEX_Y_OFFSET = di.
    asm.emit(&[
        0xBA, 0xCE, 0x01, 0xB8, 0x09, 0x00, 0xEF, 0x42, 0x89, 0xF8, 0xEF,
    ]);
    asm.jump("display_start_success");

    asm.label("display_start_get");
    asm.emit(&[0x8B, 0x0E]); // mov cx,[x_offset]
    asm.emit(&VBE_EBDA_X_OFFSET.to_le_bytes());
    asm.emit(&[0x8B, 0x16]); // mov dx,[y_offset]
    asm.emit(&VBE_EBDA_Y_OFFSET.to_le_bytes());

    asm.label("display_start_success");
    asm.emit(&[0x1F, 0x5F, 0x5E]);
    asm.jump("success");

    asm.label("display_start_failure");
    asm.emit(&[0x1F, 0x5F, 0x5E]);
    asm.jump("failure");

    asm.label("dac_format");
    asm.emit(&[0x56, 0x1E]); // push si,ds
    asm.emit(&[0x89, 0xDE, 0x81, 0xE6, 0xFF, 0x00]); // mov si,bx; and si,00ffh
    asm.emit(&[0xB8]);
    asm.emit(&VBE_EBDA_SEGMENT.to_le_bytes());
    asm.emit(&[0x8E, 0xD8]); // mov ds,ax
    asm.emit(&[0x81, 0xFE, 0x00, 0x00]); // cmp si,0 (set)
    asm.jump_if_equal("dac_format_set");
    asm.emit(&[0x81, 0xFE, 0x01, 0x00]); // cmp si,1 (get)
    asm.jump_if_equal("dac_format_get");
    asm.jump("dac_format_failure");

    asm.label("dac_format_set");
    asm.emit(&[0x88, 0xF8]); // mov al,bh
    asm.emit(&[0x3C, 0x06]); // cmp al,6
    asm.jump_if_equal("dac_format_store");
    asm.emit(&[0x3C, 0x08]); // cmp al,8
    asm.jump_if_equal("dac_format_store");
    asm.jump("dac_format_failure");

    asm.label("dac_format_store");
    asm.emit(&[0x30, 0xE4, 0xA3]); // xor ah,ah; mov [dac_width],ax
    asm.emit(&VBE_EBDA_DAC_WIDTH_OFFSET.to_le_bytes());

    asm.label("dac_format_get");
    asm.emit(&[0xA0]); // mov al,[dac_width]
    asm.emit(&VBE_EBDA_DAC_WIDTH_OFFSET.to_le_bytes());
    asm.emit(&[0x88, 0xC7]); // mov bh,al
    asm.emit(&[0x1F, 0x5E]); // pop ds,si
    asm.jump("success");

    asm.label("dac_format_failure");
    asm.emit(&[0x1F, 0x5E]); // pop ds,si
    asm.jump("failure");

    asm.label("palette_data");
    // Preserve all caller-visible operands; only AX is the VBE status result.
    asm.emit(&[0x53, 0x51, 0x52, 0x56, 0x57, 0x55, 0x1E]); // push bx,cx,dx,si,di,bp,ds
    asm.emit(&[0x81, 0xE3, 0x7F, 0x00]); // and bx,007fh (accept set-during-retrace)
    asm.emit(&[0x81, 0xFB, 0x00, 0x00]); // cmp bx,0 (set)
    asm.jump_if_equal("palette_validate");
    asm.emit(&[0x81, 0xFB, 0x01, 0x00]); // cmp bx,1 (get)
    asm.jump_if_equal("palette_validate");
    asm.jump("palette_failure");

    asm.label("palette_validate");
    asm.emit(&[0x80, 0xFE, 0x00]); // cmp dh,0
    asm.jump_if_equal("palette_validate_count");
    asm.jump("palette_failure");

    asm.label("palette_validate_count");
    asm.emit(&[0x89, 0xD6, 0x81, 0xE6, 0xFF, 0x00, 0x89, 0xCD]); // si=dx&ffh; bp=cx
    asm.emit(&[0x81, 0xFD, 0x00, 0x01]); // cmp bp,256
    asm.jump_if_below_or_equal("palette_validate_end");
    asm.jump("palette_failure");

    asm.label("palette_validate_end");
    asm.emit(&[0x89, 0xF0, 0x01, 0xE8, 0x3D, 0x00, 0x01]); // ax=si; add ax,bp; cmp ax,256
    asm.jump_if_below_or_equal("palette_load_dac_state");
    asm.jump("palette_failure");

    asm.label("palette_load_dac_state");
    asm.emit(&[0xB8]);
    asm.emit(&VBE_EBDA_SEGMENT.to_le_bytes());
    asm.emit(&[0x8E, 0xD8]); // mov ds,ax
    asm.emit(&[0x81, 0xFB, 0x00, 0x00]); // cmp bx,0
    asm.jump_if_equal("palette_set_begin");
    asm.jump("palette_get_begin");

    asm.label("palette_set_begin");
    asm.emit(&[0xBA, 0xC8, 0x03, 0x89, 0xF0, 0xEE, 0x42]); // dx=3c8h; ax=si; out dx,al; inc dx
    asm.label("palette_set_check");
    asm.emit(&[0x09, 0xED]); // or bp,bp
    asm.jump_if_equal("palette_success");

    // VBE palette entries are B,G,R,0. VGA DAC writes are R,G,B. When the guest selected the
    // VBE 8-bit DAC format, downscale to the emulated VGA DAC's native six bits before writing.
    asm.emit(&[0x26, 0x8A, 0x45, 0x02, 0x80, 0x3E]); // mov al,es:[di+2]; cmp byte [dac_width],8
    asm.emit(&VBE_EBDA_DAC_WIDTH_OFFSET.to_le_bytes());
    asm.emit(&[0x08]);
    asm.jump_if_not_equal("palette_set_r_write");
    asm.emit(&[0xD0, 0xE8, 0xD0, 0xE8]); // shr al,1 (x2)
    asm.label("palette_set_r_write");
    asm.emit(&[0xEE]); // out dx,al

    asm.emit(&[0x26, 0x8A, 0x45, 0x01, 0x80, 0x3E]); // mov al,es:[di+1]; cmp byte [dac_width],8
    asm.emit(&VBE_EBDA_DAC_WIDTH_OFFSET.to_le_bytes());
    asm.emit(&[0x08]);
    asm.jump_if_not_equal("palette_set_g_write");
    asm.emit(&[0xD0, 0xE8, 0xD0, 0xE8]); // shr al,1 (x2)
    asm.label("palette_set_g_write");
    asm.emit(&[0xEE]); // out dx,al

    asm.emit(&[0x26, 0x8A, 0x05, 0x80, 0x3E]); // mov al,es:[di]; cmp byte [dac_width],8
    asm.emit(&VBE_EBDA_DAC_WIDTH_OFFSET.to_le_bytes());
    asm.emit(&[0x08]);
    asm.jump_if_not_equal("palette_set_b_write");
    asm.emit(&[0xD0, 0xE8, 0xD0, 0xE8]); // shr al,1 (x2)
    asm.label("palette_set_b_write");
    asm.emit(&[0xEE, 0x83, 0xC7, 0x04, 0x4D]); // out dx,al; add di,4; dec bp
    asm.jump("palette_set_check");

    asm.label("palette_get_begin");
    asm.emit(&[0xBA, 0xC7, 0x03, 0x89, 0xF0, 0xEE]); // dx=3c7h; ax=si; out dx,al
    asm.emit(&[0xBA, 0xC9, 0x03]); // mov dx,3c9h
    asm.label("palette_get_check");
    asm.emit(&[0x09, 0xED]); // or bp,bp
    asm.jump_if_equal("palette_success");
    // Read R,G,B and store B,G,R,0. Expand the emulated six-bit DAC values for VBE 8-bit mode.
    asm.emit(&[0xEC, 0x80, 0x3E]); // in al,dx; cmp byte [dac_width],8
    asm.emit(&VBE_EBDA_DAC_WIDTH_OFFSET.to_le_bytes());
    asm.emit(&[0x08]);
    asm.jump_if_not_equal("palette_get_r_store");
    asm.emit(&[0xD0, 0xE0, 0xD0, 0xE0]); // shl al,1 (x2)
    asm.label("palette_get_r_store");
    asm.emit(&[0x26, 0x88, 0x45, 0x02]);

    asm.emit(&[0xEC, 0x80, 0x3E]); // in al,dx; cmp byte [dac_width],8
    asm.emit(&VBE_EBDA_DAC_WIDTH_OFFSET.to_le_bytes());
    asm.emit(&[0x08]);
    asm.jump_if_not_equal("palette_get_g_store");
    asm.emit(&[0xD0, 0xE0, 0xD0, 0xE0]); // shl al,1 (x2)
    asm.label("palette_get_g_store");
    asm.emit(&[0x26, 0x88, 0x45, 0x01]);

    asm.emit(&[0xEC, 0x80, 0x3E]); // in al,dx; cmp byte [dac_width],8
    asm.emit(&VBE_EBDA_DAC_WIDTH_OFFSET.to_le_bytes());
    asm.emit(&[0x08]);
    asm.jump_if_not_equal("palette_get_b_store");
    asm.emit(&[0xD0, 0xE0, 0xD0, 0xE0]); // shl al,1 (x2)
    asm.label("palette_get_b_store");
    asm.emit(&[0x26, 0x88, 0x05, 0x30, 0xC0, 0x26, 0x88, 0x45, 0x03]); // store B; zero/store reserved
    asm.emit(&[0x83, 0xC7, 0x04, 0x4D]); // add di,4; dec bp
    asm.jump("palette_get_check");

    asm.label("palette_success");
    asm.emit(&[0x1F, 0x5D, 0x5F, 0x5E, 0x5A, 0x59, 0x5B]); // pop ds,bp,di,si,dx,cx,bx
    asm.jump("success");

    asm.label("palette_failure");
    asm.emit(&[0x1F, 0x5D, 0x5F, 0x5E, 0x5A, 0x59, 0x5B]); // pop ds,bp,di,si,dx,cx,bx
    asm.jump("failure");

    asm.label("dpms");
    asm.jump("success");

    asm.label("ddc");
    asm.emit(&[0x80, 0xFB, 0x00]); // cmp bl,0
    asm.jump_if_equal("ddc_capabilities");
    asm.emit(&[0x80, 0xFB, 0x01]); // cmp bl,1
    asm.jump_if_not_equal("failure");
    asm.emit(&[0x09, 0xD2]); // or dx,dx
    asm.jump_if_not_equal("failure");
    asm.emit(&[0x51, 0x56, 0x57, 0x1E]); // push cx,si,di,ds
    asm.emit(&[0x0E, 0x1F]); // push cs; pop ds
    asm.emit(&[0xBE]);
    asm.emit(&VBE_ROM_EDID_OFFSET.to_le_bytes()); // mov si,EDID
    asm.emit(&[0xB9, 0x40, 0x00, 0xFC, 0xF3, 0xA5]); // mov cx,64; cld; rep movsw
    asm.emit(&[0x1F, 0x5F, 0x5E, 0x59]); // pop ds,di,si,cx
    asm.jump("success");

    asm.label("ddc_capabilities");
    asm.emit(&[0xBB, 0x00, 0x02]); // mov bx,0200h: DDC2 + EDID
    asm.jump("success");

    asm.label("failure");
    asm.emit(&[0xB8, 0x4F, 0x01, 0xF9, 0xCF]); // ax=014fh; stc; iret
    asm.label("success");
    asm.emit(&[0xB8, 0x4F, 0x00, 0xF8, 0xCF]); // ax=004fh; clc; iret

    let code = asm.finish();
    write_stub(rom, VBE_ROM_CODE_OFFSET, &code);

    let int10_entry = [
        0x80, 0xFC, 0x4F, // cmp ah,4fh
        0x75, 0x03, // jne HLT/IRET
        0xE9, // jmp VBE_ROM_CODE_OFFSET
        0, 0, 0xF4, 0xCF, // HLT; IRET for host-HLE non-VBE services
    ];
    let mut int10_entry = int10_entry;
    let jump_next = int10_stub_offset.wrapping_add(8);
    let displacement = VBE_ROM_CODE_OFFSET.wrapping_sub(jump_next);
    int10_entry[6..8].copy_from_slice(&displacement.to_le_bytes());
    write_stub(rom, int10_stub_offset, &int10_entry);

    write_stub(rom, VBE_ROM_OEM_STRING_OFFSET, b"Aero VBE BIOS\0");
    write_stub(rom, VBE_ROM_VENDOR_STRING_OFFSET, b"Aero\0");
    write_stub(rom, VBE_ROM_PRODUCT_STRING_OFFSET, b"Aero SVGA\0");
    write_stub(rom, VBE_ROM_REV_STRING_OFFSET, b"0.1\0");

    let mut mode_list = Vec::with_capacity((modes.len() + 1) * 2);
    for mode in &modes {
        mode_list.extend_from_slice(&mode.mode.to_le_bytes());
    }
    mode_list.extend_from_slice(&0xFFFFu16.to_le_bytes());
    write_stub(rom, VBE_ROM_MODE_LIST_OFFSET, &mode_list);

    for (index, mode) in modes.iter().enumerate() {
        let block = vbe
            .mode_info_block(mode.mode)
            .expect("advertised VBE mode must have a mode-info block");
        let offset = usize::from(VBE_ROM_MODE_INFO_OFFSET) + index * block.len();
        let end = offset + block.len();
        assert!(
            end <= usize::from(VBE_ROM_EDID_OFFSET),
            "VBE mode-info blocks overlap the EDID"
        );
        rom[offset..end].copy_from_slice(&block);
    }

    let edid = aero_edid::read_edid(0).expect("base EDID must exist");
    let edid_offset = usize::from(VBE_ROM_EDID_OFFSET);
    let edid_end = edid_offset + edid.len();
    assert!(
        edid_end <= usize::from(VBE_ROM_DATA_END),
        "VBE EDID overlaps the VGA font"
    );
    rom[edid_offset..edid_end].copy_from_slice(&edid);
}

fn emit_store_es_di_word(asm: &mut RomAssembler, displacement: u8, value: u16) {
    if displacement == 0 {
        asm.emit(&[0x26, 0xC7, 0x05]);
    } else {
        asm.emit(&[0x26, 0xC7, 0x45, displacement]);
    }
    asm.emit(&value.to_le_bytes());
}

fn emit_store_es_di_far_ptr(asm: &mut RomAssembler, displacement: u8, offset: u16) {
    emit_store_es_di_word(asm, displacement, offset);
    emit_store_es_di_word(asm, displacement + 2, 0xF000);
}

fn emit_store_ds_word_from_reg(asm: &mut RomAssembler, offset: u16, direct_modrm: u8) {
    asm.emit(&[0x89, direct_modrm]);
    asm.emit(&offset.to_le_bytes());
}

pub(super) fn write_vbe_lfb_runtime_state(
    memory: &mut (impl crate::memory::MemoryBus + ?Sized),
    lfb_base: u32,
) {
    memory.write_u32(EBDA_BASE + u64::from(VBE_EBDA_LFB_BASE_OFFSET), lfb_base);
}

fn emit_dispi_mode_programming(asm: &mut RomAssembler) {
    asm.emit(&[0xBA, 0xCE, 0x01]); // mov dx,01ceh

    asm.emit(&[0xB8, 0x04, 0x00, 0xEF, 0x42, 0x31, 0xC0, 0xEF, 0x4A]); // disable

    asm.emit(&[
        0xB8, 0x03, 0x00, 0xEF, 0x42, 0x31, 0xC0, 0x88, 0xC8, 0xEF, 0x4A,
    ]); // bpp
    asm.emit(&[0xB8, 0x01, 0x00, 0xEF, 0x42, 0x89, 0xF0, 0xEF, 0x4A]); // xres
    asm.emit(&[0xB8, 0x02, 0x00, 0xEF, 0x42, 0x89, 0xE8, 0xEF, 0x4A]); // yres
    asm.emit(&[0xB8, 0x05, 0x00, 0xEF, 0x42, 0x31, 0xC0, 0xEF, 0x4A]); // bank zero
    asm.emit(&[0xB8, 0x04, 0x00, 0xEF, 0x42]); // select ENABLE
                                               // The caller left the original VBE NOCLEARMEM flag on the stack. YRES has
                                               // already been written, so BP is now available as scratch. Shift VBE bit
                                               // 15 down to DISPI ENABLE bit 7 and combine it with ENABLED|LFB_ENABLED.
    asm.emit(&[0x58, 0x89, 0xC5]); // pop ax; mov bp,ax
    for _ in 0..8 {
        asm.emit(&[0xD1, 0xED]); // shr bp,1
    }
    asm.emit(&[0xB8, 0x41, 0x00, 0x09, 0xE8, 0xEF]); // mov ax,41h; or ax,bp; out dx,ax
}

fn write_stub(rom: &mut [u8], offset: u16, stub: &[u8]) {
    let off = offset as usize;
    let end = off + stub.len();
    rom[off..end].copy_from_slice(stub);
}

fn build_font8x16_cp437() -> [u8; 256 * 16] {
    // Generate the 8x16 font by vertically scaling the 8x8 table (duplicate each scanline).
    //
    // VGA BIOS INT 10h AH=11h AL=30h returns a pointer to this 8x16 table, and DOS-era software
    // often indexes it using CP437 codepoints (including the box-drawing range).
    let font8 = build_font8x8_cp437();
    let mut font16 = [0u8; 256 * 16];

    for ch in 0u16..=0xFF {
        let base8 = usize::from(ch as u8) * 8;
        let base16 = usize::from(ch as u8) * 16;
        for row in 0usize..8 {
            let bits = font8[base8 + row];
            font16[base16 + row * 2] = bits;
            font16[base16 + row * 2 + 1] = bits;
        }
    }

    font16
}

fn build_font8x8_cp437() -> [u8; 256 * 8] {
    let mut font = [0u8; 256 * 8];

    // Preserve the original 0x00..=0x7F glyphs.
    for ch in 0u16..=0x7F {
        let glyph8 = BASIC_FONTS.get((ch as u8) as char).unwrap_or([0u8; 8]);
        let base = usize::from(ch as u8) * 8;
        font[base..base + 8].copy_from_slice(&glyph8);
    }

    set_glyph8(&mut font, 0xB0, glyph_shade_light()); // ░
    set_glyph8(&mut font, 0xB1, glyph_shade_medium()); // ▒
    set_glyph8(&mut font, 0xB2, glyph_shade_dark()); // ▓

    // Single-line box drawing.
    set_glyph8(&mut font, 0xB3, glyph_box_single(true, true, false, false)); // │
    set_glyph8(&mut font, 0xB4, glyph_box_single(true, true, true, false)); // ┤
    set_glyph8(&mut font, 0xBF, glyph_box_single(false, true, true, false)); // ┐
    set_glyph8(&mut font, 0xC0, glyph_box_single(true, false, false, true)); // └
    set_glyph8(&mut font, 0xC1, glyph_box_single(true, false, true, true)); // ┴
    set_glyph8(&mut font, 0xC2, glyph_box_single(false, true, true, true)); // ┬
    set_glyph8(&mut font, 0xC3, glyph_box_single(true, true, false, true)); // ├
    set_glyph8(&mut font, 0xC4, glyph_box_single(false, false, true, true)); // ─
    set_glyph8(&mut font, 0xC5, glyph_box_single(true, true, true, true)); // ┼
    set_glyph8(&mut font, 0xD9, glyph_box_single(true, false, true, false)); // ┘
    set_glyph8(&mut font, 0xDA, glyph_box_single(false, true, false, true)); // ┌

    // Double-line/mixed box drawing (approximated).
    set_glyph8(&mut font, 0xB5, glyph_box_single(true, true, true, false)); // ╡
    set_glyph8(&mut font, 0xB6, glyph_box_single(true, true, true, false)); // ╢
    set_glyph8(&mut font, 0xB7, glyph_box_single(false, true, true, false)); // ╖
    set_glyph8(&mut font, 0xB8, glyph_box_single(false, true, true, false)); // ╕
    set_glyph8(&mut font, 0xB9, glyph_box_single(true, true, true, false)); // ╣
    set_glyph8(&mut font, 0xBA, glyph_box_double_vertical()); // ║
    set_glyph8(&mut font, 0xBB, glyph_box_single(false, true, true, false)); // ╗
    set_glyph8(&mut font, 0xBC, glyph_box_single(true, false, true, false)); // ╝
    set_glyph8(&mut font, 0xBD, glyph_box_single(true, false, true, false)); // ╜
    set_glyph8(&mut font, 0xBE, glyph_box_single(true, false, true, false)); // ╛

    set_glyph8(&mut font, 0xC6, glyph_box_single(true, true, false, true)); // ╞
    set_glyph8(&mut font, 0xC7, glyph_box_single(true, true, false, true)); // ╟
    set_glyph8(&mut font, 0xC8, glyph_box_single(true, false, false, true)); // ╚
    set_glyph8(&mut font, 0xC9, glyph_box_single(false, true, false, true)); // ╔
    set_glyph8(&mut font, 0xCA, glyph_box_single(true, false, true, true)); // ╩
    set_glyph8(&mut font, 0xCB, glyph_box_single(false, true, true, true)); // ╦
    set_glyph8(&mut font, 0xCC, glyph_box_single(true, true, false, true)); // ╠
    set_glyph8(&mut font, 0xCD, glyph_box_single(false, false, true, true)); // ═
    set_glyph8(&mut font, 0xCE, glyph_box_single(true, true, true, true)); // ╬
    set_glyph8(&mut font, 0xCF, glyph_box_single(true, false, true, true)); // ╧
    set_glyph8(&mut font, 0xD0, glyph_box_single(true, false, true, true)); // ╨
    set_glyph8(&mut font, 0xD1, glyph_box_single(false, true, true, true)); // ╤
    set_glyph8(&mut font, 0xD2, glyph_box_single(false, true, true, true)); // ╥
    set_glyph8(&mut font, 0xD3, glyph_box_single(true, false, false, true)); // ╙
    set_glyph8(&mut font, 0xD4, glyph_box_single(true, false, false, true)); // ╘
    set_glyph8(&mut font, 0xD5, glyph_box_single(false, true, false, true)); // ╒
    set_glyph8(&mut font, 0xD6, glyph_box_single(false, true, false, true)); // ╓
    set_glyph8(&mut font, 0xD7, glyph_box_single(true, true, true, true)); // ╫
    set_glyph8(&mut font, 0xD8, glyph_box_single(true, true, true, true)); // ╪

    // Block elements.
    set_glyph8(&mut font, 0xDB, [0xFF; 8]); // █
    set_glyph8(&mut font, 0xDC, glyph_block_lower_half()); // ▄
    set_glyph8(&mut font, 0xDD, glyph_block_left_half()); // ▌
    set_glyph8(&mut font, 0xDE, glyph_block_right_half()); // ▐
    set_glyph8(&mut font, 0xDF, glyph_block_upper_half()); // ▀

    font
}

fn set_glyph8(font: &mut [u8; 256 * 8], ch: u8, glyph: [u8; 8]) {
    let base = usize::from(ch) * 8;
    font[base..base + 8].copy_from_slice(&glyph);
}

fn glyph_box_single(up: bool, down: bool, left: bool, right: bool) -> [u8; 8] {
    let mut out = [0u8; 8];
    let h_row = 3usize;
    let v_col = 3usize;
    let v_bit = 1u8 << (7 - v_col);
    let left_mask = 0xFFu8 << (7 - v_col);
    let right_mask = 0xFFu8 >> v_col;

    for (y, out_row) in out.iter_mut().enumerate() {
        let mut row = 0u8;
        if (up && y <= h_row) || (down && y >= h_row) {
            row |= v_bit;
        }
        if y == h_row {
            if left {
                row |= left_mask;
            }
            if right {
                row |= right_mask;
            }
        }
        *out_row = row;
    }

    out
}

fn glyph_box_double_vertical() -> [u8; 8] {
    let mut out = [0u8; 8];
    let v_bits = 0x18u8; // bits 4 and 3
    out.fill(v_bits);
    out
}

fn glyph_block_upper_half() -> [u8; 8] {
    let mut out = [0u8; 8];
    out[..4].fill(0xFF);
    out
}

fn glyph_block_lower_half() -> [u8; 8] {
    let mut out = [0u8; 8];
    out[4..].fill(0xFF);
    out
}

fn glyph_block_left_half() -> [u8; 8] {
    let mut out = [0u8; 8];
    out.fill(0xF0); // left 4 pixels
    out
}

fn glyph_block_right_half() -> [u8; 8] {
    let mut out = [0u8; 8];
    out.fill(0x0F); // right 4 pixels
    out
}

fn glyph_shade_light() -> [u8; 8] {
    let mut out = [0u8; 8];
    for (y, row) in out.iter_mut().enumerate() {
        *row = if (y & 1) == 0 { 0x00 } else { 0x55 };
    }
    out
}

fn glyph_shade_medium() -> [u8; 8] {
    let mut out = [0u8; 8];
    for (y, row) in out.iter_mut().enumerate() {
        *row = if (y & 1) == 0 { 0x55 } else { 0xAA };
    }
    out
}

fn glyph_shade_dark() -> [u8; 8] {
    let mut out = [0u8; 8];
    for (y, row) in out.iter_mut().enumerate() {
        *row = if (y & 1) == 0 { 0xFF } else { 0xAA };
    }
    out
}
