; Scenarios for tests/halt.rs: one HALT per interrupt state worth telling apart. Each scenario
; sets up IE, IF and the interrupt master enable itself, so the tests only set PC; the bytes
; each instruction assembles to are listed beside it.
;
; Rebuild halt.gb with ./build.sh after changing this file.

; Where the scenarios that take a VBlank interrupt land.
SECTION "vblank", ROM0[$0040]
VBlank:
    jr VBlank           ; $0040: 18 FE

SECTION "entry", ROM0[$0100]
    nop                 ; $0100: 00
Done:
    jr Done             ; $0101: 18 FE

SECTION "header", ROM0[$0104]
    ds $4C, $00         ; filled in by rgbfix

; Interrupts disabled and VBlank pending: the HALT bug. `inc b` runs twice, so B ends up as 2.
SECTION "bug", ROM0[$0200]
Bug:
    di                  ; $0200: F3
    ld b, $00           ; $0201: 06 00
    ld a, $01           ; $0203: 3E 01
    ldh [$FFFF], a      ; $0205: E0 FF
    ldh [$FF0F], a      ; $0207: E0 0F
    halt                ; $0209: 76
    inc b               ; $020A: 04
.done:
    jr .done            ; $020B: 18 FE

; Nothing enabled in IE, interrupts disabled: the CPU never wakes.
SECTION "hang", ROM0[$0220]
Hang:
    di                  ; $0220: F3
    xor a               ; $0221: AF
    ldh [$FFFF], a      ; $0222: E0 FF
    halt                ; $0224: 76
.done:
    jr .done            ; $0225: 18 FE

; Nothing enabled in IE, interrupts enabled: the CPU still never wakes.
SECTION "hang ime", ROM0[$0230]
HangIme:
    xor a               ; $0230: AF
    ldh [$FFFF], a      ; $0231: E0 FF
    ei                  ; $0233: FB
    nop                 ; $0234: 00
    halt                ; $0235: 76
.done:
    jr .done            ; $0236: 18 FE

; Only the bits of IE that have no interrupt behind them are set, and all of IF.
SECTION "hang unused bits", ROM0[$0240]
HangUnusedBits:
    di                  ; $0240: F3
    ld a, $E0           ; $0241: 3E E0
    ldh [$FFFF], a      ; $0243: E0 FF
    ld a, $FF           ; $0245: 3E FF
    ldh [$FF0F], a      ; $0247: E0 0F
    halt                ; $0249: 76
.done:
    jr .done            ; $024A: 18 FE

; Interrupts enabled (by the `ei` right before) and VBlank pending: the interrupt is taken.
SECTION "dispatch", ROM0[$0250]
Dispatch:
    ld a, $01           ; $0250: 3E 01
    ldh [$FFFF], a      ; $0252: E0 FF
    ldh [$FF0F], a      ; $0254: E0 0F
    ei                  ; $0256: FB
    halt                ; $0257: 76
.done:
    jr .done            ; $0258: 18 FE

; Interrupts disabled and nothing pending: the CPU halts until VBlank, then carries on
; without taking the interrupt.
SECTION "wait", ROM0[$0260]
Wait:
    di                  ; $0260: F3
    ld a, $01           ; $0261: 3E 01
    ldh [$FFFF], a      ; $0263: E0 FF
    xor a               ; $0265: AF
    ldh [$FF0F], a      ; $0266: E0 0F
    halt                ; $0268: 76
.done:
    jr .done            ; $0269: 18 FE

; Interrupts enabled and nothing pending: the CPU halts until VBlank, then takes it.
SECTION "wait ime", ROM0[$0270]
WaitIme:
    ld a, $01           ; $0270: 3E 01
    ldh [$FFFF], a      ; $0272: E0 FF
    xor a               ; $0274: AF
    ldh [$FF0F], a      ; $0275: E0 0F
    ei                  ; $0277: FB
    nop                 ; $0278: 00
    halt                ; $0279: 76
.done:
    jr .done            ; $027A: 18 FE

; Interrupts disabled, and the only pending interrupt (STAT) is not the enabled one (VBlank):
; an ordinary wait.
SECTION "masked", ROM0[$0280]
Masked:
    di                  ; $0280: F3
    ld a, $01           ; $0281: 3E 01
    ldh [$FFFF], a      ; $0283: E0 FF
    ld a, $02           ; $0285: 3E 02
    ldh [$FF0F], a      ; $0287: E0 0F
    halt                ; $0289: 76
.done:
    jr .done            ; $028A: 18 FE
