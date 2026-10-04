//! Detects CPU accesses to VRAM and OAM that the PPU blocks.

use super::{Checker, Context, ReadKind};

/// Reports every CPU access to VRAM (`$8000-$9FFF`) or OAM (`$FE00-$FE9F`) made while the PPU
/// has the memory to itself: the read returns `$FF`, or the write is dropped.
///
/// Whether an access is blocked is the emulator's own verdict for the model it is emulating
/// and the moment of the access, so a program that only gets through because of when it
/// happens to run is not reported. Nothing is blocked while the LCD is off.
///
/// The violation kinds are:
///
/// * `vram-read` / `vram-write`: VRAM accessed while the PPU is drawing (mode 3).
/// * `oam-read` / `oam-write`: OAM accessed while the PPU is scanning it or drawing (modes 2
///   and 3).
///
/// Instruction bytes fetched from VRAM or OAM count as reads.
#[derive(Debug, Default)]
pub struct PpuBlockedAccess;

impl PpuBlockedAccess {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

const STAT: u16 = 0xFF41;
/// The bits of `STAT` holding the PPU mode.
const MODE_MASK: u8 = 0x03;

/// The memory the PPU shares with the CPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Region {
    Vram,
    Oam,
}

impl Region {
    const fn of(addr: u16) -> Option<Self> {
        match addr {
            0x8000..=0x9FFF => Some(Region::Vram),
            0xFE00..=0xFE9F => Some(Region::Oam),
            _ => None,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Region::Vram => "VRAM",
            Region::Oam => "OAM",
        }
    }
}

/// What the PPU is blocking the CPU from doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Blocked {
    oam_read: bool,
    oam_write: bool,
    vram_read: bool,
    vram_write: bool,
}

impl Blocked {
    fn current(cx: &Context<'_>) -> Self {
        Blocked {
            oam_read: cx.oam_read_blocked(),
            oam_write: cx.oam_write_blocked(),
            vram_read: cx.vram_read_blocked(),
            vram_write: cx.vram_write_blocked(),
        }
    }

    /// Whether a read from `region` returns `$FF` instead of the memory's contents.
    const fn read(self, region: Region, cgb: bool) -> bool {
        match region {
            Region::Vram => self.vram_read,
            // before the CGB, OAM cannot be read whenever it cannot be written
            Region::Oam => self.oam_read || (self.oam_write && !cgb),
        }
    }

    /// Whether a write to `region` is dropped.
    const fn write(self, region: Region) -> bool {
        match region {
            Region::Vram => self.vram_write,
            Region::Oam => self.oam_write,
        }
    }
}

fn mode(cx: &Context<'_>) -> u8 {
    cx.peek(STAT) & MODE_MASK
}

impl Checker for PpuBlockedAccess {
    fn name(&self) -> &'static str {
        "ppu-blocked"
    }

    fn on_read(&mut self, cx: &mut Context<'_>, addr: u16, _value: u8, _kind: ReadKind) {
        let Some(region) = Region::of(addr) else {
            return;
        };
        if !Blocked::current(cx).read(region, cx.is_cgb()) {
            return;
        }
        let kind = match region {
            Region::Vram => "vram-read",
            Region::Oam => "oam-read",
        };
        let detail = format!(
            "read of {} while the PPU is using it (mode {}): returns $FF",
            region.name(),
            mode(cx)
        );
        cx.report(kind, Some(addr), detail);
    }

    fn on_write(&mut self, cx: &mut Context<'_>, addr: u16, value: u8) {
        let Some(region) = Region::of(addr) else {
            return;
        };
        if !Blocked::current(cx).write(region) {
            return;
        }
        let kind = match region {
            Region::Vram => "vram-write",
            Region::Oam => "oam-write",
        };
        let detail = format!(
            "write of ${value:02X} to {} while the PPU is using it (mode {}): dropped",
            region.name(),
            mode(cx)
        );
        cx.report(kind, Some(addr), detail);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_cover_vram_and_the_sprite_table_only() {
        assert_eq!(Region::of(0x7FFF), None);
        assert_eq!(Region::of(0x8000), Some(Region::Vram));
        assert_eq!(Region::of(0x9FFF), Some(Region::Vram));
        assert_eq!(Region::of(0xA000), None);
        assert_eq!(Region::of(0xFDFF), None);
        assert_eq!(Region::of(0xFE00), Some(Region::Oam));
        assert_eq!(Region::of(0xFE9F), Some(Region::Oam));
        assert_eq!(Region::of(0xFEA0), None);
    }

    #[test]
    fn oam_read_while_only_writes_are_blocked_depends_on_the_model() {
        let blocked = Blocked {
            oam_write: true,
            ..Blocked::default()
        };
        assert!(blocked.read(Region::Oam, false));
        assert!(!blocked.read(Region::Oam, true));
        assert!(blocked.write(Region::Oam));
        assert!(!blocked.read(Region::Vram, false));
        assert!(!blocked.write(Region::Vram));
    }

    #[test]
    fn each_flag_blocks_its_own_access() {
        let oam_read = Blocked {
            oam_read: true,
            ..Blocked::default()
        };
        assert!(oam_read.read(Region::Oam, true));
        assert!(!oam_read.write(Region::Oam));

        let vram_read = Blocked {
            vram_read: true,
            ..Blocked::default()
        };
        assert!(vram_read.read(Region::Vram, true));
        assert!(!vram_read.write(Region::Vram));
        assert!(!vram_read.read(Region::Oam, false));

        let vram_write = Blocked {
            vram_write: true,
            ..Blocked::default()
        };
        assert!(vram_write.write(Region::Vram));
        assert!(!vram_write.read(Region::Vram, false));
    }
}
