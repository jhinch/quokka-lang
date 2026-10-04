; Scenarios for tests/ppu.rs: one access to [HL] per PPU state worth telling apart. Each
; scenario waits for the state itself by polling STAT, so the tests only set HL (the VRAM or
; OAM address to access), B (for the reads) and PC; the address of each access and of the loop
; a scenario ends in are listed beside them.
;
; Rebuild ppu.gb with ./build.sh after changing this file.

; Spins until the PPU mode in STAT is \1. Waiting for the mode before the wanted one first
; means the wanted one has only just begun.
MACRO wait_mode
.wait\@:
    ldh a, [$FF41]
    and $03
    cp \1
    jr nz, .wait\@
ENDM

SECTION "entry", ROM0[$0100]
    nop                 ; $0100: 00
Done:
    jr Done             ; $0101: 18 FE

SECTION "header", ROM0[$0104]
    ds $4C, $00         ; filled in by rgbfix

; Drawing (mode 3): VRAM and OAM are both blocked.
SECTION "write drawing", ROM0[$0200]
WriteDrawing:
    wait_mode 2
    wait_mode 3
    ld [hl], $42        ; $0210: 36 42
.done:
    jr .done            ; $0212: 18 FE

SECTION "read drawing", ROM0[$0220]
ReadDrawing:
    wait_mode 2
    wait_mode 3
    ld b, [hl]          ; $0230: 46
.done:
    jr .done            ; $0231: 18 FE

; OAM scan (mode 2): OAM is blocked, VRAM is not.
SECTION "write oam scan", ROM0[$0240]
WriteOamScan:
    wait_mode 0
    wait_mode 2
    ld [hl], $42        ; $0250: 36 42
.done:
    jr .done            ; $0252: 18 FE

SECTION "read oam scan", ROM0[$0260]
ReadOamScan:
    wait_mode 0
    wait_mode 2
    ld b, [hl]          ; $0270: 46
.done:
    jr .done            ; $0271: 18 FE

; HBlank (mode 0): nothing is blocked.
SECTION "write hblank", ROM0[$0280]
WriteHBlank:
    wait_mode 3
    wait_mode 0
    ld [hl], $42        ; $0290: 36 42
.done:
    jr .done            ; $0292: 18 FE

SECTION "read hblank", ROM0[$02A0]
ReadHBlank:
    wait_mode 3
    wait_mode 0
    ld b, [hl]          ; $02B0: 46
.done:
    jr .done            ; $02B1: 18 FE

; VBlank (mode 1): nothing is blocked.
SECTION "write vblank", ROM0[$02C0]
WriteVBlank:
    wait_mode 1
    ld [hl], $42        ; $02C8: 36 42
.done:
    jr .done            ; $02CA: 18 FE

SECTION "read vblank", ROM0[$02D0]
ReadVBlank:
    wait_mode 1
    ld b, [hl]          ; $02D8: 46
.done:
    jr .done            ; $02D9: 18 FE

; LCD off, then long enough for a frame to have gone by: nothing is ever blocked.
SECTION "write lcd off", ROM0[$02E0]
WriteLcdOff:
    wait_mode 1
    xor a               ; $02E8: AF
    ldh [$FF40], a      ; $02E9: E0 40
    ld c, $00           ; $02EB: 0E 00
.spin:
    dec c               ; $02ED: 0D
    jr nz, .spin        ; $02EE: 20 FD
    ld [hl], $42        ; $02F0: 36 42
.done:
    jr .done            ; $02F2: 18 FE

SECTION "read lcd off", ROM0[$0300]
ReadLcdOff:
    wait_mode 1
    xor a               ; $0308: AF
    ldh [$FF40], a      ; $0309: E0 40
    ld c, $00           ; $030B: 0E 00
.spin:
    dec c               ; $030D: 0D
    jr nz, .spin        ; $030E: 20 FD
    ld b, [hl]          ; $0310: 46
.done:
    jr .done            ; $0311: 18 FE
