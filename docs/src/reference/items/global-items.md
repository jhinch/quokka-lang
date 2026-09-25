# Global items

```ebnf
GlobalItem  =   "global" , Identifier , ":" , TypeUse , ";"
            ;
```

```quokka
global data: u8;
```

A global represents a portion of memory which is read/write. Its value is not known as compile time and can only be
accessed at runtime. Quokka does not support initializing globals like it does for [local variables](../statements/lets.md)
or for [constant declarations](const-items.md). Values much be initialized at runtime manually within code or point to
specific memory mapped I/O addresses. 

## Using specific regions of memory

By default, when a global is declared, it will be allocated statically to the general purpose RAM (WRAM) where it can
fit. However, the `region` and `address` attributes can be used to access other regions of memory such as video RAM,
Object Attribute Memory (OAM), I/O registers, and High RAM.

```quokka
// P1/JOYP register - See https://gbdev.io/pandocs/Joypad_Input.html for details
#[address = 0xFF00]
global JOYPAD: u8;

#[region = oam]
global SPRITES: [Sprite; 40];

struct Sprite {
    y: u8,
    x: u8,
    tile_index: u8,
    flags: u8,
}
```

- `address` (u16) - The starting address of the global. Defaults to the compiler automatically choosing.
- `region` - Specifies the region the global is in. Must be one of the following values:
  - `auto` (default) - The compiler automatically places the data in WRAM0 or WRAMX memory
  - `wram0` - Working RAM
  - `wramx` - Working (banked) RAM
  - `vram` - Video RAM
  - `hram` - High RAM
  - `sram` - Save RAM
  - `oam` - Object Attribute Memory

## Overlapping addresses

Global declarations can overlap the same underlying memory. The default whether this is permitted depends on the region
of memory it points to. Working RAM, save RAM and High RAM do not permit overlaps by default, but I/O registers, Video
RAM and OAM do.