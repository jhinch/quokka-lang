; Scenarios for tests/oam.rs: one instruction (or short sequence) per way of triggering the
; OAM corruption bug. The tests set the registers and PC themselves, so each instruction only
; needs a fixed address; the bytes each one assembles to are listed beside it.
;
; Rebuild oam.gb with ./build.sh after changing this file.

; Where the interrupt taken in the "interrupt" scenario lands.
SECTION "vblank", ROM0[$0040]
VBlank:
    jr VBlank           ; $0040: 18 FE

SECTION "entry", ROM0[$0100]
    nop                 ; $0100: 00
Done:
    jr Done             ; $0101: 18 FE

SECTION "header", ROM0[$0104]
    ds $4C, $00         ; filled in by rgbfix

SECTION "inc dec", ROM0[$0200]
    inc bc              ; $0200: 03
    inc de              ; $0201: 13
    inc hl              ; $0202: 23
    inc sp              ; $0203: 33
    dec bc              ; $0204: 0B
    dec de              ; $0205: 1B
    dec hl              ; $0206: 2B
    dec sp              ; $0207: 3B

SECTION "hl step", ROM0[$0210]
    ld [hli], a         ; $0210: 22
    ld a, [hli]         ; $0211: 2A
    ld [hld], a         ; $0212: 32
    ld a, [hld]         ; $0213: 3A

SECTION "ld sp hl", ROM0[$0218]
    ld sp, hl           ; $0218: F9

SECTION "push", ROM0[$0220]
    push bc             ; $0220: C5
    call Target         ; $0221: CD 60 02
    call z, Target      ; $0224: CC 60 02
    call nc, Target     ; $0227: D4 60 02
    rst $38             ; $022A: FF

SECTION "pop", ROM0[$0230]
    pop bc              ; $0230: C1
    ret                 ; $0231: C9
    reti                ; $0232: D9
    ret z               ; $0233: C8
    ret nc              ; $0234: D0

; Takes a VBlank interrupt with whatever SP the test set.
SECTION "interrupt", ROM0[$0240]
Interrupt:
    ld a, $01           ; $0240: 3E 01
    ldh [$FFFF], a      ; $0242: E0 FF
    ldh [$FF0F], a      ; $0244: E0 0F
    ei                  ; $0246: FB
    nop                 ; $0247: 00
    nop                 ; $0248: 00
.done:
    jr .done            ; $0249: 18 FE

SECTION "target", ROM0[$0260]
Target:
    jr Target           ; $0260: 18 FE

; Puts `ld bc, $0000` at $FDFE (echo RAM, written through $DDFE) so that its last operand
; byte is fetched from $FE00.
SECTION "straddle", ROM0[$0270]
Straddle:
    ld a, $01           ; $0270: 3E 01
    ld [$DDFE], a       ; $0272: EA FE DD
    jp $FDFE            ; $0275: C3 FE FD

; Puts `jr -2` at $FDFE, so that PC is $FE00 when the jump is taken.
SECTION "jr", ROM0[$0280]
JrIntoOam:
    ld a, $18           ; $0280: 3E 18
    ld [$DDFE], a       ; $0282: EA FE DD
    ld a, $FE           ; $0285: 3E FE
    ld [$DDFF], a       ; $0287: EA FF DD
    jp $FDFE            ; $028A: C3 FE FD
